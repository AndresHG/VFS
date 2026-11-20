//! Filesystem watcher: events are hints that mark directories dirty (VFS-1 AC-7).
//! Owned by task T3b. macOS/Linux: `notify` (FSEvents), honouring its rescan flag. Windows:
//! own `ReadDirectoryChangesW` loop with a 64 KB buffer; 0 bytes or an error queues a re-walk
//! of the root and re-arms the watch.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::Result;

#[allow(dead_code)] // T3a consumes these events.
pub(crate) enum WatchEvent {
    /// Something changed at this absolute path; its parent (and itself, if a known directory)
    /// becomes pending shallow.
    Dirty(PathBuf),
    /// Events were lost for this root; queue a full re-walk.
    Rescan(i64),
}

/// Receives watcher events on the watcher's own thread(s); must not block.
pub(crate) type WatchSink = Arc<dyn Fn(WatchEvent) + Send + Sync>;

pub(crate) struct RootWatcher {
    _private: (),
}

#[allow(dead_code)] // T3a owns the watcher's lifetime.
impl RootWatcher {
    /// Creates a watcher that reports through `sink` and drops events under `ignore` (the
    /// app's data directory).
    pub(crate) fn start(_sink: WatchSink, _ignore: PathBuf) -> Result<RootWatcher> {
        todo!("T3b")
    }

    /// Watches `path` recursively, reporting events for root `root_id`.
    pub(crate) fn watch(&mut self, _root_id: i64, _path: &Path) -> Result<()> {
        todo!("T3b")
    }

    pub(crate) fn unwatch(&mut self, _root_id: i64) {
        todo!("T3b")
    }
}
