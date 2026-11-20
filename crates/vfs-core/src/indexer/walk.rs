//! Listing one directory without opening files, with the exclusion list (VFS-1 AC-4, AC-8).
//! Owned by task T3a.

use std::path::Path;

#[allow(dead_code)] // T3a wires the walker into the scan loop.
pub(crate) enum EntryKind {
    File { size: u64, mtime: i64 },
    Dir { mtime: i64 },
}

#[allow(dead_code)]
pub(crate) struct Entry {
    /// `to_str()` of the name, or its lossy form when not valid Unicode.
    pub name: String,
    pub lossy: bool,
    pub kind: EntryKind,
}

/// System and junk names never indexed (`$RECYCLE.BIN`, `.DS_Store`, `._*`, `~$*`, ...).
#[allow(dead_code)]
pub(crate) fn is_excluded(_name: &str) -> bool {
    todo!("T3a")
}

/// Entries of `dir` from `read_dir` metadata; symlinks, junctions and reparse points are
/// skipped, as are excluded names and `skip` (the app's data directory).
#[allow(dead_code)]
pub(crate) fn list_dir(_dir: &Path, _skip: Option<&Path>) -> std::io::Result<Vec<Entry>> {
    todo!("T3a")
}
