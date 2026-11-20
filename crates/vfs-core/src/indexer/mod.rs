//! Background indexer (VFS-1 AC-4..AC-8): scheduler thread, resume, scan loop.
//! Owned by task T3a together with `walk.rs` and `apply.rs`; `watch.rs` belongs to T3b.

mod apply;
mod walk;
mod watch;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::types::{IndexProgress, ScanSummary};
use crate::ProgressSink;

/// Command sender, join handle and shared progress of the `vfs-indexer` thread (T3a).
pub(crate) struct IndexerHandle {
    _private: (),
}

// T3a: `spawn` always fails until implemented, so the other methods are unreachable.
#[allow(dead_code)]
impl IndexerHandle {
    /// Starts the thread (background priority via `platform`, watcher via `watch`), which
    /// first finishes roots left in `removing`, then resumes pending directories. The
    /// watcher ignores the directory holding `db_path`.
    pub(crate) fn spawn(_db_path: PathBuf, _sink: Arc<dyn ProgressSink>) -> Result<IndexerHandle> {
        Err(Error::Unavailable("indexer".into()))
    }

    pub(crate) fn roots_changed(&self) {
        todo!("T3a")
    }

    pub(crate) fn rescan(&self, _root_id: Option<i64>) {
        todo!("T3a")
    }

    pub(crate) fn set_paused(&self, _paused: bool) {
        todo!("T3a")
    }

    pub(crate) fn progress(&self) -> IndexProgress {
        todo!("T3a")
    }

    pub(crate) fn shutdown(self) {
        todo!("T3a")
    }
}

/// Finishes roots left in `removing`, then processes pending directories on the calling
/// thread until none is left or `cancel` is set.
pub(crate) fn scan_to_completion(
    _conn: &mut Connection,
    _cancel: &AtomicBool,
) -> Result<ScanSummary> {
    Err(Error::Unavailable("indexer".into()))
}

/// Marks the directories containing `paths` pending shallow, never downgrading a pending
/// deep directory (`... WHERE id = ? AND scan_state = 0`).
pub(crate) fn mark_dirty(_conn: &mut Connection, _paths: &[PathBuf]) -> Result<()> {
    Err(Error::Unavailable("indexer".into()))
}
