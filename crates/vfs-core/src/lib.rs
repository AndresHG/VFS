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
pub mod link;
pub mod matching;
mod platform;
mod rank;
mod roots;
mod search;
mod suggest;
mod types;

pub use error::{Error, OverlapRelation, Result};
pub use fold::fold_key;
pub use indexer::{IndexerOptions, DIAG_TARGET};
pub use kind::file_kind;
pub use types::*;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, InterruptHandle};

use indexer::IndexerHandle;
use search::SearchState;

/// Gives the calling thread the indexer's walk profile: lowest CPU and low memory priority,
/// normal I/O, so its SQLite commits never run at very-low I/O priority (VFS-5 R1). For
/// readers of other sources (VFS-6 bookmarks).
pub fn lower_thread_priority() {
    use platform::ThreadPriority;
    platform::OsThreadPriority::default().walk_profile();
}

/// A diagnostic record (VFS-5 R7): ids, counts and durations only, never a name (R8).
fn diag(line: &str) {
    log::info!(target: DIAG_TARGET, "{line}");
}

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
    /// Searches started so far: a query sees whether a newer one interrupted it.
    searches: Arc<AtomicU64>,
    /// Searches waiting for the search lock: a suggestion holding it stops (VFS-13 R4).
    waiting: Arc<AtomicU64>,
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

    /// Opens or creates the index and applies migrations; a damaged index is moved to
    /// `index.db.bad` and rebuilt from its roots, and a session starts (`db::open_session`).
    /// Starts no thread.
    pub fn open_with(db_path: &Path, config: SearchConfig) -> Result<Vfs> {
        let (conn, _recovered, session) = db::open_session(db_path)?;
        drop(conn);
        let reader = db::open_ro(db_path)?;
        let search_conn = db::open_ro(db_path)?;
        let search_interrupt = search_conn.get_interrupt_handle();
        Ok(Vfs {
            inner: Arc::new(Inner {
                db_path: db_path.to_path_buf(),
                config,
                reader: Mutex::new(reader),
                searcher: Mutex::new((search_conn, SearchState::default())),
                search_interrupt,
                searches: Arc::new(AtomicU64::new(0)),
                waiting: Arc::new(AtomicU64::new(0)),
                indexer: Mutex::new(None),
                session: Mutex::new(Some(session)),
            }),
        })
    }

    /// Root changes are committed; a failed backup only weakens recovery (VFS-23 AC-1).
    fn backup_roots(&self, conn: &rusqlite::Connection) {
        if let Err(e) = db::sync_roots_backup(&self.inner.db_path, conn) {
            log::warn!("root list backup not written: {e}");
        }
    }

    fn notify_roots_changed(&self) {
        if let Some(h) = lock(&self.inner.indexer).as_ref() {
            h.roots_changed();
        }
    }

    pub fn list_roots(&self) -> Result<Vec<Root>> {
        roots::list_roots(&lock(&self.inner.reader))
    }

    /// The folders of a root that could not be listed, in path order, at most 500; the
    /// root's `unreadable` holds the full count (VFS-10 R4).
    pub fn unreadable_dirs(&self, root_id: i64) -> Result<Vec<UnreadableDir>> {
        roots::unreadable_dirs(&lock(&self.inner.reader), root_id)
    }

    /// `path` must be an existing directory that is not inside another root; roots inside
    /// it are taken in (VFS-31 R2).
    pub fn add_root(&self, path: &Path) -> Result<Root> {
        let started = std::time::Instant::now();
        let mut conn = db::open_rw(&self.inner.db_path)?;
        let (root, taken) = roots::add_root(&mut conn, path)?;
        self.backup_roots(&conn);
        self.notify_roots_changed();
        diag(&roots::add_record(root.id, taken, started.elapsed()));
        Ok(root)
    }

    /// Marks the root `removing` (search ignores it at once); the indexer deletes its rows
    /// and then sends a progress snapshot without it (VFS-31 R1).
    pub fn remove_root(&self, root_id: i64) -> Result<()> {
        let started = std::time::Instant::now();
        let mut conn = db::open_rw(&self.inner.db_path)?;
        roots::remove_root(&mut conn, root_id)?;
        self.backup_roots(&conn);
        self.notify_roots_changed();
        diag(&roots::remove_record(root_id, started.elapsed()));
        Ok(())
    }

    /// Pauses or resumes indexing of one root (VFS-32): stored, so it survives a restart;
    /// the running indexer leaves it at once. `NotFound` for an unknown root or one being
    /// removed.
    pub fn set_root_paused(&self, root_id: i64, paused: bool) -> Result<()> {
        let conn = db::open_rw(&self.inner.db_path)?;
        roots::set_root_paused(&conn, root_id, paused)?;
        if let Some(h) = lock(&self.inner.indexer).as_ref() {
            h.root_paused(root_id, paused);
        }
        Ok(())
    }

    /// Runs a query; a query still running on another thread is interrupted first.
    pub fn search(&self, query: &SearchQuery) -> Result<SearchResponse> {
        let this = self.inner.searches.fetch_add(1, Ordering::SeqCst) + 1;
        self.inner.waiting.fetch_add(1, Ordering::SeqCst);
        self.inner.search_interrupt.interrupt();
        let mut guard = lock(&self.inner.searcher);
        self.inner.waiting.fetch_sub(1, Ordering::SeqCst);
        let started = std::time::Instant::now();
        let (conn, state) = &mut *guard;
        match search::search(conn, state, &self.inner.config, query) {
            // An FTS table cut short while opening is an interruption only when a newer
            // query or the deadline caused one (VFS-3 AC-7); otherwise it is an error.
            Err(e)
                if search::is_table_open_cut(&e)
                    && (self.inner.searches.load(Ordering::SeqCst) != this
                        // The earliest bound a progress handler cuts at.
                        || started.elapsed() >= self.inner.config.deadline) =>
            {
                Ok(search::interrupted_response(query, started))
            }
            r => r,
        }
    }

    /// One search that finds something for a `query` that found nothing (VFS-13): without
    /// filters, a short word respelt, or a word dropped. Counts only, within
    /// `SearchConfig::suggest_budget`. Never interrupts a search: it waits for a running one;
    /// a newer search ends it with `None`, also when its interrupt came before a try's first
    /// statement, so that search never waits out the budget.
    pub fn suggest(&self, query: &SearchQuery) -> Result<Option<Suggestion>> {
        let cancel = search::Cancel::new(&self.inner.searches, &self.inner.waiting);
        let mut guard = lock(&self.inner.searcher);
        let (conn, _) = &mut *guard;
        match suggest::suggest(conn, &self.inner.config, query, &cancel) {
            // A try cut while an FTS table opened (a newer search, or its budget) counts as
            // finding nothing (R4), as `search` reads the same cut as an interruption.
            Err(e) if search::is_table_open_cut(&e) => Ok(None),
            r => r,
        }
    }

    /// Absolute path of an indexed file, built from the original names.
    pub fn file_path(&self, file_id: i64) -> Result<PathBuf> {
        db::file_abs_path(&lock(&self.inner.reader), file_id)
    }

    /// Kind and target of an indexed shortcut or link (VFS-18), from the index only.
    pub fn link_target(&self, file_id: i64) -> Result<LinkTarget> {
        db::link_of(&lock(&self.inner.reader), file_id)
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

    /// Starts the background indexer and watcher with the default options. Idempotent.
    pub fn start_indexer(&self, sink: Arc<dyn ProgressSink>) -> Result<()> {
        self.start_indexer_with(sink, IndexerOptions::default())
    }

    /// Starts the background indexer and watcher. Idempotent.
    pub fn start_indexer_with(
        &self,
        sink: Arc<dyn ProgressSink>,
        options: IndexerOptions,
    ) -> Result<()> {
        let mut slot = lock(&self.inner.indexer);
        if slot.is_none() {
            *slot = Some(IndexerHandle::spawn(
                self.inner.db_path.clone(),
                sink,
                options,
            )?);
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

    /// Whether the default folder exclusions are turned off: «Incluir carpetas del sistema y
    /// de programas» (VFS-33 R9). Off unless the user turned it on.
    pub fn include_all_folders(&self) -> Result<bool> {
        indexer::include_all(&lock(&self.inner.reader))
    }

    /// Turns «Incluir carpetas del sistema y de programas» on or off and, when it changes,
    /// queues a full walk of every root so the index follows (VFS-33 R8).
    pub fn set_include_all_folders(&self, on: bool) -> Result<()> {
        let conn = db::open_rw(&self.inner.db_path)?;
        if indexer::include_all(&conn)? == on {
            return Ok(());
        }
        indexer::set_include_all(&conn, on)?;
        drop(conn);
        match self.rescan(None) {
            Err(Error::Unavailable(_)) | Ok(()) => Ok(()),
            Err(e) => Err(e),
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

    /// VFS-13 AC-5: a newer search that started before the first try ends a suggestion with
    /// none, and so does a search that started just before it but waits for the lock; with no
    /// newer search the same query gets one. One that starts while a try runs is
    /// `search::tests::a_newer_search_stops_a_count_without_an_interrupt`.
    #[test]
    fn a_newer_search_ends_a_suggestion() {
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
        let f = db::ingest::NewFile {
            name: "factura luz.pdf",
            size: 1,
            mtime: 1,
            lossy: false,
        };
        db::ingest::insert_file(&tx, top, &f).unwrap();
        tx.commit().unwrap();
        let q = SearchQuery {
            text: "factura".into(),
            filters: SearchFilters {
                kinds: vec![FileKind::Photo],
                ..SearchFilters::default()
            },
            limit: 1,
            offset: 0,
            sort: SortOrder::Relevance,
        };
        let mut guard = lock(&vfs.inner.searcher);
        let (conn, _) = &mut *guard;
        let config = SearchConfig::default();
        let cancel = search::Cancel::new(&vfs.inner.searches, &vfs.inner.waiting);
        let found = suggest::suggest(conn, &config, &q, &cancel).unwrap();
        assert_eq!(found.map(|s| s.kind), Some(SuggestionKind::DropFilters));
        vfs.inner.searches.fetch_add(1, Ordering::SeqCst);
        assert_eq!(suggest::suggest(conn, &config, &q, &cancel).unwrap(), None);
        // The search bumped the counter before this suggestion read it, then lost the lock.
        let cancel = search::Cancel::new(&vfs.inner.searches, &vfs.inner.waiting);
        vfs.inner.waiting.fetch_add(1, Ordering::SeqCst);
        assert_eq!(suggest::suggest(conn, &config, &q, &cancel).unwrap(), None);
        vfs.inner.waiting.fetch_sub(1, Ordering::SeqCst);
    }

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

    /// VFS-3 AC-1: adding a folder that is already a root is told apart from the other
    /// overlaps, so `vfs index` and `vfs run` resume an indexed folder instead of failing.
    #[test]
    fn same_root_is_told_apart_from_other_overlaps() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("archivo");
        std::fs::create_dir_all(tree.join("sub")).unwrap();
        let vfs = Vfs::open(&dir.path().join("index.db")).unwrap();
        vfs.add_root(&tree).unwrap();
        assert!(vfs.add_root(&tree).unwrap_err().is_same_root());
        let inside = vfs.add_root(&tree.join("sub")).unwrap_err();
        assert_eq!(inside.code(), "overlapping_root");
        assert!(!inside.is_same_root());
        // A containing folder is not an overlap: it takes the root in (VFS-31 R2).
    }

    /// VFS-23 AC-1 (issue 58): the backup root list follows every root change, and an index
    /// whose header is overwritten or which is truncated to nothing (no root readable) is
    /// rebuilt with the roots from that backup.
    #[test]
    fn destroyed_header_keeps_roots_from_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        // A path may hold a newline on macOS, never a NUL (the backup's separator).
        let second = if cfg!(unix) { "dos\nlinea" } else { "dos" };
        let trees: Vec<_> = ["uno", second, "tres"]
            .iter()
            .map(|n| {
                let t = dir.path().join(n);
                std::fs::create_dir(&t).unwrap();
                t
            })
            .collect();
        let vfs = Vfs::open(&path).unwrap();
        let ids: Vec<i64> = trees.iter().map(|t| vfs.add_root(t).unwrap().id).collect();
        vfs.remove_root(ids[2]).unwrap();
        let paths = |vfs: &Vfs| -> Vec<String> {
            let roots = vfs.list_roots().unwrap().into_iter();
            roots
                .filter(|r| r.status != RootStatus::Removing)
                .map(|r| r.path)
                .collect()
        };
        let expected = paths(&vfs);
        assert_eq!(expected.len(), 2);
        assert_eq!(db::read_roots_backup(&path), Some(expected.clone()));
        drop(vfs);
        for wreck in ["header", "truncate"] {
            let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            if wreck == "header" {
                use std::io::Write;
                (&f).write_all(&[0xA5; 4096]).unwrap();
            } else {
                f.set_len(0).unwrap();
            }
            drop(f);
            for suffix in ["-wal", "-shm"] {
                let _ = std::fs::remove_file(db::with_suffix(&path, suffix));
            }
            let vfs = Vfs::open(&path).unwrap();
            assert_eq!(paths(&vfs), expected, "{wreck}");
        }
    }

    /// VFS-23 AC-1: a backup that cannot be written never fails the root change, and the next
    /// open repairs it.
    #[test]
    fn failed_backup_write_is_repaired_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let tree = dir.path().join("archivo");
        std::fs::create_dir(&tree).unwrap();
        let vfs = Vfs::open(&path).unwrap();
        // A folder where the backup goes makes its rename fail on every platform.
        let blocker = db::with_suffix(&path, ".roots");
        std::fs::remove_file(&blocker).unwrap(); // the open wrote an empty list
        std::fs::create_dir(&blocker).unwrap();
        let root = vfs.add_root(&tree).unwrap();
        assert_eq!(db::read_roots_backup(&path), None);
        drop(vfs);
        std::fs::remove_dir(&blocker).unwrap();
        let _vfs = Vfs::open(&path).unwrap();
        assert_eq!(db::read_roots_backup(&path), Some(vec![root.path]));
    }

    /// VFS-3 AC-7: an FTS table that fails to open with no newer query and no deadline
    /// behind it is an error, never taken for an interruption.
    #[test]
    fn real_fts_failure_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let mut conn = db::open_rw(&path).unwrap();
        let tx = conn.transaction().unwrap();
        let root = db::ingest::insert_root(&tx, "/", "/datos").unwrap();
        let top: i64 = tx
            .query_row("SELECT id FROM dirs WHERE root_id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        let file = db::ingest::NewFile {
            name: "archivo notaria.pdf",
            size: 1,
            mtime: 1,
            lossy: false,
        };
        db::ingest::insert_file(&tx, top, &file).unwrap();
        tx.commit().unwrap();
        // Without its config table FTS5 cannot open: "vtable constructor failed".
        conn.execute_batch("DROP TABLE files_fts_config").unwrap();
        drop(conn);
        let vfs = Vfs::open(&path).unwrap();
        let r = vfs.search(&SearchQuery {
            text: "notaria".into(),
            filters: SearchFilters::default(),
            limit: 10,
            offset: 0,
            sort: Default::default(),
        });
        assert!(
            matches!(&r, Err(e) if search::is_table_open_cut(e)),
            "{r:?}"
        );
    }

    /// VFS-32 AC-2, AC-4, AC-5: a pause survives closing and reopening the index and holds
    /// the walk until resumed; files already indexed stay searchable while paused; an
    /// unknown root is `NotFound`, and removing a paused root deletes its pause key.
    #[test]
    fn root_pause_persists_and_keeps_search() {
        let dir = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir(tree.path().join("A")).unwrap();
        std::fs::write(tree.path().join("A/notaria.pdf"), b"x").unwrap();
        let path = dir.path().join("index.db");
        let cancel = AtomicBool::new(false);
        let vfs = Vfs::open(&path).unwrap();
        let id = vfs.add_root(tree.path()).unwrap().id;
        assert!(matches!(
            vfs.set_root_paused(id + 1, true),
            Err(Error::NotFound(_))
        ));
        vfs.set_root_paused(id, true).unwrap();
        vfs.shutdown();
        drop(vfs);

        let vfs = Vfs::open(&path).unwrap();
        assert!(vfs.list_roots().unwrap()[0].paused, "paused after reopen");
        vfs.scan_to_completion(&cancel).unwrap();
        let root = &vfs.list_roots().unwrap()[0];
        assert_eq!((root.status, root.files), (RootStatus::Pending, 0));

        vfs.set_root_paused(id, false).unwrap();
        vfs.scan_to_completion(&cancel).unwrap();
        let root = &vfs.list_roots().unwrap()[0];
        assert_eq!((root.status, root.paused), (RootStatus::Ready, false));

        vfs.set_root_paused(id, true).unwrap();
        let q = SearchQuery {
            text: "notaria".into(),
            filters: SearchFilters::default(),
            limit: 10,
            offset: 0,
            sort: Default::default(),
        };
        assert_eq!(
            vfs.search(&q).unwrap().hits.len(),
            1,
            "searchable while paused"
        );

        vfs.remove_root(id).unwrap();
        vfs.scan_to_completion(&cancel).unwrap();
        let conn = db::open_ro(&path).unwrap();
        let keys: i64 = conn
            .query_row(
                concat!(
                    "SELECT COUNT(*) FROM meta WHERE key LIKE '",
                    crate::roots::paused_prefix!(),
                    "%'"
                ),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(keys, 0, "the pause key went with the root");
        vfs.shutdown();
    }

    /// The facade is shareable across threads (Tauri state, indexer callbacks).
    #[test]
    fn vfs_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + Clone>() {}
        assert_send_sync::<Vfs>();
    }
}
