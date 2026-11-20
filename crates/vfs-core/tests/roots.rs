//! Adding, listing and removing root folders through the `Vfs` facade (VFS-1 AC-4, U4).

use std::fs;
use std::path::Path;

use vfs_core::db::ingest::ScanState;
use vfs_core::db::{self, ingest};
use vfs_core::{DriveKind, Error, OverlapRelation, RootStatus, SearchFilters, SearchQuery, Vfs};

fn open(dir: &Path) -> Vfs {
    Vfs::open(&dir.join("index.db")).unwrap()
}

fn overlap(e: Error) -> (OverlapRelation, String) {
    match e {
        Error::OverlappingRoot { relation, existing } => (relation, existing),
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
            "SELECT COUNT(*) FROM dirs WHERE root_id = ?1 AND rel_path = '' AND scan_state = ?2",
            [root.id, ScanState::PendingDeep as i64],
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

    let same = vfs.add_root(&fotos).unwrap_err();
    assert_eq!(
        same.to_string(),
        format!("overlaps an indexed folder: same {}", root.path)
    );
    assert_eq!(same.code(), "overlapping_root");
    assert_eq!(overlap(same), (OverlapRelation::Same, root.path.clone()));
    let inside = overlap(vfs.add_root(&child).unwrap_err());
    assert_eq!(inside, (OverlapRelation::Inside, root.path.clone()));
    let contains = vfs.add_root(&parent).unwrap_err();
    assert_eq!(
        contains.to_string(),
        format!("overlaps an indexed folder: contains {}", root.path)
    );
    assert_eq!(
        overlap(contains),
        (OverlapRelation::Contains, root.path.clone())
    );

    let ok = vfs.add_root(&sibling).unwrap();
    assert_ne!(ok.id, root.id);
    assert_eq!(vfs.list_roots().unwrap().len(), 2);
}

/// VFS-1 AC-4: only what hangs below the selected roots is found; a removed root is marked
/// `removing` and its files disappear from search at once. Choosing the same folder again
/// before the indexer has deleted it supersedes the removal instead of being refused.
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

    let again = vfs.add_root(&a).unwrap();
    assert_ne!(again.id, ra.id);
    assert_eq!(again.path, ra.path);
    assert_eq!(again.status, RootStatus::Pending);
    let ids: Vec<i64> = vfs.list_roots().unwrap().iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![rb.id, again.id], "the old root is gone");
    assert!(
        find("qwnot k7a").hits.is_empty(),
        "old rows were deleted, not revived"
    );
    let conn = db::open_ro(&db_path).unwrap();
    let old_dirs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dirs WHERE root_id = ?1",
            [ra.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(old_dirs, 0);

    let unknown = vfs.remove_root(9_999).unwrap_err();
    assert_eq!(unknown.code(), "not_found");
}

/// VFS-1 AC-4: a root being removed does not count as overlapping, so its parent folder can
/// be added at once.
#[test]
fn ac4_removing_root_does_not_block_its_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("Datos");
    let child = parent.join("Fotos");
    fs::create_dir_all(&child).unwrap();
    let vfs = open(tmp.path());
    let old = vfs.add_root(&child).unwrap();
    vfs.remove_root(old.id).unwrap();
    let root = vfs.add_root(&parent).unwrap();
    let roots = vfs.list_roots().unwrap();
    assert_eq!(roots.len(), 2);
    assert!(roots
        .iter()
        .any(|r| r.id == old.id && r.status == RootStatus::Removing));
    assert!(roots
        .iter()
        .any(|r| r.id == root.id && r.status == RootStatus::Pending));
}

/// VFS-1 AC-4 (non-goal: network and removable drives): roots report the kind of drive
/// they are on, from `add_root` and from `list_roots`, so the UI can warn. A temporary
/// folder is on the local system disk; a folder that no longer exists is unknown.
#[test]
fn ac4_roots_report_their_drive_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("Local");
    fs::create_dir(&a).unwrap();
    let vfs = open(tmp.path());
    let root = vfs.add_root(&a).unwrap();
    let expected = if cfg!(any(windows, target_os = "macos")) {
        DriveKind::Local
    } else {
        DriveKind::Unknown
    };
    assert_eq!(root.drive, expected);
    assert_eq!(vfs.list_roots().unwrap()[0].drive, expected);
    fs::remove_dir(&a).unwrap();
    assert_eq!(vfs.list_roots().unwrap()[0].drive, DriveKind::Unknown);
}
