//! Adding, listing and removing root folders through the `Vfs` facade (VFS-1 AC-4, U4).

use std::fs;
use std::path::Path;

use vfs_core::db::{self, ingest};
use vfs_core::{Error, RootStatus, SearchFilters, SearchQuery, Vfs};

fn open(dir: &Path) -> Vfs {
    Vfs::open(&dir.join("index.db")).unwrap()
}

fn overlap(e: Error) -> String {
    match e {
        Error::OverlappingRoot(m) => m,
        other => panic!("expected OverlappingRoot, got {other:?}"),
    }
}

/// VFS-1 AC-4: an added root is stored canonical, listed as pending with its root folder
/// queued for a deep scan.
#[test]
fn ac4_add_and_list_root() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tmp.path().join("Archivo");
    fs::create_dir(&archive).unwrap();
    let vfs = open(tmp.path());
    // A path with `..` and a trailing separator is canonicalized.
    let messy = tmp.path().join("Archivo").join("..").join("Archivo");
    let root = vfs.add_root(&messy).unwrap();
    let canon = dunce::canonicalize(&archive).unwrap();
    assert_eq!(Path::new(&root.path), canon);
    assert_eq!(root.status, RootStatus::Pending);
    assert_eq!((root.files, root.dirs), (0, 0));
    assert!(root.last_full_scan.is_none());
    let listed = vfs.list_roots().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, root.id);
    assert_eq!(listed[0].path, root.path);
    let conn = db::open_ro(&tmp.path().join("index.db")).unwrap();
    let pending: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dirs WHERE root_id = ?1 AND rel_path = '' AND scan_state = 1",
            [root.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
}

/// VFS-1 AC-4: a missing path and a file are rejected with their own error codes.
#[test]
fn ac4_rejects_missing_path_and_file() {
    let tmp = tempfile::tempdir().unwrap();
    let vfs = open(tmp.path());
    let missing = vfs.add_root(&tmp.path().join("no existe")).unwrap_err();
    assert_eq!(missing.code(), "not_found");
    let file = tmp.path().join("nota.txt");
    fs::write(&file, b"x").unwrap();
    let not_dir = vfs.add_root(&file).unwrap_err();
    assert_eq!(not_dir.code(), "not_a_directory");
    assert!(vfs.list_roots().unwrap().is_empty());
}

/// VFS-1 AC-4, U4: the same folder, a folder inside a root and a folder containing a root
/// are each rejected with a relation the UI can word; a sibling sharing a name prefix is
/// accepted.
#[test]
fn ac4_u4_overlapping_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("Datos");
    let fotos = parent.join("Fotos");
    let child = fotos.join("2019");
    let sibling = parent.join("Fotos2");
    for d in [&child, &sibling] {
        fs::create_dir_all(d).unwrap();
    }
    let vfs = open(tmp.path());
    let root = vfs.add_root(&fotos).unwrap();

    let same = overlap(vfs.add_root(&fotos).unwrap_err());
    assert_eq!(same, format!("same {}", root.path));
    let inside = overlap(vfs.add_root(&child).unwrap_err());
    assert_eq!(inside, format!("inside {}", root.path));
    let contains = overlap(vfs.add_root(&parent).unwrap_err());
    assert_eq!(contains, format!("contains {}", root.path));
    assert_eq!(Error::OverlappingRoot(contains).code(), "overlapping_root");

    let ok = vfs.add_root(&sibling).unwrap();
    assert_ne!(ok.id, root.id);
    assert_eq!(vfs.list_roots().unwrap().len(), 2);
}

/// VFS-1 AC-4: only what hangs below the selected roots is found; a removed root is marked
/// `removing`, its files disappear from search at once, and its folder can be chosen again
/// only once the indexer has deleted it.
#[test]
fn ac4_remove_root_hides_its_files() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("RaizA");
    let b = tmp.path().join("RaizB");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let db_path = tmp.path().join("index.db");
    let vfs = Vfs::open(&db_path).unwrap();
    let ra = vfs.add_root(&a).unwrap();
    let rb = vfs.add_root(&b).unwrap();

    let mut w = db::open_rw(&db_path).unwrap();
    let tx = w.transaction().unwrap();
    for (root, name) in [(ra.id, "qwnotas k7a.txt"), (rb.id, "qwnotas k7b.txt")] {
        let top: i64 = tx
            .query_row(
                "SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ''",
                [root],
                |r| r.get(0),
            )
            .unwrap();
        let f = ingest::NewFile {
            name,
            size: 1,
            mtime: 1,
            lossy: false,
        };
        ingest::insert_file(&tx, top, &f).unwrap();
    }
    tx.commit().unwrap();
    drop(w);

    let find = |text: &str| {
        vfs.search(&SearchQuery {
            text: text.into(),
            filters: SearchFilters::default(),
            limit: 100,
            offset: 0,
        })
        .unwrap()
    };
    assert_eq!(find("qwnot k7a").hits.len(), 1);
    assert_eq!(find("qwnot k7b").hits.len(), 1);

    vfs.remove_root(ra.id).unwrap();
    let roots = vfs.list_roots().unwrap();
    let removed = roots.iter().find(|r| r.id == ra.id).unwrap();
    assert_eq!(removed.status, RootStatus::Removing);
    assert!(find("qwnot k7a").hits.is_empty());
    assert_eq!(find("qwnot k7b").hits.len(), 1);
    assert_eq!(
        overlap(vfs.add_root(&a).unwrap_err()),
        format!("same {}", ra.path)
    );

    let unknown = vfs.remove_root(9_999).unwrap_err();
    assert_eq!(unknown.code(), "not_found");
}
