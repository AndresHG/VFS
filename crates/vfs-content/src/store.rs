//! The content store (VFS-2 D1): `content.db`, a SQLite file next to `index.db`, holding
//! what extraction produced. It is never the name index: deleting it switches content search
//! off and loses nothing that cannot be extracted again (R9).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::Result;

/// Version of the content store schema (`PRAGMA user_version`). A store with another
/// version is deleted and rebuilt: it holds nothing that cannot be re-extracted.
pub const STORE_VERSION: i64 = 2;

/// The columns the queue rebuild compares, so it reads a few bytes per document instead of
/// every row with its text (VFS-50 R2). Added to existing stores on open: an index changes
/// no row, so the version stays.
const DOCS_META: &str =
    "CREATE INDEX IF NOT EXISTS docs_meta ON docs(dir_id, name, size, mtime, state, reason, attempts);";

const SCHEMA: &str = r#"
-- One row per document the content indexer has processed. file_id = files.id in index.db;
-- dir_id, name, size and mtime are the index's values when the file was processed: when any
-- differs, the file is processed again (VFS-2 R3).
CREATE TABLE docs(
  file_id     INTEGER PRIMARY KEY,
  dir_id      INTEGER NOT NULL,
  name        TEXT NOT NULL,
  size        INTEGER NOT NULL,
  mtime       INTEGER NOT NULL,
  state       INTEGER NOT NULL,       -- ContentState: 2 indexed, 3 failed, 4 skipped
  reason      TEXT,                   -- SkipReason / FailReason for states 3 and 4
  bytes_read  INTEGER NOT NULL,
  extract_ms  INTEGER NOT NULL,
  truncated   INTEGER NOT NULL DEFAULT 0,
  attempts    INTEGER NOT NULL DEFAULT 1,  -- extractions of this version of the file
  title       TEXT,
  authors     TEXT,                   -- JSON array of the extractor's authors (VFS-2 R16); NULL when none
  created     TEXT,                   -- creation date as the extractor reports it; NULL when none
  text        BLOB                    -- deflate-compressed UTF-8 text, for snippets; NULL unless indexed
);
-- Addressable pieces of docs.text (VFS-2 R11): they tile the text, never cross a page,
-- and are what a passage cites. Offsets index the stored (extracted, normalized) text.
CREATE TABLE chunks(
  file_id     INTEGER NOT NULL,
  ordinal     INTEGER NOT NULL,
  start_byte  INTEGER NOT NULL,
  end_byte    INTEGER NOT NULL,
  start_char  INTEGER NOT NULL,
  end_char    INTEGER NOT NULL,
  page        INTEGER,               -- 1-based; NULL when the format has no pages
  PRIMARY KEY(file_id, ordinal)
) WITHOUT ROWID;
-- Files waiting for extraction, in on-disk order of their folders.
CREATE TABLE queue(
  file_id INTEGER PRIMARY KEY,
  dir_id  INTEGER NOT NULL,
  name    TEXT NOT NULL,
  size    INTEGER NOT NULL,
  mtime   INTEGER NOT NULL
);
CREATE INDEX queue_order ON queue(dir_id, name);
-- Keys from analyze::Analyzer, space-separated in text order; rowid = file_id.
CREATE VIRTUAL TABLE content_fts USING fts5(
  title, body, content='', contentless_delete=1,
  tokenize='unicode61 remove_diacritics 0');
-- Documents per key, for the frequency guard (VFS-2 R14).
CREATE VIRTUAL TABLE content_vocab USING fts5vocab(content_fts, row);
"#;

/// Path of the content store for an index: `content.db` next to `index.db`.
pub fn store_path(index_db: &Path) -> PathBuf {
    index_db.with_file_name("content.db")
}

/// Whether content search is switched on for this index (the store exists).
pub fn exists(index_db: &Path) -> bool {
    store_path(index_db).exists()
}

/// Deletes the content store and its WAL files (switches content search off).
pub fn delete(index_db: &Path) -> std::io::Result<()> {
    let p = store_path(index_db);
    for suffix in ["", "-wal", "-shm"] {
        let mut f = p.as_os_str().to_owned();
        f.push(suffix);
        remove_retrying(Path::new(&f))?;
    }
    Ok(())
}

/// Removes `path`, missing or not. On Windows a file another connection holds open (a search
/// in flight) cannot be removed, so it is tried again for about a second.
fn remove_retrying(path: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) if tries >= 20 => return Err(e),
            Err(_) => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
}

fn tune(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

/// Opens the store, creating it (or recreating it when its version differs or it is not a
/// readable database).
pub fn open_or_create(index_db: &Path) -> Result<Connection> {
    let path = store_path(index_db);
    let attempt = || -> Result<Connection> {
        let conn = Connection::open(&path)?;
        tune(&conn)?;
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let tables: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
        if v == 0 && tables > 0 {
            // Tables without a version: a store from before versioning (R19).
            return Err(crate::Error::StoreVersion(0));
        }
        if v == 0 {
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", STORE_VERSION)?;
        } else if v != STORE_VERSION {
            return Err(crate::Error::StoreVersion(v));
        }
        conn.execute_batch(DOCS_META)?;
        Ok(conn)
    };
    // Rebuild only a store that is unreadable or of another version: its text can be
    // re-extracted. A busy or locked store, or a passing I/O error, goes to the caller, so a
    // second indexer cannot wipe the first one's work.
    match attempt() {
        Err(e) if needs_rebuild(&e) => {
            delete(index_db)?;
            attempt()
        }
        other => other,
    }
}

fn needs_rebuild(e: &crate::Error) -> bool {
    use rusqlite::ErrorCode::{DatabaseCorrupt, NotADatabase};
    match e {
        crate::Error::StoreVersion(_) => true,
        crate::Error::Db(rusqlite::Error::SqliteFailure(f, _)) => {
            matches!(f.code, DatabaseCorrupt | NotADatabase)
        }
        _ => false,
    }
}

/// Opens `index.db` for the content indexer: read-write (it updates `files.content_state`),
/// never creating it, with the content store attached as `c`.
pub fn open_index_rw(index_db: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        index_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_secs(10))?;
    attach(&conn, index_db)?;
    Ok(conn)
}

/// Opens `index.db` read-only with the content store attached as `c` (for searching).
pub fn open_index_ro(index_db: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        index_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_secs(5))?;
    attach(&conn, index_db)?;
    Ok(conn)
}

fn attach(conn: &Connection, index_db: &Path) -> Result<()> {
    let p = store_path(index_db);
    conn.execute("ATTACH DATABASE ?1 AS c", [p.to_string_lossy()])?;
    Ok(())
}

/// Deflate-compresses text for the `docs.text` column.
pub fn compress(text: &str) -> Vec<u8> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(text.as_bytes()).expect("write to Vec");
    e.finish().expect("finish to Vec")
}

/// Inverse of [`compress`]; invalid UTF-8 is replaced, a broken blob yields what decoded.
pub fn decompress(blob: &[u8]) -> String {
    let mut out = Vec::new();
    let _ = flate2::read::DeflateDecoder::new(blob).read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-2 AC-2: stored text round-trips, accents included.
    #[test]
    fn compress_round_trips() {
        let t = "Señor Muñoz, la calefacción del año pasado. ".repeat(50);
        let c = compress(&t);
        assert!(c.len() < t.len() / 4);
        assert_eq!(decompress(&c), t);
    }

    /// VFS-2 AC-10: a garbage content store is replaced by a fresh one.
    #[test]
    fn rebuilds_unreadable_store() {
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("index.db");
        std::fs::write(store_path(&index), b"not a database at all, just bytes").unwrap();
        let conn = open_or_create(&index).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM docs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        drop(conn);
        delete(&index).unwrap();
        assert!(!exists(&index));
    }

    /// VFS-2 AC-10: only an unreadable or other-version store is rebuilt; a busy or locked
    /// one is reported, so a second indexer cannot delete the first one's work.
    #[test]
    fn rebuilds_only_unreadable_or_old_stores() {
        let sqlite = |code| {
            crate::Error::Db(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            ))
        };
        assert!(needs_rebuild(&crate::Error::StoreVersion(7)));
        assert!(needs_rebuild(&sqlite(rusqlite::ffi::SQLITE_NOTADB)));
        assert!(needs_rebuild(&sqlite(rusqlite::ffi::SQLITE_CORRUPT)));
        assert!(!needs_rebuild(&sqlite(rusqlite::ffi::SQLITE_BUSY)));
        assert!(!needs_rebuild(&sqlite(rusqlite::ffi::SQLITE_LOCKED)));
        assert!(!needs_rebuild(&sqlite(rusqlite::ffi::SQLITE_IOERR)));
    }

    /// VFS-50 AC-2 (R2): a version-2 store from before `docs_meta` gets the index on open,
    /// keeps its version and its rows, and is not rebuilt.
    #[test]
    fn old_store_gains_docs_meta_without_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("index.db");
        {
            let conn = open_or_create(&index).unwrap();
            conn.execute_batch(
                "DROP INDEX docs_meta;
                 INSERT INTO docs(file_id, dir_id, name, size, mtime, state, bytes_read, extract_ms)
                 VALUES (5, 1, 'carta.txt', 10, 20, 2, 10, 1);",
            )
            .unwrap();
        }
        let conn = open_or_create(&index).unwrap();
        let has: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'docs_meta'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM docs", [], |r| r.get(0))
            .unwrap();
        assert_eq!((has, version, rows), (1, STORE_VERSION, 1));
    }
}
