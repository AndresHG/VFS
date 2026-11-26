//! A root's cloud-only, link and unpopulated counts, through the public API (VFS-37).

use vfs_core::db::{self, ingest};
use vfs_core::Vfs;

/// VFS-37 AC-3 (R3): an index at schema 6 opens at the current version with the new columns
/// and partial index, its roots' cloud-only and link counts unknown until a full walk, no
/// folder marked unpopulated and no rescan queued.
#[test]
fn migration_leaves_counts_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    {
        let mut conn = db::open_rw(&path).unwrap();
        let tx = conn.transaction().unwrap();
        ingest::insert_root(&tx, "/", "/a").unwrap();
        tx.execute_batch(
            "DROP INDEX dirs_unpopulated; ALTER TABLE dirs DROP COLUMN unpopulated; \
             ALTER TABLE roots DROP COLUMN cloud_only_count; \
             ALTER TABLE roots DROP COLUMN links_count; \
             UPDATE dirs SET scan_state = 0;",
        )
        .unwrap();
        tx.pragma_update(None, "user_version", 6).unwrap();
        tx.commit().unwrap();
    }
    let vfs = Vfs::open(&path).unwrap();
    let root = vfs.list_roots().unwrap().remove(0);
    assert_eq!(
        (root.cloud_only, root.links, root.unpopulated),
        (None, None, 0)
    );
    let conn = db::open_ro(&path).unwrap();
    let (version, index, queued): (u32, i64, i64) = conn
        .query_row(
            "SELECT (SELECT user_version FROM pragma_user_version), \
             (SELECT COUNT(*) FROM sqlite_master WHERE name = 'dirs_unpopulated'), \
             (SELECT COUNT(*) FROM dirs WHERE scan_state <> 0)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((version, index, queued), (db::SCHEMA_VERSION, 1, 0));
}

/// VFS-50 AC-8 (R6): an incremental pass moves a ready root's file count by the files it
/// added and removed, without recounting (a count set off on purpose stays off by the same
/// amount), and the next full walk's recount sets it right.
#[test]
fn incremental_pass_shifts_the_counts() {
    use std::sync::atomic::AtomicBool;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Fotos");
    std::fs::create_dir_all(root.join("viaje")).unwrap();
    for i in 0..4 {
        std::fs::write(root.join(format!("viaje/f{i}.jpg")), b"x").unwrap();
    }
    let vfs = Vfs::open(&dir.path().join("index.db")).unwrap();
    vfs.add_root(&root).unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    let r = vfs.list_roots().unwrap().remove(0);
    assert_eq!((r.files, r.dirs), (4, 2));
    // Off by 100 on purpose: a recount would put it back to the truth.
    db::open_rw(&dir.path().join("index.db"))
        .unwrap()
        .execute("UPDATE roots SET files_count = files_count + 100", [])
        .unwrap();
    for i in 4..7 {
        std::fs::write(root.join(format!("viaje/f{i}.jpg")), b"x").unwrap();
    }
    std::fs::remove_file(root.join("viaje/f0.jpg")).unwrap();
    std::fs::create_dir(root.join("nueva")).unwrap();
    let canonical = std::path::PathBuf::from(&r.path);
    vfs.mark_dirty(&[canonical.join("viaje"), canonical.clone()])
        .unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    let r = vfs.list_roots().unwrap().remove(0);
    assert_eq!((r.files, r.dirs), (106, 3));
    // A full walk, queued as the indexer queues one (status scanning, root folder deep).
    db::open_rw(&dir.path().join("index.db"))
        .unwrap()
        .execute_batch(&format!(
            "UPDATE roots SET status = {}; UPDATE dirs SET scan_state = {} WHERE rel_path = '';",
            vfs_core::RootStatus::Scanning.code(),
            ingest::ScanState::PendingDeep as i64
        ))
        .unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    let r = vfs.list_roots().unwrap().remove(0);
    assert_eq!((r.files, r.dirs), (6, 3));
}
