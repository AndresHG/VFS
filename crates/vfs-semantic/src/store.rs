//! `vectors.db` (VFS-29 R6, D8, D10): the vectors of the content chunks, beside `content.db`
//! and never deleted with it. One vector set per (provider, model, dimension); in each, one row
//! per distinct chunk text, keyed by the hash of that text, holding a 1-bit row for the scan
//! and an int8 row with its scale for the rescore.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::{quant, ModelId, Result};

/// Version of the schema (`PRAGMA user_version`).
pub const VECTORS_VERSION: i64 = 1;

/// Hash of a chunk's text: the first 16 bytes of its BLAKE3 digest.
pub type TextHash = [u8; 16];

const SCHEMA: &str = r#"
CREATE TABLE sets(
  id        INTEGER PRIMARY KEY,
  provider  TEXT NOT NULL,
  model     TEXT NOT NULL,
  dim       INTEGER NOT NULL,
  complete  INTEGER NOT NULL DEFAULT 0,   -- every chunk text of the archive has a vector
  UNIQUE(provider, model, dim)
);
CREATE TABLE vectors(
  set_id  INTEGER NOT NULL,
  hash    BLOB NOT NULL,     -- TextHash of the chunk text
  bits    BLOB NOT NULL,     -- sign bits, dim / 8 bytes
  q8      BLOB NOT NULL,     -- int8 values, dim bytes
  scale   REAL NOT NULL,     -- value ≈ q8 × scale
  PRIMARY KEY(set_id, hash)
) WITHOUT ROWID;
"#;

/// Hash of one chunk text (D10).
pub fn text_hash(text: &str) -> TextHash {
    let d = blake3::hash(text.as_bytes());
    d.as_bytes()[..16].try_into().expect("16 bytes")
}

/// Path of the vector store for an index: `vectors.db` next to `index.db`.
pub fn store_path(index_db: &Path) -> PathBuf {
    index_db.with_file_name("vectors.db")
}

/// Whether the vector store exists (it is created only when meaning search is turned on).
pub fn exists(index_db: &Path) -> bool {
    store_path(index_db).exists()
}

/// One stored vector, as the search loads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub hash: TextHash,
    pub bits: Vec<u8>,
    pub q8: Vec<i8>,
    pub scale: f32,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens the store, creating it when missing.
    pub fn open(index_db: &Path) -> Result<Store> {
        let conn = Connection::open(store_path(index_db))?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if v == 0 {
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", VECTORS_VERSION)?;
        } else if v != VECTORS_VERSION {
            return Err(crate::Error::StoreVersion(v));
        }
        Ok(Store { conn })
    }

    /// Opens an existing store read-only (for searching); `None` when there is none.
    pub fn open_ro(index_db: &Path) -> Result<Option<Store>> {
        let path = store_path(index_db);
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Some(Store { conn }))
    }

    /// The id of `model`'s vector set, created empty when missing.
    pub fn set_id(&self, model: &ModelId) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO sets(provider, model, dim) VALUES (?1, ?2, ?3)",
            params![model.provider, model.name, model.dim as i64],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM sets WHERE provider = ?1 AND model = ?2 AND dim = ?3",
            params![model.provider, model.name, model.dim as i64],
            |r| r.get(0),
        )?)
    }

    /// The set searches use (R10): the newest complete one, with its model's dimension.
    pub fn search_set(&self) -> Result<Option<(i64, usize)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, dim FROM sets WHERE complete = 1 ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)),
            )
            .optional()?)
    }

    /// Hashes stored in a set.
    pub fn hashes(&self, set: i64) -> Result<HashSet<TextHash>> {
        let mut st = self
            .conn
            .prepare("SELECT hash FROM vectors WHERE set_id = ?1")?;
        let rows = st.query_map([set], |r| r.get::<_, Vec<u8>>(0))?;
        let mut out = HashSet::new();
        for h in rows {
            if let Ok(h) = TextHash::try_from(h?.as_slice()) {
                out.insert(h);
            }
        }
        Ok(out)
    }

    /// Stores one batch of vectors in one transaction (R7: a batch is stored whole or not at
    /// all, so a resumed run sends nothing twice).
    pub fn insert(&mut self, set: i64, batch: &[(TextHash, Vec<f32>)]) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut st = tx.prepare_cached(
                "INSERT OR REPLACE INTO vectors(set_id, hash, bits, q8, scale)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (hash, v) in batch {
                let (q, scale) = quant::int8(v);
                let q: Vec<u8> = q.into_iter().map(|x| x as u8).collect();
                st.execute(params![set, &hash[..], quant::bits(v), q, scale as f64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes the vectors of texts that no chunk has any more (R6).
    pub fn delete(&mut self, set: i64, hashes: &[TextHash]) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut st =
                tx.prepare_cached("DELETE FROM vectors WHERE set_id = ?1 AND hash = ?2")?;
            for h in hashes {
                st.execute(params![set, &h[..]])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Marks whether a set holds a vector for every chunk text.
    pub fn set_complete(&self, set: i64, complete: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE sets SET complete = ?2 WHERE id = ?1",
            params![set, complete as i64],
        )?;
        Ok(())
    }

    /// Drops every set but `keep` and their vectors (R10: once the new set is complete).
    pub fn drop_other_sets(&mut self, keep: i64) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM vectors WHERE set_id <> ?1", [keep])?;
        tx.execute("DELETE FROM sets WHERE id <> ?1", [keep])?;
        tx.commit()?;
        Ok(())
    }

    /// Every vector of a set, in hash order.
    pub fn rows(&self, set: i64) -> Result<Vec<Row>> {
        let mut st = self
            .conn
            .prepare("SELECT hash, bits, q8, scale FROM vectors WHERE set_id = ?1 ORDER BY hash")?;
        let rows = st.query_map([set], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Vec<u8>>(2)?,
                r.get::<_, f64>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (hash, bits, q8, scale) = row?;
            if let Ok(hash) = TextHash::try_from(hash.as_slice()) {
                out.push(Row {
                    hash,
                    bits,
                    q8: q8.into_iter().map(|x| x as i8).collect(),
                    scale: scale as f32,
                });
            }
        }
        Ok(out)
    }

    /// Number of vectors in a set.
    pub fn count(&self, set: i64) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM vectors WHERE set_id = ?1",
            [set],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as u64)
    }
}
