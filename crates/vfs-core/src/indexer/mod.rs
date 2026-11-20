//! Background indexer (VFS-1 AC-4..AC-8). Placeholder signatures; the bodies belong to
//! task T3, which owns this directory and `platform.rs`.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::types::{IndexProgress, ScanSummary};
use crate::ProgressSink;

/// Command sender, join handle and shared progress of the `vfs-indexer` thread (T3).
pub(crate) struct IndexerHandle {
    _private: (),
}

// T3: `spawn` always fails until implemented, so the other methods are unreachable.
#[allow(dead_code)]
impl IndexerHandle {
    pub(crate) fn spawn(_db_path: PathBuf, _sink: Arc<dyn ProgressSink>) -> Result<IndexerHandle> {
        Err(Error::Unavailable("indexer".into()))
    }

    pub(crate) fn roots_changed(&self) {
        todo!("T3")
    }

    pub(crate) fn rescan(&self, _root_id: Option<i64>) {
        todo!("T3")
    }

    pub(crate) fn set_paused(&self, _paused: bool) {
        todo!("T3")
    }

    pub(crate) fn progress(&self) -> IndexProgress {
        todo!("T3")
    }

    pub(crate) fn shutdown(self) {
        todo!("T3")
    }
}

/// Processes pending directories on the calling thread until none is left or `cancel` is set.
pub(crate) fn scan_to_completion(
    _conn: &mut Connection,
    _cancel: &AtomicBool,
) -> Result<ScanSummary> {
    Err(Error::Unavailable("indexer".into()))
}

/// Marks the directories containing `paths` pending shallow.
pub(crate) fn mark_dirty(_conn: &mut Connection, _paths: &[PathBuf]) -> Result<()> {
    Err(Error::Unavailable("indexer".into()))
}
