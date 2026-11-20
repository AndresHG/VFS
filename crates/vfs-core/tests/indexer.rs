//! Indexer walk, apply and resume over real temporary directory trees (VFS-1 AC-5..AC-8).
//! Rows are checked through a read connection with FTS queries, as search reads them.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use vfs_core::db::{self, ingest};
use vfs_core::{fold_key, IndexProgress, IndexerState, ProgressSink, Vfs};

struct Fixture {
    _db_dir: tempfile::TempDir,
    tree: tempfile::TempDir,
    db_path: PathBuf,
    vfs: Vfs,
    root_id: i64,
}

impl Fixture {
    /// An empty tree registered as a root, and its index.
    fn new() -> Fixture {
        let db_dir = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("index.db");
        let vfs = Vfs::open(&db_path).unwrap();
        let mut conn = db::open_rw(&db_path).unwrap();
        let tx = conn.transaction().unwrap();
        let root = dunce::canonicalize(tree.path()).unwrap();
        let root_id = ingest::insert_root(&tx, "test", root.to_str().unwrap()).unwrap();
        tx.commit().unwrap();
        Fixture {
            _db_dir: db_dir,
            tree,
            db_path,
            vfs,
            root_id,
        }
    }

    fn root(&self) -> PathBuf {
        dunce::canonicalize(self.tree.path()).unwrap()
    }

    fn file(&self, rel: &str, bytes: usize) -> PathBuf {
        let p = self.root().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, vec![b'x'; bytes]).unwrap();
        p
    }

    fn scan(&self) -> vfs_core::ScanSummary {
        self.vfs
            .scan_to_completion(&AtomicBool::new(false))
            .unwrap()
    }

    fn reader(&self) -> Connection {
        db::open_ro(&self.db_path).unwrap()
    }

    /// Names of files whose folded name contains `term` (>= 3 chars), via `files_fts`.
    fn find_files(&self, term: &str) -> Vec<String> {
        find_files(&self.reader(), term)
    }

    /// Every indexed file as `rel_path/name`, sorted.
    fn all_files(&self) -> Vec<String> {
        all_files(&self.reader())
    }

    /// Every indexed directory `rel_path`, sorted.
    fn all_dirs(&self) -> Vec<String> {
        let conn = self.reader();
        let mut stmt = conn
            .prepare("SELECT rel_path FROM dirs ORDER BY rel_path")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get(0)).unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.reader().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

fn find_files(conn: &Connection, term: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT f.name FROM files_fts JOIN files f ON f.id = files_fts.rowid \
             WHERE files_fts MATCH ?1 ORDER BY f.name",
        )
        .unwrap();
    let rows = stmt
        .query_map([format!("\"{}\"", fold_key(term))], |r| r.get(0))
        .unwrap();
    rows.collect::<rusqlite::Result<_>>().unwrap()
}

fn all_files(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT CASE d.rel_path WHEN '' THEN f.name ELSE d.rel_path || '/' || f.name END AS p \
             FROM files f JOIN dirs d ON d.id = f.dir_id ORDER BY p",
        )
        .unwrap();
    let rows = stmt.query_map([], |r| r.get(0)).unwrap();
    rows.collect::<rusqlite::Result<_>>().unwrap()
}

fn find_dirs_by_name(conn: &Connection, term: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT d.rel_path FROM dirs_fts JOIN dirs d ON d.id = dirs_fts.rowid \
             WHERE dirs_fts MATCH ?1 ORDER BY d.rel_path",
        )
        .unwrap();
    let rows = stmt
        .query_map([format!("name_key : \"{}\"", fold_key(term))], |r| r.get(0))
        .unwrap();
    rows.collect::<rusqlite::Result<_>>().unwrap()
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct NullSink;
impl ProgressSink for NullSink {
    fn on_progress(&self, _: &IndexProgress) {}
}

/// VFS-1 AC-4, AC-5, AC-8, decision D1: a first scan indexes every file with its metadata
/// and every directory (empty ones included) so a read connection finds them by folded
/// name; junk names are excluded and symlinks are not followed.
#[test]
fn full_scan_indexes_tree() {
    let fx = Fixture::new();
    fx.file("Gestoría/Año 2019/Notaría escritura.pdf", 10);
    fx.file("Gestoría/factura.txt", 3);
    fx.file("Fotos/IMG_0001.JPG", 7);
    fx.file("raiz.txt", 1);
    fx.file("Fotos/.DS_Store", 1);
    fx.file("Fotos/._IMG_0001.JPG", 1);
    fx.file("~$informe.docx", 1);
    fs::create_dir_all(fx.root().join("Vacía")).unwrap();
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("fuera.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(outside.path(), fx.root().join("enlace")).unwrap();
        std::os::unix::fs::symlink(fx.root(), fx.root().join("Fotos/bucle")).unwrap();
    }

    let summary = fx.scan();
    assert_eq!(summary.files_added, 4);
    assert_eq!(summary.dirs_listed, 5);
    assert_eq!(summary.errors, 0);
    assert_eq!(
        fx.all_files(),
        vec![
            "Fotos/IMG_0001.JPG",
            "Gestoría/Año 2019/Notaría escritura.pdf",
            "Gestoría/factura.txt",
            "raiz.txt"
        ]
    );
    assert_eq!(
        fx.all_dirs(),
        vec!["", "Fotos", "Gestoría", "Gestoría/Año 2019", "Vacía"]
    );
    assert_eq!(fx.find_files("notaria"), vec!["Notaría escritura.pdf"]);
    assert_eq!(fx.find_files("img_0001"), vec!["IMG_0001.JPG"]);
    assert!(fx.find_files("fuera").is_empty(), "symlink was followed");
    assert_eq!(find_dirs_by_name(&fx.reader(), "vacia"), vec!["Vacía"]);

    let (size, mtime): (i64, i64) = fx
        .reader()
        .query_row(
            "SELECT size, mtime FROM files WHERE name = 'Notaría escritura.pdf'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(size, 10);
    assert!(mtime > 1_600_000_000);
    assert_eq!(
        fx.count("SELECT COUNT(*) FROM dirs WHERE mtime <= 0"),
        0,
        "directories carry their mtime"
    );
    assert_eq!(
        fx.count("SELECT COUNT(*) FROM dirs WHERE scan_state <> 0"),
        0
    );
    let (status, files, dirs, last): (i64, i64, i64, Option<i64>) = fx
        .reader()
        .query_row(
            "SELECT status, files_count, dirs_count, last_full_scan_at FROM roots WHERE id = ?1",
            [fx.root_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!((status, files, dirs), (2, 4, 5));
    assert!(last.is_some());

    // A second scan with nothing pending does nothing.
    assert_eq!(fx.scan().dirs_listed, 0);
}

/// Pauses the indexer right after the first directory it lists.
struct PauseAfterFirstDir {
    reached: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

impl ProgressSink for PauseAfterFirstDir {
    fn on_progress(&self, p: &IndexProgress) {
        if p.state != IndexerState::Scanning || p.roots.iter().all(|r| r.dirs_done == 0) {
            return;
        }
        if let Some((reached, resume)) = self.reached.lock().unwrap().take() {
            reached.send(()).unwrap();
            resume.recv().unwrap();
        }
    }
}

/// VFS-1 AC-5, AC-6: an indexer stopped mid-scan has committed what it listed, which is
/// searchable at once; the next run resumes from the pending directories, lists each
/// remaining directory exactly once and ends with no duplicate rows.
#[test]
fn resume_after_stop_mid_scan() {
    let fx = Fixture::new();
    let mut total_files = 0;
    for a in 0..4 {
        fx.file(&format!("raiz {a}.txt"), 1);
        total_files += 1;
        for b in 0..3 {
            for c in 0..5 {
                fx.file(&format!("Carpeta {a}/Sub {b}/archivo {a}-{b}-{c}.txt"), c);
                total_files += 1;
            }
        }
    }
    let total_dirs = 1 + 4 + 4 * 3;

    let (reached_tx, reached_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let sink = Arc::new(PauseAfterFirstDir {
        reached: Mutex::new(Some((reached_tx, resume_rx))),
    });
    fx.vfs.start_indexer(sink).unwrap();
    reached_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    fx.vfs.set_paused(true);
    resume_tx.send(()).unwrap();
    wait_until("paused state", || {
        fx.vfs.progress().state == IndexerState::Paused
    });
    let progress = fx.vfs.progress();
    assert_eq!(progress.roots[0].dirs_done, 1);
    assert_eq!(progress.roots[0].dirs_pending, 4);
    fx.vfs.shutdown();

    // Committed rows are visible to a reader before the scan finishes.
    assert_eq!(fx.find_files("raiz").len(), 4);
    assert_eq!(fx.count("SELECT COUNT(*) FROM files"), 4);
    assert_eq!(
        fx.count("SELECT status FROM roots"),
        1,
        "root is still scanning"
    );

    // A new process: open the index again and finish.
    let vfs = Vfs::open(&fx.db_path).unwrap();
    let summary = vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    assert_eq!(
        summary.dirs_listed,
        total_dirs - 1,
        "no directory listed twice"
    );
    assert_eq!(summary.files_added, total_files - 4);
    assert_eq!(fx.count("SELECT COUNT(*) FROM files"), total_files as i64);
    assert_eq!(fx.count("SELECT COUNT(*) FROM dirs"), total_dirs as i64);
    assert_eq!(
        fx.count("SELECT COUNT(*) FROM (SELECT DISTINCT dir_id, name FROM files)"),
        total_files as i64
    );
    assert_eq!(fx.find_files("archivo 3-2-4").len(), 1);
    assert_eq!(fx.count("SELECT status FROM roots"), 2);
}

/// VFS-1 AC-7: a full re-walk requested from the running indexer reflects files added,
/// deleted, renamed, moved and changed in size, and folders renamed, moved and deleted.
#[test]
fn rewalk_detects_changes() {
    let fx = Fixture::new();
    fx.file("A/borrar.txt", 1);
    fx.file("A/renombrar.txt", 1);
    fx.file("A/mover.txt", 1);
    fx.file("A/cambiar.txt", 1);
    fx.file("B/queda.txt", 1);
    fx.file("C/Interior/profundo.txt", 1);
    fx.file("D/borrada/dentro.txt", 1);
    fx.scan();
    let root = fx.root();

    fs::remove_file(root.join("A/borrar.txt")).unwrap();
    fs::rename(root.join("A/renombrar.txt"), root.join("A/renombrado.txt")).unwrap();
    fs::rename(root.join("A/mover.txt"), root.join("B/mover.txt")).unwrap();
    fs::write(root.join("A/cambiar.txt"), b"mucho mas largo").unwrap();
    fx.file("B/nuevo.txt", 2);
    fs::rename(root.join("C"), root.join("C2")).unwrap();
    fs::rename(root.join("C2/Interior"), root.join("B/Interior")).unwrap();
    fs::remove_dir_all(root.join("D/borrada")).unwrap();

    fx.vfs.start_indexer(Arc::new(NullSink)).unwrap();
    fx.vfs.rescan(Some(fx.root_id)).unwrap();
    let expected = vec![
        "A/cambiar.txt",
        "A/renombrado.txt",
        "B/Interior/profundo.txt",
        "B/mover.txt",
        "B/nuevo.txt",
        "B/queda.txt",
    ];
    wait_until("re-walk", || {
        fx.all_files() == expected
            && fx.count("SELECT COUNT(*) FROM dirs WHERE scan_state <> 0") == 0
    });
    wait_until("idle", || fx.vfs.progress().state == IndexerState::Idle);
    fx.vfs.shutdown();

    assert_eq!(
        fx.all_dirs(),
        vec!["", "A", "B", "B/Interior", "C2", "D"],
        "renamed, moved and deleted folders"
    );
    assert_eq!(
        fx.count("SELECT size FROM files WHERE name = 'cambiar.txt'"),
        15
    );
    assert!(fx.find_files("renombrar").is_empty());
    assert!(fx.find_files("borrar").is_empty());
    assert_eq!(fx.find_files("profundo"), vec!["profundo.txt"]);
    assert_eq!(fx.count("SELECT COUNT(*) FROM files_fts"), 6);
    assert_eq!(fx.count("SELECT files_count FROM roots"), 6);
    assert_eq!(fx.count("SELECT status FROM roots"), 2);
}

/// VFS-1 AC-7: a watcher hint re-lists only the directory that changed (pending shallow),
/// and never downgrades a directory already pending deep.
#[test]
fn mark_dirty_relists_one_directory() {
    let fx = Fixture::new();
    fx.file("A/uno.txt", 1);
    fx.file("A/B/dos.txt", 1);
    fx.scan();
    let new_file = fx.file("A/B/tres.txt", 1);
    fs::create_dir_all(fx.root().join("A/B/Nueva/Honda")).unwrap();
    fx.file("A/B/Nueva/Honda/cuatro.txt", 1);

    fx.vfs.mark_dirty(&[new_file]).unwrap();
    let summary = fx.scan();
    assert_eq!(summary.dirs_listed, 3, "A/B plus the two new folders");
    assert_eq!(
        fx.all_files(),
        vec![
            "A/B/Nueva/Honda/cuatro.txt",
            "A/B/dos.txt",
            "A/B/tres.txt",
            "A/uno.txt"
        ]
    );

    // A pending deep directory stays deep.
    let conn = db::open_rw(&fx.db_path).unwrap();
    conn.execute("UPDATE dirs SET scan_state = 1 WHERE rel_path = 'A'", [])
        .unwrap();
    drop(conn);
    fx.vfs.mark_dirty(&[fx.root().join("A/uno.txt")]).unwrap();
    assert_eq!(
        fx.count("SELECT scan_state FROM dirs WHERE rel_path = 'A'"),
        1
    );
    assert_eq!(fx.scan().dirs_listed, 4, "deep re-list of A and below");
}

/// VFS-1 AC-8: a file whose absolute path is longer than 260 characters is indexed.
#[test]
fn long_path_over_260_chars() {
    let fx = Fixture::new();
    let segment = "carpeta con un nombre bastante largo para la prueba";
    let rel: Vec<String> = (0..6).map(|i| format!("{segment} {i}")).collect();
    let rel = format!("{}/documento final notaría.pdf", rel.join("/"));
    let path = fx.file(&rel, 5);
    assert!(path.to_str().unwrap().chars().count() > 260);

    let summary = fx.scan();
    assert_eq!(summary.errors, 0);
    assert_eq!(fx.all_files(), vec![rel]);
    assert_eq!(
        fx.find_files("final notaria"),
        vec!["documento final notaría.pdf"]
    );
}

/// VFS-1 AC-8: names written in NFC and in NFD are stored as the filesystem returns them
/// and both are found by the same accent-free query.
#[test]
fn nfc_and_nfd_names() {
    let fx = Fixture::new();
    fx.file("nfc/Notar\u{ed}a P\u{e9}rez.pdf", 1);
    fx.file("nfd/Notari\u{301}a Pe\u{301}rez.pdf", 1);
    fx.scan();

    let on_disk = |dir: &str| -> String {
        let entry = fs::read_dir(fx.root().join(dir))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        entry.file_name().into_string().unwrap()
    };
    let mut expected = vec![on_disk("nfc"), on_disk("nfd")];
    expected.sort();
    assert_eq!(fx.find_files("notaria perez"), expected);
    assert_eq!(fx.find_files("NOTARÍA"), expected);
}

/// VFS-1 AC-8 (edge cases): a directory that cannot be read is counted as an error and
/// skipped; the scan completes and indexes everything else.
#[cfg(unix)]
#[test]
fn unreadable_directory_does_not_abort_scan() {
    use std::os::unix::fs::PermissionsExt;

    let fx = Fixture::new();
    fx.file("Abierta/visible.txt", 1);
    fx.file("Cerrada/oculto.txt", 1);
    fx.file("Zeta/final.txt", 1);
    let locked = fx.root().join("Cerrada");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let readable_anyway = fs::read_dir(&locked).is_ok(); // running as root

    let summary = fx.scan();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if !readable_anyway {
        assert_eq!(summary.errors, 1);
        assert_eq!(
            fx.all_files(),
            vec!["Abierta/visible.txt", "Zeta/final.txt"]
        );
        assert!(fx.all_dirs().contains(&"Cerrada".to_string()));
    }
    assert_eq!(fx.count("SELECT status FROM roots"), 2);
    assert_eq!(
        fx.count("SELECT COUNT(*) FROM dirs WHERE scan_state <> 0"),
        0
    );
}

/// VFS-1 AC-5: reads through a separate read connection succeed while a scan is writing,
/// and only ever see committed, growing results.
#[test]
fn search_while_scanning() {
    let fx = Fixture::new();
    for a in 0..40 {
        for b in 0..25 {
            fx.file(&format!("Lote {a}/foto {a}-{b}.jpg"), 1);
        }
    }
    let db_path = fx.db_path.clone();
    let scanner = std::thread::spawn(move || {
        let vfs = Vfs::open(&db_path).unwrap();
        vfs.scan_to_completion(&AtomicBool::new(false)).unwrap()
    });
    let reader = fx.reader();
    let mut reads_during_scan = 0;
    let mut last = 0;
    while !scanner.is_finished() {
        let n = find_files(&reader, "foto").len();
        assert!(n >= last, "results only grow");
        last = n;
        reads_during_scan += 1;
    }
    let summary = scanner.join().unwrap();
    assert!(reads_during_scan > 0);
    assert_eq!(summary.files_added, 1000);
    assert_eq!(find_files(&reader, "foto").len(), 1000);
    assert_eq!(all_files(&reader).len(), 1000);
}

/// VFS-1 AC-4: a root marked `removing` is deleted completely by the next scan.
#[test]
fn removing_root_is_finished() {
    let fx = Fixture::new();
    fx.file("A/uno.txt", 1);
    fx.scan();
    let conn = db::open_rw(&fx.db_path).unwrap();
    conn.execute("UPDATE roots SET status = 4", []).unwrap();
    drop(conn);
    fx.scan();
    for table in ["roots", "dirs", "files", "files_fts", "dirs_fts"] {
        assert_eq!(
            fx.count(&format!("SELECT COUNT(*) FROM {table}")),
            0,
            "{table}"
        );
    }
}

/// VFS-1 AC-6: an unreachable root becomes `unavailable` and keeps its rows.
#[test]
fn unreachable_root_keeps_rows() {
    let fx = Fixture::new();
    fx.file("A/uno.txt", 1);
    fx.scan();
    let conn = db::open_rw(&fx.db_path).unwrap();
    conn.execute("UPDATE roots SET path = path || '-desconectada'", [])
        .unwrap();
    conn.execute("UPDATE dirs SET scan_state = 1 WHERE rel_path = ''", [])
        .unwrap();
    drop(conn);
    fx.scan();
    assert_eq!(fx.count("SELECT status FROM roots"), 3);
    assert_eq!(fx.count("SELECT COUNT(*) FROM files"), 1);
}
