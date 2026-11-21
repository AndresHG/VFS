//! Shortcut files read without the Windows shell (VFS-18): the target path of a `.lnk`
//! Shell Link ([MS-SHLLINK]) and the address of a `.url` Internet Shortcut.
//!
//! Only the bytes of the shortcut file itself are looked at: the target is never opened,
//! stat-ed or resolved, so a target on an unplugged disk or a network share costs nothing
//! and triggers no network lookup (R3). At most [`MAX_READ`] bytes are read; every offset
//! and length is checked against them, so a malformed or hostile file yields `None`, never
//! a panic or an unbounded allocation (R9).

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Bytes read from a shortcut file at most. Real `.lnk` files are 0.5 to 4 KB (the
/// property-store block of a recent Explorer link is the largest part); `.url` files are
/// a few hundred bytes. A larger file is read only up to here and parsed from that prefix.
pub const MAX_READ: usize = 64 * 1024;

/// Characters of a target kept at most: the longest path Windows accepts (32,767 UTF-16
/// units) or a long web address. Anything longer is treated as malformed.
pub const MAX_TARGET: usize = 32_767;

/// What kind of link a file is, from its name (case-insensitive extension), or `None` for
/// an ordinary file.
pub fn link_file_kind(name: &str) -> Option<LinkFileKind> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    if ext.eq_ignore_ascii_case("lnk") {
        Some(LinkFileKind::Lnk)
    } else if ext.eq_ignore_ascii_case("url") {
        Some(LinkFileKind::Url)
    } else {
        None
    }
}

/// A shortcut file format read by this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkFileKind {
    /// Windows Shell Link (`.lnk`).
    Lnk,
    /// Internet Shortcut (`.url`).
    Url,
}

/// Reads the first [`MAX_READ`] bytes of `path` and returns its target: a path for a
/// `.lnk` (absolute when the link records one), the address for a `.url`. `Ok(None)` when
/// the file holds no usable target.
pub fn read_target(path: &Path, kind: LinkFileKind) -> std::io::Result<Option<String>> {
    let mut buf = Vec::with_capacity(4096);
    File::open(path)?
        .take(MAX_READ as u64)
        .read_to_end(&mut buf)?;
    Ok(match kind {
        LinkFileKind::Lnk => parse_lnk(&buf, path.parent().and_then(Path::to_str)),
        LinkFileKind::Url => parse_url(&buf),
    })
}

/// Where the symbolic link or junction at `path` points, read from the link's own reparse
/// data (`readlink`); the target is never opened or checked (VFS-18 R3). A relative target is
/// joined to the link's folder and its `..` resolved as text, so it can be shown and matched
/// against the index.
pub fn read_symlink(path: &Path) -> std::io::Result<Option<String>> {
    let target = std::fs::read_link(path)?;
    let target = if target.is_absolute() {
        target
    } else {
        let mut abs = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for c in target.components() {
            match c {
                std::path::Component::ParentDir => {
                    abs.pop();
                }
                std::path::Component::CurDir => {}
                c => abs.push(c),
            }
        }
        abs
    };
    // `\\?\C:\…` from a junction reads as `C:\…`, the form roots are stored in.
    Ok(dunce::simplified(&target).to_str().map(str::to_owned))
}

/// Links read per transaction by [`read_pending`].
const PASS_BATCH: usize = 200;

/// The deferred link pass (VFS-18 R2, D7): reads every pending link once, in id order,
/// [`PASS_BATCH`] per transaction, until none is pending or `stop` (shutdown, pause, new
/// work) says to yield. Only the link file itself is opened. Returns the number read.
pub(crate) fn read_pending(
    conn: &mut rusqlite::Connection,
    stop: &dyn Fn() -> bool,
) -> crate::Result<usize> {
    use crate::db::{self, ingest};
    use crate::types::LinkKind;
    let mut done = 0;
    loop {
        let batch = ingest::pending_links(conn, PASS_BATCH)?;
        if batch.is_empty() {
            return Ok(done);
        }
        let mut read = Vec::with_capacity(batch.len());
        for (id, kind) in batch {
            if stop() {
                break;
            }
            // A row whose path cannot be built is marked unreadable, so it never stalls the
            // pass on every later batch.
            let path = match db::file_abs_path(conn, id) {
                Ok(p) => p,
                Err(e) => {
                    log::debug!("link {id}: {e}");
                    read.push((id, Err(())));
                    continue;
                }
            };
            let target = match kind {
                LinkKind::Shortcut => read_target(&path, LinkFileKind::Lnk),
                LinkKind::Internet => read_target(&path, LinkFileKind::Url),
                LinkKind::Symlink => read_symlink(&path),
            };
            read.push((
                id,
                target.map_err(|e| log::debug!("link {}: {e}", path.display())),
            ));
        }
        let tx = conn.transaction()?;
        for (id, t) in &read {
            ingest::set_link_target(&tx, *id, t.as_ref().map(|o| o.as_deref()).map_err(|_| ()))?;
        }
        tx.commit()?;
        done += read.len();
        if stop() {
            return Ok(done);
        }
    }
}

// ------------------------------------------------------------------------------- .lnk

const HEADER_SIZE: usize = 0x4C;
const LINK_CLSID: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

// LinkFlags ([MS-SHLLINK] 2.1.1).
const HAS_ID_LIST: u32 = 1 << 0;
const HAS_LINK_INFO: u32 = 1 << 1;
const HAS_NAME: u32 = 1 << 2;
const HAS_RELATIVE_PATH: u32 = 1 << 3;
const HAS_WORKING_DIR: u32 = 1 << 4;
const HAS_ARGUMENTS: u32 = 1 << 5;
const HAS_ICON_LOCATION: u32 = 1 << 6;
const IS_UNICODE: u32 = 1 << 7;
const FORCE_NO_LINK_INFO: u32 = 1 << 8;
const PREFER_ENVIRONMENT_PATH: u32 = 1 << 25;

// LinkInfoFlags (2.3).
const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 1 << 0;
const COMMON_NETWORK_RELATIVE_LINK: u32 = 1 << 1;
// CommonNetworkRelativeLinkFlags (2.3.2).
const VALID_DEVICE: u32 = 1 << 0;

const ENVIRONMENT_BLOCK: u32 = 0xA000_0001;

/// Bounds-checked little-endian reads over the bytes read from the file.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b = self.0.get(at..at.checked_add(2)?)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b = self.0.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// NUL-terminated code-page string starting at `at`, within `end`.
    fn ansi_z(&self, at: usize, end: usize) -> Option<String> {
        let s = self.0.get(at..end.min(self.0.len()))?;
        let n = s.iter().position(|&b| b == 0)?;
        Some(cp1252(&s[..n]))
    }

    /// NUL-terminated UTF-16LE string starting at `at`, within `end`.
    fn utf16_z(&self, at: usize, end: usize) -> Option<String> {
        let s = self.0.get(at..end.min(self.0.len()))?;
        let units: Vec<u16> = utf16_units(s).take_while(|&u| u != 0).collect();
        // No terminator inside the bounds: malformed.
        if units.len() * 2 + 2 > s.len() {
            return None;
        }
        String::from_utf16(&units).ok()
    }
}

/// The UTF-16LE code units of `s` (a trailing odd byte is ignored).
fn utf16_units(s: &[u8]) -> impl Iterator<Item = u16> + '_ {
    s.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c))
}

/// Windows-1252 to Unicode: the ANSI code page of Spanish (and Western) Windows, which
/// writes the non-Unicode strings of a link. Undefined bytes become U+FFFD.
fn cp1252(b: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '€', '\u{FFFD}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{FFFD}', 'Ž',
        '\u{FFFD}', '\u{FFFD}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
        '\u{FFFD}', 'ž', 'Ÿ',
    ];
    b.iter()
        .map(|&c| match c {
            0x80..=0x9F => HIGH[(c - 0x80) as usize],
            _ => c as char,
        })
        .collect()
}

/// The target path of a Shell Link held in `b`, or `None` when the bytes are not a link or
/// record no file-system target (a Control Panel item, an installer-advertised shortcut).
/// `lnk_dir` is the folder of the link, used only to make a relative target absolute,
/// lexically.
///
/// Order, as the shell prefers it ([MS-SHLLINK] 2.3, 2.5, 2.6.3): the environment-variable
/// target when the link asks for it, else the LinkInfo local path (`LocalBasePath` +
/// `CommonPathSuffix`), else its network path (the mapped drive letter when recorded, else
/// the `\\server\share` name, + `CommonPathSuffix`), else the environment-variable target,
/// else the relative path.
pub fn parse_lnk(b: &[u8], lnk_dir: Option<&str>) -> Option<String> {
    let r = Bytes(b);
    if r.u32(0)? as usize != HEADER_SIZE || b.get(4..20)? != LINK_CLSID {
        return None;
    }
    let flags = r.u32(0x14)?;
    let mut at = HEADER_SIZE;
    if flags & HAS_ID_LIST != 0 {
        at = at.checked_add(2 + r.u16(at)? as usize)?;
    }
    let mut info_target = None;
    if flags & HAS_LINK_INFO != 0 {
        let size = r.u32(at)? as usize;
        if size < 0x1C {
            return None;
        }
        let end = at.checked_add(size)?;
        if end > b.len() {
            return None;
        }
        if flags & FORCE_NO_LINK_INFO == 0 {
            info_target = link_info(&r, at, end);
        }
        at = end;
    }
    let unicode = flags & IS_UNICODE != 0;
    let mut relative = None;
    for (bit, keep) in [
        (HAS_NAME, false),
        (HAS_RELATIVE_PATH, true),
        (HAS_WORKING_DIR, false),
        (HAS_ARGUMENTS, false),
        (HAS_ICON_LOCATION, false),
    ] {
        if flags & bit == 0 {
            continue;
        }
        let chars = r.u16(at)? as usize;
        let len = if unicode { chars * 2 } else { chars };
        let s = b.get(at + 2..at + 2 + len)?;
        if keep {
            relative = Some(if unicode {
                String::from_utf16(&utf16_units(s).collect::<Vec<_>>()).ok()?
            } else {
                cp1252(s)
            });
        }
        at += 2 + len;
    }
    let env = environment_target(&r, at);
    let prefer_env = flags & PREFER_ENVIRONMENT_PATH != 0;
    let target = match (prefer_env, env, info_target) {
        (true, Some(e), _) => Some(e),
        (_, e, Some(i)) => Some(i).or(e),
        (_, e, None) => e,
    }
    .or_else(|| relative.and_then(|rel| absolute(&rel, lnk_dir)));
    target.filter(|t| usable(t))
}

/// The target recorded in a LinkInfo structure spanning `start..end`.
fn link_info(r: &Bytes, start: usize, end: usize) -> Option<String> {
    let header = r.u32(start + 4)? as usize;
    let info_flags = r.u32(start + 8)?;
    let field = |n: usize| -> Option<usize> { Some(start + r.u32(start + n)? as usize) };
    let unicode = header >= 0x24;
    let suffix = if unicode {
        r.utf16_z(field(0x20)?, end)
    } else {
        r.ansi_z(field(0x18)?, end)
    }
    .unwrap_or_default();
    if info_flags & VOLUME_ID_AND_LOCAL_BASE_PATH != 0 {
        let base = if unicode && r.u32(start + 0x1C)? != 0 {
            r.utf16_z(field(0x1C)?, end)?
        } else {
            r.ansi_z(field(0x10)?, end)?
        };
        if !base.is_empty() {
            return Some(join(&base, &suffix));
        }
    }
    if info_flags & COMMON_NETWORK_RELATIVE_LINK != 0 {
        let net = field(0x14)?;
        let net_flags = r.u32(net + 4)?;
        let name_at = r.u32(net + 8)? as usize;
        let device_at = r.u32(net + 12)? as usize;
        let wide = name_at > 0x14;
        let text = |ansi: usize, wide_field: usize| -> Option<String> {
            if wide {
                r.utf16_z(net + r.u32(net + wide_field)? as usize, end)
            } else {
                r.ansi_z(net + ansi, end)
            }
        };
        let device = (net_flags & VALID_DEVICE != 0)
            .then(|| text(device_at, 0x18))
            .flatten()
            .filter(|d| !d.is_empty());
        let root = match device {
            Some(d) => d,
            None => text(name_at, 0x14)?,
        };
        if !root.is_empty() {
            return Some(join(&root, &suffix));
        }
    }
    None
}

/// `base` and `suffix` joined with exactly one backslash between them.
fn join(base: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return base.to_owned();
    }
    let base = base.trim_end_matches('\\');
    let suffix = suffix.trim_start_matches('\\');
    format!("{base}\\{suffix}")
}

/// The target of the EnvironmentVariableDataBlock in the extra data starting at `at`, with
/// `%NAME%` expanded from this process's environment (no file is touched).
fn environment_target(r: &Bytes, mut at: usize) -> Option<String> {
    // Extra data is a sequence of blocks: u32 size (>= 8), u32 signature; a size below 4
    // ends it. A bounded number of blocks guards against a crafted endless chain.
    for _ in 0..64 {
        let size = r.u32(at)? as usize;
        if size < 8 {
            return None;
        }
        let end = at.checked_add(size)?;
        if end > r.0.len() {
            return None;
        }
        if r.u32(at + 4)? == ENVIRONMENT_BLOCK && size >= 0x314 {
            let wide = r.utf16_z(at + 8 + 260, at + 8 + 260 + 520);
            let raw = wide
                .filter(|s| !s.is_empty())
                .or_else(|| r.ansi_z(at + 8, at + 8 + 260))?;
            return Some(expand_env(&raw)).filter(|s| !s.is_empty());
        }
        at = end;
    }
    None
}

/// `%NAME%` replaced by the variable's value when set; unknown names stay as written.
fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) if j > 0 => {
                let name = &after[..j];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[j + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `rel` made absolute against `dir` lexically (`.` and `..` folded), or `rel` itself when
/// it is already absolute; `None` when there is no folder to resolve against.
fn absolute(rel: &str, dir: Option<&str>) -> Option<String> {
    if is_absolute(rel) {
        return Some(rel.to_owned());
    }
    let dir = dir?;
    let sep = if dir.contains('\\') || !dir.starts_with('/') {
        '\\'
    } else {
        '/'
    };
    let mut parts: Vec<&str> = dir.split(['\\', '/']).collect();
    for c in rel.split(['\\', '/']) {
        match c {
            "" | "." => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            c => parts.push(c),
        }
    }
    Some(parts.join(&sep.to_string()))
}

/// A drive path (`C:\…`), a UNC path (`\\server\…`) or a Unix absolute path.
fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with("\\\\")
        || p.starts_with('/')
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\')
}

/// A target worth storing: not empty, not absurdly long, no control characters.
fn usable(t: &str) -> bool {
    !t.is_empty() && t.chars().count() <= MAX_TARGET && !t.chars().any(char::is_control)
}

// ------------------------------------------------------------------------------- .url

/// The `URL=` value of the `[InternetShortcut]` section of an Internet Shortcut file
/// (an INI file, UTF-8 or the ANSI code page), or `None`.
pub fn parse_url(b: &[u8]) -> Option<String> {
    let text = match std::str::from_utf8(b) {
        Ok(s) => s.to_owned(),
        Err(_) => cp1252(b),
    };
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line.eq_ignore_ascii_case("[InternetShortcut]");
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case("URL") {
                let v = v.trim();
                return usable(v).then(|| v.to_owned());
            }
        }
    }
    None
}

/// A web address that may be handed to the browser: `http` or `https`, with no space,
/// quote or control character, so it is one command-line argument (VFS-6 R10, VFS-18).
pub fn is_openable_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let plain = !url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '"');
    plain && (lower.starts_with("https://") || lower.starts_with("http://"))
}

#[cfg(test)]
mod tests;
