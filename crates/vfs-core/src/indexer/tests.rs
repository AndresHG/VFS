//! Unit tests of the scheduler internals: re-walk requests, retries, renames (VFS-1 AC-7).

use std::cell::Cell;
use std::fs;

use super::*;

/// A tree with one `archivo.txt` in each of `dirs`, registered as a root.
struct Fx {
    _db_dir: tempfile::TempDir,
    tree: tempfile::TempDir,
    db_path: PathBuf,
    conn: Connection,
    root_id: i64,
}

fn fixture(dirs: &[&str]) -> Fx {
    let db_dir = tempfile::tempdir().unwrap();
    let tree = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("index.db");
    let mut conn = db::open_rw(&db_path).unwrap();
    let root = dunce::canonicalize(tree.path()).unwrap();
    for d in dirs {
        fs::create_dir_all(root.join(d)).unwrap();
        fs::write(root.join(d).join("archivo.txt"), b"x").unwrap();
    }
    let tx = conn.transaction().unwrap();
    let root_id = ingest::insert_root(&tx, "test", root.to_str().unwrap()).unwrap();
    tx.commit().unwrap();
    Fx {
        _db_dir: db_dir,
        tree,
        db_path,
        conn,
        root_id,
    }
}

impl Fx {
    fn root(&self) -> PathBuf {
        dunce::canonicalize(self.tree.path()).unwrap()
    }

    fn scan(&mut self) -> ScanSummary {
        scan_to_completion(&mut self.conn, &AtomicBool::new(false)).unwrap()
    }

    fn status_of(&self, id: i64) -> RootStatus {
        let code = self
            .conn
            .query_row("SELECT status FROM roots WHERE id = ?1", [id], |r| r.get(0))
            .unwrap();
        RootStatus::from_code(code)
    }

    fn status(&self) -> RootStatus {
        self.status_of(self.root_id)
    }

    fn last_scan(&self) -> Option<i64> {
        self.conn
            .query_row(
                "SELECT last_full_scan_at FROM roots WHERE id = ?1",
                [self.root_id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn set_last_scan(&self, secs: i64) {
        self.conn
            .execute(
                "UPDATE roots SET last_full_scan_at = ?2 WHERE id = ?1",
                params![self.root_id, secs],
            )
            .unwrap();
    }

    /// Id and `scan_state` of the directory at `rel`.
    fn dir(&self, rel: &str) -> Option<(i64, i64)> {
        self.conn
            .query_row(
                "SELECT id, scan_state FROM dirs WHERE rel_path = ?1",
                [rel],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap()
    }

    fn files(&self) -> Vec<(i64, i64)> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, dir_id FROM files ORDER BY id")
            .unwrap();
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    }

    /// One `run` with nothing to stop it, keeping `retries` across calls.
    fn run_once(&mut self, retries: &mut Retries) -> ScanSummary {
        let mut pass = Pass::default();
        run(
            &mut self.conn,
            &OsHistory,
            &mut pass,
            retries,
            &|| false,
            false,
            &mut Sched::inert(),
            &mut |_: &Connection, _: &Pass, _: bool| {},
        )
        .unwrap();
        pass.summary
    }
}

struct NullSink;
impl ProgressSink for NullSink {
    fn on_progress(&self, _: &IndexProgress) {}
}

const CLEAN: i64 = ScanState::Clean as i64;

/// VFS-1 AC-7, I10: asking for a full re-walk of a root that is already being walked
/// changes nothing (the root directory, already listed, is not marked again), one more
/// pass is owed, and the walk lists each directory once and writes `last_full_scan_at`.
#[test]
fn rewalk_request_never_restarts_a_walk() {
    let mut fx = fixture(&["A/D", "B", "C"]);
    fx.scan();
    fx.set_last_scan(1000);
    assert!(queue_rewalk(&fx.conn, &OsHistory, fx.root_id).unwrap());

    // List only the root directory, as if an overflow arrived right after it.
    let calls = Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 1
    };
    let mut pass = Pass::default();
    let outcome = run(
        &mut fx.conn,
        &OsHistory,
        &mut pass,
        &mut Retries::new(),
        &stop,
        false,
        &mut Sched::inert(),
        &mut |_: &Connection, _: &Pass, _: bool| {},
    )
    .unwrap();
    assert!(matches!(outcome, Outcome::Yielded));
    assert_eq!(fx.dir("").unwrap().1, CLEAN);

    assert!(!queue_rewalk(&fx.conn, &OsHistory, fx.root_id).unwrap());
    assert_eq!(fx.dir("").unwrap().1, CLEAN, "root directory marked again");
    let mut owed = Owed::new();
    request_rewalk(&fx.conn, &mut owed, &[fx.root_id], false).unwrap();
    assert_eq!(owed.get(&fx.root_id), Some(&true), "one more pass is owed");

    let summary = fx.scan();
    assert_eq!(summary.dirs_listed, 4, "A, A/D, B and C, once each");
    assert_eq!(fx.status(), RootStatus::Ready);
    assert!(fx.last_scan().unwrap() > 1000);
}

/// VFS-1 AC-7, I10: under a storm of watcher overflows the running indexer still finishes
/// its re-walk, writes `last_full_scan_at`, and does not start another walk right after.
#[test]
fn overflow_storm_does_not_keep_a_root_walking() {
    let dirs: Vec<String> = (0..300).map(|i| format!("d{i:03}/e")).collect();
    let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    let mut fx = fixture(&dirs);
    fx.scan();
    fx.set_last_scan(1000);
    let h = IndexerHandle::spawn(
        fx.db_path.clone(),
        Arc::new(NullSink),
        IndexerOptions::default(),
    )
    .unwrap();
    let reader = db::open_ro(&fx.db_path).unwrap();
    let read = || -> (RootStatus, Option<i64>) {
        reader
            .query_row("SELECT status, last_full_scan_at FROM roots", [], |r| {
                Ok((RootStatus::from_code(r.get(0)?), r.get(1)?))
            })
            .unwrap()
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let finished = loop {
        // Overflows sent with the wake flag land between directories, as they land between
        // the batches of a walk that takes hours.
        h.send(Cmd::Watch(watch::WatchEvent::Failed(fx.root_id)));
        let (status, last) = read();
        if status == RootStatus::Ready && last.is_some_and(|t| t > 1000) {
            break last;
        }
        assert!(Instant::now() < deadline, "the re-walk never finished");
        std::thread::sleep(Duration::from_millis(1));
    };
    let quiet = Instant::now() + Duration::from_secs(1);
    while Instant::now() < quiet {
        assert_eq!(
            read(),
            (RootStatus::Ready, finished),
            "a walk started again right after"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(h);
}

/// VFS-1 AC-7, I10: owed re-walks start one at a time and never while a root is walked; a
/// pass owed for lost events, or by a root that was being walked, waits for the cool-down
/// after that root's last walk, unless the stored time is in the future; a manual request
/// does not wait; a root being removed is dropped.
#[test]
fn owed_rewalks_start_one_at_a_time_after_cooldown() {
    let mut fx = fixture(&["A"]);
    let other_tree = tempfile::tempdir().unwrap();
    let other_path = dunce::canonicalize(other_tree.path()).unwrap();
    let other = {
        let tx = fx.conn.transaction().unwrap();
        let id = ingest::insert_root(&tx, "test", other_path.to_str().unwrap()).unwrap();
        tx.commit().unwrap();
        id
    };
    fx.scan();

    let mut owed = Owed::new();
    request_rewalk(&fx.conn, &mut owed, &[fx.root_id, other], false).unwrap();
    assert!(owed.values().all(|deferred| !deferred));
    start_owed(&fx.conn, &OsHistory, &mut owed).unwrap();
    assert_eq!(fx.status(), RootStatus::Scanning);
    assert_eq!(fx.status_of(other), RootStatus::Ready, "one at a time");
    start_owed(&fx.conn, &OsHistory, &mut owed).unwrap();
    assert_eq!(fx.status_of(other), RootStatus::Ready, "waits for the walk");

    // A manual request while the first root is walked: one more pass, deferred.
    request_rewalk(&fx.conn, &mut owed, &[fx.root_id], false).unwrap();
    assert_eq!(owed.get(&fx.root_id), Some(&true));
    fx.scan();
    start_owed(&fx.conn, &OsHistory, &mut owed).unwrap();
    assert_eq!(fx.status_of(other), RootStatus::Scanning);
    fx.scan();
    start_owed(&fx.conn, &OsHistory, &mut owed).unwrap();
    assert_eq!(fx.status(), RootStatus::Ready, "cool-down not over");
    assert!(owed.contains_key(&fx.root_id));

    fx.set_last_scan(now_secs() + 86_400);
    start_owed(&fx.conn, &OsHistory, &mut owed).unwrap();
    assert_eq!(
        fx.status(),
        RootStatus::Scanning,
        "a future time is no cool-down"
    );
    assert!(owed.is_empty());
    fx.scan();

    // An overflow right after a walk waits; a manual request then does not.
    request_rewalk(&fx.conn, &mut owed, &[fx.root_id], true).unwrap();
    start_owed(&fx.conn, &OsHistory, &mut owed).unwrap();
    assert_eq!(
        fx.status(),
        RootStatus::Ready,
        "overflow waits for the cool-down"
    );
    request_rewalk(&fx.conn, &mut owed, &[fx.root_id], false).unwrap();
    start_owed(&fx.conn, &OsHistory, &mut owed).unwrap();
    assert_eq!(
        fx.status(),
        RootStatus::Scanning,
        "a manual request does not wait"
    );
    fx.scan();

    request_rewalk(&fx.conn, &mut owed, &[fx.root_id], false).unwrap();
    fx.conn
        .execute(
            "UPDATE roots SET status = ?2 WHERE id = ?1",
            params![fx.root_id, RootStatus::Removing.code()],
        )
        .unwrap();
    start_owed(&fx.conn, &OsHistory, &mut owed).unwrap();
    assert!(owed.is_empty(), "a root being removed is dropped");
}

/// VFS-1 AC-7, I4: a last full scan stored in the future (clock set forward, then
/// corrected) counts as stale; a recent one does not.
#[test]
fn future_last_scan_is_stale() {
    let mut fx = fixture(&["A"]);
    fx.scan();
    fx.set_last_scan(now_secs());
    assert!(stale_roots(&fx.conn, PERIODIC_REWALK_SECS)
        .unwrap()
        .is_empty());
    fx.set_last_scan(now_secs() + 3 * 86_400);
    assert_eq!(
        stale_roots(&fx.conn, PERIODIC_REWALK_SECS).unwrap(),
        vec![fx.root_id]
    );
}

/// Makes a directory unreadable until dropped, so the temp dir can still be deleted.
#[cfg(unix)]
struct Locked(PathBuf);

#[cfg(unix)]
impl Locked {
    fn new(path: PathBuf) -> Locked {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        Locked(path)
    }
}

#[cfg(unix)]
impl Drop for Locked {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

/// VFS-1 AC-7, AC-8, I11: a directory that fails to list keeps its indexed contents, stays
/// pending, counts as an error and is retried only when due, each delay longer than the
/// last; after `RETRY_ATTEMPTS` failures it is left for the next full walk. Once it can be
/// read again, a due retry lists it.
#[cfg(unix)]
#[test]
fn failed_listing_is_retried_with_growing_delay() {
    let mut fx = fixture(&["Abierta", "Cerrada"]);
    fx.scan();
    let files = fx.files();
    let locked = Locked::new(fx.root().join("Cerrada"));
    if fs::read_dir(&locked.0).is_ok() {
        return; // running as root: permissions do not apply
    }
    let (cerrada, _) = fx.dir("Cerrada").unwrap();
    let mut retries = Retries::new();

    assert!(queue_rewalk(&fx.conn, &OsHistory, fx.root_id).unwrap());
    let mut last_delay = Duration::ZERO;
    for attempt in 1..=RETRY_ATTEMPTS {
        let before = Instant::now();
        let summary = fx.run_once(&mut retries);
        assert_eq!(summary.errors, 1, "attempt {attempt}");
        assert_eq!(fx.files(), files, "stale contents are kept");
        assert_eq!(fx.status(), RootStatus::Ready, "the walk still ends");
        if attempt == RETRY_ATTEMPTS {
            assert_eq!(fx.dir("Cerrada").unwrap().1, CLEAN, "gave up");
            assert!(retries.is_empty());
            break;
        }
        assert_ne!(fx.dir("Cerrada").unwrap().1, CLEAN, "left pending");
        let retry = &retries[&cerrada];
        assert_eq!(retry.attempts, attempt);
        let delay = retry.next_at.duration_since(before);
        assert!(delay > last_delay, "delay grows");
        last_delay = delay;
        assert_eq!(fx.run_once(&mut retries).errors, 0, "not due yet");
        retries.get_mut(&cerrada).unwrap().next_at = Instant::now();
    }

    // Readable again: the next due retry lists it.
    let hint = fx.root().join("Cerrada/archivo.txt");
    mark_dirty(&mut fx.conn, &[hint]).unwrap();
    assert_eq!(fx.run_once(&mut retries).errors, 1);
    drop(locked);
    retries.get_mut(&cerrada).unwrap().next_at = Instant::now();
    let summary = fx.run_once(&mut retries);
    assert_eq!((summary.errors, summary.dirs_listed), (0, 1));
    assert_eq!(fx.dir("Cerrada").unwrap().1, CLEAN);
    assert!(retries.is_empty());
}

/// Ids of the `dirs_fts` rows matching an FTS5 expression.
fn dirs_fts(conn: &Connection, query: &str) -> Vec<i64> {
    let mut stmt = conn
        .prepare("SELECT rowid FROM dirs_fts WHERE dirs_fts MATCH ?1 ORDER BY rowid")
        .unwrap();
    let rows = stmt.query_map([query], |r| r.get(0)).unwrap();
    rows.collect::<rusqlite::Result<_>>().unwrap()
}

/// VFS-4 AC-4, VFS-1 AC-7, I9: a folder moved and renamed inside a root is applied as a
/// path update:
/// directory ids and file rows are kept, paths and folder search entries follow the new
/// place, and re-listing both parents afterwards adds or removes nothing. While the source
/// still exists on disk nothing changes.
#[test]
fn directory_rename_updates_paths_in_place() {
    let mut fx = fixture(&["Viejo/Sub", "Destino"]);
    fx.scan();
    let root = fx.root();
    let (from, to) = (root.join("Viejo"), root.join("Destino/Nuevo"));
    let (viejo, _) = fx.dir("Viejo").unwrap();
    let (sub, _) = fx.dir("Viejo/Sub").unwrap();
    let (destino, _) = fx.dir("Destino").unwrap();
    let files = fx.files();
    assert!(!rename_dir(&mut fx.conn, &from, &to).unwrap());

    fs::rename(&from, &to).unwrap();
    assert!(rename_dir(&mut fx.conn, &from, &to).unwrap());
    assert_eq!(fx.dir("Destino/Nuevo").map(|d| d.0), Some(viejo));
    assert_eq!(fx.dir("Destino/Nuevo/Sub").map(|d| d.0), Some(sub));
    assert!(fx.dir("Viejo").is_none());
    let parent: i64 = fx
        .conn
        .query_row("SELECT parent_id FROM dirs WHERE id = ?1", [viejo], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(parent, destino);
    assert_eq!(dirs_fts(&fx.conn, "name_key : \"nuevo\""), vec![viejo]);
    assert_eq!(
        dirs_fts(&fx.conn, "path_key : \"destino/nuevo/sub\""),
        vec![sub]
    );
    assert!(dirs_fts(&fx.conn, "\"viejo\"").is_empty());
    let file_paths = |query: &str| -> Vec<i64> {
        let mut stmt = fx
            .conn
            .prepare("SELECT rowid FROM file_paths_fts WHERE file_paths_fts MATCH ?1")
            .unwrap();
        let rows = stmt.query_map([query], |r| r.get(0)).unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    };
    let sub_file: i64 = fx
        .conn
        .query_row("SELECT id FROM files WHERE dir_id = ?1", [sub], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        file_paths("\"nue\" AND \"sub\" AND \"arc\""),
        vec![sub_file]
    );
    assert!(file_paths("\"vie\"").is_empty());

    mark_dirty(&mut fx.conn, &[from, to]).unwrap();
    let summary = fx.scan();
    assert_eq!((summary.files_added, summary.files_removed), (0, 0));
    assert_eq!(fx.files(), files);
}

/// VFS-1 AC-4, I13: a directory that was indexed and is
/// later replaced by a symbolic link to a folder outside the root is treated as removed when
/// it or a directory below it is listed again; nothing outside the root is indexed.
#[cfg(unix)]
#[test]
fn directory_replaced_by_a_link_is_not_followed() {
    let mut fx = fixture(&["Dentro/Sub", "Queda"]);
    fx.scan();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(outside.path().join("Sub")).unwrap();
    fs::write(outside.path().join("Sub").join("fuera.txt"), b"x").unwrap();
    fs::write(outside.path().join("fuera.txt"), b"x").unwrap();
    let root = fx.root();
    fs::remove_dir_all(root.join("Dentro")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("Dentro")).unwrap();

    // A directory below the link is due for a re-list (a watcher hint or a retry) and is
    // listed before its parent notices the change.
    fx.conn
        .execute(
            "UPDATE dirs SET scan_state = ?1 WHERE rel_path = 'Dentro/Sub'",
            [ScanState::PendingShallow as i64],
        )
        .unwrap();
    let outside_rows = |fx: &Fx| -> i64 {
        fx.conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE name = 'fuera.txt'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    fx.scan();
    assert_eq!(outside_rows(&fx), 0);
    assert!(fx.dir("Dentro/Sub").is_none());
    mark_dirty(&mut fx.conn, &[root.join("Dentro")]).unwrap();
    fx.scan();
    assert_eq!(outside_rows(&fx), 0);
    assert!(fx.dir("Dentro").is_none());
    assert!(fx.dir("Dentro/Sub").is_none());
    assert!(fx.dir("Queda").is_some());
}

/// VFS-1 AC-6, C5: the unreadable copy a recovery leaves as `index.db.bad` (with `-wal` and
/// `-shm`) is kept while the rebuild is incomplete, a root that cannot be reached included,
/// and deleted once every root has finished a full scan.
#[test]
fn recovered_copy_is_deleted_once_the_rebuild_completes() {
    let mut fx = fixture(&["Fotos"]);
    let bad = db::bad_path(&fx.db_path);
    let companions: Vec<PathBuf> = ["", "-wal", "-shm"]
        .iter()
        .map(|s| PathBuf::from(format!("{}{s}", bad.display())))
        .collect();
    for p in &companions {
        fs::write(p, b"old").unwrap();
    }
    let tx = fx.conn.transaction().unwrap();
    let missing = fx.tree.path().join("no-existe");
    let gone = ingest::insert_root(&tx, "test", missing.to_str().unwrap()).unwrap();
    tx.commit().unwrap();
    fx.scan();
    assert_eq!(fx.status_of(gone), RootStatus::Unavailable);
    assert!(companions.iter().all(|p| p.exists()), "rebuild incomplete");

    let tx = fx.conn.transaction().unwrap();
    ingest::delete_root(&tx, gone).unwrap();
    tx.commit().unwrap();
    fx.scan();
    assert!(companions.iter().all(|p| !p.exists()));
}

/// Ordered log shared by a recording priority backend, the commit hook and fake probes.
type Log = Arc<Mutex<Vec<String>>>;

fn push(log: &Log, s: &str) {
    log.lock().unwrap().push(s.to_owned());
}

/// A schedule recording priority calls into `log`, with `load` as the disk probe.
fn recording_sched(
    log: &Log,
    cfg: SchedConfig,
    load: Option<Box<dyn platform::DiskLoad>>,
) -> Sched {
    Sched::new(
        cfg,
        Box::new(sched::tests::Recorder(Arc::clone(log))),
        load,
        Box::new(|_| {}),
    )
}

impl Fx {
    /// Records every commit of the fixture's connection into `log`.
    fn log_commits(&mut self, log: &Log) {
        let l = Arc::clone(log);
        self.conn
            .commit_hook(Some(move || {
                push(&l, "commit");
                false
            }))
            .unwrap();
    }

    fn run_with(&mut self, sched: &mut Sched) -> ScanSummary {
        let mut pass = Pass::default();
        run(
            &mut self.conn,
            &OsHistory,
            &mut pass,
            &mut Retries::new(),
            &|| false,
            false,
            sched,
            &mut |_: &Connection, _: &Pass, _: bool| {},
        )
        .unwrap();
        pass.summary
    }

    fn mark(&mut self, rels: &[&str]) {
        let root = self.root();
        let paths: Vec<PathBuf> = rels.iter().map(|r| root.join(r).join("x")).collect();
        mark_dirty(&mut self.conn, &paths).unwrap();
    }
}

/// VFS-5 AC-1: a first index runs entirely in the walk profile: no listing enters background
/// mode.
#[test]
fn first_index_never_enters_background_mode() {
    let mut fx = fixture(&["A/B", "C", "D/E/F"]);
    let log: Log = Arc::default();
    let mut s = recording_sched(&log, SchedConfig::default(), None);
    let summary = fx.run_with(&mut s);
    assert_eq!(summary.dirs_listed, 7);
    assert_eq!(s.mode, Mode::Walk);
    assert!(log.lock().unwrap().is_empty(), "{:?}", log.lock().unwrap());
}

/// VFS-5 AC-2, AC-3: a few watcher-marked directories are listed one by one in background
/// mode, and every commit happens after the matching background-end, never inside; more
/// than `incremental_max_dirs` pending directories use the walk profile.
#[test]
fn incremental_work_lists_in_background_and_commits_outside() {
    let mut fx = fixture(&["A", "B", "C"]);
    fx.scan();
    let log: Log = Arc::default();
    fx.log_commits(&log);
    fx.mark(&["A", "B", "C"]);
    log.lock().unwrap().clear();

    let mut s = recording_sched(&log, SchedConfig::default(), None);
    let summary = fx.run_with(&mut s);
    assert_eq!(summary.dirs_listed, 3);
    assert_eq!(s.mode, Mode::Incremental);
    let calls = log.lock().unwrap().clone();
    assert_eq!(
        calls.iter().filter(|c| *c == "bg_begin").count(),
        3,
        "{calls:?}"
    );
    let mut background = false;
    for c in &calls {
        match c.as_str() {
            "bg_begin" => {
                assert!(!background, "nested background: {calls:?}");
                background = true;
            }
            "bg_end" => background = false,
            "commit" => assert!(!background, "commit in background mode: {calls:?}"),
            _ => {}
        }
    }
    assert!(!background);
    assert!(
        calls.iter().filter(|c| *c == "commit").count() >= 3,
        "{calls:?}"
    );

    fx.mark(&["A", "B", "C"]);
    log.lock().unwrap().clear();
    let cfg = SchedConfig {
        incremental_max_dirs: 2,
        ..SchedConfig::default()
    };
    let mut s = recording_sched(&log, cfg, None);
    assert_eq!(fx.run_with(&mut s).dirs_listed, 3);
    assert_eq!(s.mode, Mode::Walk);
    assert!(!log.lock().unwrap().iter().any(|c| c == "bg_begin"));
}

/// A probe reporting a full disk queue, which also records whether another connection
/// could take the write lock at that moment.
struct BusyProbe {
    log: Log,
    other: Connection,
}

impl platform::DiskLoad for BusyProbe {
    fn sample(&mut self) -> Option<Vec<(String, f64)>> {
        let free = self
            .other
            .execute_batch("BEGIN IMMEDIATE; ROLLBACK;")
            .is_ok();
        push(
            &self.log,
            if free { "sample_free" } else { "sample_locked" },
        );
        Some(vec![("_Total".into(), 100.0)])
    }
}

/// VFS-5 AC-5, AC-6: on a permanently busy disk a walk still finishes, pausing between
/// directories, and it commits before each pause, so another writer can take the lock while
/// it waits.
#[test]
fn busy_disk_walk_commits_before_pausing_and_still_finishes() {
    let dirs: Vec<String> = (0..40).map(|i| format!("d{i:02}")).collect();
    let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    let mut fx = fixture(&dirs);
    let log: Log = Arc::default();
    fx.log_commits(&log);
    let other = Connection::open(&fx.db_path).unwrap();
    other.busy_timeout(Duration::ZERO).unwrap();
    let cfg = SchedConfig {
        sample_every: Duration::from_millis(2),
        max_backoff: Duration::from_millis(20),
        min_work: Duration::from_millis(5),
        ..SchedConfig::default()
    };
    let probe = BusyProbe {
        log: Arc::clone(&log),
        other,
    };
    let mut s = recording_sched(&log, cfg, Some(Box::new(probe)));
    let started = Instant::now();
    let summary = fx.run_with(&mut s);
    assert_eq!(
        summary.dirs_listed, 41,
        "the walk finished despite the busy disk"
    );
    assert!(started.elapsed() >= Duration::from_millis(20), "it paused");
    let calls = log.lock().unwrap().clone();
    let commits = calls.iter().filter(|c| *c == "commit").count();
    // Without back-off the 41 directories fit in one batch: a single commit.
    assert!(commits >= 2, "no commit before a pause: {calls:?}");
    assert!(
        calls.iter().any(|c| c == "sample_free"),
        "another writer never got the lock while paused: {calls:?}"
    );
    assert_eq!(fx.status(), RootStatus::Ready);
}

/// A diagnostic sink collecting records.
fn capture() -> (Log, sched::DiagSink) {
    let log: Log = Arc::default();
    let l = Arc::clone(&log);
    (log, Box::new(move |s: &str| push(&l, s)))
}

fn wait_until(what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// VFS-5 AC-7: started 2 "minutes" after boot (scaled down), the catch-up re-walk waits
/// until uptime reaches the boot grace, while a manual re-walk of another root starts at
/// once.
#[test]
fn catchup_waits_for_boot_grace_but_manual_rewalk_does_not() {
    let mut fx = fixture(&["A"]);
    let tree2 = tempfile::tempdir().unwrap();
    let root2 = dunce::canonicalize(tree2.path()).unwrap();
    fs::create_dir(root2.join("B")).unwrap();
    let tx = fx.conn.transaction().unwrap();
    let id2 = ingest::insert_root(&tx, "dos", root2.to_str().unwrap()).unwrap();
    tx.commit().unwrap();
    fx.scan();
    fx.set_last_scan(1000);
    fx.conn
        .execute("UPDATE roots SET last_full_scan_at = 1000", [])
        .unwrap();
    // No change cursors: both roots owe a catch-up walk (VFS-9 R3).
    fx.conn
        .execute("DELETE FROM meta WHERE key LIKE 'cursor/%'", [])
        .unwrap();

    let grace = Duration::from_millis(1500);
    let cfg = SchedConfig {
        boot_grace: grace,
        ..SchedConfig::default()
    };
    let (records, sink) = capture();
    let sched = Sched::new(
        cfg,
        Box::new(platform::OsThreadPriority::default()),
        None,
        sink,
    );
    let started = Instant::now();
    let h = IndexerHandle::spawn_with(
        fx.db_path.clone(),
        Arc::new(NullSink),
        sched,
        Some(Duration::from_millis(200)),
    )
    .unwrap();
    h.rescan(Some(id2));
    let reader = db::open_ro(&fx.db_path).unwrap();
    let last = |id: i64| -> i64 {
        reader
            .query_row(
                "SELECT last_full_scan_at FROM roots WHERE id = ?1",
                [id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .unwrap()
            .unwrap_or(0)
    };
    wait_until("the manual re-walk", Duration::from_secs(10), || {
        last(id2) > 1000
    });
    assert!(
        started.elapsed() < grace - Duration::from_millis(400),
        "the manual re-walk waited for the boot grace"
    );
    assert_eq!(
        last(fx.root_id),
        1000,
        "the catch-up started during the grace"
    );
    wait_until("the catch-up", Duration::from_secs(10), || {
        last(fx.root_id) > 1000
    });
    assert!(started.elapsed() >= grace - Duration::from_millis(250));
    drop(h);
    let records = records.lock().unwrap();
    assert!(
        records[0].starts_with("start uptime_s=0 catchup_deferred_s=1 probe=none"),
        "{records:?}"
    );
}

/// VFS-5 AC-8, AC-9: a running indexer writes `start`, `pass_end` and `watch_overflow`
/// records, and none of them names a file or folder, even when a listing fails.
#[test]
fn diagnostic_records_never_name_files() {
    let fx = fixture(&["Zorropálido_secreto/Quokka_privado"]);
    fs::write(fx.root().join("Ornitorrinco_carta.txt"), b"x").unwrap();
    let locked = fx.root().join("Pangolín_cerrado");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("Lémur_nota.txt"), b"x").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    }
    let (records, sink) = capture();
    let sched = Sched::new(
        SchedConfig::default(),
        Box::new(platform::OsThreadPriority::default()),
        None,
        sink,
    );
    let h = IndexerHandle::spawn_with(fx.db_path.clone(), Arc::new(NullSink), sched, None).unwrap();
    wait_until("the first pass", Duration::from_secs(20), || {
        records
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.starts_with("pass_end "))
    });
    h.send(Cmd::Watch(watch::WatchEvent::Failed(fx.root_id)));
    wait_until("the overflow record", Duration::from_secs(10), || {
        records
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.starts_with("watch_overflow root="))
    });
    drop(h);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let records = records.lock().unwrap();
    assert!(records[0].starts_with("start "), "{records:?}");
    let end = records.iter().find(|r| r.starts_with("pass_end ")).unwrap();
    assert!(end.contains("mode=walk"), "{end}");
    if cfg!(unix) {
        assert!(end.contains("errors=1"), "{end}");
    }
    let root = fx.root().to_string_lossy().into_owned();
    for r in records.iter() {
        for name in [
            "Zorro",
            "Quokka",
            "Ornitorrinco",
            "Pangol",
            "Lémur",
            "secreto",
            root.as_str(),
        ] {
            assert!(!r.contains(name), "{name} in diagnostic record {r}");
        }
    }
}

/// A change history that hands out `fake:<n>` cursors and one scripted replay.
#[derive(Default)]
struct FakeHistory {
    next: std::sync::atomic::AtomicU64,
    replay: Mutex<Option<cursor::Replayed>>,
    /// Replays asked for so far.
    replays: std::sync::atomic::AtomicUsize,
}

/// A fake history the test keeps a handle on, to script replays while the indexer runs.
impl History for Arc<FakeHistory> {
    fn current(&self, root: &Path) -> Option<String> {
        (**self).current(root)
    }

    fn replay(&self, root: &Path, cursor: &str) -> cursor::Replayed {
        (**self).replay(root, cursor)
    }
}

impl History for FakeHistory {
    fn current(&self, _root: &Path) -> Option<String> {
        Some(format!("fake:{}", self.next.fetch_add(1, Ordering::SeqCst)))
    }

    fn replay(&self, _root: &Path, _cursor: &str) -> cursor::Replayed {
        self.replays.fetch_add(1, Ordering::SeqCst);
        self.replay
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Err("no replay scripted".into()))
    }
}

impl Fx {
    fn cursor(&self) -> Option<String> {
        cursor::load(&self.conn, self.root_id).unwrap()
    }
}

/// VFS-9 AC-4, AC-6 (R2, R7): a replay marks pending shallow exactly the folders it names (an
/// unindexed one marks its nearest indexed ancestor, never the parent of an indexed one),
/// drops paths outside the root, excluded names and the data directory, stores the new
/// cursor, and owes no full walk.
#[test]
fn replay_marks_named_folders_and_stores_the_cursor() {
    let mut fx = fixture(&["A/B", "C", "D"]);
    fx.scan();
    cursor::store(&fx.conn, fx.root_id, Some("fake:old")).unwrap();
    let root = fx.root();
    let data = root.join("D");
    let history = FakeHistory::default();
    *history.replay.lock().unwrap() = Some(Ok((
        vec![
            root.join("A/B"),
            root.join("C/nueva/sub"),
            root.join("@eaDir"),
            data.clone(),
            PathBuf::from("/fuera/de/la/raiz"),
        ],
        "fake:new".into(),
    )));
    let owed = catch_up(&mut fx.conn, &history, &watch::Filter::new(data)).unwrap();
    assert!(owed.is_empty());
    let shallow = ScanState::PendingShallow as i64;
    assert_eq!(fx.dir("A/B").unwrap().1, shallow);
    assert_eq!(fx.dir("C").unwrap().1, shallow, "nearest indexed ancestor");
    for clean in ["", "A", "D"] {
        assert_eq!(fx.dir(clean).unwrap().1, CLEAN, "{clean:?}");
    }
    assert_eq!(fx.cursor().as_deref(), Some("fake:new"));
}

/// VFS-9 AC-4 (R3): a lost history, or no cursor at all, owes the root a full walk and keeps
/// the old cursor until that walk starts, which stores the cursor taken just before it.
#[test]
fn lost_history_owes_a_walk_that_stores_a_new_cursor() {
    let mut fx = fixture(&["A"]);
    fx.scan();
    cursor::store(&fx.conn, fx.root_id, None).unwrap();
    let history = FakeHistory::default();
    let filter = watch::Filter::new(PathBuf::new());
    assert_eq!(
        catch_up(&mut fx.conn, &history, &filter).unwrap(),
        vec![fx.root_id]
    );

    cursor::store(&fx.conn, fx.root_id, Some("fake:old")).unwrap();
    *history.replay.lock().unwrap() = Some(Err("journal trimmed past the cursor".into()));
    assert_eq!(
        catch_up(&mut fx.conn, &history, &filter).unwrap(),
        vec![fx.root_id]
    );
    assert_eq!(fx.cursor().as_deref(), Some("fake:old"));
    assert_eq!(fx.dir("A").unwrap().1, CLEAN, "nothing marked");

    assert!(queue_rewalk(&fx.conn, &history, fx.root_id).unwrap());
    assert_eq!(fx.cursor().as_deref(), Some("fake:0"));
    assert_eq!(fx.dir("").unwrap().1, ScanState::PendingDeep as i64);
}

/// VFS-9 AC-4 (R1, R8): a first scan stores the cursor taken when it starts, and removing
/// the root deletes it with the rows.
#[test]
fn first_scan_stores_a_cursor_and_removal_deletes_it() {
    let mut fx = fixture(&["A"]);
    let history = FakeHistory::default();
    let mut pass = Pass::default();
    run(
        &mut fx.conn,
        &history,
        &mut pass,
        &mut Retries::new(),
        &|| false,
        false,
        &mut Sched::inert(),
        &mut |_: &Connection, _: &Pass, _: bool| {},
    )
    .unwrap();
    assert_eq!(fx.status(), RootStatus::Ready);
    assert_eq!(fx.cursor().as_deref(), Some("fake:0"));

    crate::roots::remove_root(&mut fx.conn, fx.root_id).unwrap();
    assert!(finish_removing(&fx.conn, &|| false).unwrap());
    assert_eq!(fx.cursor(), None);
}

/// VFS-9 AC-4 (R8): a folder added again while its old root is still being removed deletes
/// the old root's cursor with its rows, so a root that reuses the id starts without it.
#[test]
fn readding_a_removing_root_deletes_its_cursor() {
    let mut fx = fixture(&["A"]);
    cursor::store(&fx.conn, fx.root_id, Some("fake:old")).unwrap();
    crate::roots::remove_root(&mut fx.conn, fx.root_id).unwrap();
    let root = fx.root();
    crate::roots::add_root(&mut fx.conn, &root).unwrap();
    assert_eq!(cursor::load(&fx.conn, fx.root_id).unwrap(), None);
}

/// The stored cursor of `root_id`, read on its own connection.
fn stored_cursor(db_path: &Path, root_id: i64) -> Option<String> {
    cursor::load(&db::open_ro(db_path).unwrap(), root_id).unwrap()
}

/// VFS-9 AC-4 (R6): while watched, a root's cursor advances at each check to the one taken
/// at the previous check; after a single lost-events report it stays put, check after check,
/// until a full walk stores a new one; then it advances again.
#[test]
fn cursor_holds_after_lost_events_until_a_walk() {
    let mut fx = fixture(&["A"]);
    fx.scan();
    cursor::store(&fx.conn, fx.root_id, Some("fake:start")).unwrap();
    // The start-up replay is lost, so an undeferred walk is owed and runs at once.
    let h = IndexerHandle::spawn_with_history(
        fx.db_path.clone(),
        Arc::new(NullSink),
        Sched::inert(),
        None,
        Box::new(FakeHistory::default()),
        Timing {
            cursor_every: Duration::from_millis(50),
            ..Timing::default()
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let cursor = || stored_cursor(&fx.db_path, fx.root_id);
    let wait_change = |from: Option<String>, what: &str| {
        while cursor() == from {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    wait_change(
        Some("fake:start".into()),
        "the start-up walk stored no cursor",
    );
    while fx.status() != RootStatus::Ready {
        assert!(
            Instant::now() < deadline,
            "the start-up walk never finished"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    wait_change(cursor(), "the cursor never advanced");

    h.send(Cmd::Watch(watch::WatchEvent::Failed(fx.root_id)));
    std::thread::sleep(Duration::from_millis(150));
    let held = cursor();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(cursor(), held, "the cursor advanced after lost events");
    assert_eq!(
        fx.status(),
        RootStatus::Ready,
        "the owed walk waits for its cooldown"
    );

    // A manual re-walk starts at once and stores its cursor; events lost right after it
    // hold that new cursor too.
    h.rescan(Some(fx.root_id));
    wait_change(held, "the walk stored no cursor");
    let walked = cursor();
    h.send(Cmd::Watch(watch::WatchEvent::Failed(fx.root_id)));
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        cursor(),
        walked,
        "the cursor advanced after events lost after the walk"
    );

    // The next walk releases the hold; advancing resumes after it.
    h.rescan(Some(fx.root_id));
    wait_change(walked, "the second walk stored no cursor");
    while fx.status() != RootStatus::Ready {
        assert!(Instant::now() < deadline, "the walk never finished");
        std::thread::sleep(Duration::from_millis(10));
    }
    wait_change(cursor(), "the cursor never advanced after the walk");
    drop(h);
}

/// VFS-9 AC-4 (R3, R6): a root owed a walk at start keeps its cursor through every check
/// while the walk waits (here for the boot grace), and the walk stores a new one.
#[test]
fn owed_walk_holds_the_cursor_until_it_starts() {
    let mut fx = fixture(&["A"]);
    fx.scan();
    cursor::store(&fx.conn, fx.root_id, Some("fake:start")).unwrap();
    let cfg = SchedConfig {
        boot_grace: Duration::from_millis(1500),
        ..SchedConfig::default()
    };
    let sched = Sched::new(
        cfg,
        Box::new(platform::OsThreadPriority::default()),
        None,
        Box::new(|_| {}),
    );
    // The scripted replay is lost; booted just now, so the walk waits for the grace.
    let h = IndexerHandle::spawn_with_history(
        fx.db_path.clone(),
        Arc::new(NullSink),
        sched,
        Some(Duration::ZERO),
        Box::new(FakeHistory::default()),
        Timing {
            cursor_every: Duration::from_millis(50),
            ..Timing::default()
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(
        stored_cursor(&fx.db_path, fx.root_id).as_deref(),
        Some("fake:start"),
        "the cursor advanced before the owed walk"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while stored_cursor(&fx.db_path, fx.root_id).as_deref() == Some("fake:start") {
        assert!(Instant::now() < deadline, "the owed walk stored no cursor");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(h);
}

/// VFS-9 AC-5 (R6): on the real change history, a running indexer moves a watched root's
/// cursor forward from the one its first scan stored.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn os_cursor_advances_while_running() {
    let mut fx = fixture(&["A"]);
    fx.scan();
    let first = fx.cursor().expect("the first scan stores a cursor");
    let position = |c: &str| -> i128 { c.rsplit(':').next().unwrap().parse().unwrap() };
    let h = IndexerHandle::spawn_with_history(
        fx.db_path.clone(),
        Arc::new(NullSink),
        Sched::inert(),
        None,
        Box::new(OsHistory),
        Timing {
            cursor_every: Duration::from_millis(200),
            ..Timing::default()
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut n = 0;
    loop {
        // Every change moves the history forward.
        n += 1;
        fs::write(fx.root().join(format!("A/nuevo {n}.txt")), b"x").unwrap();
        if stored_cursor(&fx.db_path, fx.root_id).is_some_and(|c| position(&c) > position(&first)) {
            break;
        }
        assert!(Instant::now() < deadline, "the cursor never advanced");
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(h);
}

/// VFS-18 R9: a link file that could not be read is read again after the next full walk,
/// not before.
#[test]
fn unreadable_link_is_retried_after_a_full_walk() {
    let mut fx = fixture(&["A"]);
    fs::write(
        fx.root().join("A").join("Banco.url"),
        "[InternetShortcut]\r\nURL=https://example.com/\r\n",
    )
    .unwrap();
    fx.scan();
    let id: i64 = fx
        .conn
        .query_row("SELECT file_id FROM links", [], |r| r.get(0))
        .unwrap();
    let state = |conn: &Connection| -> i64 {
        conn.query_row("SELECT state FROM links WHERE file_id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    };
    // The deferred pass found it locked.
    let tx = fx.conn.transaction().unwrap();
    ingest::set_link_target(&tx, id, Err(())).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        crate::link::read_pending(&mut fx.conn, &|| false).unwrap(),
        0
    );
    assert_eq!(state(&fx.conn), ingest::LinkState::Unreadable as i64);

    assert!(queue_rewalk(&fx.conn, &OsHistory, fx.root_id).unwrap());
    fx.scan();
    assert_eq!(state(&fx.conn), ingest::LinkState::Pending as i64);
    assert_eq!(
        crate::link::read_pending(&mut fx.conn, &|| false).unwrap(),
        1
    );
    assert_eq!(state(&fx.conn), ingest::LinkState::Read as i64);
}

/// VFS-18 R9: a pending link whose path cannot be built (its file row is gone) is marked
/// unreadable; the pass goes on and reads the other links instead of failing every time.
#[test]
fn a_broken_link_row_does_not_stall_the_link_pass() {
    let mut fx = fixture(&["A"]);
    fs::write(
        fx.root().join("A").join("Banco.url"),
        "[InternetShortcut]\r\nURL=https://example.com/\r\n",
    )
    .unwrap();
    fx.scan();
    fx.conn
        .execute(
            "INSERT INTO links(file_id, kind) VALUES (0, ?1)",
            [crate::types::LinkKind::Shortcut.code()],
        )
        .unwrap();
    assert_eq!(
        crate::link::read_pending(&mut fx.conn, &|| false).unwrap(),
        2
    );
    let states: Vec<(i64, i64)> = fx
        .conn
        .prepare("SELECT file_id, state FROM links ORDER BY file_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(states[0], (0, ingest::LinkState::Unreadable as i64));
    assert_eq!(states[1].1, ingest::LinkState::Read as i64);
}

/// The unreadable reason stored on the directory `rel`, if any (VFS-10).
fn reason_of(conn: &Connection, rel: &str) -> Option<UnreadableReason> {
    conn.query_row("SELECT error FROM dirs WHERE rel_path = ?1", [rel], |r| {
        r.get::<_, Option<i64>>(0)
    })
    .unwrap()
    .map(UnreadableReason::from_code)
}

/// The root's unreadable count as the folders panel reads it (VFS-10 R4).
fn unreadable_count(conn: &Connection) -> u64 {
    crate::roots::list_roots(conn).unwrap()[0].unreadable
}

/// VFS-10 AC-1, AC-2: a folder the user cannot read is recorded "no permission" at its first
/// failure and counts at once, while it is still retried; a re-walk that fails again keeps
/// it; once readable and listed, it is forgotten; a deleted folder takes its reason along.
#[cfg(unix)]
#[test]
fn unreadable_folder_is_recorded_kept_and_forgotten() {
    let mut fx = fixture(&["Abierta", "Cerrada", "Borrada"]);
    fx.scan();
    let locked = Locked::new(fx.root().join("Cerrada"));
    if fs::read_dir(&locked.0).is_ok() {
        return; // running as root: permissions do not apply
    }
    let (cerrada, _) = fx.dir("Cerrada").unwrap();
    let mut retries = Retries::new();
    assert!(queue_rewalk(&fx.conn, &OsHistory, fx.root_id).unwrap());
    fx.run_once(&mut retries);
    assert_eq!(
        reason_of(&fx.conn, "Cerrada"),
        Some(UnreadableReason::NoPermission)
    );
    assert_eq!(unreadable_count(&fx.conn), 1);
    assert_ne!(fx.dir("Cerrada").unwrap().1, CLEAN, "still retried");
    assert_eq!(retries[&cerrada].attempts, 1);
    let list = crate::roots::unreadable_dirs(&fx.conn, fx.root_id).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, cerrada);
    assert_eq!(Path::new(&list[0].path), fx.root().join("Cerrada"));
    assert_eq!(list[0].reason, UnreadableReason::NoPermission);
    assert_eq!(reason_of(&fx.conn, "Abierta"), None);

    // A later full walk queues it again; failing again keeps it counted.
    retries.clear();
    assert!(queue_rewalk(&fx.conn, &OsHistory, fx.root_id).unwrap());
    fx.run_once(&mut retries);
    assert_eq!(unreadable_count(&fx.conn), 1);

    // Readable again: the next listing forgets it.
    drop(locked);
    retries.get_mut(&cerrada).unwrap().next_at = Instant::now();
    fx.run_once(&mut retries);
    assert_eq!(reason_of(&fx.conn, "Cerrada"), None);
    assert_eq!(unreadable_count(&fx.conn), 0);

    // A recorded folder deleted from disk goes with its reason.
    let (borrada, _) = fx.dir("Borrada").unwrap();
    let tx = fx.conn.transaction().unwrap();
    ingest::record_unreadable(&tx, borrada, UnreadableReason::DiskError, true).unwrap();
    tx.commit().unwrap();
    assert_eq!(unreadable_count(&fx.conn), 1);
    fs::remove_dir_all(fx.root().join("Borrada")).unwrap();
    assert!(queue_rewalk(&fx.conn, &OsHistory, fx.root_id).unwrap());
    fx.scan();
    assert_eq!(unreadable_count(&fx.conn), 0);
}

/// VFS-10 AC-6: a folder whose listing fails with an error other than "no permission" (here
/// a file now stands where the folder was) gets no reason while it is retried, and "disk
/// error" once its retries are exhausted; a re-walk that fails again keeps it.
#[test]
fn other_failures_count_only_after_the_retries() {
    let mut fx = fixture(&["Abierta", "Rota"]);
    fx.scan();
    fs::remove_dir_all(fx.root().join("Rota")).unwrap();
    fs::write(fx.root().join("Rota"), b"x").unwrap();
    let (rota, _) = fx.dir("Rota").unwrap();
    let tx = fx.conn.transaction().unwrap();
    ingest::set_dir_state(&tx, rota, ingest::ScanState::PendingShallow).unwrap();
    tx.commit().unwrap();
    let mut retries = Retries::new();
    for attempt in 1..=RETRY_ATTEMPTS {
        assert_eq!(fx.run_once(&mut retries).errors, 1, "attempt {attempt}");
        if attempt < RETRY_ATTEMPTS {
            assert_eq!(reason_of(&fx.conn, "Rota"), None, "attempt {attempt}");
            assert_eq!(unreadable_count(&fx.conn), 0);
            retries.get_mut(&rota).unwrap().next_at = Instant::now();
        }
    }
    assert_eq!(
        reason_of(&fx.conn, "Rota"),
        Some(UnreadableReason::DiskError)
    );
    assert_eq!(unreadable_count(&fx.conn), 1);

    // Queued again and failing once more: still counted (VFS-10 AC-2).
    let tx = fx.conn.transaction().unwrap();
    ingest::set_dir_state(&tx, rota, ingest::ScanState::PendingShallow).unwrap();
    tx.commit().unwrap();
    fx.run_once(&mut retries);
    assert_eq!(unreadable_count(&fx.conn), 1);
}

/// VFS-10 AC-2: removing a root takes the reasons of its folders along.
#[test]
fn removing_a_root_forgets_its_unreadable_folders() {
    let mut fx = fixture(&["A"]);
    fx.scan();
    let (a, _) = fx.dir("A").unwrap();
    let tx = fx.conn.transaction().unwrap();
    ingest::record_unreadable(&tx, a, UnreadableReason::CloudOnly, true).unwrap();
    tx.commit().unwrap();
    crate::roots::remove_root(&mut fx.conn, fx.root_id).unwrap();
    assert!(finish_removing(&fx.conn, &|| false).unwrap());
    let left: i64 = fx
        .conn
        .query_row(
            "SELECT COUNT(*) FROM dirs WHERE error IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0);
}

/// The file `name` in the folder `dir` (`/`-separated, below the root) as (id, cloud-only
/// bit).
fn cloud_bit(conn: &Connection, dir: &str, name: &str) -> (i64, bool) {
    conn.query_row(
        "SELECT f.id, f.flags & ?3 FROM files f JOIN dirs d ON d.id = f.dir_id \
         WHERE d.rel_path = ?1 AND f.name = ?2",
        rusqlite::params![dir, name, ingest::FLAG_CLOUD_ONLY],
        |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
    )
    .unwrap()
}

/// VFS-25 AC-3: a cloud-only `.url` is never opened by the link pass (it would get its
/// address as target if it were): no target, unreadable; once a listing finds it local, its
/// link row is pending again and the next pass reads the address.
#[test]
fn cloud_only_link_is_never_opened() {
    let mut fx = fixture(&["A"]);
    fs::write(
        fx.root().join("A").join("Banco.url"),
        "[InternetShortcut]\r\nURL=https://example.com/\r\n",
    )
    .unwrap();
    fx.scan();
    let (id, _) = cloud_bit(&fx.conn, "A", "Banco.url");
    let tx = fx.conn.transaction().unwrap();
    ingest::set_cloud_only(&tx, id, true).unwrap();
    tx.commit().unwrap();
    let link = |conn: &Connection| -> (i64, Option<String>) {
        conn.query_row(
            "SELECT state, target FROM links WHERE file_id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(
        crate::link::read_pending(&mut fx.conn, &|| false).unwrap(),
        1
    );
    assert_eq!(link(&fx.conn), (ingest::LinkState::Unreadable as i64, None));

    // The folder is listed again and the file is local (no placeholder bit here).
    let hint = fx.root().join("A").join("Banco.url");
    mark_dirty(&mut fx.conn, &[hint]).unwrap();
    fx.run_once(&mut Retries::new());
    assert_eq!(cloud_bit(&fx.conn, "A", "Banco.url"), (id, false));
    assert_eq!(link(&fx.conn).0, ingest::LinkState::Pending as i64);
    crate::link::read_pending(&mut fx.conn, &|| false).unwrap();
    assert_eq!(
        link(&fx.conn),
        (
            ingest::LinkState::Read as i64,
            Some("https://example.com/".into())
        )
    );
}

/// VFS-25 AC-2 (Windows): a file marked `FILE_ATTRIBUTE_OFFLINE` is indexed cloud-only and its
/// hit says so, an ordinary file's does not; clearing the attribute with size and time
/// unchanged clears the flag at the next listing; setting it with a size change sets it again.
#[cfg(windows)]
#[test]
fn offline_attribute_marks_a_file_cloud_only() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        SetFileAttributesW, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_OFFLINE,
    };
    let set = |path: &Path, attrs: u32| {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: `wide` is a NUL-terminated path that outlives the call.
        assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), attrs) }, 0);
    };
    let mut fx = fixture(&["A"]);
    let nube = fx.root().join("A").join("boda.mp4");
    fs::write(&nube, b"xx").unwrap();
    set(&nube, FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_ARCHIVE);
    fx.scan();
    assert!(cloud_bit(&fx.conn, "A", "boda.mp4").1);
    assert!(!cloud_bit(&fx.conn, "A", "archivo.txt").1);
    let vfs = crate::Vfs::open(&fx.db_path).unwrap();
    let hits = vfs
        .search(&crate::SearchQuery {
            text: String::new(),
            filters: crate::SearchFilters {
                folder: Some(fx.root().join("A").to_string_lossy().into_owned()),
                ..Default::default()
            },
            limit: 10,
            offset: 0,
            sort: Default::default(),
        })
        .unwrap()
        .hits;
    let by_name = |n: &str| hits.iter().find(|h| h.name == n).unwrap().cloud_only;
    assert!(by_name("boda.mp4"));
    assert!(!by_name("archivo.txt"));

    set(&nube, FILE_ATTRIBUTE_ARCHIVE);
    mark_dirty(&mut fx.conn, std::slice::from_ref(&nube)).unwrap();
    fx.run_once(&mut Retries::new());
    assert!(!cloud_bit(&fx.conn, "A", "boda.mp4").1);

    fs::write(&nube, b"xxxx").unwrap();
    set(&nube, FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_ARCHIVE);
    mark_dirty(&mut fx.conn, &[nube]).unwrap();
    fx.run_once(&mut Retries::new());
    assert!(cloud_bit(&fx.conn, "A", "boda.mp4").1);
}

/// An indexer on `fx` with a shared fake history and short clocks: cursor checks every
/// 50 ms, replays 300 ms after the last overflow and at most 1.5 s after the first. Returns
/// once the start-up walk (owed: no cursor) has finished and stored a cursor.
fn overflow_indexer(fx: &Fx) -> (IndexerHandle, Arc<FakeHistory>) {
    let history = Arc::new(FakeHistory::default());
    let h = IndexerHandle::spawn_with_history(
        fx.db_path.clone(),
        Arc::new(NullSink),
        Sched::inert(),
        None,
        Box::new(Arc::clone(&history)),
        Timing {
            cursor_every: Duration::from_millis(50),
            replay_quiet: Duration::from_millis(300),
            replay_max: Duration::from_millis(1500),
        },
    )
    .unwrap();
    wait_for("the start-up walk", || {
        fx.status_of(fx.root_id) == RootStatus::Ready
            && stored_cursor(&fx.db_path, fx.root_id).is_some()
    });
    (h, history)
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Deletes the row of `name` from the index only (the disk is untouched, so no watcher
/// event): it comes back only if its folder is listed again.
fn drop_row(fx: &Fx, name: &str) {
    db::open_rw(&fx.db_path)
        .unwrap()
        .execute("DELETE FROM files WHERE name = ?1", [name])
        .unwrap();
}

fn has_row(fx: &Fx, name: &str) -> bool {
    db::open_ro(&fx.db_path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM files WHERE name = ?1", [name], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
        > 0
}

/// VFS-26 AC-1 (R2, R3): three overflows within the quiet delay give one replay; a cursor
/// check before it leaves the stored cursor alone; the replay re-lists exactly the two
/// folders it names (the third keeps a dropped row), stores its cursor and owes no walk; cursor checks advance it again afterwards.
/// Overflows that keep coming still get their replay by the maximum delay.
#[test]
fn overflow_burst_gives_one_replay() {
    let fx = fixture(&["A", "B", "C"]);
    for (d, f) in [("A", "a.txt"), ("B", "b.txt"), ("C", "c.txt")] {
        fs::write(fx.root().join(d).join(f), b"x").unwrap();
    }
    let (h, history) = overflow_indexer(&fx);
    let checked = fx.last_scan();
    wait_for("the cursor advanced", || {
        stored_cursor(&fx.db_path, fx.root_id).is_some_and(|c| c != "fake:0")
    });
    for f in ["a.txt", "b.txt", "c.txt"] {
        drop_row(&fx, f);
    }
    let named = vec![fx.root().join("A"), fx.root().join("B")];
    *history.replay.lock().unwrap() = Some(Ok((named, "fake:replayed".into())));
    let before = history.replays.load(Ordering::SeqCst);
    for _ in 0..3 {
        h.send(Cmd::Watch(watch::WatchEvent::Overflow(fx.root_id)));
        std::thread::sleep(Duration::from_millis(50));
    }
    let frozen = stored_cursor(&fx.db_path, fx.root_id);
    std::thread::sleep(Duration::from_millis(150)); // three cursor checks
    assert_eq!(
        stored_cursor(&fx.db_path, fx.root_id),
        frozen,
        "a check moved the cursor"
    );
    wait_for("the replay", || {
        has_row(&fx, "a.txt") && has_row(&fx, "b.txt")
    });
    assert_eq!(history.replays.load(Ordering::SeqCst) - before, 1);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!has_row(&fx, "c.txt"), "C was listed: a walk ran");
    assert_eq!(fx.last_scan(), checked, "a full walk ran");
    wait_for("the cursor advanced after the replay", || {
        stored_cursor(&fx.db_path, fx.root_id)
            .is_some_and(|c| c != "fake:replayed" && Some(&c) != frozen.as_ref())
    });

    // A burst that never pauses is replayed by the maximum delay.
    *history.replay.lock().unwrap() = Some(Ok((vec![fx.root().join("C")], "fake:second".into())));
    let started = Instant::now();
    while !has_row(&fx, "c.txt") {
        assert!(started.elapsed() < Duration::from_secs(5), "never replayed");
        h.send(Cmd::Watch(watch::WatchEvent::Overflow(fx.root_id)));
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(h);
}

/// VFS-26 AC-2 (R3): a root held after a failed watch, with a walk owed for it, overflows
/// once its watch is armed again: the replay stores its cursor and releases the hold, and the
/// earlier walk is still owed (it runs once its cooldown has passed).
#[test]
fn replay_on_a_held_root_keeps_the_owed_walk() {
    let fx = fixture(&["A"]);
    let (h, history) = overflow_indexer(&fx);
    let checked = fx.last_scan();
    h.send(Cmd::Watch(watch::WatchEvent::Failed(fx.root_id)));
    std::thread::sleep(Duration::from_millis(150));
    let held = stored_cursor(&fx.db_path, fx.root_id);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(stored_cursor(&fx.db_path, fx.root_id), held, "not held");
    *history.replay.lock().unwrap() = Some(Ok((Vec::new(), "fake:replayed".into())));
    h.send(Cmd::Watch(watch::WatchEvent::Overflow(fx.root_id)));
    wait_for("the replay", || {
        history.replays.load(Ordering::SeqCst) == 1
            && stored_cursor(&fx.db_path, fx.root_id).is_some()
    });
    wait_for("the hold released", || {
        stored_cursor(&fx.db_path, fx.root_id)
            .is_some_and(|c| c != "fake:replayed" && Some(&c) != held.as_ref())
    });
    assert_eq!(
        fx.last_scan(),
        checked,
        "the owed walk waits for its cooldown"
    );
    // Two hours ago: the cooldown is over, not yet the weekly re-walk.
    fx.set_last_scan(now_secs() - 7200);
    wait_for("the owed walk", || {
        fx.last_scan() != Some(now_secs() - 7200)
    });
    drop(h);
}

/// Proves a deferred walk is owed for `fx`'s root: once its cooldown is made to have passed
/// (last full scan two hours ago, short of the weekly re-walk), a walk runs.
fn assert_walk_owed(fx: &Fx) {
    let ago = now_secs() - 7200;
    fx.set_last_scan(ago);
    wait_for("the owed walk", || fx.last_scan() != Some(ago));
}

/// VFS-26 AC-3 (R4): an overflow whose replay reports the history lost, of a root with no
/// cursor, or of a root that is unreachable, falls back to the deferred lost-events walk
/// with the cursor held.
#[test]
fn overflow_without_usable_history_holds_and_owes_a_walk() {
    // History lost: nothing scripted, so the fake reports it lost.
    let fx = fixture(&["A"]);
    let (h, history) = overflow_indexer(&fx);
    let checked = fx.last_scan();
    h.send(Cmd::Watch(watch::WatchEvent::Overflow(fx.root_id)));
    wait_for("the replay", || history.replays.load(Ordering::SeqCst) == 1);
    let held = stored_cursor(&fx.db_path, fx.root_id);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        stored_cursor(&fx.db_path, fx.root_id),
        held,
        "the cursor moved"
    );
    assert_eq!(fx.last_scan(), checked, "the walk is deferred");
    assert_walk_owed(&fx);
    drop(h);

    // No cursor at all: no replay is attempted, none is stored, a walk is owed.
    let fx = fixture(&["A"]);
    let (h, history) = overflow_indexer(&fx);
    cursor::store(&db::open_rw(&fx.db_path).unwrap(), fx.root_id, None).unwrap();
    let before = history.replays.load(Ordering::SeqCst);
    h.send(Cmd::Watch(watch::WatchEvent::Overflow(fx.root_id)));
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(history.replays.load(Ordering::SeqCst), before);
    assert_eq!(
        stored_cursor(&fx.db_path, fx.root_id),
        None,
        "a cursor was stored"
    );
    assert_walk_owed(&fx);
    drop(h);

    // Unreachable when the replay is due: no replay, the cursor holds, a walk is owed.
    let fx = fixture(&["A"]);
    let (h, history) = overflow_indexer(&fx);
    let away = fx.root().with_file_name("lejos");
    fs::rename(fx.root(), &away).unwrap();
    let before = history.replays.load(Ordering::SeqCst);
    let held = stored_cursor(&fx.db_path, fx.root_id);
    h.send(Cmd::Watch(watch::WatchEvent::Overflow(fx.root_id)));
    std::thread::sleep(Duration::from_millis(600));
    fs::rename(&away, fx.tree.path()).unwrap();
    assert_eq!(history.replays.load(Ordering::SeqCst), before);
    assert_eq!(stored_cursor(&fx.db_path, fx.root_id), held);
    assert_walk_owed(&fx);
    drop(h);
}

/// VFS-26 AC-4 (R5): a failed watch never replays; an overflow followed by a failed watch
/// before the replay is due never replays either; the cursor holds and a walk is owed.
#[test]
fn failed_watch_never_replays() {
    let fx = fixture(&["A"]);
    let (h, history) = overflow_indexer(&fx);
    *history.replay.lock().unwrap() = Some(Ok((Vec::new(), "fake:replayed".into())));
    h.send(Cmd::Watch(watch::WatchEvent::Failed(fx.root_id)));
    h.send(Cmd::Watch(watch::WatchEvent::Overflow(fx.root_id)));
    h.send(Cmd::Watch(watch::WatchEvent::Failed(fx.root_id)));
    std::thread::sleep(Duration::from_millis(150));
    let held = stored_cursor(&fx.db_path, fx.root_id);
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(history.replays.load(Ordering::SeqCst), 0);
    assert_eq!(stored_cursor(&fx.db_path, fx.root_id), held);
    assert_walk_owed(&fx);
    drop(h);
}

/// VFS-26 AC-6 (R3, real history on Windows and macOS): an overflow injected after a folder
/// was created replays the real history: the new folder is indexed and no walk runs (a row
/// dropped from an untouched folder stays missing). Without usable history the overflow
/// falls back to the lost-events walk.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn overflow_replays_the_real_history() {
    let fx = fixture(&["A", "Quieta"]);
    fs::write(fx.root().join("Quieta").join("testigo.txt"), b"x").unwrap();
    std::thread::sleep(Duration::from_millis(1100)); // FSEvents assigns ids late
    let h = IndexerHandle::spawn_with_history(
        fx.db_path.clone(),
        Arc::new(NullSink),
        Sched::inert(),
        None,
        Box::new(OsHistory),
        Timing {
            cursor_every: Duration::from_secs(3600),
            replay_quiet: Duration::from_millis(200),
            replay_max: Duration::from_secs(2),
        },
    )
    .unwrap();
    wait_for("the start-up walk", || fx.status() == RootStatus::Ready);
    let stored = stored_cursor(&fx.db_path, fx.root_id);
    let checked = fx.last_scan();
    drop_row(&fx, "testigo.txt");
    std::thread::sleep(Duration::from_millis(1100)); // FSEvents assigns ids late
    fs::create_dir(fx.root().join("A").join("Nueva")).unwrap();
    fs::write(fx.root().join("A").join("Nueva").join("dentro.txt"), b"x").unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    h.send(Cmd::Watch(watch::WatchEvent::Overflow(fx.root_id)));
    if stored.is_none() {
        eprintln!("no usable change history on this volume: checking the walk fallback");
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(stored_cursor(&fx.db_path, fx.root_id), None);
        drop(h);
        return;
    }
    wait_for("the replay stored its cursor", || {
        stored_cursor(&fx.db_path, fx.root_id) != stored
    });
    wait_for("the new folder indexed", || has_row(&fx, "dentro.txt"));
    assert!(
        !has_row(&fx, "testigo.txt"),
        "an untouched folder was listed"
    );
    assert_eq!(fx.last_scan(), checked, "a full walk ran");
    drop(h);
}
