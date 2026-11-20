//! Diffs a directory listing against the index and writes the changes (VFS-1 AC-6, AC-7).
//! Owned by task T3a.

use std::collections::{HashMap, HashSet};

use rusqlite::Transaction;

use super::walk::{Entry, EntryKind};
use crate::db::ingest::{self, NewFile, ScanState};
use crate::error::Result;
use crate::types::ScanSummary;

/// Applies `entries` (the current listing of `dir_id`, whose path relative to its root is
/// `rel_path`) through `db::ingest`, marks new subdirectories pending deep (and existing
/// ones too when `deep`), sets the directory clean, and returns the number of changed rows
/// for commit batching.
///
/// Children are matched by exact name, so a rename or a move is a delete plus an insert
/// (a moved folder is re-walked at its new place). Two on-disk names with the same lossy
/// form keep only the first.
pub(crate) fn apply_listing(
    tx: &Transaction,
    root_id: i64,
    dir_id: i64,
    rel_path: &str,
    deep: bool,
    entries: &[Entry],
    summary: &mut ScanSummary,
) -> Result<u64> {
    let (files, dirs) = ingest::list_children(tx, dir_id)?;
    let mut files: HashMap<String, ingest::ChildFile> =
        files.into_iter().map(|f| (f.name.clone(), f)).collect();
    let mut dirs: HashMap<String, ingest::ChildDir> =
        dirs.into_iter().map(|d| (d.name.clone(), d)).collect();
    let mut seen_files = HashSet::new();
    let mut seen_dirs = HashSet::new();
    let mut changed = 0u64;

    for entry in entries {
        match entry.kind {
            EntryKind::File { size, mtime } => {
                if !seen_files.insert(entry.name.as_str()) {
                    continue;
                }
                match files.remove(&entry.name) {
                    Some(old) => {
                        if old.size != size || old.mtime != mtime {
                            ingest::update_file(tx, old.id, size, mtime)?;
                            summary.files_updated += 1;
                            changed += 1;
                        }
                    }
                    None => {
                        let file = NewFile {
                            name: &entry.name,
                            size,
                            mtime,
                            lossy: entry.lossy,
                        };
                        ingest::insert_file(tx, dir_id, &file)?;
                        summary.files_added += 1;
                        changed += 1;
                    }
                }
            }
            EntryKind::Dir { mtime } => {
                if !seen_dirs.insert(entry.name.as_str()) {
                    continue;
                }
                match dirs.remove(&entry.name) {
                    Some(old) => {
                        if old.mtime != mtime {
                            ingest::set_dir_mtime(tx, old.id, mtime)?;
                            changed += 1;
                        }
                        if deep {
                            ingest::set_dir_state(tx, old.id, ScanState::PendingDeep)?;
                            changed += 1;
                        }
                    }
                    None => {
                        let child = if rel_path.is_empty() {
                            entry.name.clone()
                        } else {
                            format!("{rel_path}/{}", entry.name)
                        };
                        ingest::insert_dir(tx, root_id, dir_id, &child, mtime)?;
                        changed += 1;
                    }
                }
            }
        }
    }

    for gone in files.into_values() {
        ingest::delete_file(tx, gone.id)?;
        summary.files_removed += 1;
        changed += 1;
    }
    for gone in dirs.into_values() {
        let removed = ingest::delete_dir_subtree(tx, gone.id)?;
        summary.files_removed += removed;
        changed += 1 + removed;
    }
    ingest::set_dir_state(tx, dir_id, ScanState::Clean)?;
    summary.dirs_listed += 1;
    Ok(changed + 1)
}
