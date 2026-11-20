//! VFS core: index schema, folding, search, roots and the background indexer (VFS-1).
//! No UI dependency; used by `vfs-cli` and the Tauri app through the [`Vfs`] facade.

pub mod api;
pub mod content;
pub mod db;
mod error;
mod fold;
mod indexer;
mod kind;
mod platform;
mod types;

pub use error::{Error, Result};
pub use fold::fold_key;
pub use kind::file_kind;
pub use types::*;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::Connection;

use indexer::IndexerHandle;

/// Receives indexer progress; implemented by the app (Tauri events) and the CLI.
pub trait ProgressSink: Send + Sync + 'static {
    fn on_progress(&self, progress: &IndexProgress);
    fn on_index_changed(&self) {}
}

/// Handle to one index. Cheap to clone; all clones share the connections and the indexer.
#[derive(Clone)]
pub struct Vfs {
    inner: Arc<Inner>,
}

struct Inner {
    db_path: PathBuf,
    /// Reader connection used by search and queries.
    reader: Mutex<Connection>,
    indexer: Mutex<Option<IndexerHandle>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Vfs {
    /// Opens or creates the index at `db_path` and applies migrations. Starts no thread.
    pub fn open(db_path: &Path) -> Result<Vfs> {
        drop(db::open_rw(db_path)?);
        let reader = db::open_ro(db_path)?;
        Ok(Vfs {
            inner: Arc::new(Inner {
                db_path: db_path.to_path_buf(),
                reader: Mutex::new(reader),
                indexer: Mutex::new(None),
            }),
        })
    }

    pub fn list_roots(&self) -> Result<Vec<Root>> {
        todo!("T2: roots.rs")
    }

    /// `path` must be an existing directory that neither contains nor is inside another root.
    pub fn add_root(&self, _path: &Path) -> Result<Root> {
        todo!("T2: roots.rs (writes through a short-lived db::open_rw, then notifies the indexer)")
    }

    pub fn remove_root(&self, _root_id: i64) -> Result<()> {
        todo!("T2: roots.rs (sets status removing, then notifies the indexer)")
    }

    pub fn search(&self, _query: &SearchQuery) -> Result<SearchResponse> {
        todo!("T2: search.rs")
    }

    /// Absolute path of an indexed file, built from the original names.
    pub fn file_path(&self, _file_id: i64) -> Result<PathBuf> {
        todo!("T2: search.rs")
    }

    pub fn stats(&self) -> Result<IndexStats> {
        let conn = lock(&self.inner.reader);
        let count = |sql: &str| -> Result<u64> {
            Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64)
        };
        let files = count("SELECT COUNT(*) FROM files")?;
        let dirs = count("SELECT COUNT(*) FROM dirs")?;
        let roots = count("SELECT COUNT(*) FROM roots")? as u32;
        let schema_version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let mut wal = self.inner.db_path.clone().into_os_string();
        wal.push("-wal");
        let db_bytes = [self.inner.db_path.as_os_str(), wal.as_os_str()]
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();
        Ok(IndexStats {
            files,
            dirs,
            roots,
            db_bytes,
            schema_version,
        })
    }

    /// Starts the background indexer and watcher. Idempotent.
    pub fn start_indexer(&self, sink: Arc<dyn ProgressSink>) -> Result<()> {
        let mut slot = lock(&self.inner.indexer);
        if slot.is_none() {
            *slot = Some(IndexerHandle::spawn(self.inner.db_path.clone(), sink)?);
        }
        Ok(())
    }

    /// Queues a full re-walk of one root, or of all roots with `None`. Fails with
    /// `Unavailable` when the indexer is not running.
    pub fn rescan(&self, root_id: Option<i64>) -> Result<()> {
        match lock(&self.inner.indexer).as_ref() {
            Some(h) => {
                h.rescan(root_id);
                Ok(())
            }
            None => Err(Error::Unavailable("indexer not running".into())),
        }
    }

    pub fn set_paused(&self, paused: bool) {
        if let Some(h) = lock(&self.inner.indexer).as_ref() {
            h.set_paused(paused);
        }
    }

    /// Progress of the running indexer, or an idle snapshot with no roots when it is not
    /// running.
    pub fn progress(&self) -> IndexProgress {
        match lock(&self.inner.indexer).as_ref() {
            Some(h) => h.progress(),
            None => IndexProgress {
                state: IndexerState::Idle,
                files_indexed: 0,
                roots: Vec::new(),
            },
        }
    }

    /// Stops the indexer after the current directory and joins the thread.
    pub fn shutdown(&self) {
        if let Some(h) = lock(&self.inner.indexer).take() {
            h.shutdown();
        }
    }

    /// Foreground variant for the CLI and tests: processes pending directories on the calling
    /// thread until none is left or `cancel` is set. No watcher, no priority change.
    pub fn scan_to_completion(&self, cancel: &AtomicBool) -> Result<ScanSummary> {
        let mut conn = db::open_rw(&self.inner.db_path)?;
        indexer::scan_to_completion(&mut conn, cancel)
    }

    /// Marks the directories containing `paths` for a shallow re-list (what the watcher does).
    pub fn mark_dirty(&self, paths: &[PathBuf]) -> Result<()> {
        let mut conn = db::open_rw(&self.inner.db_path)?;
        indexer::mark_dirty(&mut conn, paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-1 AC-15: the facade opens a fresh index (schema version 1, empty) and reports it.
    #[test]
    fn open_and_stats_on_empty_index() {
        let dir = tempfile::tempdir().unwrap();
        let vfs = Vfs::open(&dir.path().join("index.db")).unwrap();
        let stats = vfs.stats().unwrap();
        assert_eq!((stats.files, stats.dirs, stats.roots), (0, 0, 0));
        assert_eq!(stats.schema_version, db::SCHEMA_VERSION);
        assert!(stats.db_bytes > 0);
        assert_eq!(vfs.progress().state, IndexerState::Idle);
        vfs.shutdown();
    }

    /// VFS-1 AC-5: the facade is shareable across threads (Tauri state, indexer callbacks).
    #[test]
    fn vfs_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + Clone>() {}
        assert_send_sync::<Vfs>();
    }
}
