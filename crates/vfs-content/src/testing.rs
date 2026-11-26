//! Writing the content store by hand, for other crates' tests (feature `test-support`): a
//! document with its chunks as the indexer stores them, the queue, and deleting the store.

use std::path::Path;

use crate::{chunk, store, Result};

/// Stores `text` as document `file_id` (named `doc<id>.txt`), with its chunks, replacing any
/// earlier version.
pub fn write_doc(index_db: &Path, file_id: i64, text: &str) -> Result<()> {
    let conn = store::open_or_create(index_db)?;
    conn.execute(
        "INSERT OR REPLACE INTO docs(file_id, dir_id, name, size, mtime, state, bytes_read,
            extract_ms, text) VALUES (?1, 1, ?2, 1, 1, 2, 1, 1, ?3)",
        rusqlite::params![file_id, format!("doc{file_id}.txt"), store::compress(text)],
    )?;
    conn.execute("DELETE FROM chunks WHERE file_id = ?1", [file_id])?;
    for c in chunk::chunks(text, &[]) {
        conn.execute(
            "INSERT INTO chunks VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            rusqlite::params![
                file_id,
                c.ordinal,
                c.start_byte as i64,
                c.end_byte as i64,
                c.start_char as i64,
                c.end_char as i64
            ],
        )?;
    }
    Ok(())
}

/// Removes document `file_id` and its chunks.
pub fn remove_doc(index_db: &Path, file_id: i64) -> Result<()> {
    let conn = store::open_or_create(index_db)?;
    conn.execute("DELETE FROM docs WHERE file_id = ?1", [file_id])?;
    conn.execute("DELETE FROM chunks WHERE file_id = ?1", [file_id])?;
    Ok(())
}

/// Puts document `file_id` in the queue (`queued`) or empties the queue.
pub fn set_queued(index_db: &Path, file_id: i64, queued: bool) -> Result<()> {
    let conn = store::open_or_create(index_db)?;
    if queued {
        conn.execute(
            "INSERT OR REPLACE INTO queue VALUES (?1, 1, ?2, 1, 1)",
            rusqlite::params![file_id, format!("doc{file_id}.txt")],
        )?;
    } else {
        conn.execute("DELETE FROM queue", [])?;
    }
    Ok(())
}

/// Deletes the content store, as switching content search off or a rebuild does.
pub fn delete_store(index_db: &Path) -> std::io::Result<()> {
    store::delete(index_db)
}
