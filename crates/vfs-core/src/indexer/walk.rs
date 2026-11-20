//! Listing one directory without opening files, with the exclusion list (VFS-1 AC-4, AC-8).
//! Owned by task T3a.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) enum EntryKind {
    File { size: u64, mtime: i64 },
    Dir { mtime: i64 },
}

pub(crate) struct Entry {
    /// `to_str()` of the name, or its lossy form when not valid Unicode.
    pub name: String,
    pub lossy: bool,
    pub kind: EntryKind,
}

/// Exact names (compared ignoring ASCII case) that are never indexed, nor watched.
const EXCLUDED: &[&str] = &[
    "$RECYCLE.BIN",
    "System Volume Information",
    ".Trashes",
    ".Trash",
    ".Spotlight-V100",
    ".fseventsd",
    ".DocumentRevisions-V100",
    ".TemporaryItems",
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    // Synology Drive client and DSM metadata, recycle bin and snapshots.
    ".SynologyWorkingDirectory",
    "@eaDir",
    "#recycle",
    "#snapshot",
];

/// System, sync-client and temporary names never indexed, and whose watcher events are
/// dropped (`$RECYCLE.BIN`, `.DS_Store`, `._*`, `~$*`, `.~lock.*#`, `~*.tmp`, ...). The one
/// list both the walker and the watcher use (VFS-1 I8).
pub(crate) fn is_excluded(name: &str) -> bool {
    let tmp_suffix = name.len() >= 4
        && name.is_char_boundary(name.len() - 4)
        && name[name.len() - 4..].eq_ignore_ascii_case(".tmp");
    name.starts_with("._")
        || name.starts_with("~$")
        || (name.starts_with('~') && tmp_suffix)
        || (name.starts_with(".~lock.") && name.ends_with('#'))
        || EXCLUDED.iter().any(|e| e.eq_ignore_ascii_case(name))
}

/// Unix seconds of a timestamp, floored (negative before 1970, so 1969-12-31 23:59:59.5
/// is -1, not 0); 0 when the platform has none.
pub(crate) fn unix_secs(t: std::io::Result<SystemTime>) -> i64 {
    match t {
        Ok(t) => match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => {
                let d = e.duration();
                -(d.as_secs() as i64) - i64::from(d.subsec_nanos() > 0)
            }
        },
        Err(_) => 0,
    }
}

/// True when `root` or a directory of `rel` below it (`/`-separated) is now a symbolic link,
/// junction or other name-surrogate reparse point (`FileType::is_symlink`). An indexed
/// directory replaced by one is treated as gone: the walk never follows a link, even where a
/// real directory used to be.
pub(crate) fn behind_link(root: &Path, rel: &str) -> bool {
    let is_link = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink());
    let mut p = root.to_path_buf();
    if is_link(&p) {
        return true;
    }
    for c in rel.split('/').filter(|c| !c.is_empty()) {
        p.push(c);
        if is_link(&p) {
            return true;
        }
    }
    false
}

/// Entries of `dir` from `read_dir` metadata; symlinks, junctions and reparse points are
/// skipped, as are excluded names and `skip` (the app's data directory).
///
/// No file is opened: on Windows type and metadata come from the enumeration record, on
/// Unix from `lstat`. `FileType::is_symlink` is true for symlinks and for Windows
/// name-surrogate reparse points (junctions, mount points), so those are never followed;
/// cloud placeholders are not name surrogates and are listed from their metadata without
/// being downloaded. Entries that vanish or fail while listing are skipped, except that an
/// entry whose metadata is denied (a Unix folder readable but not searchable) fails the
/// whole listing, so the folder is retried like an unlistable one instead of being marked
/// clean without those entries.
pub(crate) fn list_dir(dir: &Path, skip: Option<&Path>) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let os_name = entry.file_name();
        let (name, lossy) = match os_name.to_str() {
            Some(s) => (s.to_owned(), false),
            None => (os_name.to_string_lossy().into_owned(), true),
        };
        if is_excluded(&name) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Err(e),
            Err(_) => continue,
        };
        let mtime = unix_secs(meta.modified());
        let kind = if file_type.is_dir() {
            if skip.is_some_and(|s| entry.path() == s) {
                continue;
            }
            EntryKind::Dir { mtime }
        } else if file_type.is_file() {
            EntryKind::File {
                size: meta.len(),
                mtime,
            }
        } else {
            continue;
        };
        out.push(Entry { name, lossy, kind });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-1 AC-4, I8: system, sync-client and temporary names are excluded, ordinary names
    /// are not.
    #[test]
    fn exclusions() {
        for n in [
            "$RECYCLE.BIN",
            "$Recycle.Bin",
            "System Volume Information",
            ".DS_Store",
            "thumbs.db",
            "Desktop.ini",
            "._IMG_0001.JPG",
            "~$informe.docx",
            ".fseventsd",
            ".SynologyWorkingDirectory",
            "@eaDir",
            "#recycle",
            "#Snapshot",
            ".~lock.informe.odt#",
            "~WRL0001.tmp",
            "~DF1234.TMP",
        ] {
            assert!(is_excluded(n), "{n}");
        }
        for n in [
            "Notaría.pdf",
            ".bashrc",
            "Trash",
            "informe~$.docx",
            "_x",
            "recycle",
            "copia.tmp",
            "~notas.txt",
            ".~lock.informe.odt",
        ] {
            assert!(!is_excluded(n), "{n}");
        }
    }
}
