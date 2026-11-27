//! Read access to the stored chunks for other crates (VFS-29 R5): the embedder walks every
//! document's chunks with their text, through this reader, without opening the content store
//! itself (VFS-2 AC-16).

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::store::{decompress, store_path};
use crate::Result;

/// A read-only view of the documents and chunks of the content store.
pub struct ChunkReader {
    conn: Connection,
}

impl ChunkReader {
    /// `Err(Error::Off)` when content search is off for this index.
    pub fn open(index_db: &Path) -> Result<ChunkReader> {
        let path = store_path(index_db);
        if !path.exists() {
            return Err(crate::Error::Off);
        }
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(ChunkReader { conn })
    }

    /// Ids of the documents whose text is stored, ascending.
    pub fn doc_ids(&self) -> Result<Vec<i64>> {
        let mut st = self
            .conn
            .prepare("SELECT file_id FROM docs WHERE text IS NOT NULL ORDER BY file_id")?;
        let ids = st
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// The chunks of one document with their text, `(ordinal, text)` in order (VFS-2 R11);
    /// empty when the document has no stored text.
    pub fn chunk_texts(&self, file_id: i64) -> Result<Vec<(u32, String)>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT text FROM docs WHERE file_id = ?1", [file_id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        let Some(blob) = blob else {
            return Ok(Vec::new());
        };
        let text = decompress(&blob);
        let mut st = self.conn.prepare_cached(
            "SELECT ordinal, start_byte, end_byte FROM chunks WHERE file_id = ?1 ORDER BY ordinal",
        )?;
        let rows = st.query_map([file_id], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (ordinal, sb, eb) = row?;
            if let Some(piece) = text.get(sb.max(0) as usize..eb.max(0) as usize) {
                out.push((ordinal, piece.to_string()));
            }
        }
        Ok(out)
    }

    /// Stored chunks, for estimates (VFS-29 R1a: the time to embed this archive).
    pub fn chunk_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM chunks", [], |r| r.get(0))?;
        Ok(n.max(0) as u64)
    }

    /// Whether no document waits in the content queue: every document the indexer found has
    /// been read, skipped or failed, so the chunks are the archive's whole text (VFS-29 R6).
    pub fn queue_empty(&self) -> Result<bool> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM queue", [], |r| r.get(0))?;
        Ok(n == 0)
    }
}
