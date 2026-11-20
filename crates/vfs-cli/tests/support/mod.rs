//! Shared harness for the process-level robustness tests (VFS-3): a seeded tree generator,
//! the `vfs` binary as a child process, the on-disk truth, and the index invariants every
//! crash, churn or soak run must leave behind.

#![allow(dead_code)] // each test binary uses a different subset

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};

/// Path of the `vfs` binary built for this test run.
pub const VFS_BIN: &str = env!("CARGO_BIN_EXE_vfs");

/// True when the slow, full-size variants are requested (`VFS_STRESS=1`, set by the
/// scheduled CI job). The default run exercises the same code paths at a smaller size.
pub fn stress() -> bool {
    std::env::var("VFS_STRESS").is_ok_and(|v| v == "1")
}

/// `small` normally, `large` under `VFS_STRESS=1`.
pub fn scaled(small: usize, large: usize) -> usize {
    if stress() {
        large
    } else {
        small
    }
}

/// Deterministic splitmix64, so a failing seed can be replayed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// Words for generated names: accents in NFC and NFD, Spanish archive vocabulary.
const WORDS: &[&str] = &[
    "factura",
    "Notaría",
    "Notari\u{0301}a",
    "año",
    "a\u{0303}no",
    "Banco",
    "escritura",
    "foto",
    "Cádiz",
    "viaje",
    "hipoteca",
    "IMG",
    "presupuesto",
    "Gestoría",
    "recibo",
    "copia",
];
const EXTS: &[&str] = &["pdf", "jpg", "docx", "txt", "mp4", "xlsx"];

/// A unique, valid name made of words, a counter and an optional extension.
pub fn name(rng: &mut Rng, n: u64, ext: bool) -> String {
    let a = rng.pick(WORDS);
    let b = rng.pick(WORDS);
    if ext {
        format!("{a} {b} {n}.{}", rng.pick(EXTS))
    } else {
        format!("{a} {b} {n}")
    }
}

/// Creates about `files` files in about `files / 20` directories nested up to 6 levels
/// below `root`, with small varied sizes. Returns the directories created (relative).
pub fn gen_tree(root: &Path, rng: &mut Rng, files: usize) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![PathBuf::new()];
    let mut counter = 0u64;
    let ndirs = (files / 20).max(1);
    for _ in 0..ndirs {
        let parent = rng.pick(&dirs).clone();
        if parent.components().count() >= 6 {
            continue;
        }
        counter += 1;
        let rel = parent.join(name(rng, counter, false));
        fs::create_dir_all(root.join(&rel)).unwrap();
        dirs.push(rel);
    }
    for _ in 0..files {
        let dir = rng.pick(&dirs).clone();
        counter += 1;
        let path = root.join(dir).join(name(rng, counter, true));
        fs::write(&path, vec![b'x'; rng.below(64) as usize]).unwrap();
    }
    dirs
}

/// Folders and files under a root, by '/'-separated relative path; files map to their size.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub dirs: BTreeSet<String>,
    pub files: BTreeMap<String, u64>,
}

/// What is on disk under `root`, as the indexer should see it: symlinks skipped, names that
/// are not valid Unicode skipped (the generators never make them).
pub fn disk_truth(root: &Path) -> Snapshot {
    fn walk(dir: &Path, rel: &str, out: &mut Snapshot) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let Some(name) = e.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(meta) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            let child = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if meta.is_dir() {
                out.dirs.insert(child.clone());
                walk(&e.path(), &child, out);
            } else if meta.is_file() {
                out.files.insert(child, meta.len());
            }
        }
    }
    let mut out = Snapshot::default();
    walk(root, "", &mut out);
    out
}

/// What the index holds for the root at `root_path` (exact stored path).
pub fn index_snapshot(conn: &Connection, root_path: &str) -> Snapshot {
    let root_id: i64 = conn
        .query_row("SELECT id FROM roots WHERE path = ?1", [root_path], |r| {
            r.get(0)
        })
        .unwrap_or_else(|e| panic!("root {root_path} missing from the index: {e}"));
    let mut out = Snapshot::default();
    let mut stmt = conn
        .prepare("SELECT rel_path FROM dirs WHERE root_id = ?1 AND rel_path <> ''")
        .unwrap();
    for rel in stmt
        .query_map([root_id], |r| r.get::<_, String>(0))
        .unwrap()
    {
        out.dirs.insert(rel.unwrap());
    }
    let mut stmt = conn
        .prepare(
            "SELECT d.rel_path, f.name, f.size FROM files f JOIN dirs d ON d.id = f.dir_id \
             WHERE d.root_id = ?1",
        )
        .unwrap();
    let rows = stmt
        .query_map([root_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .unwrap();
    for row in rows {
        let (rel, name, size) = row.unwrap();
        let path = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        out.files.insert(path, size as u64);
    }
    out
}

/// Asserts the index equals the disk, printing the first differences.
pub fn assert_matches_disk(conn: &Connection, root_path: &str, root: &Path, context: &str) {
    let disk = disk_truth(root);
    let index = index_snapshot(conn, root_path);
    if disk != index {
        let missing: Vec<_> = disk
            .files
            .keys()
            .filter(|k| !index.files.contains_key(*k))
            .take(5)
            .collect();
        let extra: Vec<_> = index
            .files
            .keys()
            .filter(|k| !disk.files.contains_key(*k))
            .take(5)
            .collect();
        let dmissing: Vec<_> = disk.dirs.difference(&index.dirs).take(5).collect();
        let dextra: Vec<_> = index.dirs.difference(&disk.dirs).take(5).collect();
        let sizes: Vec<_> = disk
            .files
            .iter()
            .filter(|(k, v)| index.files.get(*k).is_some_and(|s| s != *v))
            .take(5)
            .collect();
        panic!(
            "{context}: index != disk ({} vs {} files, {} vs {} dirs)\n missing files {missing:?}\n \
             extra files {extra:?}\n missing dirs {dmissing:?}\n extra dirs {dextra:?}\n \
             wrong sizes {sizes:?}",
            index.files.len(),
            disk.files.len(),
            index.dirs.len(),
            disk.dirs.len()
        );
    }
}

/// A plain read-write connection that applies no migration, for checking invariants.
pub fn raw_conn(db: &Path) -> Connection {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    conn.busy_timeout(Duration::from_secs(30)).unwrap();
    conn
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// The FTS5 tables and the base table whose ids are their rowids.
const FTS_TABLES: &[(&str, &str)] = &[
    ("files_fts", "files"),
    ("file_paths_fts", "files"),
    ("dirs_fts", "dirs"),
];

/// Structural invariants of an index that has just been opened by `Vfs::open` (so any
/// recovery already ran): SQLite and FTS5 integrity checks pass, every FTS table holds
/// exactly one entry per base row, every file and folder is found by its exact folded name,
/// and no row is duplicated. `sample` bounds the per-row name lookups (0 = all).
pub fn check_structure(db: &Path, sample: usize, context: &str) {
    let conn = raw_conn(db);
    let verdict: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(verdict, "ok", "{context}: integrity_check");
    for (fts, base) in FTS_TABLES {
        conn.execute(
            &format!("INSERT INTO {fts}({fts}) VALUES('integrity-check')"),
            [],
        )
        .unwrap_or_else(|e| panic!("{context}: {fts} integrity-check: {e}"));
        let orphans = count(
            &conn,
            &format!("SELECT COUNT(*) FROM (SELECT rowid FROM {fts} EXCEPT SELECT id FROM {base})"),
        );
        assert_eq!(
            orphans, 0,
            "{context}: {fts} entries point at missing {base} rows"
        );
        let missing = count(
            &conn,
            &format!("SELECT COUNT(*) FROM (SELECT id FROM {base} EXCEPT SELECT rowid FROM {fts})"),
        );
        assert_eq!(missing, 0, "{context}: {base} rows without a {fts} entry");
        let entries = count(&conn, &format!("SELECT COUNT(*) FROM {fts}"));
        let rows = count(&conn, &format!("SELECT COUNT(*) FROM {base}"));
        assert_eq!(entries, rows, "{context}: {fts} has duplicate entries");
    }
    let dup_files = count(
        &conn,
        "SELECT COUNT(*) FROM (SELECT dir_id, name FROM files GROUP BY 1, 2 HAVING COUNT(*) > 1)",
    );
    let dup_dirs = count(
        &conn,
        "SELECT COUNT(*) FROM (SELECT root_id, rel_path FROM dirs GROUP BY 1, 2 HAVING COUNT(*) > 1)",
    );
    assert_eq!((dup_files, dup_dirs), (0, 0), "{context}: duplicate rows");
    let orphan_files = count(
        &conn,
        "SELECT COUNT(*) FROM files WHERE dir_id NOT IN (SELECT id FROM dirs)",
    );
    let orphan_dirs = count(
        &conn,
        "SELECT COUNT(*) FROM dirs WHERE root_id NOT IN (SELECT id FROM roots) \
         OR (parent_id IS NOT NULL AND parent_id NOT IN (SELECT id FROM dirs))",
    );
    assert_eq!(
        (orphan_files, orphan_dirs),
        (0, 0),
        "{context}: rows whose folder or root is gone"
    );
    let limit = if sample == 0 { -1 } else { sample as i64 };
    for (fts, sql) in [
        (
            "files_fts",
            "SELECT id, name_key FROM files ORDER BY random() LIMIT ?1",
        ),
        (
            "dirs_fts",
            "SELECT id, name_key FROM dirs ORDER BY random() LIMIT ?1",
        ),
    ] {
        let rows: Vec<(i64, String)> = conn
            .prepare(sql)
            .unwrap()
            .query_map([limit], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let column = if *fts == *"dirs_fts" {
            "name_key : "
        } else {
            ""
        };
        let mut stmt = conn
            .prepare(&format!(
                "SELECT 1 FROM {fts} WHERE {fts} MATCH ?1 AND rowid = ?2"
            ))
            .unwrap();
        for (id, key) in rows {
            if key.chars().count() < 3 {
                continue;
            }
            let expr = format!("{column}\"{}\"", key.replace('"', "\"\""));
            let found = stmt.exists(rusqlite::params![expr, id]).unwrap();
            assert!(
                found,
                "{context}: {fts} does not find row {id} by its name {key:?}"
            );
        }
    }
}

/// No folder waits to be listed.
pub fn assert_nothing_pending(db: &Path, context: &str) {
    let pending = count(
        &raw_conn(db),
        "SELECT COUNT(*) FROM dirs WHERE scan_state <> 0",
    );
    assert_eq!(pending, 0, "{context}: folders left pending");
}

/// Root paths as stored.
pub fn root_paths(db: &Path) -> Vec<String> {
    raw_conn(db)
        .prepare("SELECT path FROM roots ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The root path the index stores for `dir`.
pub fn stored_root(dir: &Path) -> String {
    dunce::canonicalize(dir)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

/// Runs `vfs index --db <db> <roots>` to completion and asserts success.
pub fn index(db: &Path, roots: &[&Path]) {
    let out = Command::new(VFS_BIN)
        .arg("index")
        .arg("--db")
        .arg(db)
        .args(roots)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "vfs index failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Runs `vfs run --until-idle` (the app's background indexer and watcher, with its
/// catch-up re-walk) and asserts it reaches "up to date" within `timeout`.
pub fn run_until_idle(db: &Path, roots: &[&Path], timeout: Duration) {
    let mut child = Command::new(VFS_BIN)
        .arg("run")
        .arg("--until-idle")
        .arg("--db")
        .arg(db)
        .args(roots)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let mut err = String::new();
            use std::io::Read;
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut err)
                .unwrap();
            assert!(status.success(), "vfs run failed: {err}");
            return;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            panic!("vfs run did not reach up to date within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Spawns `vfs <args>` with stdout and stderr discarded, to be killed.
pub fn spawn(args: &[&std::ffi::OsStr]) -> Child {
    Command::new(VFS_BIN)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// Hard kill (SIGKILL on Unix, TerminateProcess on Windows): no destructor, no clean
/// shutdown, the session marker stays behind.
pub fn kill(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// A long-running `vfs run` (no `--until-idle`) whose progress lines are read on a thread.
pub struct App {
    pub child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    pub last: Option<String>,
}

impl App {
    pub fn start(db: &Path, roots: &[&Path]) -> App {
        let mut child = Command::new(VFS_BIN)
            .arg("run")
            .arg("--db")
            .arg(db)
            .args(roots)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout: ChildStdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        App {
            child,
            stdin,
            lines: rx,
            last: None,
        }
    }

    /// Waits for a progress line saying every root is up to date; false on timeout.
    pub fn wait_up_to_date(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    let up = is_up_to_date_line(&line);
                    self.last = Some(line);
                    if up {
                        return true;
                    }
                }
                Err(_) => return false,
            }
        }
    }

    /// Drains pending lines without waiting; true if one of them says up to date.
    pub fn drain(&mut self) -> bool {
        let mut up = false;
        while let Ok(line) = self.lines.try_recv() {
            up = is_up_to_date_line(&line);
            self.last = Some(line);
        }
        up
    }

    /// Closes stdin (clean shutdown) and waits for the process.
    pub fn stop(mut self) {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        assert!(status.success(), "vfs run exited with {status}");
    }
}

fn is_up_to_date_line(line: &str) -> bool {
    let Ok(p) = serde_json::from_str::<vfs_core::IndexProgress>(line) else {
        return false;
    };
    p.state == vfs_core::IndexerState::Idle
        && !p.roots.is_empty()
        && p.roots
            .iter()
            .all(|r| r.status == vfs_core::RootStatus::Ready && r.dirs_pending == 0)
}

/// Resident memory of a process in bytes, when the platform can tell.
pub fn rss_bytes(pid: u32) -> Option<u64> {
    #[cfg(unix)]
    {
        let out = Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let kib: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        Some(kib * 1024)
    }
    #[cfg(windows)]
    {
        // "Image","PID","Session","#","Mem Usage" with Mem Usage like "12,345 K".
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mem = text.trim().rsplit("\",\"").next()?;
        let digits: String = mem.chars().filter(char::is_ascii_digit).collect();
        Some(digits.parse::<u64>().ok()? * 1024)
    }
}
