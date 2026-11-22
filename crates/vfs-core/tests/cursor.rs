//! Start-up replay of the change history over real temporary trees (VFS-9 AC-1..AC-3). The
//! replay runs where the system keeps a history VFS can read: the NTFS USN journal on
//! Windows, the FSEvents event store on macOS.
#![cfg(any(windows, target_os = "macos"))]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use vfs_core::db::{self, ingest};
use vfs_core::{IndexProgress, IndexerOptions, ProgressSink, RootStatus, Vfs};

struct NullSink;
impl ProgressSink for NullSink {
    fn on_progress(&self, _: &IndexProgress) {}
}

struct Fixture {
    _db_dir: tempfile::TempDir,
    tree: tempfile::TempDir,
    db_path: PathBuf,
    vfs: Vfs,
    root_id: i64,
}

impl Fixture {
    /// A tree with `files` registered as a root and scanned once.
    fn scanned(files: &[&str]) -> Fixture {
        let db_dir = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("index.db");
        let vfs = Vfs::open(&db_path).unwrap();
        let mut conn = db::open_rw(&db_path).unwrap();
        let tx = conn.transaction().unwrap();
        let root = dunce::canonicalize(tree.path()).unwrap();
        let root_id = ingest::insert_root(&tx, "test", root.to_str().unwrap()).unwrap();
        tx.commit().unwrap();
        let fx = Fixture {
            _db_dir: db_dir,
            tree,
            db_path,
            vfs,
            root_id,
        };
        for f in files {
            fx.file(f);
        }
        // Their creation must be history older than the cursor the scan takes.
        settle();
        fx.vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
        fx
    }

    fn root(&self) -> PathBuf {
        dunce::canonicalize(self.tree.path()).unwrap()
    }

    fn file(&self, rel: &str) {
        let p = self.root().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"x").unwrap();
    }

    fn conn(&self) -> Connection {
        db::open_rw(&self.db_path).unwrap()
    }

    fn cursor(&self) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                [format!("cursor/{}", self.root_id)],
                |r| r.get(0),
            )
            .ok()
    }

    fn set_cursor(&self, value: &str) {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
                (format!("cursor/{}", self.root_id), value),
            )
            .unwrap();
    }

    /// Deletes the row of `name` behind the indexer's back: it comes back only if its
    /// folder is listed again.
    fn drop_row(&self, name: &str) {
        let n = self
            .conn()
            .execute("DELETE FROM files WHERE name = ?1", [name])
            .unwrap();
        assert_eq!(n, 1, "{name}");
    }

    /// Every indexed file as `rel_path/name`, sorted.
    fn all_files(&self) -> Vec<String> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT CASE d.rel_path WHEN '' THEN f.name ELSE d.rel_path || '/' || f.name END \
                 FROM files f JOIN dirs d ON d.id = f.dir_id ORDER BY 1",
            )
            .unwrap();
        let rows = stmt.query_map([], |r| r.get(0)).unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    }

    /// Status, `last_full_scan_at` and pending directories of the only root.
    fn state(&self) -> (RootStatus, Option<i64>, i64) {
        self.conn()
            .query_row(
                "SELECT status, last_full_scan_at, \
                 (SELECT COUNT(*) FROM dirs WHERE scan_state <> 0) FROM roots",
                [],
                |r| Ok((RootStatus::from_code(r.get(0)?), r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    fn last_scan(&self) -> Option<i64> {
        self.state().1
    }
}

/// Starts the indexer without the boot grace: a CI runner has often been up for less than
/// the default, which would defer the catch-up walks (VFS-5 R5).
fn start(vfs: &Vfs) {
    let no_grace = IndexerOptions {
        boot_grace: Duration::ZERO,
    };
    vfs.start_indexer_with(Arc::new(NullSink), no_grace)
        .unwrap();
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn names(list: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = list.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

/// Lets the system record the changes just made (FSEvents assigns ids asynchronously).
fn settle() {
    std::thread::sleep(Duration::from_millis(1100));
}

/// VFS-9 AC-1 (R1, R2): changes made while closed are replayed at start, without a full
/// walk: the root stays ready with the same `last_full_scan_at`, and a row deleted from an
/// untouched folder stays missing, because that folder is not listed again.
#[test]
fn start_replays_changes_without_a_full_walk() {
    let fx = Fixture::scanned(&["A/a1.txt", "B/b1.txt", "B/b2.txt", "C/testigo.txt"]);
    let first = fx.cursor().expect("the first scan stores a cursor");
    let checked = fx.last_scan().unwrap();
    fx.drop_row("testigo.txt");
    settle();
    fx.file("A/nuevo.txt");
    fs::rename(fx.root().join("A/a1.txt"), fx.root().join("A/a2.txt")).unwrap();
    fs::remove_file(fx.root().join("B/b1.txt")).unwrap();
    fx.file("D/sub/hondo.txt");
    settle();

    start(&fx.vfs);
    let want = names(&["A/a2.txt", "A/nuevo.txt", "B/b2.txt", "D/sub/hondo.txt"]);
    wait_until("replay", || {
        fx.all_files() == want && fx.state().0 == RootStatus::Ready && fx.state().2 == 0
    });
    // A full walk, had one started, would have restored the row by now or soon after.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(fx.all_files(), want);
    assert_eq!(fx.state(), (RootStatus::Ready, Some(checked), 0));
    assert_ne!(fx.cursor(), Some(first), "the replay stores the new cursor");
    fx.vfs.shutdown();
}

/// VFS-9 AC-1 (R2, R7), VFS-1 AC-8: changes made while closed in a folder whose path is
/// longer than 260 characters are replayed too. Windows names such a folder with the `\\?\`
/// prefix the stored root lacks; the stress run lost them for good (nightly run 45008368202).
#[test]
fn start_replays_changes_below_a_long_path() {
    let segment = "carpeta con un nombre bastante largo para la prueba";
    let deep: Vec<String> = (0..6).map(|i| format!("{segment} {i}")).collect();
    let deep = deep.join("/");
    let fx = Fixture::scanned(&[&format!("{deep}/viejo.txt"), "corto/testigo.txt"]);
    assert!(fx.root().join(&deep).as_os_str().len() > 260);
    settle();
    fx.file(&format!("{deep}/nuevo notaría.txt"));
    fs::remove_file(fx.root().join(format!("{deep}/viejo.txt"))).unwrap();
    settle();

    start(&fx.vfs);
    let new = format!("{deep}/nuevo notaría.txt");
    let want = names(&[new.as_str(), "corto/testigo.txt"]);
    wait_until("replay below a long path", || {
        fx.all_files() == want && fx.state().0 == RootStatus::Ready && fx.state().2 == 0
    });
    fx.vfs.shutdown();
}

/// VFS-9 AC-2 (R1, R2): a walk cut short resumes after a restart, and a file added while
/// closed to a folder it had already listed is replayed into the index; folders already
/// listed and untouched are not listed again.
#[test]
fn interrupted_walk_resumes_and_replays_listed_folders() {
    let fx = Fixture::scanned(&["A/a1.txt", "B/testigo.txt", "C/c1.txt"]);
    // As if the walk that stored this cursor had listed the root, A and B, but not C.
    fx.conn()
        .execute_batch(
            "UPDATE roots SET status = 1; UPDATE dirs SET scan_state = 1 WHERE rel_path = 'C';",
        )
        .unwrap();
    fx.drop_row("testigo.txt");
    fs::remove_file(fx.root().join("C/c1.txt")).unwrap();
    fx.file("C/c2.txt");
    settle();
    fx.file("A/mientras cerrado.txt");
    settle();

    start(&fx.vfs);
    let want = names(&["A/a1.txt", "A/mientras cerrado.txt", "C/c2.txt"]);
    wait_until("resumed walk", || {
        fx.all_files() == want && fx.state().0 == RootStatus::Ready && fx.state().2 == 0
    });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(fx.all_files(), want, "B was listed again");
    fx.vfs.shutdown();
}

/// VFS-9 AC-3 (R3, R4, R5): a stored cursor that does not parse, belongs to the other
/// system, or names another volume, journal or event store, is not replayed: the root gets
/// the full catch-up walk, which lists the untouched folder again.
#[test]
fn unusable_cursor_falls_back_to_a_full_walk() {
    let fx = Fixture::scanned(&["A/a1.txt", "C/testigo.txt"]);
    let good = fx.cursor().expect("the first scan stores a cursor");
    let mut fields: Vec<&str> = good.split(':').collect();
    // Field 1: the volume serial (Windows) or the event-store UUID (macOS).
    fields[1] = if cfg!(windows) {
        "00000001"
    } else {
        "00000000-0000-0000-0000-000000000000"
    };
    let other_store = fields.join(":");
    let mut bad = vec![
        "basura".to_string(),
        if cfg!(windows) {
            "fsevents:00000000-0000-0000-0000-000000000000:5".to_string()
        } else {
            "usn:00000001:0000000000000001:5".to_string()
        },
        other_store,
    ];
    if cfg!(windows) {
        let mut f: Vec<&str> = good.split(':').collect();
        f[2] = "0000000000000001";
        bad.push(f.join(":"));
    }
    for cursor in bad {
        std::thread::sleep(Duration::from_millis(1100));
        let before = fx.last_scan().unwrap();
        fx.set_cursor(&cursor);
        fx.drop_row("testigo.txt");
        start(&fx.vfs);
        wait_until(&format!("full walk after {cursor}"), || {
            fx.all_files() == names(&["A/a1.txt", "C/testigo.txt"])
                && fx.state().0 == RootStatus::Ready
                && fx.last_scan().is_some_and(|t| t > before)
        });
        fx.vfs.shutdown();
        assert_ne!(fx.cursor(), Some(cursor), "the walk stores a fresh cursor");
    }
}
