//! The content indexer (VFS-2 R1–R5): decides which documents need extraction, drives the
//! worker one file at a time, and stores text and keys in the content store.
//!
//! `content.db` is the source of truth for what was processed; `files.content_state` in
//! `index.db` mirrors it (VFS-2 D2) and is reconciled at the start of every run, so a crash
//! between the two commits heals itself.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use serde::Serialize;
use vfs_core::content::ContentState;
use vfs_core::FileKind;

use crate::analyze::Analyzer;
use crate::chunk::chunks;
use crate::extract::SUPPORTED;
use crate::protocol::{FailReason, Outcome, Request, Response, SkipReason};
use crate::store;
use crate::supervisor::{Supervisor, WorkerConfig};
use crate::Result;

/// A file that timed out is tried once more on a later run before its failure is final: at
/// background priority on a busy disk a healthy file can be starved past the limit.
const TIMEOUT_ATTEMPTS: i64 = 2;

/// Limits and pacing of one indexing run.
#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub worker: WorkerConfig,
    /// Larger documents are skipped unread (VFS-2 budget: 100 MB).
    pub max_input_bytes: u64,
    /// Extracted text kept per file (VFS-2 budget: 1 MB).
    pub max_text_bytes: usize,
    /// Files per content-store transaction.
    pub batch_size: usize,
    /// Stop after this many extractions (used to test resuming, VFS-2 AC-7).
    pub max_files: Option<u64>,
}

impl IndexOptions {
    /// The VFS-2 budget values with the given worker command.
    pub fn new(program: impl Into<std::path::PathBuf>, args: Vec<String>) -> IndexOptions {
        IndexOptions {
            worker: WorkerConfig {
                program: program.into(),
                args,
                timeout: Duration::from_secs(30),
                mem_cap: 512 << 20,
            },
            max_input_bytes: 100 << 20,
            max_text_bytes: 1 << 20,
            batch_size: 64,
            max_files: None,
        }
    }
}

/// What one run did. `files_opened` and `bytes_read` count only what the worker read.
#[derive(Debug, Default, Clone, Serialize)]
pub struct RunSummary {
    /// Documents queued for extraction at the start of the run.
    pub queued: u64,
    pub indexed: u64,
    pub failed: u64,
    pub skipped: u64,
    /// Skips decided without opening the file (unsupported extension, too large).
    pub skipped_unread: u64,
    pub files_opened: u64,
    pub bytes_read: u64,
    /// Bytes of text stored (before compression) and of compressed text.
    pub text_bytes: u64,
    pub compressed_bytes: u64,
    pub truncated: u64,
    /// Chunks stored for the documents indexed in this run.
    pub chunks: u64,
    /// Sum of the worker's own time per file.
    pub worker_ms: u64,
    pub worker_starts: u64,
    pub elapsed_ms: u64,
    /// Outcome counts by reason (`indexed`, `skipped:password`, `failed:timeout`, ...).
    pub reasons: BTreeMap<String, u64>,
    /// Files still queued when the run stopped (0 when it finished).
    pub remaining: u64,
}

/// The snake_case name of a skip or fail reason, as stored in `docs.reason`.
fn reason_str<T: Serialize>(r: &T) -> String {
    let v = serde_json::to_value(r).unwrap_or_default();
    v.as_str().unwrap_or("unknown").to_string()
}

/// SQL predicate: the lower-cased name of `files f` ends with a supported extension.
fn supported_sql() -> String {
    SUPPORTED
        .iter()
        .map(|e| format!("lower(f.name) LIKE '%.{e}'"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Rebuilds the queue from `files` and `c.docs` and reconciles `content_state`. Returns
/// (queued, skipped without reading).
fn enqueue(conn: &mut Connection, opts: &IndexOptions) -> Result<(u64, u64)> {
    let doc = FileKind::Document.code();
    let tx = conn.transaction()?;
    // Processed rows whose file vanished or changed identity, size or mtime are stale: drop
    // their text and keys so the file is processed again (R3) and old text is not found.
    let stale: Vec<i64> = {
        let mut st = tx.prepare(
            "SELECT d.file_id FROM c.docs d LEFT JOIN files f ON f.id = d.file_id
             WHERE f.id IS NULL OR f.dir_id <> d.dir_id OR f.name <> d.name
                OR f.size <> d.size OR f.mtime <> d.mtime",
        )?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    {
        let mut del_fts = tx.prepare("DELETE FROM c.content_fts WHERE rowid = ?1")?;
        let mut del_doc = tx.prepare("DELETE FROM c.docs WHERE file_id = ?1")?;
        let mut del_chunks = tx.prepare("DELETE FROM c.chunks WHERE file_id = ?1")?;
        for id in &stale {
            del_fts.execute([id])?;
            del_doc.execute([id])?;
            del_chunks.execute([id])?;
        }
    }
    // Documents that are decided without being read.
    let supported = supported_sql();
    let unread = tx.execute(
        &format!(
            "INSERT INTO c.docs(file_id, dir_id, name, size, mtime, state, reason, bytes_read, extract_ms)
             SELECT f.id, f.dir_id, f.name, f.size, f.mtime, ?1,
                    CASE WHEN NOT ({supported}) THEN ?4 ELSE ?5 END, 0, 0
             FROM files f LEFT JOIN c.docs d ON d.file_id = f.id
             WHERE f.kind = ?2 AND d.file_id IS NULL
               AND (NOT ({supported}) OR f.size > ?3)"
        ),
        params![
            ContentState::Skipped as i64,
            doc,
            opts.max_input_bytes as i64,
            reason_str(&SkipReason::Unsupported),
            reason_str(&SkipReason::TooLarge)
        ],
    )? as u64;
    tx.execute("DELETE FROM c.queue", [])?;
    let queued = tx.execute(
        &format!(
            "INSERT INTO c.queue(file_id, dir_id, name, size, mtime)
             SELECT f.id, f.dir_id, f.name, f.size, f.mtime
             FROM files f LEFT JOIN c.docs d ON d.file_id = f.id
             WHERE f.kind = ?1 AND d.file_id IS NULL AND ({supported}) AND f.size <= ?2"
        ),
        params![doc, opts.max_input_bytes as i64],
    )? as u64;
    // Timeouts get one more try (stale rows were deleted above, so these are unchanged).
    let retries = tx.execute(
        "INSERT INTO c.queue(file_id, dir_id, name, size, mtime)
         SELECT d.file_id, d.dir_id, d.name, d.size, d.mtime FROM c.docs d
         WHERE d.state = ?1 AND d.reason = ?2 AND d.attempts < ?3",
        params![
            ContentState::Failed as i64,
            reason_str(&FailReason::Timeout),
            TIMEOUT_ATTEMPTS
        ],
    )? as u64;
    let queued = queued + retries;
    tx.commit()?;
    // Mirror into index.db (separate commit: WAL does not make attached commits atomic).
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE files SET content_state = ?1
         WHERE id IN (SELECT file_id FROM c.queue) AND content_state <> ?1",
        [ContentState::Queued as i64],
    )?;
    tx.execute(
        "UPDATE files SET content_state = (SELECT state FROM c.docs d WHERE d.file_id = files.id)
         WHERE id IN (SELECT file_id FROM c.docs)
           AND content_state <> (SELECT state FROM c.docs d WHERE d.file_id = files.id)",
        [],
    )?;
    tx.execute(
        "UPDATE files SET content_state = ?1 WHERE kind = ?2 AND content_state <> ?1
           AND id NOT IN (SELECT file_id FROM c.docs) AND id NOT IN (SELECT file_id FROM c.queue)",
        params![ContentState::None as i64, doc],
    )?;
    tx.commit()?;
    Ok((queued, unread))
}

struct Done {
    file_id: i64,
    dir_id: i64,
    name: String,
    size: i64,
    mtime: i64,
    resp: Response,
}

fn commit_batch(
    conn: &mut Connection,
    batch: &mut Vec<Done>,
    analyzer: &Analyzer,
    sum: &mut RunSummary,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut ins_doc = tx.prepare_cached(
            "INSERT OR REPLACE INTO c.docs(file_id, dir_id, name, size, mtime, state, reason,
                 bytes_read, extract_ms, truncated, title, text, attempts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                 COALESCE((SELECT attempts FROM c.docs WHERE file_id = ?1), 0) + 1)",
        )?;
        let mut del_fts = tx.prepare_cached("DELETE FROM c.content_fts WHERE rowid = ?1")?;
        let mut del_chunks = tx.prepare_cached("DELETE FROM c.chunks WHERE file_id = ?1")?;
        let mut ins_chunk = tx.prepare_cached(
            "INSERT INTO c.chunks(file_id, ordinal, start_byte, end_byte, start_char, end_char, page)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        let mut ins_fts =
            tx.prepare_cached("INSERT INTO c.content_fts(rowid, title, body) VALUES (?1, ?2, ?3)")?;
        let mut del_q = tx.prepare_cached("DELETE FROM c.queue WHERE file_id = ?1")?;
        for d in batch.iter() {
            del_chunks.execute([d.file_id])?;
            let (state, reason, truncated, title, blob) = match &d.resp.outcome {
                Outcome::Text {
                    title,
                    body,
                    truncated,
                    page_starts,
                } => {
                    for c in chunks(body, page_starts) {
                        ins_chunk.execute(params![
                            d.file_id,
                            c.ordinal,
                            c.start_byte as i64,
                            c.end_byte as i64,
                            c.start_char as i64,
                            c.end_char as i64,
                            c.page
                        ])?;
                        sum.chunks += 1;
                    }
                    let blob = store::compress(body);
                    sum.text_bytes += body.len() as u64;
                    sum.compressed_bytes += blob.len() as u64;
                    sum.truncated += *truncated as u64;
                    let t = title
                        .as_deref()
                        .map(|t| analyzer.index_text(t))
                        .unwrap_or_default();
                    del_fts.execute([d.file_id])?;
                    ins_fts.execute(params![d.file_id, t, analyzer.index_text(body)])?;
                    (
                        ContentState::Indexed,
                        None,
                        *truncated,
                        title.clone(),
                        Some(blob),
                    )
                }
                Outcome::Skipped(r) => (
                    ContentState::Skipped,
                    Some(reason_str(r)),
                    false,
                    None,
                    None,
                ),
                Outcome::Failed { reason, .. } => (
                    ContentState::Failed,
                    Some(reason_str(reason)),
                    false,
                    None,
                    None,
                ),
            };
            ins_doc.execute(params![
                d.file_id,
                d.dir_id,
                d.name,
                d.size,
                d.mtime,
                state as i64,
                reason,
                d.resp.bytes_read as i64,
                d.resp.ms as i64,
                truncated,
                title,
                blob
            ])?;
            del_q.execute([d.file_id])?;
        }
    }
    tx.commit()?;
    let tx = conn.transaction()?;
    {
        let mut upd = tx.prepare_cached("UPDATE files SET content_state = ?2 WHERE id = ?1")?;
        for d in batch.iter() {
            let state = match d.resp.outcome {
                Outcome::Text { .. } => ContentState::Indexed,
                Outcome::Skipped(_) => ContentState::Skipped,
                Outcome::Failed { .. } => ContentState::Failed,
            };
            upd.execute(params![d.file_id, state as i64])?;
        }
    }
    tx.commit()?;
    batch.clear();
    Ok(())
}

/// Runs the content indexer over `index_db` until the queue is empty, `stop` is set, or
/// `opts.max_files` extractions are done. Creates the content store if needed (this is what
/// switches content search on).
pub fn run(index_db: &Path, opts: &IndexOptions, stop: &AtomicBool) -> Result<RunSummary> {
    let started = Instant::now();
    drop(store::open_or_create(index_db)?);
    let mut conn = store::open_index_rw(index_db)?;
    let (queued, unread) = enqueue(&mut conn, opts)?;
    let mut sum = RunSummary {
        queued,
        skipped_unread: unread,
        skipped: unread,
        ..Default::default()
    };
    if unread > 0 {
        *sum.reasons.entry("skipped:unread".into()).or_default() += unread;
    }
    let analyzer = Analyzer::default();
    let mut sup = Supervisor::new(opts.worker.clone());
    let mut batch: Vec<Done> = Vec::new();
    let mut last_commit = Instant::now();
    let mut processed = 0u64;
    'outer: loop {
        // Finished files leave the queue when their batch commits; commit before reading it.
        commit_batch(&mut conn, &mut batch, &analyzer, &mut sum)?;
        let todo: Vec<(i64, i64, String, i64, i64)> = {
            let mut st = conn.prepare_cached(
                "SELECT file_id, dir_id, name, size, mtime FROM c.queue
                 ORDER BY dir_id, name LIMIT 256",
            )?;
            let rows = st.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        if todo.is_empty() {
            break;
        }
        for (file_id, dir_id, name, size, mtime) in todo {
            if stop.load(Ordering::Relaxed) || opts.max_files.is_some_and(|m| processed >= m) {
                break 'outer;
            }
            let path = vfs_core::db::file_abs_path(&conn, file_id)?;
            let resp = sup.extract(&Request {
                path,
                max_input_bytes: opts.max_input_bytes,
                max_text_bytes: opts.max_text_bytes,
            });
            processed += 1;
            sum.worker_ms += resp.ms;
            sum.bytes_read += resp.bytes_read;
            if resp.bytes_read > 0
                || !matches!(resp.outcome, Outcome::Skipped(SkipReason::TooLarge))
            {
                sum.files_opened += 1;
            }
            let key = match &resp.outcome {
                Outcome::Text { .. } => {
                    sum.indexed += 1;
                    "indexed".to_string()
                }
                Outcome::Skipped(r) => {
                    sum.skipped += 1;
                    format!("skipped:{}", reason_str(r))
                }
                Outcome::Failed { reason, .. } => {
                    sum.failed += 1;
                    format!("failed:{}", reason_str(reason))
                }
            };
            *sum.reasons.entry(key).or_default() += 1;
            batch.push(Done {
                file_id,
                dir_id,
                name,
                size,
                mtime,
                resp,
            });
            if batch.len() >= opts.batch_size || last_commit.elapsed() > Duration::from_secs(2) {
                commit_batch(&mut conn, &mut batch, &analyzer, &mut sum)?;
                last_commit = Instant::now();
            }
        }
    }
    commit_batch(&mut conn, &mut batch, &analyzer, &mut sum)?;
    sum.remaining =
        conn.query_row("SELECT count(*) FROM c.queue", [], |r| r.get::<_, i64>(0))? as u64;
    sum.worker_starts = sup.spawned;
    sum.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(sum)
}

/// Counts of documents by content state, from the content store and the queue.
#[derive(Debug, Default, Clone, Serialize)]
pub struct StoreStats {
    pub indexed: u64,
    pub failed: u64,
    pub skipped: u64,
    pub queued: u64,
    pub store_bytes: u64,
}

pub fn stats(index_db: &Path) -> Result<StoreStats> {
    if !store::exists(index_db) {
        return Err(crate::Error::Off);
    }
    let conn = store::open_or_create(index_db)?;
    let count = |state: ContentState| -> rusqlite::Result<u64> {
        conn.query_row(
            "SELECT count(*) FROM docs WHERE state = ?1",
            [state as i64],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n as u64)
    };
    let mut store_bytes = 0;
    let p = store::store_path(index_db);
    for suffix in ["", "-wal"] {
        let mut f = p.as_os_str().to_owned();
        f.push(suffix);
        store_bytes += std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
    }
    Ok(StoreStats {
        indexed: count(ContentState::Indexed)?,
        failed: count(ContentState::Failed)?,
        skipped: count(ContentState::Skipped)?,
        queued: conn.query_row("SELECT count(*) FROM queue", [], |r| r.get::<_, i64>(0))? as u64,
        store_bytes,
    })
}
