//! Background indexer (VFS-1 AC-4..AC-8): scheduler thread, resume, scan loop.
//! Owned by task T3a together with `walk.rs` and `apply.rs`; `watch.rs` belongs to T3b.
//!
//! `dirs.scan_state` is the persistent work queue: listing a pending directory diffs it
//! against the index (`apply`), queues its new subdirectories, and marks it clean, all in
//! the current transaction. A first scan, a full re-walk (root directory pending deep) and a
//! watcher hint (one directory pending shallow) all run this same loop. Transactions commit
//! between directories every ~20,000 changed rows or 5 s, so a killed run redoes at most
//! the uncommitted directories.
//!
//! Full re-walks (start-up catch-up, manual, overflow, stale) are requests kept in `owed`
//! and started one at a time, only while no root is being walked (VFS-1 I4, I10). A root
//! with a change cursor is not walked at start: the changes since its cursor are replayed
//! into pending directories instead (VFS-9, `cursor.rs`).
//!
//! Priority and pacing follow `sched` (VFS-5): walks run in the walk profile and back off
//! while the disk is busy; small incremental work lists each directory in background mode
//! and commits it alone, outside background mode, so the write lock is never held at
//! very-low I/O priority.

mod apply;
mod cursor;
mod sched;
mod walk;
mod watch;

pub use sched::DIAG_TARGET;
pub(crate) use walk::cloud_only_attributes;

/// Options of the background indexer.
#[derive(Clone, Debug)]
pub struct IndexerOptions {
    /// The start-up catch-up waits until the system has been up this long (VFS-5 R5).
    pub boot_grace: Duration,
}

impl Default for IndexerOptions {
    fn default() -> Self {
        IndexerOptions {
            boot_grace: SchedConfig::default().boot_grace,
        }
    }
}

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::db::ingest::ScanState;
use crate::db::{self, ingest};
use crate::error::Result;
use crate::fold::fold_key;
use crate::types::{
    IndexProgress, IndexerState, RootProgress, RootStatus, ScanSummary, UnreadableReason,
};
use crate::{platform, ProgressSink};
use cursor::{History, OsHistory};
use sched::{Mode, Sched, SchedConfig, Step};

/// Commit after this many changed rows (VFS-1 I5)...
const BATCH_ROWS: u64 = 20_000;
/// ...or when the transaction is this old.
const BATCH_AGE: Duration = Duration::from_secs(5);
const PROGRESS_EVERY: Duration = Duration::from_millis(500);
const CHANGED_EVERY: Duration = Duration::from_secs(2);
/// Watcher hints are coalesced this long before their directories are marked.
const DIRTY_COALESCE: Duration = Duration::from_secs(2);
/// While running, roots are re-walked when their last full scan is older than this.
const PERIODIC_REWALK_SECS: i64 = 7 * 24 * 3600;
const PERIODIC_CHECK: Duration = Duration::from_secs(3600);
const IDLE_WAIT: Duration = Duration::from_secs(60);
/// A deferred re-walk (events lost, or asked for while its root was being walked) starts no
/// sooner than this after the root's last full scan ended (VFS-1 I10).
const OWED_COOLDOWN_SECS: i64 = 3600;
/// A directory that fails to list is retried after this, doubling each time (VFS-1 I11)...
const RETRY_FIRST: Duration = Duration::from_secs(60);
/// ...and left alone until the next full walk after this many failed attempts.
const RETRY_ATTEMPTS: u32 = 6;
/// Roots that could not be watched are retried this often, and at once when they come back.
const WATCH_RETRY: Duration = Duration::from_secs(60);
/// A watched root's change cursor advances this often, to the position taken at the
/// previous check (VFS-9 R6).
const CURSOR_EVERY: Duration = Duration::from_secs(300);
/// A root's history is replayed this long after its last watcher overflow (VFS-26 R2)…
const REPLAY_QUIET: Duration = Duration::from_secs(10);
/// …and never later than this after the first overflow of the burst.
const REPLAY_MAX: Duration = Duration::from_secs(60);

/// The worker's clocks for change cursors and overflow replays (tests use short ones).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    pub cursor_every: Duration,
    pub replay_quiet: Duration,
    pub replay_max: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            cursor_every: CURSOR_EVERY,
            replay_quiet: REPLAY_QUIET,
            replay_max: REPLAY_MAX,
        }
    }
}

/// A replay owed to a root after watcher overflows: the burst's first overflow and when it
/// is due (VFS-26 R2).
#[derive(Clone, Copy, Debug)]
struct ReplayDue {
    first: Instant,
    at: Instant,
}

/// SQL condition of a directory waiting to be listed. The value of `ScanState::Clean` is
/// spelled into the SQL text, not bound, so SQLite can prove the query matches the partial
/// index `dirs_pending` (`schema.sql`); `INDEXED BY` refuses to prepare otherwise.
fn pending_condition() -> String {
    format!("scan_state <> {}", ScanState::Clean as i64)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A root is reachable when it is a real directory; one replaced by a link is unavailable,
/// never followed (root paths are canonical, so a link there is a change on disk).
fn reachable(path: &str) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
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

/// Commits `tx` if one is open, timing it for the diagnostic log (VFS-5 R7).
fn commit(tx: &mut Option<Transaction<'_>>, sched: &mut Sched) -> Result<()> {
    if let Some(t) = tx.take() {
        let start = Instant::now();
        t.commit()?;
        sched.diag.commit(start.elapsed());
    }
    Ok(())
}

/// Sleeps `d` in short slices; returns true as soon as `stop()` turns true.
fn pause(d: Duration, stop: &dyn Fn() -> bool) -> bool {
    const SLICE: Duration = Duration::from_millis(100);
    let end = Instant::now() + d;
    loop {
        if stop() {
            return true;
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        std::thread::sleep(left.min(SLICE));
    }
}

/// Mode of the next pass: walk while a root is walked in full or many directories are
/// pending (VFS-5 R1, R2).
fn pass_mode(conn: &Connection, cfg: &SchedConfig) -> Result<Mode> {
    if !walking(conn)?.is_empty() {
        return Ok(Mode::Walk);
    }
    let pending: i64 = conn
        .prepare_cached(&format!(
            "SELECT COUNT(*) FROM (SELECT 1 FROM dirs INDEXED BY dirs_pending WHERE {} LIMIT ?1)",
            pending_condition()
        ))?
        .query_row([cfg.incremental_max_dirs as i64 + 1], |r| r.get(0))?;
    Ok(sched::choose_mode(false, pending as u64, cfg))
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
    /// A root unavailable before became reachable again in this pass.
    back_online: bool,
}

/// Failed listings of one directory, left pending until `next_at` (VFS-1 I11).
struct Retry {
    attempts: u32,
    next_at: Instant,
}

/// Directory id -> its failed listings, kept across passes by the caller of `run`.
type Retries = HashMap<i64, Retry>;

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
/// committing). With `one_batch`, also yields after each commit. Directories waiting in
/// `retries` are skipped until they are due.
#[allow(clippy::too_many_arguments)]
fn run(
    conn: &mut Connection,
    history: &dyn History,
    pass: &mut Pass,
    retries: &mut Retries,
    stop: &dyn Fn() -> bool,
    one_batch: bool,
    sched: &mut Sched,
    tick: Tick,
) -> Result<Outcome> {
    let conn: &Connection = conn;
    if !finish_removing(conn, stop)? {
        return Ok(Outcome::Yielded);
    }
    sched.mode = pass_mode(conn, &sched.cfg)?;
    let skip = data_dir(conn);
    let roots: Vec<ActiveRoot> = {
        let mut stmt =
            conn.prepare("SELECT id, path, status FROM roots WHERE status <> ?1 ORDER BY id")?;
        let rows = stmt.query_map([RootStatus::Removing.code()], |r| {
            Ok(ActiveRoot {
                id: r.get(0)?,
                path: r.get(1)?,
                status: RootStatus::from_code(r.get(2)?),
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    sched.drives = roots
        .iter()
        .filter_map(|r| platform::drive_of(Path::new(&r.path)))
        .chain(skip.as_deref().and_then(platform::drive_of))
        .collect();
    for mut root in roots {
        if !reachable(&root.path) {
            if root.status != RootStatus::Unavailable {
                set_root_status(conn, root.id, RootStatus::Unavailable)?;
            }
            continue;
        }
        if root.status == RootStatus::Unavailable {
            // Back online: a full re-walk catches whatever changed while it was away, once
            // no other root is being walked (VFS-1 I10).
            if !queue_rewalk(conn, history, root.id)? {
                continue;
            }
            root.status = RootStatus::Scanning;
            pass.back_online = true;
            sched.mode = sched::Mode::Walk;
        }
        match scan_root(
            conn,
            history,
            &mut root,
            pass,
            retries,
            stop,
            one_batch,
            skip.as_deref(),
            sched,
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
    discard_recovered_copy(conn)?;
    Ok(Outcome::Done)
}

/// After a recovery (VFS-1 C5) the unreadable index waits as `<index>.bad`, with its `-wal`
/// and `-shm`, until the rebuild is complete: every root ready with a finished full scan.
/// Then it is deleted, so a multi-GB copy does not stay on the system drive.
fn discard_recovered_copy(conn: &Connection) -> Result<()> {
    let Some(db_path) = conn.path().filter(|p| !p.is_empty()).map(PathBuf::from) else {
        return Ok(());
    };
    let bad = db::bad_path(&db_path);
    if !bad.exists() {
        return Ok(());
    }
    let (roots, unfinished): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(status <> ?2 OR last_full_scan_at IS NULL), 0) \
         FROM roots WHERE status <> ?1",
        params![RootStatus::Removing.code(), RootStatus::Ready.code()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if roots == 0 || unfinished > 0 {
        return Ok(());
    }
    for suffix in ["", "-wal", "-shm"] {
        let mut p = bad.as_os_str().to_owned();
        p.push(suffix);
        match std::fs::remove_file(PathBuf::from(p)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    log::info!("rebuild after recovery complete; removed the unreadable copy");
    Ok(())
}

/// Lists the pending directories of one root. In walk mode the transaction spans a batch
/// of directories; in incremental mode each directory is listed with no transaction open
/// and committed alone (VFS-5 R3). A back-off pause always commits first.
#[allow(clippy::too_many_arguments)]
fn scan_root(
    conn: &Connection,
    history: &dyn History,
    root: &mut ActiveRoot,
    pass: &mut Pass,
    retries: &mut Retries,
    stop: &dyn Fn() -> bool,
    one_batch: bool,
    skip: Option<&Path>,
    sched: &mut Sched,
    tick: Tick,
) -> Result<RootOutcome> {
    let mut tx: Option<Transaction<'_>> = None;
    let mut rows = 0u64;
    let mut started = Instant::now();
    loop {
        if stop() {
            commit(&mut tx, sched)?;
            pass.current = None;
            return Ok(RootOutcome::Yielded);
        }
        while let Step::Wait(d) = sched.step() {
            // Never hold the write lock while the disk is left to others (VFS-5 R4).
            commit(&mut tx, sched)?;
            let paused = Instant::now();
            let stopped = pause(d, stop);
            sched.diag.backoff(paused.elapsed());
            if stopped {
                pass.current = None;
                return Ok(RootOutcome::Yielded);
            }
        }
        let now = Instant::now();
        let waiting: Vec<String> = retries
            .iter()
            .filter(|(_, r)| r.next_at > now)
            .map(|(id, _)| id.to_string())
            .collect();
        let not_waiting = if waiting.is_empty() {
            String::new()
        } else {
            format!(" AND id NOT IN ({})", waiting.join(","))
        };
        let next: Option<(i64, String, i64)> = tx
            .as_deref()
            .unwrap_or(conn)
            .prepare_cached(&format!(
                "SELECT id, rel_path, scan_state FROM dirs INDEXED BY dirs_pending \
                 WHERE root_id = ?1 AND {}{not_waiting} ORDER BY id DESC LIMIT 1",
                pending_condition()
            ))?
            .query_row([root.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((dir_id, rel_path, state)) = next else {
            commit(&mut tx, sched)?;
            pass.current = None;
            return Ok(RootOutcome::Finished);
        };
        pass.finished.remove(&root.id);
        pass.current = Some((root.id, rel_path.clone()));
        let (added, removed) = (pass.summary.files_added, pass.summary.files_removed);
        let abs = db::join_rel(&root.path, &rel_path);
        let deep = state == ingest::ScanState::PendingDeep as i64;
        // A first scan's cursor is taken before its first listing: every change from then on
        // is either listed by it or replayed from this cursor (VFS-9 R1).
        let first_cursor =
            (root.status == RootStatus::Pending).then(|| history.current(Path::new(&root.path)));
        let listing = sched.listing(|| {
            if walk::behind_link(Path::new(&root.path), &rel_path) {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            } else {
                walk::list_dir(&abs, skip)
            }
        });
        let open = match tx.take() {
            Some(open) => open,
            None => begin(conn)?,
        };
        let t: &Transaction<'_> = tx.insert(open);
        if let Some(now) = first_cursor {
            cursor::store(t, root.id, now.as_deref())?;
            set_root_status(t, root.id, RootStatus::Scanning)?;
            root.status = RootStatus::Scanning;
        }
        if !matches!(&listing, Err(e) if e.kind() != std::io::ErrorKind::NotFound) {
            retries.remove(&dir_id);
        }
        match listing {
            Ok(entries) => {
                rows += apply::apply_listing(
                    t,
                    root.id,
                    dir_id,
                    &rel_path,
                    deep,
                    &entries,
                    &mut pass.summary,
                )?;
                if rel_path.is_empty() {
                    let mtime = walk::unix_secs(std::fs::metadata(&abs).and_then(|m| m.modified()));
                    ingest::set_dir_mtime(t, dir_id, mtime)?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if rel_path.is_empty() || !reachable(&root.path) {
                    commit(&mut tx, sched)?;
                    pass.current = None;
                    return Ok(RootOutcome::Unavailable);
                }
                // Gone since its parent was listed: drop it and everything below.
                let gone = ingest::delete_dir_subtree(t, dir_id)?;
                pass.summary.files_removed += gone;
                rows += 1 + gone;
            }
            Err(e) => {
                // Permission or I/O error (a busy or syncing disk): keep what is indexed,
                // count it, and leave the directory pending for a retry after a growing
                // delay; after `RETRY_ATTEMPTS` failures it waits for the next full walk.
                log::warn!("cannot list {}: {e}", abs.display());
                pass.summary.errors += 1;
                let retry = retries.entry(dir_id).or_insert(Retry {
                    attempts: 0,
                    next_at: now,
                });
                retry.attempts += 1;
                // Its reason is recorded when it should show (VFS-10 R1): "no permission" at
                // once (it is still retried), any other only once the retries are exhausted.
                let reason = walk::unreadable_reason(&e, cfg!(windows));
                if retry.attempts >= RETRY_ATTEMPTS {
                    retries.remove(&dir_id);
                    ingest::record_unreadable(t, dir_id, reason, true)?;
                } else {
                    retry.next_at = now + RETRY_FIRST * 2u32.pow(retry.attempts - 1);
                    if reason == UnreadableReason::NoPermission {
                        ingest::record_unreadable(t, dir_id, reason, false)?;
                    }
                }
                rows += 1;
            }
        }
        let tally = pass.tally.entry(root.id).or_default();
        tally.dirs_done += 1;
        tally.files_delta += (pass.summary.files_added - added) as i64;
        tally.files_delta -= (pass.summary.files_removed - removed) as i64;

        let incremental = sched.mode == Mode::Incremental;
        if incremental {
            commit(&mut tx, sched)?;
        }
        if rows >= BATCH_ROWS || started.elapsed() >= BATCH_AGE {
            commit(&mut tx, sched)?;
            tick(conn, pass, true);
            if one_batch {
                pass.current = None;
                return Ok(RootOutcome::Yielded);
            }
            rows = 0;
            started = Instant::now();
        } else {
            tick(tx.as_deref().unwrap_or(conn), pass, incremental);
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
        set_root_status(&tx, root.id, RootStatus::Ready)?;
    } else {
        tx.execute(
            "UPDATE roots SET status = ?2, last_full_scan_at = ?3 WHERE id = ?1",
            params![root.id, RootStatus::Ready.code(), now_secs()],
        )?;
        // A link file that could not be read (locked, antivirus) is read again by the next
        // deferred pass after each full walk (VFS-18 R9).
        ingest::retry_unreadable_links(&tx, root.id)?;
        tx.execute(
            "INSERT INTO files_fts(files_fts, rank) VALUES('merge', 500)",
            [],
        )?;
        tx.execute(
            "INSERT INTO dirs_fts(dirs_fts, rank) VALUES('merge', 500)",
            [],
        )?;
        tx.execute(
            "INSERT INTO file_paths_fts(file_paths_fts, rank) VALUES('merge', 500)",
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
         WHERE (?1 IS NULL AND status <> ?2) OR id = ?1",
        params![root_id, RootStatus::Ready.code()],
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

/// Ids of the roots being walked in full: a first scan (`pending`) or a re-walk (`scanning`).
fn walking(conn: &Connection) -> Result<HashSet<i64>> {
    let mut stmt = conn.prepare_cached("SELECT id FROM roots WHERE status IN (?1, ?2)")?;
    let rows = stmt.query_map(
        [RootStatus::Pending.code(), RootStatus::Scanning.code()],
        |r| r.get(0),
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Starts a full re-walk of `root_id` (its root directory pending deep, the root scanning)
/// unless some root is already being walked: then nothing changes and it returns false, so
/// a walk in progress is never restarted and only one runs at a time (VFS-1 I10). Returns
/// true when started, or when the root is gone or being removed. The root's change cursor
/// is taken just before, and stored with the marks (VFS-9 R1).
fn queue_rewalk(conn: &Connection, history: &dyn History, root_id: i64) -> Result<bool> {
    if !walking(conn)?.is_empty() {
        return Ok(false);
    }
    let path: Option<String> = conn
        .query_row("SELECT path FROM roots WHERE id = ?1", [root_id], |r| {
            r.get(0)
        })
        .optional()?;
    let now = path.and_then(|p| history.current(Path::new(&p)));
    let tx = begin(conn)?;
    let marked = tx.execute(
        "UPDATE roots SET status = ?2 WHERE id = ?1 AND status <> ?3",
        params![
            root_id,
            RootStatus::Scanning.code(),
            RootStatus::Removing.code()
        ],
    )?;
    if marked > 0 {
        tx.execute(
            "UPDATE dirs SET scan_state = ?2 WHERE root_id = ?1 AND rel_path = ''",
            params![root_id, ScanState::PendingDeep as i64],
        )?;
        cursor::store(&tx, root_id, now.as_deref())?;
    }
    tx.commit()?;
    Ok(true)
}

/// Re-walks owed, by root id: `true` when deferred, so it waits until the root's last full
/// scan is `OWED_COOLDOWN_SECS` old (VFS-1 I10).
type Owed = BTreeMap<i64, bool>;

/// Records a full re-walk request for each of `ids`, started by `start_owed` as soon as no
/// root is being walked. It is deferred when it comes from `lost_events` (an overflow, a
/// watch that failed) or when the root is being walked now (one more pass after this one),
/// so churn never chains full walks back to back; manual requests otherwise are not, and
/// the start-up catch-up never is (`Worker::owe_catchup`).
/// Requests for the same root merge, the undeferred one winning.
fn request_rewalk(
    conn: &Connection,
    owed: &mut Owed,
    ids: &[i64],
    lost_events: bool,
) -> Result<()> {
    let busy = walking(conn)?;
    for &id in ids {
        let deferred = lost_events || busy.contains(&id);
        owed.entry(id)
            .and_modify(|d| *d = *d && deferred)
            .or_insert(deferred);
    }
    Ok(())
}

/// Starts the first owed re-walk that is due, when no root is being walked. Owed roots that
/// are gone or being removed are dropped. A deferred one is due once its root's last full
/// scan is `OWED_COOLDOWN_SECS` old, missing, or in the future (a clock set back).
fn start_owed(conn: &Connection, history: &dyn History, owed: &mut Owed) -> Result<()> {
    if owed.is_empty() || !walking(conn)?.is_empty() {
        return Ok(());
    }
    let now = now_secs();
    let mut start = None;
    let mut gone = Vec::new();
    for (&id, &deferred) in owed.iter() {
        let row: Option<(i64, Option<i64>)> = conn
            .query_row(
                "SELECT status, last_full_scan_at FROM roots WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            None => gone.push(id),
            Some((status, _)) if status == RootStatus::Removing.code() => gone.push(id),
            Some((_, Some(last))) if deferred && last <= now && now - last < OWED_COOLDOWN_SECS => {
            }
            Some(_) => {
                start = Some(id);
                break;
            }
        }
    }
    for id in gone {
        owed.remove(&id);
    }
    if let Some(id) = start {
        owed.remove(&id);
        queue_rewalk(conn, history, id)?;
    }
    Ok(())
}

/// Ids of every root not being removed.
fn live_root_ids(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached("SELECT id FROM roots WHERE status <> ?1 ORDER BY id")?;
    let rows = stmt.query_map([RootStatus::Removing.code()], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Deletes the rows of roots in `removing` in batches, then the root rows (VFS-1 plan
/// I6). Returns false when `stop()` interrupted it.
fn finish_removing(conn: &Connection, stop: &dyn Fn() -> bool) -> Result<bool> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM roots WHERE status = ?1")?;
        let rows = stmt.query_map([RootStatus::Removing.code()], |r| r.get(0))?;
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
                cursor::store(&tx, root_id, None)?;
                tx.commit()?;
                break;
            }
            let mut rows = 0u64;
            for dir_id in dirs {
                rows += ingest::delete_files_fts(&tx, "f.dir_id = ?1", [dir_id])? as u64;
                tx.execute(
                    "DELETE FROM links WHERE file_id IN (SELECT id FROM files WHERE dir_id = ?1)",
                    [dir_id],
                )?;
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
            .prepare_cached(&format!(
                "SELECT COUNT(*) FROM dirs INDEXED BY dirs_pending WHERE root_id = ?1 AND {}",
                pending_condition()
            ))?
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
    /// Starts the thread (priority and back-off via `sched`, watcher via `watch`), which
    /// first finishes roots left in `removing`, then resumes pending directories. The
    /// watcher ignores the directory holding `db_path`.
    pub(crate) fn spawn(
        db_path: PathBuf,
        sink: Arc<dyn ProgressSink>,
        options: IndexerOptions,
    ) -> Result<IndexerHandle> {
        let cfg = SchedConfig {
            boot_grace: options.boot_grace,
            ..SchedConfig::default()
        };
        let sched = Sched::new(
            cfg,
            Box::new(platform::OsThreadPriority::default()),
            platform::disk_load(),
            sched::log_sink(),
        );
        IndexerHandle::spawn_with(db_path, sink, sched, platform::uptime())
    }

    /// `spawn` with a given schedule and system uptime (tests use fakes).
    fn spawn_with(
        db_path: PathBuf,
        sink: Arc<dyn ProgressSink>,
        sched: Sched,
        uptime: Option<Duration>,
    ) -> Result<IndexerHandle> {
        Self::spawn_with_history(
            db_path,
            sink,
            sched,
            uptime,
            Box::new(OsHistory),
            Timing::default(),
        )
    }

    /// `spawn_with` and a given change history and cursor interval (tests use fakes).
    fn spawn_with_history(
        db_path: PathBuf,
        sink: Arc<dyn ProgressSink>,
        mut sched: Sched,
        uptime: Option<Duration>,
        history: Box<dyn History + Send>,
        timing: Timing,
    ) -> Result<IndexerHandle> {
        let mut conn = db::open_rw(&db_path)?;
        sched::install_checkpoint_hook(
            &mut conn,
            Arc::clone(&sched.checkpoints),
            db::WAL_AUTOCHECKPOINT_PAGES,
        )?;
        let catchup_delay = sched::catchup_delay(uptime, sched.cfg.boot_grace);
        prepare_data_dir(&db_path, &mut sched, uptime, catchup_delay);
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
            watch_failed: HashSet::new(),
            last_watch_try: Instant::now(),
            dirty: Vec::new(),
            dirty_since: None,
            pass: Pass::default(),
            retries: Retries::new(),
            owed: Owed::new(),
            published: None,
            last_progress: None,
            last_changed: Instant::now(),
            last_periodic: Instant::now(),
            sched,
            catchup_at: Instant::now() + catchup_delay,
            catchup: Vec::new(),
            db_path,
            history,
            timing,
            last_cursor: Instant::now(),
            candidates: HashMap::new(),
            lost: HashMap::new(),
            replay_due: HashMap::new(),
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

    /// Makes the thread look for pending directories now (after `mark_dirty`).
    pub(crate) fn wake(&self) {
        self.send(Cmd::Wake);
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
    /// Live roots whose last watch attempt failed (VFS-1 I12).
    watch_failed: HashSet<i64>,
    last_watch_try: Instant,
    dirty: Vec<PathBuf>,
    dirty_since: Option<Instant>,
    pass: Pass,
    retries: Retries,
    owed: Owed,
    /// Last state sent to the sink outside a scan, to avoid repeating it.
    published: Option<IndexerState>,
    last_progress: Option<Instant>,
    last_changed: Instant,
    last_periodic: Instant,
    sched: Sched,
    /// The start-up catch-up re-walks in `catchup` are requested at `catchup_at`, once the
    /// system has been up for the boot grace (VFS-5 R5).
    catchup_at: Instant,
    catchup: Vec<i64>,
    db_path: PathBuf,
    history: Box<dyn History + Send>,
    last_cursor: Instant,
    /// Per watched root, the cursor taken at the last check, stored at the next one if the
    /// root was watched without lost events meanwhile (VFS-9 R6).
    candidates: HashMap<i64, String>,
    /// Roots that lost events, with the cursor stored when they did: theirs stays put until
    /// a full walk stores another (VFS-9 R6).
    lost: HashMap<i64, Option<String>>,
    timing: Timing,
    /// Roots whose history is to be replayed after a watcher overflow (VFS-26 R2); their
    /// cursor is frozen meanwhile.
    replay_due: HashMap<i64, ReplayDue>,
}

/// Prepares the folder holding the index (VFS-5 R9): not content-indexed on Windows, a
/// warning when it lies in a Synology sync root, and the diagnostic `start` record.
fn prepare_data_dir(
    db_path: &Path,
    sched: &mut Sched,
    uptime: Option<Duration>,
    catchup_delay: Duration,
) {
    let dir = db_path.parent().unwrap_or(Path::new(""));
    let mut synced = false;
    if !dir.as_os_str().is_empty() {
        if let Err(e) = platform::exclude_from_content_index(dir) {
            log::warn!("index folder still content-indexed: {e}");
        }
        synced = platform::inside_sync_root(dir);
        if synced {
            log::warn!("the index folder lies inside a Synology sync root");
        }
    }
    let line = format!(
        "start uptime_s={} catchup_deferred_s={} probe={} data_dir_in_sync_root={synced}",
        uptime.map_or_else(|| "?".to_owned(), |u| u.as_secs().to_string()),
        catchup_delay.as_secs(),
        if sched.has_probe() { "pdh" } else { "none" },
    );
    sched.diag.emit(&line);
}

impl Worker {
    fn run(mut self) {
        // The walk profile is the thread's base state; background mode is only entered
        // around single listings of incremental work (VFS-5 R1, R2).
        self.sched.walk_profile();
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
            if self.last_cursor.elapsed() >= self.timing.cursor_every {
                self.last_cursor = Instant::now();
                self.advance_cursors();
            }
            self.run_due_replays();
            if !self.catchup.is_empty() && Instant::now() >= self.catchup_at {
                let ids = std::mem::take(&mut self.catchup);
                self.owe_catchup(&ids);
            }
            self.memory_record();
            if self.last_periodic.elapsed() >= PERIODIC_CHECK {
                self.last_periodic = Instant::now();
                let r = stale_roots(&self.conn, PERIODIC_REWALK_SECS)
                    .and_then(|ids| request_rewalk(&self.conn, &mut self.owed, &ids, false));
                self.log_err(r);
            }
            let r = start_owed(&self.conn, &*self.history, &mut self.owed);
            self.log_err(r);
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
            let published = &mut self.published;
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
                            // The next idle or paused state must be sent again.
                            *published = None;
                        }
                        Err(e) => log::warn!("progress: {e}"),
                    }
                    *last_progress = Some(Instant::now());
                }
            };
            let outcome = run(
                &mut self.conn,
                &*self.history,
                &mut self.pass,
                &mut self.retries,
                &stop,
                true,
                &mut self.sched,
                &mut tick,
            );
            let back_online = std::mem::take(&mut self.pass.back_online);
            if !self.watch_failed.is_empty()
                && (back_online || self.last_watch_try.elapsed() >= WATCH_RETRY)
            {
                self.sync_watches();
            }
            match outcome {
                Ok(Outcome::Yielded) => {
                    self.published = None;
                    let s = &mut self.sched;
                    s.diag.tick(
                        Instant::now(),
                        s.mode,
                        &self.pass.summary,
                        self.retries.len(),
                        &s.checkpoints,
                    );
                }
                Ok(Outcome::Done) => {
                    if self.pass.summary.dirs_listed > 0 || self.pass.summary.errors > 0 {
                        self.sink.on_index_changed();
                        self.last_changed = Instant::now();
                    }
                    let s = &mut self.sched;
                    s.diag.pass_end(
                        Instant::now(),
                        s.mode,
                        &self.pass.summary,
                        self.retries.len(),
                        &s.checkpoints,
                    );
                    self.pass = Pass::default();
                    // An owed walk that is due starts before idle is reported, so "up to
                    // date" never shows while a catch-up is still owed (VFS-3 AC-5).
                    let r = start_owed(&self.conn, &*self.history, &mut self.owed);
                    self.log_err(r);
                    if walking(&self.conn).is_ok_and(|w| !w.is_empty()) {
                        continue;
                    }
                    // VFS-18 D7: shortcut targets are read once the walks are done.
                    let shared = &self.shared;
                    let yield_now = || {
                        shared.stop.load(Ordering::SeqCst)
                            || shared.paused.load(Ordering::SeqCst)
                            || shared.wake.load(Ordering::SeqCst)
                    };
                    match crate::link::read_pending(&mut self.conn, &yield_now) {
                        Ok(0) => {}
                        Ok(_) => {
                            self.sink.on_index_changed();
                            self.last_changed = Instant::now();
                        }
                        Err(e) => log::warn!("link pass: {e}"),
                    }
                    self.publish(IndexerState::Idle);
                    let mut timeout = match self.dirty_since {
                        Some(t) => DIRTY_COALESCE.saturating_sub(t.elapsed()),
                        None => IDLE_WAIT,
                    };
                    if let Some(due) = self.retries.values().map(|r| r.next_at).min() {
                        timeout = timeout.min(due.saturating_duration_since(Instant::now()));
                    }
                    if !self.catchup.is_empty() {
                        let due = self.catchup_at.saturating_duration_since(Instant::now());
                        timeout = timeout.min(due);
                    }
                    timeout = timeout.min(
                        self.timing
                            .cursor_every
                            .saturating_sub(self.last_cursor.elapsed()),
                    );
                    if let Some(due) = self.replay_due.values().map(|d| d.at).min() {
                        timeout = timeout.min(due.saturating_duration_since(Instant::now()));
                    }
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

    /// The diagnostic `memory` record: working set, WAL and index size (VFS-5 R7).
    fn memory_record(&mut self) {
        let now = Instant::now();
        if !self.sched.diag.memory_due(now) {
            return;
        }
        let size = |p: &Path| std::fs::metadata(p).ok().map(|m| m.len());
        let mut wal = self.db_path.as_os_str().to_owned();
        wal.push(db::WAL_SUFFIX);
        let (wal, main) = (size(Path::new(&wal)), size(&self.db_path));
        self.sched.diag.memory(now, wal, main);
    }

    fn log_err(&self, r: Result<()>) {
        if let Err(e) = r {
            log::warn!("indexer: {e}");
        }
    }

    /// Finishes removals and starts watching; then replays each root's change cursor and
    /// owes a catch-up re-walk to every root whose history is missing or lost: changes made
    /// while the app was closed are not otherwise seen (VFS-1 I4, VFS-9 R2, R3). Watching
    /// first leaves no gap between the replayed history and the live events. The replay
    /// runs at once (it reads the journal and marks folders; their re-listing is incremental
    /// work); the catch-up walks wait for the boot grace (VFS-5 R5), and roots never fully
    /// scanned are walked at once.
    fn init(&mut self) -> Result<()> {
        finish_removing(&self.conn, &|| false)?;
        refresh_counts(&self.conn, None)?;
        self.watcher = start_watcher(self.watch_tx.clone(), self.ignore.clone());
        self.sync_watches();
        let filter = watch::Filter::new(self.ignore.clone());
        let ids = catch_up(&mut self.conn, &*self.history, &filter)?;
        // Their cursors (or lack of one) stay put until the owed walk stores a new one,
        // however long it waits for the boot grace or another walk (VFS-9 R3, R6).
        for &id in &ids {
            self.events_lost(id);
        }
        if Instant::now() >= self.catchup_at {
            self.owe_catchup(&ids);
        } else {
            self.catchup = ids;
        }
        Ok(())
    }

    /// Owes the start-up catch-up walk of `ids`, never deferred, not even for a root whose
    /// walk was interrupted: that walk may be an older one (a power cut drops the last
    /// commits), and folders it already listed may have changed since (VFS-3 AC-5).
    fn owe_catchup(&mut self, ids: &[i64]) {
        for &id in ids {
            self.owed.insert(id, false);
        }
    }

    /// Holds `root_id`'s cursor where it is until a full walk stores another: changes were
    /// missed, and the owed walk may not run before the app closes (VFS-9 R6).
    fn events_lost(&mut self, root_id: i64) {
        self.candidates.remove(&root_id);
        // Each loss records the cursor stored now, so a walk that stored one just before
        // this loss does not release the hold (VFS-9 R6).
        match cursor::load(&self.conn, root_id) {
            Ok(c) => {
                self.lost.insert(root_id, c);
            }
            Err(e) => log::warn!("indexer: {e}"),
        }
    }

    /// Writes the pending watcher hints, stores each watched root's previous candidate
    /// cursor, and takes a new one (VFS-9 R6). A root that lost events or is not watched
    /// keeps its cursor until its next full walk stores one.
    fn advance_cursors(&mut self) {
        let paths = std::mem::take(&mut self.dirty);
        self.dirty_since = None;
        if let Err(e) = mark_dirty(&mut self.conn, &paths) {
            // The hints stay unwritten: storing a cursor now could skip their changes.
            log::warn!("indexer: {e}");
            self.dirty = paths;
            self.dirty_since = Some(Instant::now());
            return;
        }
        let roots = match root_paths(&self.conn) {
            Ok(r) => r,
            Err(e) => return log::warn!("indexer: {e}"),
        };
        for (id, path) in roots {
            // A replay is due: changes were dropped after the stored cursor (VFS-26 R2).
            if self.replay_due.contains_key(&id) {
                self.candidates.remove(&id);
                continue;
            }
            if let Some(at_loss) = self.lost.get(&id) {
                match cursor::load(&self.conn, id) {
                    // A full walk has stored its own cursor since: advance again from now.
                    Ok(now) if now != *at_loss => {
                        self.lost.remove(&id);
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("indexer: {e}"),
                }
                self.candidates.remove(&id);
                continue;
            }
            if !self.watched.contains(&id) || self.watch_failed.contains(&id) {
                self.candidates.remove(&id);
                continue;
            }
            if let Some(c) = self.candidates.remove(&id) {
                let r = cursor::store(&self.conn, id, Some(&c));
                self.log_err(r);
            }
            if let Some(c) = self.history.current(&path) {
                self.candidates.insert(id, c);
            }
        }
    }

    /// Replays the history of every root whose overflow replay is due (VFS-26 R3); a replay
    /// that cannot run or fails falls back to the lost-events walk (R4).
    fn run_due_replays(&mut self) {
        let now = Instant::now();
        let due: Vec<i64> = self
            .replay_due
            .iter()
            .filter(|(_, d)| d.at <= now)
            .map(|(&id, _)| id)
            .collect();
        for id in due {
            self.replay_due.remove(&id);
            let started = Instant::now();
            let outcome = self.replay_root(id);
            let ms = started.elapsed().as_millis();
            match outcome {
                Ok(Some(folders)) => self.sched.diag.emit(&format!(
                    "overflow_replay root={id} folders={folders} ms={ms} result=replayed"
                )),
                Ok(None) | Err(_) => {
                    if let Err(e) = outcome {
                        log::warn!("indexer: replay of root {id}: {e}");
                    }
                    self.sched.diag.emit(&format!(
                        "overflow_replay root={id} folders=0 ms={ms} result=lost"
                    ));
                    self.events_lost(id);
                    self.request(&[id], true);
                }
            }
        }
    }

    /// Writes the pending watcher hints, replays `root_id`'s history from its stored cursor,
    /// marks the changed folders and stores the new cursor in one transaction (VFS-26 R3).
    /// `Ok(None)` when it cannot replay: no cursor, history lost, root unreachable or being
    /// removed.
    fn replay_root(&mut self, root_id: i64) -> Result<Option<usize>> {
        let paths = std::mem::take(&mut self.dirty);
        self.dirty_since = None;
        if let Err(e) = mark_dirty(&mut self.conn, &paths) {
            self.dirty = paths;
            self.dirty_since = Some(Instant::now());
            return Err(e);
        }
        let Some((_, root)) = root_paths(&self.conn)?
            .into_iter()
            .find(|(id, _)| *id == root_id)
        else {
            return Ok(None);
        };
        let Some(stored) = cursor::load(&self.conn, root_id)? else {
            return Ok(None);
        };
        if !reachable(&root.to_string_lossy()) {
            return Ok(None);
        }
        let history = &*self.history;
        let replayed = self.sched.listing(|| history.replay(&root, &stored));
        let Ok((changed, next)) = replayed else {
            return Ok(None);
        };
        let filter = watch::Filter::new(self.ignore.clone());
        let changed: Vec<PathBuf> = changed
            .iter()
            .filter_map(|p| under_root(p, &root, cfg!(windows)))
            .filter(|p| !filter.dropped(p, &root))
            .collect();
        let tx = begin(&self.conn)?;
        mark_paths(&tx, &[(root_id, root)], &changed, true)?;
        cursor::store(&tx, root_id, Some(&next))?;
        tx.commit()?;
        Ok(Some(changed.len()))
    }

    fn request(&mut self, ids: &[i64], lost_events: bool) {
        let r = request_rewalk(&self.conn, &mut self.owed, ids, lost_events);
        self.log_err(r);
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
            Cmd::Rescan(Some(root_id)) => {
                self.catchup.retain(|&id| id != root_id);
                self.request(&[root_id], false)
            }
            Cmd::Rescan(None) => match live_root_ids(&self.conn) {
                Ok(ids) => {
                    self.catchup.clear();
                    self.request(&ids, false)
                }
                Err(e) => log::warn!("indexer: {e}"),
            },
            Cmd::Watch(watch::WatchEvent::Dirty(path)) => {
                self.dirty.push(path);
                self.dirty_since.get_or_insert_with(Instant::now);
            }
            Cmd::Watch(watch::WatchEvent::Overflow(root_id)) => {
                // Events were dropped but the watch is armed: replay the history once the
                // burst is over, with the cursor frozen until then (VFS-26 R2).
                self.sched.diag.overflow(root_id);
                self.candidates.remove(&root_id);
                let now = Instant::now();
                let t = self.timing;
                let due = self.replay_due.entry(root_id).or_insert(ReplayDue {
                    first: now,
                    at: now,
                });
                due.at = (now + t.replay_quiet).min(due.first + t.replay_max);
            }
            Cmd::Watch(watch::WatchEvent::Failed(root_id)) => {
                // Nothing watched for a while: as before VFS-26 (R5).
                self.sched.diag.overflow(root_id);
                self.replay_due.remove(&root_id);
                self.events_lost(root_id);
                self.request(&[root_id], true)
            }
            Cmd::Watch(watch::WatchEvent::Renamed { from, to }) => {
                match rename_dir(&mut self.conn, &from, &to) {
                    Ok(true) => log::debug!("{} renamed in place", from.display()),
                    Ok(false) => {}
                    Err(e) => log::warn!("rename {}: {e}", from.display()),
                }
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

    /// Watches every root not being removed and unwatches the others. A root that cannot be
    /// watched is retried later (`watch_failed`); once watched after a failure, it owes a
    /// re-walk for the changes nobody saw meanwhile (VFS-1 I12).
    fn sync_watches(&mut self) {
        self.last_watch_try = Instant::now();
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        let roots: rusqlite::Result<Vec<(i64, String)>> = self
            .conn
            .prepare("SELECT id, path FROM roots WHERE status <> ?1")
            .and_then(|mut s| {
                s.query_map([RootStatus::Removing.code()], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?
                .collect()
            });
        let roots = match roots {
            Ok(r) => r,
            Err(e) => return log::warn!("watch roots: {e}"),
        };
        let live: HashSet<i64> = roots.iter().map(|(id, _)| *id).collect();
        for gone in self.watched.difference(&live).copied().collect::<Vec<_>>() {
            watcher.unwatch(gone);
            self.watched.remove(&gone);
            self.candidates.remove(&gone);
        }
        self.watch_failed.retain(|id| live.contains(id));
        let mut recovered = Vec::new();
        let mut failed = Vec::new();
        for (id, path) in roots {
            if !self.watched.contains(&id) {
                match watcher.watch(id, Path::new(&path)) {
                    Ok(()) => {
                        self.watched.insert(id);
                        if self.watch_failed.remove(&id) {
                            recovered.push(id);
                        }
                    }
                    Err(e) => {
                        log::warn!("cannot watch {path}: {e}");
                        self.watch_failed.insert(id);
                        failed.push(id);
                    }
                }
            }
        }
        for &id in failed.iter().chain(&recovered) {
            self.events_lost(id);
        }
        if !recovered.is_empty() {
            self.request(&recovered, true);
        }
    }
}

/// Starts the watcher. A failing watcher never stops indexing: periodic and on-demand re-walks still catch changes (VFS-1 AC-7).
fn start_watcher(tx: Sender<Cmd>, ignore: PathBuf) -> Option<watch::RootWatcher> {
    let sink: watch::WatchSink = Arc::new(move |event| {
        let _ = tx.send(Cmd::Watch(event));
    });
    match watch::RootWatcher::start(sink, ignore) {
        Ok(w) => Some(w),
        Err(e) => {
            log::warn!("file watcher unavailable: {e}");
            None
        }
    }
}

/// Ready roots whose last full scan is older than `age_secs`, or later than now: a time in
/// the future (the clock was set forward, then corrected) proves nothing (VFS-1 I4).
fn stale_roots(conn: &Connection, age_secs: i64) -> Result<Vec<i64>> {
    let now = now_secs();
    let mut stmt = conn.prepare(
        "SELECT id FROM roots WHERE status = ?3 AND (last_full_scan_at IS NULL \
         OR last_full_scan_at < ?1 OR last_full_scan_at > ?2)",
    )?;
    let rows = stmt.query_map(
        params![now - age_secs, now, RootStatus::Ready.code()],
        |r| r.get(0),
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Deletes `root_id`'s change cursor, for a root deleted outside `finish_removing`
/// (VFS-9 R8).
pub(crate) fn forget_cursor(conn: &Connection, root_id: i64) -> Result<()> {
    cursor::store(conn, root_id, None)
}

/// Finishes roots left in `removing`, then processes pending directories on the calling
/// thread until none is left or `cancel` is set. A directory that fails to list is tried
/// once per call and stays pending.
pub(crate) fn scan_to_completion(
    conn: &mut Connection,
    cancel: &AtomicBool,
) -> Result<ScanSummary> {
    let mut pass = Pass::default();
    let stop = || cancel.load(Ordering::SeqCst);
    run(
        conn,
        &OsHistory,
        &mut pass,
        &mut Retries::new(),
        &stop,
        false,
        &mut Sched::inert(),
        &mut |_: &Connection, _: &Pass, _: bool| {},
    )?;
    Ok(pass.summary)
}

/// `rel` as stored in `dirs.rel_path`: '/'-separated components.
fn rel_string(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Applies a directory rename or move inside one root as a path update (VFS-1 I9): the
/// `dirs` rows of the subtree get their new `rel_path` and keys and the folder FTS entries
/// and the files' `file_paths_fts` entries are rewritten, in one transaction; files keep
/// their rows, since they point at directory ids. Returns false, changing nothing, unless `from` is an indexed directory now gone from
/// disk, `to` is a real directory in the same root that is not indexed yet, and the parent
/// of `to` is indexed; the watcher's `Dirty` events then re-list both parents as usual.
pub(crate) fn rename_dir(conn: &mut Connection, from: &Path, to: &Path) -> Result<bool> {
    let roots: Vec<(i64, PathBuf)> = {
        let mut stmt = conn.prepare("SELECT id, path FROM roots WHERE status <> ?1")?;
        let rows = stmt.query_map([RootStatus::Removing.code()], |r| {
            Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?)))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let Some((root_id, rel_from, rel_to)) = roots.iter().find_map(|(id, root)| {
        let from = from.strip_prefix(root).ok()?;
        let to = to.strip_prefix(root).ok()?;
        Some((*id, rel_string(from), rel_string(to)))
    }) else {
        return Ok(false);
    };
    let to_is_dir = std::fs::symlink_metadata(to).is_ok_and(|m| m.is_dir());
    if rel_from.is_empty()
        || rel_to.is_empty()
        || !to_is_dir
        || std::fs::symlink_metadata(from).is_ok()
    {
        return Ok(false);
    }
    let (parent_rel, name) = rel_to.rsplit_once('/').unwrap_or(("", rel_to.as_str()));
    let tx = conn.transaction()?;
    let find = |rel: &str| -> Result<Option<i64>> {
        Ok(tx
            .prepare_cached("SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ?2")?
            .query_row(params![root_id, rel], |r| r.get(0))
            .optional()?)
    };
    let (Some(dir_id), None, Some(parent_id)) =
        (find(&rel_from)?, find(&rel_to)?, find(parent_rel)?)
    else {
        return Ok(false);
    };
    let subtree: Vec<(i64, String, String)> = {
        // Descendants of `p` are exactly the rel_paths in [p + '/', p + '0'): '0' follows '/'.
        let mut stmt = tx.prepare(
            "SELECT id, rel_path, name_key FROM dirs WHERE root_id = ?1 AND (rel_path = ?2 \
             OR (rel_path >= ?2 || '/' AND rel_path < ?2 || '0'))",
        )?;
        let rows = stmt.query_map(params![root_id, rel_from], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    {
        let mut update = tx.prepare_cached(
            "UPDATE dirs SET rel_path = ?2, path_key = ?3, name_key = ?4 WHERE id = ?1",
        )?;
        let mut fts_delete = tx.prepare_cached("DELETE FROM dirs_fts WHERE rowid = ?1")?;
        let mut fts_insert = tx.prepare_cached(
            "INSERT INTO dirs_fts(rowid, path_key, name_key) VALUES (?1, ?2, ?3)",
        )?;
        let mut file_paths_insert = tx.prepare_cached(
            "INSERT INTO file_paths_fts(rowid, path) \
             SELECT id, ?2 || '/' || name_key FROM files WHERE dir_id = ?1",
        )?;
        for (id, rel, name_key) in subtree {
            let new_rel = format!("{rel_to}{}", &rel[rel_from.len()..]);
            let path_key = fold_key(&new_rel);
            let name_key = if id == dir_id {
                fold_key(name)
            } else {
                name_key
            };
            // The files' path rows are deleted by their old value, before the update.
            ingest::delete_file_paths_fts(&tx, "f.dir_id = ?1", [id])?;
            update.execute(params![id, new_rel, path_key, name_key])?;
            fts_delete.execute([id])?;
            fts_insert.execute(params![id, path_key, name_key])?;
            // `new_rel` is never empty, so every row is "folder path_key/name_key".
            file_paths_insert.execute(params![id, path_key])?;
        }
    }
    tx.execute(
        "UPDATE dirs SET parent_id = ?2 WHERE id = ?1",
        params![dir_id, parent_id],
    )?;
    tx.commit()?;
    Ok(true)
}

/// Absolute paths of every root not being removed.
fn root_paths(conn: &Connection) -> Result<Vec<(i64, PathBuf)>> {
    let mut stmt = conn.prepare("SELECT id, path FROM roots WHERE status <> ?1")?;
    let rows = stmt.query_map([RootStatus::Removing.code()], |r| {
        Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?)))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Marks the directories containing `paths` pending shallow, never downgrading a pending
/// deep directory (only a clean directory becomes pending shallow).
///
/// A path that is itself an indexed directory is marked too. A parent that is not indexed
/// yet marks its nearest indexed ancestor. Paths outside every root are ignored.
pub(crate) fn mark_dirty(conn: &mut Connection, paths: &[PathBuf]) -> Result<()> {
    let roots = root_paths(conn)?;
    let tx = conn.transaction()?;
    mark_paths(&tx, &roots, paths, false)?;
    tx.commit()?;
    Ok(())
}

/// `path` spelled under `root` as stored, when it lies below `root`; compared ignoring case
/// when `ignore_case` (Windows: the change journal names folders as they are now, so a root
/// renamed only in case would otherwise match no replayed path, VFS-9 R7). `None` outside.
fn under_root(path: &Path, root: &Path, ignore_case: bool) -> Option<PathBuf> {
    if path.starts_with(root) {
        return Some(path.to_path_buf());
    }
    if !ignore_case {
        return None;
    }
    let fold = |c: std::path::Component<'_>| c.as_os_str().to_string_lossy().to_lowercase();
    let mut rest = path.components();
    for want in root.components() {
        if rest.next().map(fold) != Some(fold(want)) {
            return None;
        }
    }
    let mut out = root.to_path_buf();
    out.extend(rest);
    Some(out)
}

/// The marking of `mark_dirty`. With `dirs_only`, each path is a directory whose entries
/// changed (a replayed change, VFS-9 R2): it is marked, or its nearest indexed ancestor when
/// it is not indexed, and its parent is left alone.
fn mark_paths(
    tx: &Connection,
    roots: &[(i64, PathBuf)],
    paths: &[PathBuf],
    dirs_only: bool,
) -> Result<()> {
    let (shallow, clean) = (ScanState::PendingShallow as i64, ScanState::Clean as i64);
    let mut find = tx.prepare_cached("SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ?2")?;
    let mut mark =
        tx.prepare_cached("UPDATE dirs SET scan_state = ?2 WHERE id = ?1 AND scan_state = ?3")?;
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
            mark.execute(params![id, shallow, clean])?;
            if dirs_only {
                continue;
            }
        }
        let mut n = comps.len().saturating_sub(1);
        loop {
            let ancestor = comps[..n].join("/");
            if let Some(id) = find
                .query_row(params![root_id, ancestor], |r| r.get::<_, i64>(0))
                .optional()?
            {
                mark.execute(params![id, shallow, clean])?;
                break;
            }
            if n == 0 {
                break;
            }
            n -= 1;
        }
    }
    Ok(())
}

/// Replays the change cursor of every root not being removed (VFS-9 R2, R3, R7): the
/// directories that changed under a root, minus the excluded ones and the app data
/// directory, become pending shallow, stored with the new cursor in one transaction.
/// Returns the roots that owe a full walk instead: no cursor, unreachable, or history lost.
fn catch_up(
    conn: &mut Connection,
    history: &dyn History,
    filter: &watch::Filter,
) -> Result<Vec<i64>> {
    let mut owed = Vec::new();
    for (id, root) in root_paths(conn)? {
        let Some(stored) = cursor::load(conn, id)? else {
            owed.push(id);
            continue;
        };
        if !reachable(&root.to_string_lossy()) {
            owed.push(id);
            continue;
        }
        match history.replay(&root, &stored) {
            Ok((paths, next)) => {
                let paths: Vec<PathBuf> = paths
                    .iter()
                    .filter_map(|p| under_root(p, &root, cfg!(windows)))
                    .filter(|p| !filter.dropped(p, &root))
                    .collect();
                let tx = begin(conn)?;
                mark_paths(&tx, &[(id, root.clone())], &paths, true)?;
                cursor::store(&tx, id, Some(&next))?;
                tx.commit()?;
                log::info!("root {id}: replayed {} changed folders", paths.len());
            }
            Err(why) => {
                log::info!("root {id}: change history unusable ({why}); full walk");
                owed.push(id);
            }
        }
    }
    Ok(owed)
}

#[cfg(test)]
mod tests;
