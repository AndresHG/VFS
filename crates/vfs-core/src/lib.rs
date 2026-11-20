//! VFS core: index schema, folding, search, roots and the background indexer (VFS-1).
//! No UI dependency; used by `vfs-cli` and the Tauri app through the [`Vfs`] facade.

pub mod api;
pub mod content;
pub mod db;
mod error;
mod fold;
mod fuzzy;
mod indexer;
mod kind;
mod platform;
mod rank;
mod roots;
mod search;
mod types;

pub use error::{Error, Result};
pub use fold::fold_key;
pub use kind::file_kind;
pub use types::*;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, InterruptHandle};

use indexer::IndexerHandle;
use search::SearchState;

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
    config: SearchConfig,
    /// Reader for roots, stats and paths.
    reader: Mutex<Connection>,
    /// Dedicated search reader, interrupted by each new query.
    searcher: Mutex<(Connection, SearchState)>,
    search_interrupt: InterruptHandle,
    indexer: Mutex<Option<IndexerHandle>>,
    /// Open session marker; dropped by `shutdown` or with the last clone (clean shutdown).
    session: Mutex<Option<db::Session>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Vfs {
    /// Opens or creates the index at `db_path` with the default [`SearchConfig`]. Starts no
    /// thread.
    pub fn open(db_path: &Path) -> Result<Vfs> {
        Vfs::open_with(db_path, SearchConfig::default())
    }

    /// Opens or creates the index and applies migrations; an unreadable index is moved to
    /// `index.db.bad` and rebuilt from its roots, and a session starts (`db::open_session`).
    /// Starts no thread.
    pub fn open_with(db_path: &Path, config: SearchConfig) -> Result<Vfs> {
        let (conn, _recovered, session) = db::open_session(db_path)?;
        drop(conn);
        let reader = db::open_ro(db_path)?;
        let search_conn = db::open_search(db_path)?;
        let search_interrupt = search_conn.get_interrupt_handle();
        Ok(Vfs {
            inner: Arc::new(Inner {
                db_path: db_path.to_path_buf(),
                config,
                reader: Mutex::new(reader),
                searcher: Mutex::new((search_conn, SearchState::default())),
                search_interrupt,
                indexer: Mutex::new(None),
                session: Mutex::new(Some(session)),
            }),
        })
    }

    fn notify_roots_changed(&self) {
        if let Some(h) = lock(&self.inner.indexer).as_ref() {
            h.roots_changed();
        }
    }

    pub fn list_roots(&self) -> Result<Vec<Root>> {
        roots::list_roots(&lock(&self.inner.reader))
    }

    /// `path` must be an existing directory that neither contains nor is inside another root.
    pub fn add_root(&self, path: &Path) -> Result<Root> {
        let root = roots::add_root(&mut db::open_rw(&self.inner.db_path)?, path)?;
        self.notify_roots_changed();
        Ok(root)
    }

    pub fn remove_root(&self, root_id: i64) -> Result<()> {
        roots::remove_root(&mut db::open_rw(&self.inner.db_path)?, root_id)?;
        self.notify_roots_changed();
        Ok(())
    }

    /// Runs a query; a query still running on another thread is interrupted first.
    pub fn search(&self, query: &SearchQuery) -> Result<SearchResponse> {
        self.inner.search_interrupt.interrupt();
        let mut guard = lock(&self.inner.searcher);
        let (conn, state) = &mut *guard;
        search::search(conn, state, &self.inner.config, query)
    }

    /// Absolute path of an indexed file, built from the original names.
    pub fn file_path(&self, file_id: i64) -> Result<PathBuf> {
        db::file_abs_path(&lock(&self.inner.reader), file_id)
    }

    /// Absolute path of an indexed folder (a hit with `is_dir`).
    pub fn dir_path(&self, dir_id: i64) -> Result<PathBuf> {
        db::dir_abs_path(&lock(&self.inner.reader), dir_id)
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
        wal.push(db::WAL_SUFFIX);
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

    /// Stops the indexer after the current directory and joins the thread, then records a
    /// clean shutdown so the next open skips the integrity check (also done when the last
    /// clone is dropped).
    pub fn shutdown(&self) {
        if let Some(h) = lock(&self.inner.indexer).take() {
            h.shutdown();
        }
        drop(lock(&self.inner.session).take());
    }

    /// Foreground variant for the CLI and tests: processes pending directories on the calling
    /// thread until none is left or `cancel` is set. No watcher, no priority change.
    pub fn scan_to_completion(&self, cancel: &AtomicBool) -> Result<ScanSummary> {
        let mut conn = db::open_rw(&self.inner.db_path)?;
        indexer::scan_to_completion(&mut conn, cancel)
    }

    /// Marks the directories containing `paths` for a shallow re-list (what the watcher does)
    /// and wakes the running indexer to do it now.
    pub fn mark_dirty(&self, paths: &[PathBuf]) -> Result<()> {
        let mut conn = db::open_rw(&self.inner.db_path)?;
        indexer::mark_dirty(&mut conn, paths)?;
        if let Some(h) = lock(&self.inner.indexer).as_ref() {
            h.wake();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The facade opens a fresh index (current schema version, empty) and reports it.
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

    /// VFS-1 AC-7: without a running indexer, `rescan` is refused as `Unavailable` and
    /// `set_paused` is a no-op.
    #[test]
    fn indexer_controls_without_indexer() {
        let dir = tempfile::tempdir().unwrap();
        let vfs = Vfs::open(&dir.path().join("index.db")).unwrap();
        assert!(matches!(vfs.rescan(None), Err(Error::Unavailable(_))));
        assert!(matches!(vfs.rescan(Some(1)), Err(Error::Unavailable(_))));
        vfs.set_paused(true);
        let p = vfs.progress();
        assert_eq!(
            (p.state, p.files_indexed, p.roots.len()),
            (IndexerState::Idle, 0, 0)
        );
    }

    /// VFS-1 AC-5: stats count what is indexed.
    #[test]
    fn stats_count_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let vfs = Vfs::open(&path).unwrap();
        let mut conn = db::open_rw(&path).unwrap();
        let tx = conn.transaction().unwrap();
        let root = db::ingest::insert_root(&tx, "/", "/datos").unwrap();
        let top: i64 = tx
            .query_row("SELECT id FROM dirs WHERE root_id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        db::ingest::insert_dir(&tx, root, top, "a", 0).unwrap();
        for name in ["x.pdf", "y.jpg"] {
            let file = db::ingest::NewFile {
                name,
                size: 1,
                mtime: 1,
                lossy: false,
            };
            db::ingest::insert_file(&tx, top, &file).unwrap();
        }
        tx.commit().unwrap();
        let s = vfs.stats().unwrap();
        assert_eq!((s.files, s.dirs, s.roots), (2, 2, 1));
    }

    /// VFS-1 AC-4: every root gets a non-empty volume key, the same for folders on one volume.
    #[test]
    fn volume_key_is_stable_per_volume() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let ka = platform::volume_key(&a);
        assert!(!ka.is_empty());
        assert_eq!(ka, platform::volume_key(&b));
        assert_eq!(ka, platform::volume_key(&a));
    }

    /// VFS-1 AC-6: opening over a garbage file recovers to a fresh index and keeps the bad
    /// file as `index.db.bad`.
    #[test]
    fn open_recovers_garbage_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        std::fs::write(&path, b"this is not a sqlite database, just garbage bytes").unwrap();
        let vfs = Vfs::open(&path).unwrap();
        assert_eq!(vfs.stats().unwrap().schema_version, db::SCHEMA_VERSION);
        assert!(dir.path().join("index.db.bad").exists());
    }

    /// VFS-1 AC-13: paths of files and folders are rebuilt from the original names.
    #[test]
    fn file_and_dir_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let vfs = Vfs::open(&path).unwrap();
        let mut conn = db::open_rw(&path).unwrap();
        let tx = conn.transaction().unwrap();
        let root = db::ingest::insert_root(&tx, "/", "/datos").unwrap();
        let top: i64 = tx
            .query_row("SELECT id FROM dirs WHERE root_id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        let sub = db::ingest::insert_dir(&tx, root, top, "Año 2019", 0).unwrap();
        let file = db::ingest::insert_file(
            &tx,
            sub,
            &db::ingest::NewFile {
                name: "Notaría.pdf",
                size: 1,
                mtime: 1,
                lossy: false,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            vfs.dir_path(sub).unwrap(),
            Path::new("/datos").join("Año 2019")
        );
        assert_eq!(
            vfs.file_path(file).unwrap(),
            Path::new("/datos").join("Año 2019").join("Notaría.pdf")
        );
        assert!(matches!(vfs.file_path(sub + 100), Err(Error::NotFound(_))));
    }

    /// VFS-1 AC-6: a session marker exists while the index is open and is removed by
    /// `shutdown()` or by dropping the last clone, so the next open skips the integrity check.
    #[test]
    fn session_marker_ends_on_shutdown_and_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let marker = db::session_dir(&path);

        let vfs = Vfs::open(&path).unwrap();
        assert!(marker.exists());
        vfs.shutdown();
        assert!(!marker.exists());
        drop(vfs);

        let vfs = Vfs::open(&path).unwrap();
        let clone = vfs.clone();
        drop(vfs);
        assert!(marker.exists(), "a clone keeps the session open");
        drop(clone);
        assert!(!marker.exists());
    }

    /// VFS-1 AC-6: two handles on one index; closing one keeps the other's session marker
    /// and does not make the next open look like a crash recovery.
    #[test]
    fn two_handles_share_an_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let marker = db::session_dir(&path);
        let app = Vfs::open(&path).unwrap();
        let cli = Vfs::open(&path).unwrap();
        cli.shutdown();
        assert!(marker.exists(), "the app's session is still open");
        let (probe, unclean) = db::begin_session(&path).unwrap();
        assert!(
            !unclean,
            "the app's live session is not taken for a crashed one"
        );
        drop(probe);
        assert!(marker.exists());
        drop(app);
        assert!(!marker.exists());
    }

    /// The facade is shareable across threads (Tauri state, indexer callbacks).
    #[test]
    fn vfs_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + Clone>() {}
        assert_send_sync::<Vfs>();
    }
}
