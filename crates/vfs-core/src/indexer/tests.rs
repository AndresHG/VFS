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
    #[cfg(unix)]
    fn run_once(&mut self, retries: &mut Retries) -> ScanSummary {
        let mut pass = Pass::default();
        run(
            &mut self.conn,
            &mut pass,
            retries,
            &|| false,
            false,
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
    assert!(queue_rewalk(&fx.conn, fx.root_id).unwrap());

    // List only the root directory, as if an overflow arrived right after it.
    let calls = Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 1
    };
    let mut pass = Pass::default();
    let outcome = run(
        &mut fx.conn,
        &mut pass,
        &mut Retries::new(),
        &stop,
        false,
        &mut |_: &Connection, _: &Pass, _: bool| {},
    )
    .unwrap();
    assert!(matches!(outcome, Outcome::Yielded));
    assert_eq!(fx.dir("").unwrap().1, CLEAN);

    assert!(!queue_rewalk(&fx.conn, fx.root_id).unwrap());
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
    let h = IndexerHandle::spawn(fx.db_path.clone(), Arc::new(NullSink)).unwrap();
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
        h.send(Cmd::Watch(watch::WatchEvent::Rescan(fx.root_id)));
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
    start_owed(&fx.conn, &mut owed).unwrap();
    assert_eq!(fx.status(), RootStatus::Scanning);
    assert_eq!(fx.status_of(other), RootStatus::Ready, "one at a time");
    start_owed(&fx.conn, &mut owed).unwrap();
    assert_eq!(fx.status_of(other), RootStatus::Ready, "waits for the walk");

    // A manual request while the first root is walked: one more pass, deferred.
    request_rewalk(&fx.conn, &mut owed, &[fx.root_id], false).unwrap();
    assert_eq!(owed.get(&fx.root_id), Some(&true));
    fx.scan();
    start_owed(&fx.conn, &mut owed).unwrap();
    assert_eq!(fx.status_of(other), RootStatus::Scanning);
    fx.scan();
    start_owed(&fx.conn, &mut owed).unwrap();
    assert_eq!(fx.status(), RootStatus::Ready, "cool-down not over");
    assert!(owed.contains_key(&fx.root_id));

    fx.set_last_scan(now_secs() + 86_400);
    start_owed(&fx.conn, &mut owed).unwrap();
    assert_eq!(
        fx.status(),
        RootStatus::Scanning,
        "a future time is no cool-down"
    );
    assert!(owed.is_empty());
    fx.scan();

    // An overflow right after a walk waits; a manual request then does not.
    request_rewalk(&fx.conn, &mut owed, &[fx.root_id], true).unwrap();
    start_owed(&fx.conn, &mut owed).unwrap();
    assert_eq!(
        fx.status(),
        RootStatus::Ready,
        "overflow waits for the cool-down"
    );
    request_rewalk(&fx.conn, &mut owed, &[fx.root_id], false).unwrap();
    start_owed(&fx.conn, &mut owed).unwrap();
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
    start_owed(&fx.conn, &mut owed).unwrap();
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

    assert!(queue_rewalk(&fx.conn, fx.root_id).unwrap());
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
