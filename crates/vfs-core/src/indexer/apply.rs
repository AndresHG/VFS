//! Diffs a directory listing against the index and writes the changes (VFS-1 AC-6, AC-7).
//! Owned by task T3a.

use rusqlite::Transaction;

use super::walk::Entry;
use crate::error::Result;
use crate::types::ScanSummary;

/// Applies `entries` (the current listing of `dir_id`, whose path relative to its root is
/// `rel_path`) through `db::ingest`, marks new subdirectories pending deep (and existing
/// ones too when `deep`), sets the directory clean, and returns the number of changed rows
/// for commit batching.
#[allow(dead_code)] // T3a wires this into the scan loop.
pub(crate) fn apply_listing(
    _tx: &Transaction,
    _root_id: i64,
    _dir_id: i64,
    _rel_path: &str,
    _deep: bool,
    _entries: &[Entry],
    _summary: &mut ScanSummary,
) -> Result<u64> {
    todo!("T3a")
}
