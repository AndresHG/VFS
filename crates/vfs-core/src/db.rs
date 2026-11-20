//! Connections, pragmas, migrations and the write helpers shared by the indexer and the
//! synthetic database generator (`ingest`), so both produce exactly the same layout.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::{Error, Result};

/// `PRAGMA user_version` written by this build. Version 2 (phase 2, VFS-1 AC-15) adds
/// `doc_fts` and `content_jobs` without touching existing tables.
pub const SCHEMA_VERSION: u32 = 1;

const SCHEMA_V1: &str = include_str!("schema.sql");

/// Pragmas shared by every connection. `mmap_size=0` keeps resident memory bounded by the
/// page caches (VFS-1 AC-9).
fn apply_common_pragmas(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_millis(5000))?;
    let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
    conn.execute_batch(
        "PRAGMA synchronous=NORMAL;
         PRAGMA temp_store=MEMORY;
         PRAGMA foreign_keys=OFF;",
    )?;
    let _mmap: Option<i64> = conn.query_row("PRAGMA mmap_size=0", [], |r| r.get(0)).ok();
    Ok(())
}

fn schema_version(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Opens (creating if needed) the index for writing: writer pragmas, then creates or
/// migrates the schema. Fails with `SchemaTooNew` for an index from a newer build.
pub fn open_rw(db_path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(db_path)?;
    apply_common_pragmas(&conn)?;
    conn.execute_batch(
        "PRAGMA cache_size=-32768;
         PRAGMA wal_autocheckpoint=4000;",
    )?;
    let _limit: i64 = conn.query_row("PRAGMA journal_size_limit=67108864", [], |r| r.get(0))?;
    let version = schema_version(&conn)?;
    if version > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew);
    }
    if version == 0 {
        let tx = conn.transaction()?;
        tx.execute_batch(SCHEMA_V1)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
    }
    Ok(conn)
}

/// Opens an existing index for reading only (`query_only=1`, 64 MB page cache).
pub fn open_ro(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    apply_common_pragmas(&conn)?;
    conn.execute_batch(
        "PRAGMA cache_size=-65536;
         PRAGMA query_only=1;",
    )?;
    if schema_version(&conn)? > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew);
    }
    Ok(conn)
}

/// Write helpers. Every FTS row is written here, in the caller's transaction, next to its
/// base row (`files_fts.rowid = files.id`, `dirs_fts.rowid = dirs.id`).
pub mod ingest {
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::{params, Connection, OptionalExtension, Transaction};

    use crate::error::{Error, Result};
    use crate::fold::fold_key;
    use crate::kind::file_kind;

    /// Value of `dirs.scan_state`; the persistent work queue of the indexer.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ScanState {
        Clean = 0,
        PendingDeep = 1,
        PendingShallow = 2,
    }

    pub struct NewFile<'a> {
        pub name: &'a str,
        pub size: u64,
        pub mtime: i64,
        /// The on-disk name was not valid Unicode and `name` is its lossy form (`flags` bit 0).
        pub lossy: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ChildFile {
        pub id: i64,
        pub name: String,
        pub size: u64,
        pub mtime: i64,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ChildDir {
        pub id: i64,
        /// Last component of `rel_path`.
        pub name: String,
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Inserts a root (status pending) and its root directory row (`rel_path = ''`,
    /// pending deep); creates the volume row if needed. `path` is absolute with native
    /// separators and no trailing separator. Returns the root id.
    pub fn insert_root(tx: &Transaction, volume_key: &str, path: &str) -> Result<i64> {
        tx.execute(
            "INSERT INTO volumes(volume_key) VALUES (?1) ON CONFLICT(volume_key) DO NOTHING",
            [volume_key],
        )?;
        let volume_id: i64 = tx.query_row(
            "SELECT id FROM volumes WHERE volume_key = ?1",
            [volume_key],
            |r| r.get(0),
        )?;
        let path_key = fold_key(&path.replace('\\', "/"));
        tx.execute(
            "INSERT INTO roots(volume_id, path, path_key, status, added_at) VALUES (?1, ?2, ?3, 0, ?4)",
            params![volume_id, path, path_key, now()],
        )?;
        let root_id = tx.last_insert_rowid();
        insert_dir_row(tx, root_id, None, "")?;
        Ok(root_id)
    }

    fn insert_dir_row(
        tx: &Transaction,
        root_id: i64,
        parent_id: Option<i64>,
        rel_path: &str,
    ) -> Result<i64> {
        let path_key = fold_key(rel_path);
        tx.execute(
            "INSERT INTO dirs(root_id, parent_id, rel_path, path_key, scan_state) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![root_id, parent_id, rel_path, path_key, ScanState::PendingDeep as i64],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO dirs_fts(rowid, path_key) VALUES (?1, ?2)",
            params![id, path_key],
        )?;
        Ok(id)
    }

    /// Inserts a directory (pending deep). `rel_path` is '/'-separated original names
    /// relative to the root.
    pub fn insert_dir(
        tx: &Transaction,
        root_id: i64,
        parent_id: i64,
        rel_path: &str,
    ) -> Result<i64> {
        insert_dir_row(tx, root_id, Some(parent_id), rel_path)
    }

    /// Inserts a file and its `files_fts` row; computes `name_key` and `kind`.
    pub fn insert_file(tx: &Transaction, dir_id: i64, file: &NewFile) -> Result<i64> {
        let name_key = fold_key(file.name);
        tx.execute(
            "INSERT INTO files(dir_id, name, name_key, kind, size, mtime, flags) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                dir_id,
                file.name,
                name_key,
                file_kind(file.name).code(),
                file.size as i64,
                file.mtime,
                i64::from(file.lossy),
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO files_fts(rowid, name_key) VALUES (?1, ?2)",
            params![id, name_key],
        )?;
        Ok(id)
    }

    pub fn update_file(tx: &Transaction, file_id: i64, size: u64, mtime: i64) -> Result<()> {
        tx.execute(
            "UPDATE files SET size = ?2, mtime = ?3 WHERE id = ?1",
            params![file_id, size as i64, mtime],
        )?;
        Ok(())
    }

    /// Deletes a file and its `files_fts` row.
    pub fn delete_file(tx: &Transaction, file_id: i64) -> Result<()> {
        tx.execute("DELETE FROM files_fts WHERE rowid = ?1", [file_id])?;
        tx.execute("DELETE FROM files WHERE id = ?1", [file_id])?;
        Ok(())
    }

    /// Deletes a directory, every directory below it and all their files, with their FTS
    /// rows. Returns the number of files removed.
    pub fn delete_dir_subtree(tx: &Transaction, dir_id: i64) -> Result<u64> {
        let (root_id, rel_path): (i64, String) = tx
            .query_row(
                "SELECT root_id, rel_path FROM dirs WHERE id = ?1",
                [dir_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("directory {dir_id}")))?;
        // Descendants of `p` are exactly the rel_paths in [p + '/', p + '0'): '0' follows '/'.
        let subtree = if rel_path.is_empty() {
            "SELECT id FROM dirs WHERE root_id = ?1"
        } else {
            "SELECT id FROM dirs WHERE root_id = ?1 AND (rel_path = ?2 \
             OR (rel_path >= ?2 || '/' AND rel_path < ?2 || '0'))"
        };
        let all = params![root_id, rel_path];
        let p = if rel_path.is_empty() { &all[..1] } else { all };
        let removed: i64 = tx.query_row(
            &format!("SELECT COUNT(*) FROM files WHERE dir_id IN ({subtree})"),
            p,
            |r| r.get(0),
        )?;
        tx.execute(
            &format!(
                "DELETE FROM files_fts WHERE rowid IN \
                 (SELECT id FROM files WHERE dir_id IN ({subtree}))"
            ),
            p,
        )?;
        tx.execute(&format!("DELETE FROM files WHERE dir_id IN ({subtree})"), p)?;
        tx.execute(
            &format!("DELETE FROM dirs_fts WHERE rowid IN ({subtree})"),
            p,
        )?;
        tx.execute(&format!("DELETE FROM dirs WHERE id IN ({subtree})"), p)?;
        Ok(removed as u64)
    }

    pub fn set_dir_state(tx: &Transaction, dir_id: i64, state: ScanState) -> Result<()> {
        tx.execute(
            "UPDATE dirs SET scan_state = ?2 WHERE id = ?1",
            params![dir_id, state as i64],
        )?;
        Ok(())
    }

    /// The files and the child directories of one directory, as stored.
    pub fn list_children(
        conn: &Connection,
        dir_id: i64,
    ) -> Result<(Vec<ChildFile>, Vec<ChildDir>)> {
        let mut stmt =
            conn.prepare_cached("SELECT id, name, size, mtime FROM files WHERE dir_id = ?1")?;
        let files = stmt
            .query_map([dir_id], |r| {
                Ok(ChildFile {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    size: r.get::<_, i64>(2)? as u64,
                    mtime: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = conn.prepare_cached("SELECT id, rel_path FROM dirs WHERE parent_id = ?1")?;
        let dirs = stmt
            .query_map([dir_id], |r| {
                let rel: String = r.get(1)?;
                Ok(ChildDir {
                    id: r.get(0)?,
                    name: rel.rsplit('/').next().unwrap_or_default().to_string(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((files, dirs))
    }
}

#[cfg(test)]
mod tests {
    use super::ingest::*;
    use super::*;
    use crate::types::FileKind;

    fn fts_ids(conn: &Connection, table: &str, term: &str) -> Vec<i64> {
        let sql = format!("SELECT rowid FROM {table} WHERE {table} MATCH ?1 ORDER BY rowid");
        let mut stmt = conn.prepare(&sql).unwrap();
        stmt.query_map([format!("\"{term}\"")], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<i64>>>()
            .unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// VFS-1 AC-10: the bundled SQLite supports contentless-delete FTS5 trigram tables
    /// (>= 3.43) and the schema is created once, at version 1, in WAL mode.
    #[test]
    fn schema_creates_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let conn = open_rw(&path).unwrap();
        let version: String = conn
            .query_row("SELECT sqlite_version()", [], |r| r.get(0))
            .unwrap();
        let parts: Vec<u32> = version.split('.').map(|p| p.parse().unwrap()).collect();
        assert!(
            (parts[0], parts[1]) >= (3, 43),
            "SQLite {version} lacks contentless_delete"
        );
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        drop(conn);
        let again = open_rw(&path).unwrap();
        assert_eq!(count(&again, "SELECT COUNT(*) FROM roots"), 0);
        let ro = open_ro(&path).unwrap();
        assert!(
            ro.execute("DELETE FROM roots", []).is_err(),
            "reader is query-only"
        );
    }

    /// VFS-1 AC-6: an index written by a newer build is refused, not overwritten.
    #[test]
    fn newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        open_rw(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(matches!(open_rw(&path), Err(Error::SchemaTooNew)));
        assert!(matches!(open_ro(&path), Err(Error::SchemaTooNew)));
    }

    /// VFS-1 AC-8: open_ro does not create a missing index.
    #[test]
    fn open_ro_requires_existing_index() {
        let dir = tempfile::tempdir().unwrap();
        assert!(open_ro(&dir.path().join("missing.db")).is_err());
    }

    /// VFS-1 AC-4, AC-8, AC-10: ingest round trip — roots, directories and files are stored
    /// with folded keys and FTS rows, listed back, updated, and a subtree delete removes
    /// exactly the subtree (siblings sharing a name prefix survive).
    #[test]
    fn ingest_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open_rw(&dir.path().join("index.db")).unwrap();
        let tx = conn.transaction().unwrap();
        let root = insert_root(&tx, "/", "/Users/ana/Archivo").unwrap();
        let root_dir: i64 = tx
            .query_row(
                "SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ''",
                [root],
                |r| r.get(0),
            )
            .unwrap();
        let gestoria = insert_dir(&tx, root, root_dir, "Gestoría").unwrap();
        let ano = insert_dir(&tx, root, gestoria, "Gestoría/Año 2019").unwrap();
        let sibling = insert_dir(&tx, root, root_dir, "Gestoría2").unwrap();
        let nfd = "Notari\u{0301}a Pe\u{0301}rez escritura 2019.pdf";
        let deep = insert_file(
            &tx,
            ano,
            &NewFile {
                name: nfd,
                size: 10,
                mtime: 100,
                lossy: false,
            },
        )
        .unwrap();
        let mid = insert_file(
            &tx,
            gestoria,
            &NewFile {
                name: "IMG_0001.JPG",
                size: 20,
                mtime: 200,
                lossy: true,
            },
        )
        .unwrap();
        let keep = insert_file(
            &tx,
            sibling,
            &NewFile {
                name: "notaria copia.pdf",
                size: 30,
                mtime: 300,
                lossy: false,
            },
        )
        .unwrap();
        tx.commit().unwrap();

        let (name, name_key, kind, flags): (String, String, i64, i64) = conn
            .query_row(
                "SELECT name, name_key, kind, flags FROM files WHERE id = ?1",
                [deep],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(name, nfd, "original name is stored untouched");
        assert_eq!(name_key, "notaria perez escritura 2019.pdf");
        assert_eq!(FileKind::from_code(kind), FileKind::Document);
        assert_eq!(flags, 0);
        let flags: i64 = conn
            .query_row("SELECT flags FROM files WHERE id = ?1", [mid], |r| r.get(0))
            .unwrap();
        assert_eq!(flags, 1);
        let root_key: String = conn
            .query_row("SELECT path_key FROM roots WHERE id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(root_key, "/users/ana/archivo");

        assert_eq!(fts_ids(&conn, "files_fts", "notaria"), vec![deep, keep]);
        assert_eq!(fts_ids(&conn, "dirs_fts", "ano 2019"), vec![ano]);
        assert_eq!(
            fts_ids(&conn, "dirs_fts", "gestoria"),
            vec![gestoria, ano, sibling]
        );

        let (files, dirs) = list_children(&conn, gestoria).unwrap();
        assert_eq!(
            files,
            vec![ChildFile {
                id: mid,
                name: "IMG_0001.JPG".into(),
                size: 20,
                mtime: 200
            }]
        );
        assert_eq!(
            dirs,
            vec![ChildDir {
                id: ano,
                name: "Año 2019".into()
            }]
        );

        let tx = conn.transaction().unwrap();
        update_file(&tx, mid, 21, 201).unwrap();
        set_dir_state(&tx, gestoria, ScanState::Clean).unwrap();
        tx.commit().unwrap();
        assert_eq!(list_children(&conn, gestoria).unwrap().0[0].size, 21);
        assert_eq!(
            count(
                &conn,
                &format!("SELECT scan_state FROM dirs WHERE id = {gestoria}")
            ),
            0
        );

        let tx = conn.transaction().unwrap();
        delete_file(&tx, keep).unwrap();
        tx.commit().unwrap();
        assert_eq!(fts_ids(&conn, "files_fts", "notaria"), vec![deep]);

        let tx = conn.transaction().unwrap();
        let tail = insert_file(
            &tx,
            sibling,
            &NewFile {
                name: "otro.txt",
                size: 1,
                mtime: 1,
                lossy: false,
            },
        )
        .unwrap();
        assert_eq!(delete_dir_subtree(&tx, gestoria).unwrap(), 2);
        tx.commit().unwrap();
        assert!(fts_ids(&conn, "files_fts", "notaria").is_empty());
        assert!(fts_ids(&conn, "files_fts", "img_").is_empty());
        assert_eq!(fts_ids(&conn, "dirs_fts", "gestoria"), vec![sibling]);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM dirs"), 2);
        assert_eq!(fts_ids(&conn, "files_fts", "otro"), vec![tail]);

        let tx = conn.transaction().unwrap();
        assert_eq!(delete_dir_subtree(&tx, root_dir).unwrap(), 1);
        tx.commit().unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM dirs"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM files"), 0);
        assert!(fts_ids(&conn, "files_fts", "otro").is_empty());
    }
}
