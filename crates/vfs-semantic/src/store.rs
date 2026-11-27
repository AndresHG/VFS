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
pub const VECTORS_VERSION: i64 = 2;

/// Vectors of a new set from which its mean is taken (the first batch is at least this big).
pub const MEAN_SAMPLE: usize = 64;

/// Hash of a chunk's text: the first 16 bytes of its BLAKE3 digest.
pub type TextHash = [u8; 16];

const SCHEMA: &str = r#"
CREATE TABLE sets(
  id        INTEGER PRIMARY KEY,
  provider  TEXT NOT NULL,
  model     TEXT NOT NULL,
  dim       INTEGER NOT NULL,
  complete  INTEGER NOT NULL DEFAULT 0,   -- has covered every chunk text of the archive once (D20)
  mean      BLOB,                         -- f32 × dim, from the set's first batch; bits are taken from v - mean
  UNIQUE(provider, model, dim)
);
CREATE TABLE vectors(
  set_id  INTEGER NOT NULL,
  hash    BLOB NOT NULL,     -- TextHash of the chunk text
  bits    BLOB NOT NULL,     -- sign bits of (v - set mean), dim / 8 bytes
  q8      BLOB NOT NULL,     -- int8 values, dim bytes
  scale   REAL NOT NULL,     -- value ≈ q8 × scale
  PRIMARY KEY(set_id, hash)
) WITHOUT ROWID;
-- Where each chunk text occurs (copies share a hash), rewritten by every embedding run, so a
-- search maps a vector to its chunks without reading the content store's text.
CREATE TABLE locations(
  file_id  INTEGER NOT NULL,
  ordinal  INTEGER NOT NULL,
  hash     BLOB NOT NULL,
  PRIMARY KEY(file_id, ordinal)
) WITHOUT ROWID;
"#;

/// Tables of earlier schema versions, dropped before the current schema is created: vectors
/// can always be embedded again.
const DROP_ALL: &str = "DROP TABLE IF EXISTS sets; DROP TABLE IF EXISTS vectors;
                        DROP TABLE IF EXISTS locations;";

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
        if v < VECTORS_VERSION {
            // Version 1 (29c part 1, never released) kept uncentred bits.
            conn.execute_batch(DROP_ALL)?;
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

    /// The mean the set's sign bits are taken against; `None` before its first batch.
    pub fn mean(&self, set: i64) -> Result<Option<Vec<f32>>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT mean FROM sets WHERE id = ?1", [set], |r| r.get(0))
            .optional()?
            .flatten();
        Ok(blob.map(|b| {
            b.as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect()
        }))
    }

    /// Stores one batch of vectors in one transaction (R7: a batch is stored whole or not at
    /// all, so a resumed run sends nothing twice). A set's first batch also fixes its mean:
    /// these models' vectors share a large common component, and sign bits taken around the
    /// mean keep the 1-bit scan as good as a float search (measured in 29c on the 29b corpus).
    pub fn insert(&mut self, set: i64, batch: &[(TextHash, Vec<f32>)]) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let mean = match self.mean(set)? {
            Some(m) => m,
            None => {
                let dim = batch[0].1.len();
                let mut m = vec![0f32; dim];
                for (_, v) in batch {
                    m.iter_mut()
                        .zip(v)
                        .for_each(|(a, x)| *a += x / batch.len() as f32);
                }
                let blob: Vec<u8> = m.iter().flat_map(|x| x.to_le_bytes()).collect();
                self.conn.execute(
                    "UPDATE sets SET mean = ?2 WHERE id = ?1",
                    params![set, blob],
                )?;
                m
            }
        };
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
                st.execute(params![
                    set,
                    &hash[..],
                    quant::centred_bits(v, &mean),
                    q,
                    scale as f64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Replaces where each chunk text occurs: `(hash, file_id, ordinal)` for every chunk.
    pub fn set_locations(&mut self, locations: &[(TextHash, i64, u32)]) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM locations", [])?;
        {
            let mut st = tx.prepare_cached(
                "INSERT OR REPLACE INTO locations(file_id, ordinal, hash) VALUES (?1, ?2, ?3)",
            )?;
            for (hash, file_id, ordinal) in locations {
                st.execute(params![file_id, ordinal, &hash[..]])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every chunk's text hash and place, `(hash, file_id, ordinal)`, sorted by hash: copies
    /// of a text sit together, and a search finds them by binary search.
    pub fn locations(&self) -> Result<Vec<(TextHash, i64, u32)>> {
        let mut st = self
            .conn
            .prepare("SELECT hash, file_id, ordinal FROM locations")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (h, file_id, ordinal) = row?;
            if let Ok(h) = TextHash::try_from(h.as_slice()) {
                out.push((h, file_id, ordinal));
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    /// The hashes and sign bits of a set, for the in-memory scan: the bits of row `i` are
    /// `bits[i * dim / 8 ..][.. dim / 8]`.
    pub fn bit_rows(&self, set: i64) -> Result<(Vec<TextHash>, Vec<u8>)> {
        let mut st = self
            .conn
            .prepare("SELECT hash, bits FROM vectors WHERE set_id = ?1 ORDER BY hash")?;
        let rows = st.query_map([set], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        let (mut hashes, mut bits) = (Vec::new(), Vec::new());
        for row in rows {
            let (h, b) = row?;
            if let Ok(h) = TextHash::try_from(h.as_slice()) {
                hashes.push(h);
                bits.extend_from_slice(&b);
            }
        }
        Ok((hashes, bits))
    }

    /// The int8 row and scale of one text, for the rescore.
    pub fn q8(&self, set: i64, hash: &TextHash) -> Result<Option<(Vec<i8>, f32)>> {
        let mut st = self
            .conn
            .prepare_cached("SELECT q8, scale FROM vectors WHERE set_id = ?1 AND hash = ?2")?;
        Ok(st
            .query_row(params![set, &hash[..]], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, f64>(1)?))
            })
            .optional()?
            .map(|(q, s)| (q.into_iter().map(|x| x as i8).collect(), s as f32)))
    }

    /// Deletes the vectors of texts that no chunk has any more (R6).
    pub fn delete(&mut self, set: i64, hashes: &[TextHash]) -> Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
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
        // Written only when it changes, so an unchanged archive writes nothing (VFS-50).
        self.conn.execute(
            "UPDATE sets SET complete = ?2 WHERE id = ?1 AND complete <> ?2",
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
