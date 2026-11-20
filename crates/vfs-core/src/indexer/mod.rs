//! Background indexer (VFS-1 AC-4..AC-8): scheduler thread, resume, scan loop.
//! Owned by task T3a together with `walk.rs` and `apply.rs`; `watch.rs` belongs to T3b.
//!
//! `dirs.scan_state` is the persistent work queue: listing a pending directory diffs it
//! against the index (`apply`), queues its new subdirectories, and marks it clean, all in
//! the current transaction. A first scan, a full re-walk (root directory pending deep) and a
//! watcher hint (one directory pending shallow) all run this same loop. Transactions commit
//! between directories every ~20,000 changed rows or 5 s, so a killed run redoes at most
//! the uncommitted directories.

mod apply;
mod walk;
mod watch;

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::db::{self, ingest};
use crate::error::Result;
use crate::types::{IndexProgress, IndexerState, RootProgress, RootStatus, ScanSummary};
use crate::{platform, ProgressSink};

/// Commit after this many changed rows (VFS-1 plan addendum I5)...
const BATCH_ROWS: u64 = 20_000;
/// ...or when the transaction is this old.
const BATCH_AGE: Duration = Duration::from_secs(5);
const PROGRESS_EVERY: Duration = Duration::from_millis(500);
const CHANGED_EVERY: Duration = Duration::from_secs(2);
/// Watcher hints are coalesced this long before their directories are marked.
const DIRTY_COALESCE: Duration = Duration::from_secs(2);
/// On start, a root whose last full scan is older than this is re-walked (events may have
/// been lost while the app was closed).
const STARTUP_REWALK_SECS: i64 = 24 * 3600;
/// While running, roots are re-walked when their last full scan is older than this.
const PERIODIC_REWALK_SECS: i64 = 7 * 24 * 3600;
const PERIODIC_CHECK: Duration = Duration::from_secs(3600);
const IDLE_WAIT: Duration = Duration::from_secs(60);

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn reachable(path: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_dir())
}

/// The directory holding the index, never indexed even when it lies inside a root.
fn data_dir(conn: &Connection) -> Option<PathBuf> {
    let dir = Path::new(conn.path()?).parent()?;
    if dir.as_os_str().is_empty() {
        return None;
    }
    Some(dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()))
}

fn begin(conn: &Connection) -> Result<Transaction<'_>> {
    Ok(Transaction::new_unchecked(
        conn,
        TransactionBehavior::Immediate,
    )?)
}

/// Per-root counters of the current pass, for progress.
#[derive(Default)]
struct Tally {
    dirs_done: u64,
    /// Files added minus removed since `roots.files_count` was last refreshed.
    files_delta: i64,
}

/// State of one pass: from the moment work appears until no directory is pending.
#[derive(Default)]
struct Pass {
    summary: ScanSummary,
    tally: HashMap<i64, Tally>,
    /// Root id and `rel_path` of the directory being listed.
    current: Option<(i64, String)>,
    /// Roots already finalized in this pass with no work since.
    finished: HashSet<i64>,
}

enum Outcome {
    Done,
    Yielded,
}

enum RootOutcome {
    Finished,
    Unavailable,
    Yielded,
}

struct ActiveRoot {
    id: i64,
    path: String,
    status: RootStatus,
}

/// Called after each directory with the connection (inside the open transaction unless
/// `committed`), the pass, and whether a commit just happened.
type Tick<'a> = &'a mut dyn FnMut(&Connection, &Pass, bool);

/// Finishes roots in `removing`, then lists pending directories, root by root, depth-first,
/// until none is left (`Done`) or `stop()` turns true between directories (`Yielded`, after
/// committing). With `one_batch`, also yields after each commit.
fn run(
    conn: &mut Connection,
    pass: &mut Pass,
    stop: &dyn Fn() -> bool,
    one_batch: bool,
    tick: Tick,
) -> Result<Outcome> {
    let conn: &Connection = conn;
    if !finish_removing(conn, stop)? {
        return Ok(Outcome::Yielded);
    }
    let skip = data_dir(conn);
    let roots: Vec<ActiveRoot> = {
        let mut stmt =
            conn.prepare("SELECT id, path, status FROM roots WHERE status <> 4 ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(ActiveRoot {
                id: r.get(0)?,
                path: r.get(1)?,
                status: RootStatus::from_code(r.get(2)?),
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for mut root in roots {
        if !reachable(&root.path) {
            if root.status != RootStatus::Unavailable {
                set_root_status(conn, root.id, RootStatus::Unavailable)?;
            }
            continue;
        }
        if root.status == RootStatus::Unavailable {
            // Back online: a full re-walk catches whatever changed while it was away.
            queue_rewalk(conn, Some(root.id))?;
            root.status = RootStatus::Scanning;
        }
        match scan_root(
            conn,
            &mut root,
            pass,
            stop,
            one_batch,
            skip.as_deref(),
            tick,
        )? {
            RootOutcome::Finished => finalize_root(conn, &root, pass)?,
            RootOutcome::Unavailable => {
                log::warn!("root {} became unreachable", root.path);
                set_root_status(conn, root.id, RootStatus::Unavailable)?;
            }
            RootOutcome::Yielded => return Ok(Outcome::Yielded),
        }
    }
    Ok(Outcome::Done)
}

fn scan_root(
    conn: &Connection,
    root: &mut ActiveRoot,
    pass: &mut Pass,
    stop: &dyn Fn() -> bool,
    one_batch: bool,
    skip: Option<&Path>,
    tick: Tick,
) -> Result<RootOutcome> {
    let mut tx = begin(conn)?;
    let mut rows = 0u64;
    let mut started = Instant::now();
    loop {
        if stop() {
            tx.commit()?;
            pass.current = None;
            return Ok(RootOutcome::Yielded);
        }
        let next: Option<(i64, String, i64)> = tx
            .prepare_cached(
                "SELECT id, rel_path, scan_state FROM dirs INDEXED BY dirs_pending \
                 WHERE root_id = ?1 AND scan_state <> 0 ORDER BY id DESC LIMIT 1",
            )?
            .query_row([root.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((dir_id, rel_path, state)) = next else {
            tx.commit()?;
            pass.current = None;
            return Ok(RootOutcome::Finished);
        };
        if root.status == RootStatus::Pending {
            set_root_status(&tx, root.id, RootStatus::Scanning)?;
            root.status = RootStatus::Scanning;
        }
        pass.finished.remove(&root.id);
        pass.current = Some((root.id, rel_path.clone()));
        let (added, removed) = (pass.summary.files_added, pass.summary.files_removed);
        let abs = db::join_rel(&root.path, &rel_path);
        let deep = state == ingest::ScanState::PendingDeep as i64;
        match walk::list_dir(&abs, skip) {
            Ok(entries) => {
                rows += apply::apply_listing(
                    &tx,
                    root.id,
                    dir_id,
                    &rel_path,
                    deep,
                    &entries,
                    &mut pass.summary,
                )?;
                if rel_path.is_empty() {
                    let mtime = walk::unix_secs(std::fs::metadata(&abs).and_then(|m| m.modified()));
                    ingest::set_dir_mtime(&tx, dir_id, mtime)?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if rel_path.is_empty() || !reachable(&root.path) {
                    tx.commit()?;
                    pass.current = None;
                    return Ok(RootOutcome::Unavailable);
                }
                // Gone since its parent was listed: drop it and everything below.
                let gone = ingest::delete_dir_subtree(&tx, dir_id)?;
                pass.summary.files_removed += gone;
                rows += 1 + gone;
            }
            Err(e) => {
                // Permission or I/O error: keep what is indexed, count it, move on.
                log::warn!("cannot list {}: {e}", abs.display());
                pass.summary.errors += 1;
                ingest::set_dir_state(&tx, dir_id, ingest::ScanState::Clean)?;
                rows += 1;
            }
        }
        let tally = pass.tally.entry(root.id).or_default();
        tally.dirs_done += 1;
        tally.files_delta += (pass.summary.files_added - added) as i64;
        tally.files_delta -= (pass.summary.files_removed - removed) as i64;

        if rows >= BATCH_ROWS || started.elapsed() >= BATCH_AGE {
            tx.commit()?;
            tick(conn, pass, true);
            if one_batch {
                pass.current = None;
                return Ok(RootOutcome::Yielded);
            }
            tx = begin(conn)?;
            rows = 0;
            started = Instant::now();
        } else {
            tick(&tx, pass, false);
        }
    }
}

/// Marks a root ready once nothing is pending: refreshes its counts and, at the end of a
/// full scan, stamps `last_full_scan_at` and merges the FTS segments once.
fn finalize_root(conn: &Connection, root: &ActiveRoot, pass: &mut Pass) -> Result<()> {
    let tally = pass.tally.entry(root.id).or_default();
    if pass.finished.contains(&root.id)
        || (root.status == RootStatus::Ready && tally.dirs_done == 0)
    {
        return Ok(());
    }
    let tx = begin(conn)?;
    refresh_counts(&tx, Some(root.id))?;
    if root.status == RootStatus::Ready {
        tx.execute("UPDATE roots SET status = 2 WHERE id = ?1", [root.id])?;
    } else {
        tx.execute(
            "UPDATE roots SET status = 2, last_full_scan_at = ?2 WHERE id = ?1",
            params![root.id, now_secs()],
        )?;
        tx.execute(
            "INSERT INTO files_fts(files_fts, rank) VALUES('merge', 500)",
            [],
        )?;
        tx.execute(
            "INSERT INTO dirs_fts(dirs_fts, rank) VALUES('merge', 500)",
            [],
        )?;
    }
    tx.commit()?;
    tally.files_delta = 0;
    pass.finished.insert(root.id);
    Ok(())
}

/// Recomputes `files_count` and `dirs_count` of one root, or of every root not ready.
fn refresh_counts(conn: &Connection, root_id: Option<i64>) -> Result<()> {
    conn.execute(
        "UPDATE roots SET \
           files_count = (SELECT COUNT(*) FROM files WHERE dir_id IN \
                            (SELECT id FROM dirs WHERE dirs.root_id = roots.id)), \
           dirs_count = (SELECT COUNT(*) FROM dirs WHERE dirs.root_id = roots.id) \
         WHERE (?1 IS NULL AND status <> 2) OR id = ?1",
        [root_id],
    )?;
    Ok(())
}

fn set_root_status(conn: &Connection, root_id: i64, status: RootStatus) -> Result<()> {
    conn.execute(
        "UPDATE roots SET status = ?2 WHERE id = ?1",
        params![root_id, status.code()],
    )?;
    Ok(())
}

/// Queues a full re-walk: the root directory becomes pending deep and the root scanning.
fn queue_rewalk(conn: &Connection, root_id: Option<i64>) -> Result<()> {
    conn.execute(
        "UPDATE dirs SET scan_state = 1 WHERE rel_path = '' AND root_id IN \
         (SELECT id FROM roots WHERE status <> 4 AND (?1 IS NULL OR id = ?1))",
        [root_id],
    )?;
    conn.execute(
        "UPDATE roots SET status = 1 WHERE status <> 4 AND (?1 IS NULL OR id = ?1)",
        [root_id],
    )?;
    Ok(())
}

/// Deletes the rows of roots in `removing` in batches, then the root rows (VFS-1 plan
/// addendum I6). Returns false when `stop()` interrupted it.
fn finish_removing(conn: &Connection, stop: &dyn Fn() -> bool) -> Result<bool> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM roots WHERE status = 4")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for root_id in ids {
        loop {
            if stop() {
                return Ok(false);
            }
            let tx = begin(conn)?;
            let dirs: Vec<i64> = {
                let mut stmt = tx.prepare("SELECT id FROM dirs WHERE root_id = ?1 LIMIT 1000")?;
                let rows = stmt.query_map([root_id], |r| r.get(0))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            if dirs.is_empty() {
                tx.execute("DELETE FROM roots WHERE id = ?1", [root_id])?;
                tx.commit()?;
                break;
            }
            let mut rows = 0u64;
            for dir_id in dirs {
                rows += tx.execute(
                    "DELETE FROM files_fts WHERE rowid IN (SELECT id FROM files WHERE dir_id = ?1)",
                    [dir_id],
                )? as u64;
                tx.execute("DELETE FROM files WHERE dir_id = ?1", [dir_id])?;
                tx.execute("DELETE FROM dirs_fts WHERE rowid = ?1", [dir_id])?;
                tx.execute("DELETE FROM dirs WHERE id = ?1", [dir_id])?;
                rows += 1;
                if rows >= BATCH_ROWS {
                    break;
                }
            }
            tx.commit()?;
        }
    }
    Ok(true)
}

fn build_progress(conn: &Connection, pass: &Pass, state: IndexerState) -> Result<IndexProgress> {
    let rows: Vec<(i64, String, i64, i64)> = {
        let mut stmt =
            conn.prepare_cached("SELECT id, path, status, files_count FROM roots ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut roots = Vec::with_capacity(rows.len());
    let mut total = 0u64;
    for (id, path, status, files_count) in rows {
        let pending: i64 = conn
            .prepare_cached(
                "SELECT COUNT(*) FROM dirs INDEXED BY dirs_pending \
                 WHERE root_id = ?1 AND scan_state <> 0",
            )?
            .query_row([id], |r| r.get(0))?;
        let tally = pass.tally.get(&id);
        let files = (files_count + tally.map_or(0, |t| t.files_delta)).max(0) as u64;
        total += files;
        let current_dir = match &pass.current {
            Some((r, rel)) if *r == id => {
                Some(db::join_rel(&path, rel).to_string_lossy().into_owned())
            }
            _ => None,
        };
        roots.push(RootProgress {
            root_id: id,
            path,
            status: RootStatus::from_code(status),
            dirs_done: tally.map_or(0, |t| t.dirs_done),
            dirs_pending: pending as u64,
            files_indexed: files,
            current_dir,
        });
    }
    Ok(IndexProgress {
        state,
        files_indexed: total,
        roots,
    })
}

fn idle_progress() -> IndexProgress {
    IndexProgress {
        state: IndexerState::Idle,
        files_indexed: 0,
        roots: Vec::new(),
    }
}

enum Cmd {
    Wake,
    RootsChanged,
    Rescan(Option<i64>),
    Watch(watch::WatchEvent),
}

struct Shared {
    paused: AtomicBool,
    stop: AtomicBool,
    /// A command that should interrupt the current batch is queued.
    wake: AtomicBool,
    progress: Mutex<IndexProgress>,
}

/// Command sender, join handle and shared progress of the `vfs-indexer` thread (T3a).
pub(crate) struct IndexerHandle {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
    join: Option<JoinHandle<()>>,
}

impl IndexerHandle {
    /// Starts the thread (background priority via `platform`, watcher via `watch`), which
    /// first finishes roots left in `removing`, then resumes pending directories. The
    /// watcher ignores the directory holding `db_path`.
    pub(crate) fn spawn(db_path: PathBuf, sink: Arc<dyn ProgressSink>) -> Result<IndexerHandle> {
        let conn = db::open_rw(&db_path)?;
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            paused: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            wake: AtomicBool::new(false),
            progress: Mutex::new(idle_progress()),
        });
        let ignore = db_path
            .parent()
            .map(|p| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
            .unwrap_or_default();
        let worker = Worker {
            conn,
            rx,
            watch_tx: tx.clone(),
            ignore,
            shared: Arc::clone(&shared),
            sink,
            watcher: None,
            watched: HashSet::new(),
            dirty: Vec::new(),
            dirty_since: None,
            pass: Pass::default(),
            published: None,
            last_progress: None,
            last_changed: Instant::now(),
            last_periodic: Instant::now(),
        };
        let join = std::thread::Builder::new()
            .name("vfs-indexer".into())
            .spawn(move || worker.run())?;
        Ok(IndexerHandle {
            tx,
            shared,
            join: Some(join),
        })
    }

    fn send(&self, cmd: Cmd) {
        self.shared.wake.store(true, Ordering::SeqCst);
        let _ = self.tx.send(cmd);
    }

    pub(crate) fn roots_changed(&self) {
        self.send(Cmd::RootsChanged);
    }

    pub(crate) fn rescan(&self, root_id: Option<i64>) {
        self.send(Cmd::Rescan(root_id));
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::SeqCst);
        self.send(Cmd::Wake);
    }

    pub(crate) fn progress(&self) -> IndexProgress {
        self.shared
            .progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Stops after the current directory (committing it) and joins the thread.
    pub(crate) fn shutdown(self) {
        drop(self);
    }
}

impl Drop for IndexerHandle {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Cmd::Wake);
        if let Some(join) = self.join.take() {
            if join.join().is_err() {
                log::error!("indexer thread panicked");
            }
        }
    }
}

struct Worker {
    conn: Connection,
    rx: Receiver<Cmd>,
    watch_tx: Sender<Cmd>,
    ignore: PathBuf,
    shared: Arc<Shared>,
    sink: Arc<dyn ProgressSink>,
    watcher: Option<watch::RootWatcher>,
    watched: HashSet<i64>,
    dirty: Vec<PathBuf>,
    dirty_since: Option<Instant>,
    pass: Pass,
    /// Last state sent to the sink outside a scan, to avoid repeating it.
    published: Option<IndexerState>,
    last_progress: Option<Instant>,
    last_changed: Instant,
    last_periodic: Instant,
}

impl Worker {
    fn run(mut self) {
        platform::enter_background_priority();
        if let Err(e) = self.init() {
            log::error!("indexer start: {e}");
        }
        loop {
            self.shared.wake.store(false, Ordering::SeqCst);
            loop {
                match self.rx.try_recv() {
                    Ok(cmd) => self.handle(cmd),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }
            if self.shared.stop.load(Ordering::SeqCst) {
                return;
            }
            self.flush_dirty();
            if self.last_periodic.elapsed() >= PERIODIC_CHECK {
                self.last_periodic = Instant::now();
                self.log_err(queue_stale(&self.conn, PERIODIC_REWALK_SECS));
            }
            if self.shared.paused.load(Ordering::SeqCst) {
                self.publish(IndexerState::Paused);
                self.wait(Duration::from_secs(1));
                continue;
            }
            let shared = Arc::clone(&self.shared);
            let stop = move || {
                shared.stop.load(Ordering::SeqCst)
                    || shared.paused.load(Ordering::SeqCst)
                    || shared.wake.load(Ordering::SeqCst)
            };
            let sink = &self.sink;
            let progress = &self.shared.progress;
            let last_progress = &mut self.last_progress;
            let last_changed = &mut self.last_changed;
            let mut tick = |conn: &Connection, pass: &Pass, committed: bool| {
                if committed && last_changed.elapsed() >= CHANGED_EVERY {
                    sink.on_index_changed();
                    *last_changed = Instant::now();
                }
                if last_progress.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                    match build_progress(conn, pass, IndexerState::Scanning) {
                        Ok(p) => {
                            *progress.lock().unwrap_or_else(|e| e.into_inner()) = p.clone();
                            sink.on_progress(&p);
                        }
                        Err(e) => log::warn!("progress: {e}"),
                    }
                    *last_progress = Some(Instant::now());
                }
            };
            let outcome = run(&mut self.conn, &mut self.pass, &stop, true, &mut tick);
            match outcome {
                Ok(Outcome::Yielded) => self.published = None,
                Ok(Outcome::Done) => {
                    if self.pass.summary.dirs_listed > 0 || self.pass.summary.errors > 0 {
                        self.sink.on_index_changed();
                        self.last_changed = Instant::now();
                    }
                    self.pass = Pass::default();
                    self.publish(IndexerState::Idle);
                    let timeout = match self.dirty_since {
                        Some(t) => DIRTY_COALESCE.saturating_sub(t.elapsed()),
                        None => IDLE_WAIT,
                    };
                    self.wait(timeout.max(Duration::from_millis(10)));
                }
                Err(e) => {
                    log::error!("indexer: {e}");
                    self.pass.current = None;
                    self.publish(IndexerState::Idle);
                    self.wait(Duration::from_secs(5));
                }
            }
        }
    }

    fn log_err(&self, r: Result<()>) {
        if let Err(e) = r {
            log::warn!("indexer: {e}");
        }
    }

    fn init(&mut self) -> Result<()> {
        finish_removing(&self.conn, &|| false)?;
        refresh_counts(&self.conn, None)?;
        queue_stale(&self.conn, STARTUP_REWALK_SECS)?;
        self.watcher = start_watcher(self.watch_tx.clone(), self.ignore.clone());
        self.sync_watches();
        Ok(())
    }

    fn wait(&mut self, timeout: Duration) {
        match self.rx.recv_timeout(timeout) {
            Ok(cmd) => self.handle(cmd),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => self.shared.stop.store(true, Ordering::SeqCst),
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Wake => {}
            Cmd::RootsChanged => self.sync_watches(),
            Cmd::Rescan(root_id) => self.log_err(queue_rewalk(&self.conn, root_id)),
            Cmd::Watch(watch::WatchEvent::Dirty(path)) => {
                self.dirty.push(path);
                self.dirty_since.get_or_insert_with(Instant::now);
            }
            Cmd::Watch(watch::WatchEvent::Rescan(root_id)) => {
                self.log_err(queue_rewalk(&self.conn, Some(root_id)))
            }
        }
    }

    fn flush_dirty(&mut self) {
        if self
            .dirty_since
            .is_some_and(|t| t.elapsed() >= DIRTY_COALESCE)
        {
            let paths = std::mem::take(&mut self.dirty);
            self.dirty_since = None;
            let r = mark_dirty(&mut self.conn, &paths);
            self.log_err(r);
        }
    }

    /// Sends a progress snapshot for a state outside scanning, once per state change.
    fn publish(&mut self, state: IndexerState) {
        if self.published == Some(state) {
            return;
        }
        match build_progress(&self.conn, &self.pass, state) {
            Ok(p) => {
                *self
                    .shared
                    .progress
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = p.clone();
                self.sink.on_progress(&p);
                self.published = Some(state);
            }
            Err(e) => log::warn!("progress: {e}"),
        }
    }

    /// Watches every root not being removed and unwatches the others.
    fn sync_watches(&mut self) {
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        let roots: rusqlite::Result<Vec<(i64, String)>> = self
            .conn
            .prepare("SELECT id, path FROM roots WHERE status <> 4")
            .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect());
        let roots = match roots {
            Ok(r) => r,
            Err(e) => return log::warn!("watch roots: {e}"),
        };
        let live: HashSet<i64> = roots.iter().map(|(id, _)| *id).collect();
        for gone in self.watched.difference(&live).copied().collect::<Vec<_>>() {
            watcher.unwatch(gone);
            self.watched.remove(&gone);
        }
        for (id, path) in roots {
            if !self.watched.contains(&id) {
                match watcher.watch(id, Path::new(&path)) {
                    Ok(()) => {
                        self.watched.insert(id);
                    }
                    Err(e) => log::warn!("cannot watch {path}: {e}"),
                }
            }
        }
    }
}

/// Starts the watcher. A failing watcher (or one not available on this platform yet) never
/// stops indexing: periodic and on-demand re-walks still catch changes (VFS-1 AC-7).
fn start_watcher(tx: Sender<Cmd>, ignore: PathBuf) -> Option<watch::RootWatcher> {
    let sink: watch::WatchSink = Arc::new(move |event| {
        let _ = tx.send(Cmd::Watch(event));
    });
    match catch_unwind(AssertUnwindSafe(|| watch::RootWatcher::start(sink, ignore))) {
        Ok(Ok(w)) => Some(w),
        Ok(Err(e)) => {
            log::warn!("file watcher unavailable: {e}");
            None
        }
        Err(_) => {
            log::warn!("file watcher unavailable");
            None
        }
    }
}

/// Queues a re-walk of every ready root whose last full scan is older than `age_secs`.
fn queue_stale(conn: &Connection, age_secs: i64) -> Result<()> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM roots WHERE status = 2 \
             AND (last_full_scan_at IS NULL OR last_full_scan_at < ?1)",
        )?;
        let rows = stmt.query_map([now_secs() - age_secs], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for id in ids {
        queue_rewalk(conn, Some(id))?;
    }
    Ok(())
}

/// Finishes roots left in `removing`, then processes pending directories on the calling
/// thread until none is left or `cancel` is set.
pub(crate) fn scan_to_completion(
    conn: &mut Connection,
    cancel: &AtomicBool,
) -> Result<ScanSummary> {
    let mut pass = Pass::default();
    let stop = || cancel.load(Ordering::SeqCst);
    run(
        conn,
        &mut pass,
        &stop,
        false,
        &mut |_: &Connection, _: &Pass, _: bool| {},
    )?;
    Ok(pass.summary)
}

/// Marks the directories containing `paths` pending shallow, never downgrading a pending
/// deep directory (`... WHERE id = ? AND scan_state = 0`).
///
/// A path that is itself an indexed directory is marked too. A parent that is not indexed
/// yet marks its nearest indexed ancestor. Paths outside every root are ignored.
pub(crate) fn mark_dirty(conn: &mut Connection, paths: &[PathBuf]) -> Result<()> {
    let roots: Vec<(i64, PathBuf)> = {
        let mut stmt = conn.prepare("SELECT id, path FROM roots WHERE status <> 4")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?)))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let tx = conn.transaction()?;
    {
        let mut find =
            tx.prepare_cached("SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ?2")?;
        let mut mark =
            tx.prepare_cached("UPDATE dirs SET scan_state = 2 WHERE id = ?1 AND scan_state = 0")?;
        for path in paths {
            let Some((root_id, rel)) = roots
                .iter()
                .find_map(|(id, root)| path.strip_prefix(root).ok().map(|rel| (*id, rel)))
            else {
                continue;
            };
            let comps: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            if let Some(id) = find
                .query_row(params![root_id, comps.join("/")], |r| r.get::<_, i64>(0))
                .optional()?
            {
                mark.execute([id])?;
            }
            let mut n = comps.len().saturating_sub(1);
            loop {
                let ancestor = comps[..n].join("/");
                if let Some(id) = find
                    .query_row(params![root_id, ancestor], |r| r.get::<_, i64>(0))
                    .optional()?
                {
                    mark.execute([id])?;
                    break;
                }
                if n == 0 {
                    break;
                }
                n -= 1;
            }
        }
    }
    tx.commit()?;
    Ok(())
}
