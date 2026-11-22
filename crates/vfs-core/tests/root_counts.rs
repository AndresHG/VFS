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
