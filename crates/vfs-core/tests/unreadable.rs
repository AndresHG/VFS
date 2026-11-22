//! Folders that could not be read, through the public API (VFS-10).

use vfs_core::db::{self, ingest};
use vfs_core::Vfs;

/// VFS-10 AC-5: an index at the previous schema version gets `dirs.error` and its partial
/// index when opened, with no folder unreadable and no rescan queued.
#[test]
fn migration_adds_the_reason_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    {
        let mut conn = db::open_rw(&path).unwrap();
        let tx = conn.transaction().unwrap();
        ingest::insert_root(&tx, "/", "/a").unwrap();
        tx.execute_batch(
            "DROP INDEX dirs_unpopulated; ALTER TABLE dirs DROP COLUMN unpopulated; \
             ALTER TABLE roots DROP COLUMN cloud_only_count; ALTER TABLE roots DROP COLUMN links_count; \
             DROP INDEX dirs_unreadable; ALTER TABLE dirs DROP COLUMN error; \
             UPDATE dirs SET scan_state = 0;",
        )
        .unwrap();
        tx.pragma_update(None, "user_version", 5).unwrap();
        tx.commit().unwrap();
    }
    let vfs = Vfs::open(&path).unwrap();
    let roots = vfs.list_roots().unwrap();
    assert_eq!(roots[0].unreadable, 0);
    assert!(vfs.unreadable_dirs(roots[0].id).unwrap().is_empty());
    let conn = db::open_ro(&path).unwrap();
    let (version, index, queued): (u32, i64, i64) = conn
        .query_row(
            "SELECT (SELECT user_version FROM pragma_user_version), \
             (SELECT COUNT(*) FROM sqlite_master WHERE name = 'dirs_unreadable'), \
             (SELECT COUNT(*) FROM dirs WHERE scan_state <> 0)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((version, index, queued), (db::SCHEMA_VERSION, 1, 0));
}
