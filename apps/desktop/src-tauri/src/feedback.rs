//! "Enviar comentarios" (VFS-20): a plain-text diagnostic report the user reads before it goes
//! anywhere, saved as a `.zip` and optionally handed to a prefilled GitHub issue. Nothing here
//! makes a network request (R8): the browser is opened on the user's click.
//!
//! The report never holds a root path, a file or folder name or any other path unless the user
//! ticks the names box (R3, R4). Diagnostic records (`vfs::diag`, VFS-5 R8) carry only counts
//! and durations and are kept as written; every other log line goes through [`redact_line`].

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use vfs_core::api::ApiError;
use vfs_core::{DriveKind, IndexStats, IndexerState, Root};

use crate::bookmarks::BookmarkState;
use crate::AppState;

/// Log target of panic records written by [`install_panic_hook`].
pub const PANIC_TARGET: &str = "vfs::panic";

/// What replaces a path in a redacted line.
pub const REDACTED: &str = "<ruta>";

/// What replaces a panic's message, or a whole continuation line, when names are left out
/// (VFS-30 R1, R3).
pub const OMITTED: &str = "<omitido>";

/// Most bytes of log lines "Ver el informe" shows: the newest ones (VFS-30 R4). The `.zip`
/// holds all.
pub const PREVIEW_LOG_BYTES: usize = 200 * 1024;

/// New-issue page of the project; the `comentarios` form lives in `.github/ISSUE_TEMPLATE/`.
pub const ISSUE_URL: &str = "https://github.com/AndresHG/VFS/issues/new";

/// File name of the issue form, and the ids of the fields the URL fills (R6).
pub const ISSUE_TEMPLATE: &str = "comentarios.yml";
pub const FIELD_TEXT: &str = "que-paso";
pub const FIELD_SUMMARY: &str = "resumen";

/// Longest prefilled issue URL (R6): GitHub and browsers refuse much longer ones.
pub const MAX_URL_CHARS: usize = 8000;

/// Name of the report inside the `.zip`.
pub const REPORT_ENTRY: &str = "informe.txt";

/// Base name of the log files (`vfs.log`, rotated as `vfs_<date>.log`), see `install_logger`.
pub const LOG_BASE: &str = "vfs";

/// Whether a root's disk has to seek (R2). Network shares and removable drives are `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskKind {
    Ssd,
    Hdd,
    Unknown,
}

impl DiskKind {
    fn name(self) -> &'static str {
        match self {
            DiskKind::Ssd => "ssd",
            DiskKind::Hdd => "hdd",
            DiskKind::Unknown => "unknown",
        }
    }
}

/// Firefox bookmarks as the report shows them (VFS-6 settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkInfo {
    pub enabled: bool,
    pub profiles: u32,
    pub count: u32,
}

/// Everything the report says about the machine and the index, gathered by [`gather`] in the
/// app and built by hand in tests.
#[derive(Debug, Clone)]
pub struct Environment {
    pub version: String,
    pub os: String,
    pub roots: Vec<(Root, DiskKind)>,
    pub stats: Option<IndexStats>,
    pub wal_bytes: Option<u64>,
    pub indexer: IndexerState,
    pub bookmarks: Option<BookmarkInfo>,
    /// Strings to redact wherever they appear, besides anything that reads as a path: the root
    /// paths and the user's home folder.
    pub private: Vec<String>,
    /// Unix time the report is built at, and the local time as text.
    pub now: i64,
    pub now_text: String,
}

/// One log file: its name and its text.
#[derive(Debug, Clone)]
pub struct LogFile {
    pub name: String,
    pub text: String,
}

/// The report: `summary` is everything but the user's text, the log lines and the root paths
/// (what the public issue URL carries), `text` the whole report (what the `.zip` holds and the
/// preview shows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub summary: String,
    pub text: String,
}

/// Writes every panic to the log as an error with its source location, then runs the hook
/// that was installed before (the default prints to stderr) (R7).
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map_or_else(|| "?".to_owned(), |l| format!("{}:{}", l.file(), l.line()));
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "?".to_owned());
        log::error!(target: PANIC_TARGET, "panic at {location}: {message}");
        log::logger().flush();
        previous(info);
    }));
}

/// The kept log files in `dir`, oldest first: rotated `vfs_<date>.log` files sort by their
/// date, and the active `vfs.log` comes last. Unreadable files are skipped.
pub fn read_logs(dir: &Path) -> Vec<LogFile> {
    let active = format!("{LOG_BASE}.log");
    let rotated = format!("{LOG_BASE}_");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with(&rotated) && n.ends_with(".log"))
        .collect();
    names.sort();
    names.push(active);
    names
        .into_iter()
        .filter_map(|name| {
            let bytes = std::fs::read(dir.join(&name)).ok()?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            Some(LogFile { name, text })
        })
        .collect()
}

/// The parts of a log line written by `tauri-plugin-log`: `[date][time][target][LEVEL] msg`.
/// `None` for a line without that prefix (the continuation of a multi-line message). Returns
/// the target, the level and where the message starts.
pub fn parse_line(line: &str) -> Option<(&str, &str, usize)> {
    let mut rest = line;
    let mut fields = [""; 4];
    for field in &mut fields {
        let inner = rest.strip_prefix('[')?;
        let end = inner.find(']')?;
        *field = &inner[..end];
        rest = &inner[end + 1..];
    }
    let message_start = line.len() - rest.len() + usize::from(rest.starts_with(' '));
    Some((fields[2], fields[3], message_start))
}

/// Where a path could start at byte `i` of `s`: a drive (`C:\`, `C:/`) or UNC (`\\`) prefix,
/// a `/` or `~/` that begins a word, or one of the `private` strings. Returns whether one does.
fn path_starts_at(s: &str, i: usize, private: &[String]) -> bool {
    let rest = &s[i..];
    if private.iter().any(|p| rest.starts_with(p.as_str())) {
        return true;
    }
    let b = rest.as_bytes();
    let word_start = i == 0
        || matches!(
            s.as_bytes()[i - 1],
            b' ' | b'\t'
                | b'"'
                | b'\''
                | b'`'
                | b'('
                | b'['
                | b'{'
                | b'<'
                | b'|'
                | b'='
                | b':'
                | b','
        );
    if !word_start {
        return false;
    }
    let drive =
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/');
    drive || rest.starts_with("\\\\") || rest.starts_with('/') || rest.starts_with("~/")
}

/// How many bytes of `rest`, where a path starts, belong to it: up to the closing `"` when the
/// path was opened by one (`quoted`), else the line end; a Windows path (`posix` false) also
/// ends at the first `": "`, since a colon cannot be in a Windows name and every log format puts
/// `": "` before its error text. A macOS name can hold `": "` (Finder shows it as "/"), so a
/// POSIX path never ends there (R3). Names hold spaces, parentheses, commas and apostrophes, so
/// nothing else ends a path: this over-redacts rather than leaks, e.g. `root C:\a b became
/// unreachable` keeps only `root`.
fn path_len(rest: &str, quoted: bool, posix: bool) -> usize {
    let colon = if posix { None } else { rest.find(": ") };
    let quote = if quoted { rest.find('"') } else { None };
    colon.into_iter().chain(quote).min().unwrap_or(rest.len())
}

/// Replaces every path in `message` with [`REDACTED`] (R3). A `private` string that matches is
/// always consumed whole, even if it holds a delimiter.
pub fn redact_message(message: &str, private: &[String]) -> String {
    let mut out = String::with_capacity(message.len());
    let mut i = 0;
    while i < message.len() {
        if path_starts_at(message, i, private) {
            let rest = &message[i..];
            let known = private
                .iter()
                .filter(|p| rest.starts_with(p.as_str()))
                .map(|p| p.len())
                .max()
                .unwrap_or(0);
            let quoted = i > 0 && message.as_bytes()[i - 1] == b'"';
            let posix = rest.starts_with('/') || rest.starts_with("~/");
            let tail = path_len(&rest[known..], quoted, posix);
            out.push_str(REDACTED);
            i += (known + tail).max(1);
            continue;
        }
        let c = message[i..].chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// A panic record's message with names left out (VFS-30 R1): its payload is free text (Rust's own
/// messages quote names in backticks, a search term can be in it), so only the location is
/// kept: `panic at <file>:<line>: <omitido>`.
fn redact_panic(message: &str) -> String {
    let location = message.strip_prefix("panic at ").and_then(|rest| {
        rest.match_indices(": ")
            .map(|(i, _)| &rest[..i])
            .find(|loc| {
                loc.rsplit_once(':')
                    .is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
    });
    match location {
        Some(loc) => format!("panic at {loc}: {OMITTED}"),
        None => OMITTED.to_owned(),
    }
}

/// One log line as the report shows it with names left out (VFS-20 R3, VFS-30 R1–R3):
/// diagnostic records verbatim,
/// a panic record with only its location, any other line with its prefix kept and its
/// message redacted. A continuation line (no prefix) is kept as written only when the record
/// it continues is a diagnostic one (`after_diag`); any other is replaced whole, since a name
/// holding a line break leaves its tail there.
pub fn redact_line(line: &str, private: &[String], after_diag: bool) -> String {
    match parse_line(line) {
        Some((target, _, _)) if target == vfs_core::DIAG_TARGET => line.to_owned(),
        Some((target, _, start)) if target == PANIC_TARGET => {
            format!("{}{}", &line[..start], redact_panic(&line[start..]))
        }
        Some((_, _, start)) => {
            format!(
                "{}{}",
                &line[..start],
                redact_message(&line[start..], private)
            )
        }
        None if after_diag => line.to_owned(),
        None => OMITTED.to_owned(),
    }
}

/// Counts of warnings, errors and panics in the logs (R2).
fn log_counts(logs: &[LogFile]) -> (usize, usize, usize, usize) {
    let (mut lines, mut warnings, mut errors, mut panics) = (0, 0, 0, 0);
    for line in logs.iter().flat_map(|f| f.text.lines()) {
        lines += 1;
        if let Some((target, level, _)) = parse_line(line) {
            match level {
                "WARN" => warnings += 1,
                "ERROR" => errors += 1,
                _ => {}
            }
            if target == PANIC_TARGET {
                panics += 1;
            }
        }
    }
    (lines, warnings, errors, panics)
}

/// The serde name of a contract enum (`ready`, `network`, `paused`…), as the UI sees it.
fn wire_name<T: Serialize>(v: T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => "?".to_owned(),
    }
}

/// "3 d", "5 h", "12 min" since `then`, or "never".
fn age(now: i64, then: Option<i64>) -> String {
    let Some(then) = then else {
        return "never".to_owned();
    };
    let s = (now - then).max(0);
    if s >= 86_400 {
        format!("{} d ago", s / 86_400)
    } else if s >= 3_600 {
        format!("{} h ago", s / 3_600)
    } else {
        format!("{} min ago", s / 60)
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

/// Builds the report (VFS-20 R2–R4). Its labels are English: it is read by maintainers. Only
/// the newest `log_budget` bytes of log lines are included, whole lines only (the preview passes
/// [`PREVIEW_LOG_BYTES`], the `.zip` `usize::MAX`); the counts always cover every line.
pub fn build_report(
    env: &Environment,
    description: &str,
    include_names: bool,
    logs: &[LogFile],
    log_budget: usize,
) -> Report {
    let mut s = String::new();
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    let _ = writeln!(s, "## App");
    let _ = writeln!(s, "version: {}", env.version);
    let _ = writeln!(s, "os: {}", env.os);
    let _ = writeln!(s, "generated: {}", env.now_text);
    let _ = writeln!(s, "names_included: {}", yes_no(include_names));
    let _ = writeln!(s, "\n## Folders");
    if env.roots.is_empty() {
        let _ = writeln!(s, "(none)");
    }
    // The root paths go only into the saved report, never into the summary the public issue
    // URL carries (R6): `named_roots` is this section with them.
    let roots_start = s.len();
    let mut named_roots = String::new();
    for (root, disk) in &env.roots {
        let line = format!(
            "root {}: drive={} disk={} status={} files={} folders={} last_full_scan={}\n",
            root.id,
            wire_name(root.drive),
            disk.name(),
            wire_name(root.status),
            root.files,
            root.dirs,
            age(env.now, root.last_full_scan),
        );
        s.push_str(&line);
        named_roots.push_str(&line);
        if include_names {
            let _ = writeln!(named_roots, "  path: {}", root.path);
        }
    }
    let roots_end = s.len();
    let _ = writeln!(s, "\n## Index");
    match &env.stats {
        Some(st) => {
            let _ = writeln!(
                s,
                "files: {}\nfolders: {}\nroots: {}\nsize_mb: {}\nwal_mb: {}\nschema: {}",
                st.files,
                st.dirs,
                st.roots,
                mb(st.db_bytes),
                env.wal_bytes.map_or_else(|| "?".to_owned(), mb),
                st.schema_version,
            );
        }
        None => {
            let _ = writeln!(s, "(unavailable)");
        }
    }
    let _ = writeln!(s, "\n## Settings");
    let _ = writeln!(s, "indexer: {}", wire_name(env.indexer));
    match &env.bookmarks {
        Some(b) if b.enabled => {
            let _ = writeln!(
                s,
                "firefox_bookmarks: on (profiles={} bookmarks={})",
                b.profiles, b.count
            );
        }
        Some(_) => {
            let _ = writeln!(s, "firefox_bookmarks: off");
        }
        None => {
            let _ = writeln!(s, "firefox_bookmarks: unavailable");
        }
    }
    let (lines, warnings, errors, panics) = log_counts(logs);
    let _ = writeln!(s, "\n## Log");
    let _ = writeln!(
        s,
        "files: {}\nlines: {lines}\nwarnings: {warnings}\nerrors: {errors}\npanics: {panics}",
        logs.len()
    );
    let summary = s;

    let total: usize = logs.iter().map(|f| f.text.len()).sum();
    let mut skip = total.saturating_sub(log_budget);
    let mut text = String::with_capacity(summary.len() + total.min(log_budget) + 512);
    text.push_str("VFS diagnostic report\n\n## What happened\n");
    let description = description.trim();
    text.push_str(if description.is_empty() {
        "(empty)"
    } else {
        description
    });
    text.push_str("\n\n");
    text.push_str(&summary[..roots_start]);
    text.push_str(&named_roots);
    text.push_str(&summary[roots_end..]);
    text.push_str("\n## Log lines\n");
    if skip > 0 {
        text.push_str("(older lines are left out of this preview; the .zip holds them all)\n");
    }
    for file in logs {
        if skip >= file.text.len() {
            skip -= file.text.len();
            continue;
        }
        // Start at the first whole line after the bytes skipped.
        let start = if skip == 0 || file.text.as_bytes()[skip - 1] == b'\n' {
            skip
        } else {
            file.text.as_bytes()[skip..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(file.text.len(), |p| skip + p + 1)
        };
        skip = 0;
        let _ = writeln!(text, "--- {} ---", file.name);
        let mut after_diag = false;
        for line in file.text[start..].lines() {
            if include_names {
                text.push_str(line);
            } else {
                text.push_str(&redact_line(line, &env.private, after_diag));
            }
            text.push('\n');
            if let Some((target, _, _)) = parse_line(line) {
                after_diag = target == vfs_core::DIAG_TARGET;
            }
        }
    }
    Report { summary, text }
}

/// Percent-encodes everything but RFC 3986 unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// `s` percent-encoded in at most `budget` bytes: shortened, ending in "…", when longer.
fn encode_within(s: &str, budget: usize) -> String {
    let full = encode(s);
    if full.len() <= budget {
        return full;
    }
    let ellipsis = encode("…");
    let mut cut = String::new();
    for c in s.chars() {
        let mut buf = [0u8; 4];
        let piece = encode(c.encode_utf8(&mut buf));
        if cut.len() + piece.len() + ellipsis.len() > budget {
            break;
        }
        cut.push_str(&piece);
    }
    cut + &ellipsis
}

/// The prefilled new-issue address (R6): the `comentarios` form with the user's text and the
/// report summary, never a log line. The summary takes at most half of [`MAX_URL_CHARS`] (many
/// roots), the text the rest; either is shortened, ending in "…", to fit.
pub fn github_url(description: &str, summary: &str) -> String {
    let head = format!(
        "{ISSUE_URL}?template={}&{FIELD_SUMMARY}={}&{FIELD_TEXT}=",
        encode(ISSUE_TEMPLATE),
        encode_within(summary, MAX_URL_CHARS / 2)
    );
    let budget = MAX_URL_CHARS.saturating_sub(head.len());
    head + &encode_within(description.trim(), budget)
}

/// Where the report is saved (R5): Downloads, else Desktop, else home; the first that exists.
pub fn export_dir(candidates: &[Option<PathBuf>]) -> Option<PathBuf> {
    candidates.iter().flatten().find(|d| d.is_dir()).cloned()
}

/// `vfs-informe-<stamp>.zip` in `dir`, or `-2`, `-3`… when taken.
pub fn report_path(dir: &Path, stamp: &str) -> PathBuf {
    let base = format!("vfs-informe-{stamp}");
    let mut path = dir.join(format!("{base}.zip"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base}-{n}.zip"));
        n += 1;
    }
    path
}

/// Writes `text` as [`REPORT_ENTRY`] into a deflated `.zip` at `path`: first to a temporary
/// file beside it, renamed at the end, so a kill mid-write leaves no half report.
pub fn write_zip(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("zip.part");
    let result = (|| {
        let file = std::fs::File::create(&tmp)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(REPORT_ENTRY, options)
            .map_err(std::io::Error::other)?;
        zip.write_all(text.as_bytes())?;
        zip.finish().map_err(std::io::Error::other)?.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Opens a web address in the default browser (R6). Only [`github_url`]'s output reaches it:
/// fully percent-encoded, so it is one safe argument.
fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let ok = std::process::Command::new("/usr/bin/open")
            .arg(url)
            .status()?
            .success();
        if ok {
            Ok(())
        } else {
            Err(std::io::Error::other("open failed"))
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let verb = win::wide("open");
        let file = win::wide(url);
        // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the call.
        let code = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        // ShellExecuteW returns a value greater than 32 on success.
        if code > 32 {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "ShellExecuteW failed ({code})"
            )))
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
    }
}

/// OS name, version and architecture, e.g. `macos 15.6 (aarch64)`, `windows 10.0.26100 23H2
/// (x86_64)`.
fn os_text() -> String {
    let version = os_version().unwrap_or_else(|| "?".to_owned());
    format!(
        "{} {version} ({})",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

#[cfg(target_os = "macos")]
fn os_version() -> Option<String> {
    let out = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[cfg(windows)]
fn os_version() -> Option<String> {
    let build = win::current_version("CurrentBuild")?;
    let display = win::current_version("DisplayVersion").unwrap_or_default();
    Some(format!("10.0.{build} {display}").trim().to_owned())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn os_version() -> Option<String> {
    None
}

/// Whether the disk holding `path` has to seek (R2). Only local drives are asked.
fn disk_kind(path: &Path, drive: DriveKind) -> DiskKind {
    if drive != DriveKind::Local {
        return DiskKind::Unknown;
    }
    platform_disk_kind(path).unwrap_or(DiskKind::Unknown)
}

/// macOS: the device behind `path` (`df -P`), then its `SolidState` key (`diskutil info`).
#[cfg(target_os = "macos")]
fn platform_disk_kind(path: &Path) -> Option<DiskKind> {
    let df = std::process::Command::new("/bin/df")
        .arg("-P")
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&df.stdout).into_owned();
    let device = text.lines().nth(1)?.split_whitespace().next()?.to_owned();
    if !device.starts_with("/dev/") {
        return None;
    }
    let info = std::process::Command::new("/usr/sbin/diskutil")
        .args(["info", "-plist", &device])
        .output()
        .ok()?;
    solid_state_from_plist(&String::from_utf8_lossy(&info.stdout))
}

/// The `SolidState` value of a `diskutil info -plist` output.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn solid_state_from_plist(plist: &str) -> Option<DiskKind> {
    let after = &plist[plist.find("<key>SolidState</key>")? + "<key>SolidState</key>".len()..];
    let value = after.trim_start();
    if value.starts_with("<true/>") {
        Some(DiskKind::Ssd)
    } else if value.starts_with("<false/>") {
        Some(DiskKind::Hdd)
    } else {
        None
    }
}

/// Windows: `IOCTL_STORAGE_QUERY_PROPERTY` / `StorageDeviceSeekPenaltyProperty` on the volume
/// holding `path`; works without administrator rights (no access asked of the device).
#[cfg(windows)]
fn platform_disk_kind(path: &Path) -> Option<DiskKind> {
    win::seek_penalty(path).map(|seeks| if seeks { DiskKind::Hdd } else { DiskKind::Ssd })
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_disk_kind(_path: &Path) -> Option<DiskKind> {
    None
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    pub fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
    }

    /// A string value of `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`.
    pub fn current_version(name: &str) -> Option<String> {
        use windows_sys::Win32::System::Registry::{
            RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
        };
        let key = wide("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
        let value = wide(name);
        let mut buf = [0u16; 128];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: the strings are NUL-terminated; `buf` holds `size` bytes.
        let r = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if r != 0 {
            return None;
        }
        let n = (size as usize / 2).saturating_sub(1).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }

    /// `DEVICE_SEEK_PENALTY_DESCRIPTOR` with its BOOLEAN read as a byte.
    #[repr(C)]
    #[derive(Default)]
    struct SeekPenalty {
        version: u32,
        size: u32,
        incurs_seek_penalty: u8,
    }

    pub fn seek_penalty(path: &Path) -> Option<bool> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, GetVolumePathNameW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows_sys::Win32::System::Ioctl::{
            PropertyStandardQuery, StorageDeviceSeekPenaltyProperty, IOCTL_STORAGE_QUERY_PROPERTY,
            STORAGE_PROPERTY_QUERY,
        };
        use windows_sys::Win32::System::IO::DeviceIoControl;

        let file = wide(path);
        let mut volume = [0u16; 261];
        // SAFETY: `file` is NUL-terminated; `volume` holds its stated length.
        if unsafe { GetVolumePathNameW(file.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) }
            == 0
        {
            return None;
        }
        let end = volume.iter().position(|&c| c == 0)?;
        // "C:\" -> "\\.\C:"; mount folders and volume GUID paths are not asked.
        let letter = String::from_utf16_lossy(&volume[..end]);
        let letter = letter.strip_suffix('\\')?;
        if letter.len() != 2 || !letter.ends_with(':') {
            return None;
        }
        let device = wide(format!("\\\\.\\{letter}"));
        // SAFETY: `device` is NUL-terminated; no access right is asked, which is enough for
        // the property query and needs no administrator rights.
        let handle = unsafe {
            CreateFileW(
                device.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceSeekPenaltyProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        let mut out = SeekPenalty::default();
        let mut returned = 0u32;
        // SAFETY: `handle` is open; both buffers live across the call with the sizes given.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                (&query as *const STORAGE_PROPERTY_QUERY).cast(),
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                (&mut out as *mut SeekPenalty).cast(),
                std::mem::size_of::<SeekPenalty>() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        // SAFETY: `handle` was opened above and is closed once.
        unsafe { CloseHandle(handle) };
        (ok != 0 && returned as usize >= std::mem::size_of::<SeekPenalty>())
            .then_some(out.incurs_seek_penalty != 0)
    }
}

/// Gathers the [`Environment`] of the running app.
pub fn gather<R: Runtime>(app: &AppHandle<R>, state: &AppState) -> Environment {
    let vfs = &state.vfs;
    let roots: Vec<(Root, DiskKind)> = vfs
        .list_roots()
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let disk = disk_kind(Path::new(&r.path), r.drive);
            (r, disk)
        })
        .collect();
    let mut private: Vec<String> = roots.iter().map(|(r, _)| r.path.clone()).collect();
    if let Ok(home) = app.path().home_dir() {
        private.push(home.to_string_lossy().into_owned());
    }
    // Longest first, so a root inside the home folder is matched whole; very short strings
    // ("/", "C:\") are left to the generic rule.
    private.retain(|p| p.len() > 3);
    private.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let wal_bytes = app
        .path()
        .app_local_data_dir()
        .ok()
        .and_then(|d| std::fs::metadata(d.join(format!("{}-wal", crate::INDEX_FILE))).ok())
        .map(|m| m.len());
    let bookmarks = app.try_state::<BookmarkState>().map(|b| {
        let s = b.bookmarks.status();
        BookmarkInfo {
            enabled: s.enabled,
            profiles: s.profiles,
            count: s.count,
        }
    });
    let now = chrono::Local::now();
    Environment {
        version: app.package_info().version.to_string(),
        os: os_text(),
        roots,
        stats: vfs.stats().ok(),
        wal_bytes,
        indexer: vfs.progress().state,
        bookmarks,
        private,
        now: now.timestamp(),
        now_text: now.format("%Y-%m-%d %H:%M %:z").to_string(),
    }
}

fn io_error(e: impl std::fmt::Display) -> ApiError {
    vfs_core::Error::Io(std::io::Error::other(e.to_string())).into()
}

fn current_report<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    description: &str,
    include_names: bool,
    log_budget: usize,
) -> Report {
    let logs = app
        .path()
        .app_local_data_dir()
        .map(|d| read_logs(&d.join("logs")))
        .unwrap_or_default();
    build_report(
        &gather(app, state),
        description,
        include_names,
        &logs,
        log_budget,
    )
}

/// "Ver el informe" (VFS-20 R1, VFS-30 R4): the report as it would be saved, with only the newest
/// [`PREVIEW_LOG_BYTES`] of log lines, so a slow PC never loads the whole log into the page.
#[tauri::command(async)]
pub fn feedback_report<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    description: String,
    include_names: bool,
) -> Result<String, ApiError> {
    Ok(current_report(&app, &state, &description, include_names, PREVIEW_LOG_BYTES).text)
}

/// Where the report was saved.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackSaved {
    /// The folder, as the user's system names it, and the file name.
    pub folder: String,
    pub file: String,
}

/// "Exportar diagnóstico (.zip)" (R5) and, with `github`, "Enviar por GitHub" (R6): saves the
/// report, shows it selected in Explorer/Finder, and opens the prefilled issue.
#[tauri::command(async)]
pub fn feedback_save<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    description: String,
    include_names: bool,
    github: bool,
) -> Result<FeedbackSaved, ApiError> {
    let report = current_report(&app, &state, &description, include_names, usize::MAX);
    let p = app.path();
    let dir = export_dir(&[
        p.download_dir().ok(),
        p.desktop_dir().ok(),
        p.home_dir().ok(),
    ])
    .ok_or_else(|| io_error("no folder to save the report in"))?;
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M").to_string();
    let path = report_path(&dir, &stamp);
    write_zip(&path, &report.text).map_err(io_error)?;
    if let Err(e) = crate::reveal::reveal(&path) {
        log::warn!("reveal report: {e}");
    }
    if github {
        open_url(&github_url(&description, &report.summary)).map_err(io_error)?;
    }
    Ok(FeedbackSaved {
        folder: dir.to_string_lossy().into_owned(),
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(id: i64, path: &str) -> Root {
        Root {
            id,
            path: path.into(),
            status: vfs_core::RootStatus::Ready,
            files: 1200,
            dirs: 80,
            last_full_scan: Some(1_000_000 - 3 * 86_400),
            drive: DriveKind::Local,
            unreadable: 0,
        }
    }

    fn env(roots: &[&str]) -> Environment {
        let roots: Vec<(Root, DiskKind)> = roots
            .iter()
            .enumerate()
            .map(|(i, p)| (root(i as i64 + 1, p), DiskKind::Hdd))
            .collect();
        let mut private: Vec<String> = roots.iter().map(|(r, _)| r.path.clone()).collect();
        private.push("/Users/ana".into());
        private.sort_by_key(|p| std::cmp::Reverse(p.len()));
        Environment {
            version: "0.2.0".into(),
            os: "windows 10.0.19045 22H2 (x86_64)".into(),
            roots,
            stats: Some(IndexStats {
                files: 1200,
                dirs: 80,
                roots: 1,
                db_bytes: 5 * 1024 * 1024,
                schema_version: 4,
            }),
            wal_bytes: Some(1024 * 1024),
            indexer: IndexerState::Idle,
            bookmarks: Some(BookmarkInfo {
                enabled: true,
                profiles: 1,
                count: 230,
            }),
            private,
            now: 1_000_000,
            now_text: "2026-11-21 10:30 +01:00".into(),
        }
    }

    const DIAG: &str = "[2026-11-21][09:00:00][vfs::diag][INFO] pass mode=walk period_s=60 dirs=120 files_added=3000 errors=0";

    /// Every warning format the code writes today that can carry a path or a name (VFS-5 R8
    /// kept info lines out; warnings and errors still name folders), plus free-text panic
    /// payloads (a slicing panic quoting a path in backticks, a bare search term), paths after
    /// a backtick or `<`, and a macOS name holding a line break.
    fn hostile_log() -> String {
        [
            DIAG,
            r"[2026-11-21][09:00:01][vfs_core::indexer][WARN] cannot list D:\Fotos familia\Viaje a Roma 2019: Acceso denegado. (os error 5)",
            r"[2026-11-21][09:00:02][vfs_core::indexer][WARN] root D:\Fotos familia became unreachable",
            r"[2026-11-21][09:00:03][vfs_core::indexer::watch][WARN] watch D:\Fotos familia failed: Access is denied. (os error 5)",
            r"[2026-11-21][09:00:04][vfs_core::indexer][WARN] rename \\NAS\compartido\Herencia Gómez.docx: not found: \\NAS\compartido\Herencia Gómez.docx",
            "[2026-11-21][09:00:05][vfs_core::db][WARN] index /Users/ana/Library/Application Support/vfs/index.db unusable (file is not a database); recreating it",
            "[2026-11-21][09:00:06][vfs_bookmarks][WARN] Firefox profile /Users/ana/Library/Application Support/Firefox/Profiles/x1.default-release: locked",
            "[2026-11-21][09:00:07][vfs_desktop_lib::commands][WARN] mark_dirty /Volumes/Archivo/Notaría Pérez/escritura 2019.pdf: gone",
            r"[2026-11-21][09:00:08][vfs_core::indexer][WARN] cannot watch D:\Fotos familia: overlaps an indexed folder: inside D:\Fotos familia\Viejas",
            r"[2026-11-21][09:00:08][vfs_core::indexer][WARN] cannot list E:\Archivo (2019)\Nueva carpeta (2)\Casa d'Ana: Acceso denegado. (os error 5)",
            r"[2026-11-21][09:00:08][vfs_core::indexer][WARN] root E:\Archivo (2019) became unreachable",
            r"[2026-11-21][09:00:09][vfs::panic][ERROR] panic at crates/vfs-core/src/x.rs:10: called `Result::unwrap()` on an `Err` value: NotFound(D:\Fotos familia\Boda Laura.jpg)",
            r"[2026-11-21][09:00:09][vfs::panic][ERROR] panic at library/core/src/str/mod.rs:68: byte index 9 is not a char boundary; it is inside 'ñ' (bytes 8..10) of `D:\Archivo\Boda año.jpg`",
            "[2026-11-21][09:00:09][vfs::panic][ERROR] panic at crates/vfs-core/src/search.rs:40: no match for `Bautizo Leo`",
            r"[2026-11-21][09:00:09][vfs_core::indexer][WARN] skipped `F:\Recuerdos\Cumple Marta.png`: locked",
            r"[2026-11-21][09:00:09][vfs_core::indexer][WARN] open <G:\Escaneos\Contrato alquiler.pdf> failed",
            "[2026-11-21][09:00:09][vfs_core::indexer][WARN] cannot list /Volumes/Archivo/Nota\nsecreta banco: Permission denied",
            r#"[2026-11-21][09:00:10][vfs_core::indexer][ERROR] indexer: not a directory: "/Users/ana/Documentos/Declaración renta.pdf""#,
            "continuation line ~/Escritorio/Carta al banco.odt",
        ]
        .join("\n")
    }

    const SECRETS: &[&str] = &[
        "Fotos familia",
        "Viaje a Roma",
        "Herencia",
        "Gómez",
        "compartido",
        "/Users/ana",
        "Firefox/Profiles",
        "Notaría",
        "escritura",
        "Viejas",
        "Boda Laura",
        "Declaración",
        "Carta al banco",
        "Archivo (2019)",
        "(2019)",
        "Nueva carpeta",
        "Casa d'Ana",
        "Boda año",
        "Bautizo Leo",
        "Recuerdos",
        "Cumple Marta",
        "Escaneos",
        "Contrato alquiler",
        "secreta banco",
    ];

    fn logs() -> Vec<LogFile> {
        vec![
            LogFile {
                name: "vfs_2026-11-01_00-00-00.log".into(),
                text: "[2026-11-01][00:00:00][vfs::diag][INFO] start uptime_s=120 catchup_deferred_s=0 probe=pdh data_dir_in_sync_root=false\n".into(),
            },
            LogFile {
                name: "vfs.log".into(),
                text: hostile_log(),
            },
        ]
    }

    /// VFS-20 AC-1: the report carries the version, OS, each root's drive, disk, status and
    /// counts, the index stats, the settings, the log-level counts and the log lines, oldest
    /// file first.
    #[test]
    fn report_holds_every_section() {
        let r = build_report(
            &env(&[r"D:\Fotos familia"]),
            "Va lento",
            false,
            &logs(),
            usize::MAX,
        );
        for want in [
            "## What happened\nVa lento",
            "version: 0.2.0",
            "os: windows 10.0.19045 22H2 (x86_64)",
            "root 1: drive=local disk=hdd status=ready files=1200 folders=80 last_full_scan=3 d ago",
            "files: 1200\nfolders: 80\nroots: 1\nsize_mb: 5.0\nwal_mb: 1.0\nschema: 4",
            "indexer: idle",
            "firefox_bookmarks: on (profiles=1 bookmarks=230)",
            "files: 2\nlines: 21\nwarnings: 13\nerrors: 4\npanics: 3",
            "names_included: no",
        ] {
            assert!(r.text.contains(want), "missing {want:?} in\n{}", r.text);
        }
        let old = r.text.find("--- vfs_2026-11-01").unwrap();
        let new = r.text.find("--- vfs.log ---").unwrap();
        assert!(old < new);
        assert!(r.text.contains(DIAG));
        // The summary is what the issue URL carries: no user text, no log line.
        assert!(!r.summary.contains("Va lento"));
        assert!(!r.summary.contains("pass mode=walk"));
        assert!(r.text.contains(&r.summary));
    }

    /// VFS-20 AC-2: with the box unticked no root path, file or folder name survives in any
    /// warning format, `<ruta>` takes their place, and diagnostic lines are unchanged.
    /// VFS-30 AC-1: nor in a panic payload or a name's tail after a line break; each panic
    /// keeps only its location.
    #[test]
    fn report_without_names_redacts_every_path() {
        let r = build_report(
            &env(&[r"D:\Fotos familia", r"E:\Archivo (2019)"]),
            "",
            false,
            &logs(),
            usize::MAX,
        );
        for secret in SECRETS {
            assert!(!r.text.contains(secret), "{secret:?} leaked in\n{}", r.text);
        }
        assert!(r.text.contains(DIAG));
        assert!(r
            .text
            .contains("[vfs_core::indexer][WARN] cannot list <ruta>: Acceso denegado."));
        assert!(r
            .text
            .lines()
            .any(|l| l.ends_with("[vfs_bookmarks][WARN] Firefox profile <ruta>")));
        for panic in [
            "panic at crates/vfs-core/src/x.rs:10: <omitido>",
            "panic at library/core/src/str/mod.rs:68: <omitido>",
            "panic at crates/vfs-core/src/search.rs:40: <omitido>",
        ] {
            assert!(r.text.lines().any(|l| l.ends_with(panic)), "{panic}");
        }
        assert!(r.text.contains("[WARN] skipped `<ruta>: locked"));
        assert!(r.text.contains("[WARN] open <<ruta>"));
        assert_eq!(r.text.lines().filter(|l| *l == OMITTED).count(), 2);
        assert!(!r.text.contains("path: "));
    }

    /// VFS-20 AC-2: the generic rule alone (no root given) still catches Windows, UNC, macOS
    /// and home-relative paths, and leaves ordinary words and our own source paths alone.
    /// VFS-30 AC-2: also after a backtick, `<`, `{` or `|`.
    #[test]
    fn generic_rule_redacts_paths_without_known_roots() {
        let none: &[String] = &[];
        assert_eq!(
            redact_message(r"cannot list C:\Mis cosas\a b: denied", none),
            "cannot list <ruta>: denied"
        );
        assert_eq!(redact_message("x=/home/u/a b (gone)", none), "x=<ruta>");
        // Parentheses and apostrophes are common in names and never end a path.
        assert_eq!(
            redact_message(r"cannot list D:\F\Nueva carpeta (2)\Boda.jpg: denied", none),
            "cannot list <ruta>: denied"
        );
        assert_eq!(
            redact_message(r"list D:\Casa d'Ana\x.jpg: e", none),
            "list <ruta>: e"
        );
        // A quoted path ends at its closing quote.
        assert_eq!(
            redact_message(r#"not a dir: "/a/Casa b.txt" (x)"#, none),
            r#"not a dir: "<ruta>" (x)"#
        );
        assert_eq!(redact_message(r"open \\srv\a\b", none), "open <ruta>");
        // A path after a backtick, `<`, `{` or `|` (Rust's messages, Debug output, tables).
        assert_eq!(redact_message(r"of `C:\a\Boda.jpg`", none), "of `<ruta>");
        assert_eq!(
            redact_message("open </a/Boda.jpg> failed", none),
            "open <<ruta>"
        );
        assert_eq!(redact_message("Entry {/a/Boda.jpg}", none), "Entry {<ruta>");
        assert_eq!(redact_message(r"x |D:\Boda.jpg| y", none), "x |<ruta>");
        assert_eq!(redact_message("at ~/Docs/x", none), "at <ruta>");
        assert_eq!(
            redact_message("km/h and a/b at src/x.rs:3", none),
            "km/h and a/b at src/x.rs:3"
        );
        assert_eq!(
            redact_message("checkpoint: database is locked", none),
            "checkpoint: database is locked"
        );
        // A root path without a drive letter (a mount folder) is caught as a known string.
        let roots = vec!["Archivo\\Fotos".to_owned()];
        assert_eq!(
            redact_message("watch Archivo\\Fotos\\x: e", &roots),
            "watch <ruta>: e"
        );
    }

    /// VFS-20 AC-2: a macOS name typed with "/" in Finder is stored with ":", so `": "` can be
    /// inside a POSIX path: such a path runs to the line end (or its closing quote), while a
    /// Windows path, whose names cannot hold a colon, still ends at `": "`.
    #[test]
    fn finder_slash_in_a_name_never_ends_a_path() {
        let none: &[String] = &[];
        assert_eq!(
            redact_message(
                "cannot list /Volumes/Archivo/Nota: banco/x.pdf: Permission denied",
                none
            ),
            "cannot list <ruta>"
        );
        assert_eq!(
            redact_message("at ~/Docs/Carta: Ana.odt: gone", none),
            "at <ruta>"
        );
        assert_eq!(
            redact_message(r#"not a dir: "/a/Nota: banco.txt" (x)"#, none),
            r#"not a dir: "<ruta>" (x)"#
        );
        let roots = vec!["/Volumes/Archivo".to_owned()];
        assert_eq!(
            redact_message("watch /Volumes/Archivo/Nota: banco failed: denied", &roots),
            "watch <ruta>"
        );
        assert_eq!(
            redact_message(r"cannot list D:\Nota banco\x: Acceso denegado.", none),
            "cannot list <ruta>: Acceso denegado."
        );
    }

    /// VFS-30 AC-3: a panic keeps only its location, whatever its payload; a continuation line
    /// is kept only after a diagnostic record, any other is replaced whole (a macOS name can
    /// hold a line break).
    #[test]
    fn panics_and_continuations_keep_no_free_text() {
        let none: &[String] = &[];
        let panic = |m: &str| format!("[2026-11-21][09:00:09][vfs::panic][ERROR] {m}");
        assert_eq!(
            redact_line(
                &panic(r"panic at C:\src\x.rs:5: of `Declaración renta`"),
                none,
                false
            ),
            panic(r"panic at C:\src\x.rs:5: <omitido>")
        );
        assert_eq!(
            redact_line(&panic("panic at x.rs: 3: Boda: y"), none, false),
            panic(OMITTED)
        );
        assert_eq!(redact_line("Boda Laura.jpg", none, false), OMITTED);
        assert_eq!(redact_line("dirs=3 files=9", none, true), "dirs=3 files=9");

        let mut env = env(&[]);
        env.private.clear();
        let log = |text: &str| {
            vec![LogFile {
                name: "vfs.log".into(),
                text: text.into(),
            }]
        };
        let text = build_report(
            &env,
            "",
            false,
            &log("[d][t][vfs_core::indexer][WARN] cannot list /a/Nota\nsecreta banco: denied\n"),
            usize::MAX,
        )
        .text;
        assert!(!text.contains("secreta"), "{text}");
        assert!(text.ends_with("cannot list <ruta>\n<omitido>\n"), "{text}");
        let text = build_report(
            &env,
            "",
            false,
            &log(&format!("{DIAG}\n  dirs=3\n")),
            usize::MAX,
        )
        .text;
        assert!(text.ends_with(&format!("{DIAG}\n  dirs=3\n")), "{text}");
    }

    /// VFS-30 AC-5: the preview holds the summary and only the newest
    /// [`PREVIEW_LOG_BYTES`] of log lines, whole lines, and says the rest is in the `.zip`;
    /// the counts still cover every line.
    #[test]
    fn preview_stays_bounded() {
        let line =
            |i: usize| format!("[2026-11-21][09:00:00][vfs_core::indexer][WARN] warning {i:07}");
        let big: String = (0..60_000).map(|i| line(i) + "\n").collect();
        assert!(big.len() > 10 * PREVIEW_LOG_BYTES);
        let logs = vec![
            LogFile {
                name: "vfs_2026-11-01_00-00-00.log".into(),
                text: big.clone(),
            },
            LogFile {
                name: "vfs.log".into(),
                text: big,
            },
        ];
        let env = env(&[]);
        let preview = build_report(&env, "", false, &logs, PREVIEW_LOG_BYTES);
        assert!(
            preview.text.len() <= preview.summary.len() + PREVIEW_LOG_BYTES + 1024,
            "{}",
            preview.text.len()
        );
        assert!(preview.text.contains("lines: 120000\nwarnings: 120000"));
        assert!(preview.text.contains("the .zip holds them all"));
        assert!(preview.text.ends_with(&(line(59_999) + "\n")));
        assert!(!preview.text.contains("--- vfs_2026-11-01"));
        let first_kept = preview.text.lines().find(|l| l.contains("[WARN]")).unwrap();
        assert!(first_kept.starts_with("[2026-11-21]"), "{first_kept}");

        // A budget that ends exactly at a line start keeps that whole line.
        let one = build_report(&env, "", false, &logs, line(0).len() + 1);
        let kept: Vec<&str> = one.text.lines().filter(|l| l.contains("[WARN]")).collect();
        assert_eq!(kept, [line(59_999)]);

        let full = build_report(&env, "", false, &logs, usize::MAX);
        assert!(full.text.contains(&line(0)));
        assert!(!full.text.contains("the .zip holds them all"));
    }

    /// VFS-20 AC-3: with the box ticked the root paths and the original lines appear.
    #[test]
    fn report_with_names_keeps_them() {
        let r = build_report(&env(&[r"D:\Fotos familia"]), "", true, &logs(), usize::MAX);
        assert!(r.text.contains(r"path: D:\Fotos familia"));
        assert!(r.text.contains(&hostile_log()));
        assert!(r.text.contains("names_included: yes"));
    }

    /// VFS-20 AC-4: the zip is named by date and time, never overwrites an earlier one, and
    /// holds `informe.txt` equal to the previewed text; the folder falls back to Desktop,
    /// then home.
    #[test]
    fn report_zip_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let text = build_report(&env(&[]), "Hola ñandú", false, &logs(), usize::MAX).text;
        let path = report_path(tmp.path(), "2026-11-21-1030");
        assert_eq!(path.file_name().unwrap(), "vfs-informe-2026-11-21-1030.zip");
        write_zip(&path, &text).unwrap();
        assert!(!tmp
            .path()
            .join("vfs-informe-2026-11-21-1030.zip.part")
            .exists());
        let second = report_path(tmp.path(), "2026-11-21-1030");
        assert_eq!(
            second.file_name().unwrap(),
            "vfs-informe-2026-11-21-1030-2.zip"
        );

        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(zip.len(), 1);
        let mut entry = zip.by_name(REPORT_ENTRY).unwrap();
        let mut read = String::new();
        std::io::Read::read_to_string(&mut entry, &mut read).unwrap();
        assert_eq!(read, text);

        let missing = tmp.path().join("Descargas");
        let desktop = tmp.path().join("Escritorio");
        std::fs::create_dir(&desktop).unwrap();
        assert_eq!(
            export_dir(&[
                Some(missing.clone()),
                Some(desktop.clone()),
                Some(tmp.path().into())
            ]),
            Some(desktop)
        );
        assert_eq!(
            export_dir(&[Some(missing), None, Some(tmp.path().into())]),
            Some(tmp.path().to_path_buf())
        );
    }

    /// VFS-20 AC-5: the issue URL uses the `comentarios` form, carries the (shortened) text
    /// and the summary but no log line, stays under the limit, and is fully percent-encoded;
    /// the form declares the fields the URL fills.
    #[test]
    fn github_url_is_short_and_encoded() {
        let r = build_report(&env(&[r"D:\Fotos familia"]), "", false, &logs(), usize::MAX);
        let long = "No encuentra la escritura & <otras> cosas ñ ".repeat(500);
        let url = github_url(&long, &r.summary);
        assert!(
            url.starts_with(&format!("{ISSUE_URL}?template=comentarios.yml&")),
            "{url}"
        );
        assert!(url.len() <= MAX_URL_CHARS, "{}", url.len());
        assert!(url.contains(&format!(
            "&{FIELD_SUMMARY}=%23%23%20App%0Aversion%3A%200.2.0"
        )));
        assert!(url.contains(&format!("&{FIELD_TEXT}=No%20encuentra")));
        assert!(url.ends_with(&encode("…")));
        assert!(!url.contains("pass%20mode"));
        let query = url.split_once('?').unwrap().1;
        assert!(
            query
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~%=&".contains(&b)),
            "{query}"
        );

        let huge = github_url(&long, &"root 1: drive=local disk=hdd\n".repeat(2000));
        assert!(huge.len() <= MAX_URL_CHARS, "{}", huge.len());
        assert!(huge.contains(&format!("&{FIELD_TEXT}=No%20encuentra")));

        let short = github_url("Va lento", &r.summary);
        assert!(short.ends_with(&format!("&{FIELD_TEXT}=Va%20lento")));

        let form = include_str!("../../../../.github/ISSUE_TEMPLATE/comentarios.yml");
        assert!(form.contains(&format!("id: {FIELD_TEXT}")));
        assert!(form.contains(&format!("id: {FIELD_SUMMARY}")));
    }

    /// VFS-20 AC-5: with the names box ticked the root paths are in the saved report but never
    /// in the issue URL, which becomes a public page.
    #[test]
    fn github_url_never_holds_root_paths() {
        let roots = [r"D:\Fotos familia", "/Volumes/Archivo/Notaría"];
        let r = build_report(&env(&roots), "Va lento", true, &logs(), usize::MAX);
        let url = github_url("Va lento", &r.summary);
        for root in roots {
            assert!(r.text.contains(&format!("path: {root}")), "{}", r.text);
            assert!(!url.contains(&encode(root)), "{url}");
        }
        for secret in ["Fotos", "Notar", "path%3A"] {
            assert!(!url.contains(secret), "{secret:?} in {url}");
        }
        assert!(url.contains("names_included%3A%20yes"), "{url}");
        assert!(url.contains("root%201%3A%20drive%3Dlocal"), "{url}");
    }

    /// VFS-20 AC-1: `diskutil` output maps to a disk kind.
    #[test]
    fn solid_state_key_is_read() {
        let ssd = "<dict>\n\t<key>SolidState</key>\n\t<true/>\n</dict>";
        let hdd = "<key>SolidState</key>\n<false/>";
        assert_eq!(solid_state_from_plist(ssd), Some(DiskKind::Ssd));
        assert_eq!(solid_state_from_plist(hdd), Some(DiskKind::Hdd));
        assert_eq!(solid_state_from_plist("<dict></dict>"), None);
        assert_eq!(
            disk_kind(Path::new("/"), DriveKind::Network),
            DiskKind::Unknown
        );
    }

    /// VFS-20 AC-1: log files are read oldest first, the active file last.
    #[test]
    fn logs_are_read_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        for (name, text) in [
            ("vfs.log", "c"),
            ("vfs_2026-11-02_00-00-00.log", "b"),
            ("vfs_2026-11-01_00-00-00.log", "a"),
            ("otro.txt", "x"),
        ] {
            std::fs::write(tmp.path().join(name), text).unwrap();
        }
        let got: Vec<String> = read_logs(tmp.path()).into_iter().map(|f| f.text).collect();
        assert_eq!(got, ["a", "b", "c"]);
        assert!(read_logs(&tmp.path().join("nada")).is_empty());
    }
}
