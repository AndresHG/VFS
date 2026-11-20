//! Crash and power-loss safety (VFS-3 AC-1..AC-5): the real indexer runs as a `vfs` child
//! process and is hard-killed at random points; the index left behind must open, pass every
//! structural invariant, and converge to the on-disk truth on the next run. Torn writes are
//! simulated by truncating or overwriting the database, its WAL and the session marker.
//!
//! The default run does a few iterations per scenario; `VFS_STRESS=1` does hundreds over a
//! larger tree (the scheduled CI job). Every iteration prints its seed on failure.

mod support;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use support::*;

const CONVERGE: Duration = Duration::from_secs(300);

struct Setup {
    _tmp: tempfile::TempDir,
    tree: PathBuf,
    db: PathBuf,
}

/// A generated tree of `files` files and the path of a (not yet created) index.
fn setup(seed: u64, files: usize) -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("archivo");
    fs::create_dir(&tree).unwrap();
    gen_tree(&tree, &mut Rng::new(seed), files);
    let db_dir = tmp.path().join("datos");
    fs::create_dir(&db_dir).unwrap();
    Setup {
        db: db_dir.join("index.db"),
        tree,
        _tmp: tmp,
    }
}

fn remove_index(db: &Path) {
    let dir = db.parent().unwrap();
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            fs::remove_dir_all(p).unwrap();
        } else {
            fs::remove_file(p).unwrap();
        }
    }
}

/// Kill delay uniform in `[0, max]`.
fn delay(rng: &mut Rng, max: Duration) -> Duration {
    Duration::from_micros(rng.below(max.as_micros() as u64 + 1))
}

/// The `vfs` subcommands the scenarios kill: the foreground scan and the app's indexer.
#[derive(Debug, Clone, Copy)]
enum Cmd {
    Index,
    Run,
}

impl Cmd {
    fn arg(self) -> &'static str {
        match self {
            Cmd::Index => "index",
            Cmd::Run => "run",
        }
    }
}

fn args<'a>(cmd: Cmd, db: &'a Path, tree: &'a Path) -> Vec<&'a OsStr> {
    vec![
        OsStr::new(cmd.arg()),
        OsStr::new("--db"),
        db.as_os_str(),
        tree.as_os_str(),
    ]
}

/// The full recovery contract after any interruption: the index opens (recovering if it
/// must), keeps its root, passes the structural checks, and one more run brings it to
/// exactly the disk with nothing pending.
fn check_and_converge(s: &Setup, context: &str) {
    let vfs = vfs_core::Vfs::open(&s.db).unwrap_or_else(|e| panic!("{context}: open: {e}"));
    drop(vfs);
    let root = stored_root(&s.tree);
    assert_eq!(
        root_paths(&s.db),
        vec![root.clone()],
        "{context}: the root list did not survive"
    );
    check_structure(&s.db, 200, context);
    run_until_idle(&s.db, &[&s.tree], CONVERGE);
    check_structure(&s.db, 0, &format!("{context}, after convergence"));
    assert_nothing_pending(&s.db, context);
    assert_matches_disk(&raw_conn(&s.db), &root, &s.tree, context);
}

/// Time of one uninterrupted run of `cmd` from an empty index.
fn measure(s: &Setup, cmd: Cmd) -> Duration {
    remove_index(&s.db);
    let t = Instant::now();
    match cmd {
        Cmd::Index => index(&s.db, &[&s.tree]),
        Cmd::Run => run_until_idle(&s.db, &[&s.tree], CONVERGE),
    }
    t.elapsed()
}

/// VFS-3 AC-1, VFS-1 AC-6: killing the indexer at any point of a first scan (between
/// folders, mid-batch, mid-commit, during the final merge) and again during the resume
/// leaves an index that opens, is consistent, and converges to the disk.
#[test]
fn kill_during_first_scan_then_resume() {
    let iterations = scaled(6, 200);
    let s = setup(1, scaled(3_000, 60_000));
    let full = measure(&s, Cmd::Index);
    let mut rng = Rng::new(0xC0FFEE);
    for i in 0..iterations {
        remove_index(&s.db);
        let cmd = if i % 2 == 0 { Cmd::Index } else { Cmd::Run };
        let child = spawn(&args(cmd, &s.db, &s.tree));
        std::thread::sleep(delay(&mut rng, full));
        kill(child);
        if rng.chance(30) && s.db.exists() {
            // Interrupt the resume as well.
            let child = spawn(&args(Cmd::Index, &s.db, &s.tree));
            std::thread::sleep(delay(&mut rng, full / 2));
            kill(child);
        }
        if !s.db.exists() {
            continue; // killed before the index was created
        }
        check_and_converge(&s, &format!("first scan, iteration {i} ({cmd:?})"));
    }
}

/// Renames, moves, deletions and creations that a catch-up re-walk or the watcher's
/// in-place rename must apply.
fn churn(tree: &Path, rng: &mut Rng, round: u64) {
    let truth = disk_truth(tree);
    let dirs: Vec<&String> = truth.dirs.iter().collect();
    let files: Vec<&String> = truth.files.keys().collect();
    if !dirs.is_empty() {
        // Rename a folder with its whole subtree in place.
        let from = tree.join(rng.pick(&dirs).as_str());
        if from.exists() {
            let to = from.with_file_name(format!("renombrada {round}"));
            let _ = fs::rename(&from, &to);
        }
    }
    if dirs.len() > 2 {
        // Move a folder under another one (skipped when it would go inside itself).
        let a = tree.join(rng.pick(&dirs).as_str());
        let b = tree.join(rng.pick(&dirs).as_str());
        if a.exists() && b.exists() && !b.starts_with(&a) {
            let _ = fs::rename(&a, b.join(format!("movida {round}")));
        }
    }
    for _ in 0..20 {
        let f = tree.join(rng.pick(&files).as_str());
        let _ = fs::remove_file(f);
    }
    let base = tree.join(format!("nueva {round}"));
    fs::create_dir_all(&base).unwrap();
    for n in 0..30 {
        fs::write(base.join(name(rng, n, true)), b"nuevo").unwrap();
    }
}

/// VFS-3 AC-2, VFS-1 AC-6/AC-7: with a complete index, folders are renamed, moved, deleted
/// and created, and the app is killed while it applies them: during the catch-up re-walk at
/// start, or while the watcher applies a folder rename in place. The index converges.
#[test]
fn kill_during_rewalk_and_rename() {
    let iterations = scaled(6, 150);
    let s = setup(2, scaled(2_000, 30_000));
    let full = measure(&s, Cmd::Run);
    let mut rng = Rng::new(0xBADC0DE);
    for i in 0..iterations {
        churn(&s.tree, &mut rng, i as u64);
        if i % 2 == 0 {
            // Killed during the catch-up re-walk that applies changes made while closed.
            let child = spawn(&args(Cmd::Run, &s.db, &s.tree));
            std::thread::sleep(delay(&mut rng, full));
            kill(child);
        } else {
            // Killed while the running app applies a rename seen by the watcher.
            let mut app = App::start(&s.db, &[&s.tree]);
            assert!(app.wait_up_to_date(CONVERGE), "app never up to date");
            let truth = disk_truth(&s.tree);
            let dirs: Vec<&String> = truth.dirs.iter().collect();
            let from = s.tree.join(rng.pick(&dirs).as_str());
            let _ = fs::rename(&from, from.with_file_name(format!("en vivo {i}")));
            std::thread::sleep(delay(&mut rng, Duration::from_millis(3_000)));
            kill(app.child);
        }
        check_and_converge(&s, &format!("rewalk, iteration {i}"));
    }
}

/// Damages the last page of a checkpointed index and leaves a stale session marker (an
/// earlier session that did not end cleanly), so the next open fails the integrity check
/// and must recover; the roots table, near the start of the file, stays readable.
fn wreck(db: &Path) {
    let mut bytes = fs::read(db).unwrap();
    let len = bytes.len();
    assert!(len > 16 * 4096, "index too small to damage only its tail");
    bytes[len - 4096..].fill(0xA5);
    fs::write(db, bytes).unwrap();
    let mut session = db.as_os_str().to_owned();
    session.push(".session");
    fs::create_dir_all(&session).unwrap();
    fs::write(Path::new(&session).join("999999-0.lock"), b"").unwrap();
}

/// VFS-3 AC-3, VFS-1 AC-6: killed at any moment of index recovery (moving the damaged
/// index aside, creating the new one, re-adding the roots), the next start still knows the
/// user's folders and rebuilds the index.
#[test]
fn kill_during_recovery_keeps_roots() {
    let iterations = scaled(10, 300);
    let s = setup(3, 3_000);
    let mut rng = Rng::new(0xFEED);
    index(&s.db, &[&s.tree]);
    // Time of a recovery alone: open a wrecked index (no root argument).
    wreck(&s.db);
    let t = Instant::now();
    let out = std::process::Command::new(VFS_BIN)
        .args(["search", "--db"])
        .arg(&s.db)
        .arg("x")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let recovery = t.elapsed();
    for i in 0..iterations {
        wreck(&s.db);
        // `search` opens the index (recovering it) and adds no root, so a lost root list
        // is not silently restored by the command line.
        let child = spawn(&[
            OsStr::new("search"),
            OsStr::new("--db"),
            s.db.as_os_str(),
            OsStr::new("x"),
        ]);
        std::thread::sleep(delay(&mut rng, recovery));
        kill(child);
        let vfs = vfs_core::Vfs::open(&s.db).unwrap();
        let roots: Vec<String> = vfs
            .list_roots()
            .unwrap()
            .into_iter()
            .map(|r| r.path)
            .collect();
        drop(vfs);
        assert_eq!(
            roots,
            vec![stored_root(&s.tree)],
            "recovery iteration {i}: folders lost after a kill during recovery"
        );
        check_and_converge(&s, &format!("recovery, iteration {i}"));
    }
}

/// Turns a current index into a version-1 index: the per-file path index dropped.
fn downgrade_to_v1(db: &Path) {
    let conn = raw_conn(db);
    conn.execute_batch("DROP TABLE file_paths_fts; PRAGMA user_version = 1;")
        .unwrap();
}

/// VFS-3 AC-4: killed during the version 1 to 2 migration, the index is either still at
/// version 1 or fully at version 2 (never half-filled), and converges.
#[test]
fn kill_during_migration() {
    let iterations = scaled(6, 100);
    let s = setup(4, scaled(3_000, 60_000));
    index(&s.db, &[&s.tree]);
    let template = s._tmp.path().join("v1.db");
    fs::copy(&s.db, &template).unwrap();
    downgrade_to_v1(&template);
    let restore = || {
        remove_index(&s.db);
        fs::copy(&template, &s.db).unwrap();
    };
    restore();
    let t = Instant::now();
    index(&s.db, &[&s.tree]);
    let full = t.elapsed();
    let mut rng = Rng::new(0x5EED);
    for i in 0..iterations {
        restore();
        let child = spawn(&args(Cmd::Index, &s.db, &s.tree));
        std::thread::sleep(delay(&mut rng, full));
        kill(child);
        let conn = raw_conn(&s.db);
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        let has_table: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE name = 'file_paths_fts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            has_table,
            version >= 2,
            "migration iteration {i}: version {version} with path index present = {has_table}"
        );
        drop(conn);
        check_and_converge(&s, &format!("migration, iteration {i}"));
    }
    fs::remove_file(template).unwrap();
}

/// One random torn write or corruption of the database, its WAL or the session marker.
fn tear(db: &Path, rng: &mut Rng) -> String {
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    let mut session = db.as_os_str().to_owned();
    session.push(".session");
    let markers: Vec<PathBuf> = fs::read_dir(&session)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    let mut targets = vec![db.to_path_buf()];
    if Path::new(&wal).exists() {
        targets.push(PathBuf::from(&wal));
    }
    targets.extend(markers);
    let target = rng.pick(&targets).clone();
    let len = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    let offset = if len == 0 { 0 } else { rng.below(len) };
    let what = match rng.below(3) {
        0 => {
            let f = fs::OpenOptions::new().write(true).open(&target).unwrap();
            f.set_len(offset).unwrap();
            "truncated"
        }
        1 => {
            use std::io::{Seek, SeekFrom, Write};
            let mut f = fs::OpenOptions::new().write(true).open(&target).unwrap();
            f.seek(SeekFrom::Start(offset)).unwrap();
            let junk: Vec<u8> = (0..1 + rng.below(512)).map(|_| rng.next() as u8).collect();
            f.write_all(&junk).unwrap();
            "overwritten"
        }
        _ => {
            use std::io::{Seek, SeekFrom, Write};
            // A torn page: a 4 KiB-aligned block zeroed, as after a partial sector write.
            let mut f = fs::OpenOptions::new().write(true).open(&target).unwrap();
            f.seek(SeekFrom::Start(offset / 4096 * 4096)).unwrap();
            f.write_all(&[0u8; 4096]).unwrap();
            "zeroed page"
        }
    };
    format!("{what} {} at {offset} of {len}", target.display())
}

/// VFS-3 AC-5, VFS-1 AC-6: after a kill leaves a WAL and a stale session marker behind,
/// the database, the WAL or the marker is truncated or corrupted at a random offset. The
/// next open never fails, and the index is consistent and converges to the disk (rebuilt
/// from the salvaged roots when it had to be discarded).
#[test]
fn torn_writes_are_recovered() {
    let iterations = scaled(12, 300);
    let s = setup(5, scaled(1_500, 20_000));
    let full = measure(&s, Cmd::Index);
    let mut rng = Rng::new(0x7EA2);
    for i in 0..iterations {
        // A complete index, then an interrupted re-index, so a WAL and a marker remain.
        if !s.db.exists() || rng.chance(20) {
            remove_index(&s.db);
            index(&s.db, &[&s.tree]);
        }
        churn(&s.tree, &mut rng, 1_000 + i as u64);
        let child = spawn(&args(Cmd::Run, &s.db, &s.tree));
        std::thread::sleep(full / 2 + delay(&mut rng, full));
        kill(child);
        let what = tear(&s.db, &mut rng);
        let context = format!("torn write {i}: {what}");
        let vfs = vfs_core::Vfs::open(&s.db).unwrap_or_else(|e| panic!("{context}: open: {e}"));
        let roots = vfs.list_roots().unwrap();
        drop(vfs);
        if roots.is_empty() {
            // The root row itself was destroyed with the page that held it: nothing left
            // to salvage. The user adds the folder again; the index must accept it.
            index(&s.db, &[&s.tree]);
        }
        check_and_converge(&s, &context);
    }
}
