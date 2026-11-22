//! Adding a folder that contains listed roots takes them in (VFS-31 R2, R3), and root
//! changes leave diagnostic records with no names (R5).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, Once};
use std::thread::ThreadId;

use rusqlite::Connection;
use vfs_core::{
    db, Error, OverlapRelation, SearchFilters, SearchQuery, SearchResponse, Vfs, DIAG_TARGET,
};

/// Keeps every diagnostic record logged in this test binary, with the thread that logged it
/// (tests run in parallel; here every record comes from the calling thread).
struct Capture;
static RECORDS: Mutex<Vec<(ThreadId, String)>> = Mutex::new(Vec::new());

impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target() == DIAG_TARGET
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            RECORDS
                .lock()
                .unwrap()
                .push((std::thread::current().id(), r.args().to_string()));
        }
    }
    fn flush(&self) {}
}

fn capture() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(log::LevelFilter::Info);
    });
}

struct Env {
    tmp: tempfile::TempDir,
    vfs: Vfs,
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let vfs = Vfs::open(&tmp.path().join("index.db")).unwrap();
        Env { tmp, vfs }
    }

    /// `rel` below the canonical temporary folder, created as a folder.
    fn dir(&self, rel: &str) -> PathBuf {
        let p = dunce::canonicalize(self.tmp.path()).unwrap().join(rel);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn file(&self, rel: &str) {
        let p = self.dir("").join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    fn scan(&self) {
        self.vfs
            .scan_to_completion(&AtomicBool::new(false))
            .unwrap();
    }

    fn reader(&self) -> Connection {
        db::open_ro(&self.tmp.path().join("index.db")).unwrap()
    }

    /// Every indexed file as "root path | rel path" -> file id.
    fn files(&self) -> BTreeMap<String, i64> {
        let conn = self.reader();
        let mut stmt = conn
            .prepare(
                "SELECT r.path, CASE d.rel_path WHEN '' THEN f.name \
                 ELSE d.rel_path || '/' || f.name END, f.id \
                 FROM files f JOIN dirs d ON d.id = f.dir_id JOIN roots r ON r.id = d.root_id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    format!("{}|{}", r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                    r.get(2)?,
                ))
            })
            .unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    }

    fn find(&self, text: &str) -> SearchResponse {
        self.vfs
            .search(&SearchQuery {
                text: text.into(),
                filters: SearchFilters::default(),
                limit: 100,
                offset: 0,
                sort: Default::default(),
            })
            .unwrap()
    }

    /// Folder rows sharing a root and a path: none, or the walk inserted the moved tree
    /// again beside the moved rows.
    fn duplicate_dirs(&self) -> i64 {
        self.reader()
            .query_row(
                "SELECT COUNT(*) FROM (SELECT 1 FROM dirs GROUP BY root_id, rel_path \
                 HAVING COUNT(*) > 1)",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
}

/// VFS-31 AC-2 (issue 154): adding a folder that contains an indexed root takes the root in:
/// one root is listed, the inner files keep their ids and are found by name and by path
/// under the new root before any walk, and after the walk the index holds every file on
/// disk once, with the same ids. Two inner roots under one intermediate folder are taken
/// in together.
#[test]
fn adding_a_parent_takes_the_inner_roots_in() {
    let env = Env::new();
    env.file("Datos/raiz.txt");
    env.file("Datos/Fotos/verano qz9.jpg");
    env.file("Datos/Fotos/2019/playa qz9.jpg");
    env.file("Datos/Docs/Banco/extracto qz9.pdf");
    env.file("Datos/Docs/Notas/lista qz9.txt");
    let fotos = env.vfs.add_root(&env.dir("Datos/Fotos")).unwrap();
    let banco = env.vfs.add_root(&env.dir("Datos/Docs/Banco")).unwrap();
    let notas = env.vfs.add_root(&env.dir("Datos/Docs/Notas")).unwrap();
    env.scan();
    let before: Vec<i64> = env.files().into_values().collect();
    assert_eq!(before.len(), 4);

    let datos = env.dir("Datos");
    let root = env.vfs.add_root(&datos).unwrap();
    let roots = env.vfs.list_roots().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, root.id);
    assert!(![fotos.id, banco.id, notas.id].contains(&root.id));
    assert_eq!(roots[0].files, 4, "taken-in files count before the walk");
    let d = datos.to_str().unwrap();
    let moved = env.files();
    assert_eq!(
        moved.keys().cloned().collect::<Vec<_>>(),
        vec![
            format!("{d}|Docs/Banco/extracto qz9.pdf"),
            format!("{d}|Docs/Notas/lista qz9.txt"),
            format!("{d}|Fotos/2019/playa qz9.jpg"),
            format!("{d}|Fotos/verano qz9.jpg"),
        ]
    );
    let mut ids: Vec<i64> = moved.values().copied().collect();
    ids.sort();
    let mut was = before.clone();
    was.sort();
    assert_eq!(ids, was, "the files keep their ids");
    // By path: the folder names now in the path index under the new root.
    let hits = env.find("fotos playa qz9").hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(Path::new(&hits[0].dir), datos.join("Fotos").join("2019"));
    assert_eq!(env.find("banco extracto qz9").hits.len(), 1);
    let folder = env.find("notas").hits;
    assert!(folder
        .iter()
        .any(|h| h.is_dir && Path::new(&h.dir) == datos.join("Docs")));

    env.scan();
    let walked = env.files();
    assert_eq!(walked.len(), 5, "{walked:?}");
    assert!(walked.contains_key(&format!("{d}|raiz.txt")));
    for (k, id) in &moved {
        assert_eq!(walked.get(k), Some(id), "{k} kept its id through the walk");
    }
    assert_eq!(env.duplicate_dirs(), 0);
    assert_eq!(env.find("verano qz9").hits.len(), 1);
    assert_eq!(env.vfs.list_roots().unwrap()[0].files, 5);
}

/// VFS-31 AC-3: the same folder and a folder inside a root are still refused, with their
/// relation, and nothing changes.
#[test]
fn same_and_inside_are_still_refused() {
    let env = Env::new();
    let fotos = env.dir("Datos/Fotos");
    let root = env.vfs.add_root(&fotos).unwrap();
    let rel = |e: Error| match e {
        Error::OverlappingRoot { relation, .. } => relation,
        other => panic!("expected OverlappingRoot, got {other:?}"),
    };
    assert_eq!(
        rel(env.vfs.add_root(&fotos).unwrap_err()),
        OverlapRelation::Same
    );
    assert_eq!(
        rel(env.vfs.add_root(&env.dir("Datos/Fotos/2019")).unwrap_err()),
        OverlapRelation::Inside
    );
    let roots = env.vfs.list_roots().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, root.id);
}

/// VFS-31 AC-5: adding, removing and finishing a removal each leave a diagnostic record of
/// `key=number` pairs, with no folder or file name.
#[test]
fn root_changes_leave_records_without_names() {
    capture();
    let env = Env::new();
    env.file("Secreto/Fotos/boda.jpg");
    let fotos = env.vfs.add_root(&env.dir("Secreto/Fotos")).unwrap();
    env.scan();
    let root = env.vfs.add_root(&env.dir("Secreto")).unwrap();
    env.vfs.remove_root(root.id).unwrap();
    env.scan();
    let me = std::thread::current().id();
    let records: Vec<String> = RECORDS
        .lock()
        .unwrap()
        .iter()
        .filter(|(t, _)| *t == me)
        .map(|(_, r)| r.clone())
        .collect();
    let of = |event: &str, id: i64| {
        records
            .iter()
            .find(|r| r.starts_with(&format!("{event} id={id} ")))
            .unwrap_or_else(|| panic!("no {event} record for root {id} in {records:?}"))
            .clone()
    };
    let lines = [
        of("root_add", fotos.id),
        of("root_add", root.id),
        of("root_remove", root.id),
        of("root_removed", root.id),
    ];
    assert!(lines[1].contains(" taken_roots=1 taken_dirs=1 taken_files=1 "));
    assert!(lines[3].contains(" dirs=2 files=1 "), "{}", lines[3]);
    for line in &lines {
        let mut words = line.split(' ');
        words.next();
        for pair in words {
            let (key, value) = pair.split_once('=').expect(line);
            assert!(!key.is_empty() && value.parse::<u64>().is_ok(), "{line}");
        }
        assert!(
            !line.contains("Secreto") && !line.contains("boda"),
            "{line}"
        );
    }
}
