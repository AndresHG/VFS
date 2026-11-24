//! The content indexer (VFS-2 R1–R5): decides which documents need extraction, drives the
//! worker one file at a time, and stores text and keys in the content store.
//!
//! `content.db` is the source of truth for what was processed; `files.content_state` in
//! `index.db` mirrors it (VFS-2 D2) and is reconciled at the start of every run, so a crash
//! between the two commits heals itself.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use vfs_core::content::ContentState;
use vfs_core::db::ingest::FLAG_CLOUD_ONLY;
use vfs_core::FileKind;

use crate::analyze::Analyzer;
use crate::chunk::chunks;
use crate::cloud;
use crate::protocol::{FailReason, Outcome, Request, Response, SkipReason, SUPPORTED};
use crate::store;
use crate::supervisor::{Halt, StartError, Supervisor, WorkerConfig};
use crate::Result;

/// A file that timed out is tried once more on a later run before its failure is final: at
/// background priority on a busy disk a healthy file can be starved past the limit.
const TIMEOUT_ATTEMPTS: i64 = 2;

/// Worker crashes in a row (each on a different file) that end a run (VFS-2 R17).
const UNSTABLE_CRASHES: u32 = 5;

/// `index.db` `meta` key holding the input-size budget in MB (VFS-2 R21; written by
/// VFS-34's setting).
pub use vfs_core::content::CONTENT_MAX_MB_KEY;

/// File ids per `mirror` transaction (R20: short transactions on `index.db`).
const MIRROR_WINDOW: i64 = 5_000;

/// Limits and pacing of one indexing run.
#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub worker: WorkerConfig,
    /// Larger documents are skipped unread. `None` reads the budget from `index.db`
    /// (`content_max_mb`, else 100 MB; VFS-2 R21).
    pub max_input_bytes: Option<u64>,
    /// Extracted text kept per file (VFS-2 budget: 1 MB).
    pub max_text_bytes: usize,
    /// Files per content-store transaction.
    pub batch_size: usize,
    /// Stop after this many extractions (used to test resuming, VFS-2 AC-7).
    pub max_files: Option<u64>,
    /// Page limit of `content.db` for this run; tests use it to make the store "full"
    /// (VFS-2 AC-20). `None` in normal use.
    pub max_store_pages: Option<u32>,
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
                stop_grace: Duration::from_secs(5),
            },
            max_input_bytes: None,
            max_text_bytes: 1 << 20,
            batch_size: 64,
            max_files: None,
            max_store_pages: None,
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
    /// Outcome counts by kind, unread skips included (serialized as `indexed`,
    /// `skipped:password`, `failed:timeout`, ...).
    #[serde(serialize_with = "reasons_as_strings")]
    pub reasons: BTreeMap<OutcomeKind, u64>,
    /// Files still queued when the run stopped (0 when it finished).
    pub remaining: u64,
    /// Why the run ended early, when it could not go on (VFS-2 R17).
    pub stopped: Option<StopReason>,
    pub stop_detail: Option<String>,
}

/// What happened to one document in a run: the key of [`RunSummary::reasons`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OutcomeKind {
    Indexed,
    Skipped(SkipReason),
    Failed(FailReason),
}

impl std::fmt::Display for OutcomeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutcomeKind::Indexed => write!(f, "indexed"),
            OutcomeKind::Skipped(r) => write!(f, "skipped:{}", reason_str(r)),
            OutcomeKind::Failed(r) => write!(f, "failed:{}", reason_str(r)),
        }
    }
}

impl From<&Outcome> for OutcomeKind {
    fn from(o: &Outcome) -> OutcomeKind {
        match o {
            Outcome::Text { .. } => OutcomeKind::Indexed,
            Outcome::Skipped(r) => OutcomeKind::Skipped(*r),
            Outcome::Failed { reason, .. } => OutcomeKind::Failed(*reason),
        }
    }
}

fn reasons_as_strings<S: serde::Serializer>(
    m: &BTreeMap<OutcomeKind, u64>,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    s.collect_map(m.iter().map(|(k, n)| (k.to_string(), n)))
}

/// Why a run stopped before its queue was empty (VFS-2 R17). Files not yet processed keep
/// their state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The worker binary is missing, blocked or does not answer.
    WorkerCannotStart,
    /// The worker is from another build.
    VersionMismatch,
    /// Five crashes in a row on five different files.
    WorkerUnstable,
    /// `content.db` could not be written (disk full, I/O error).
    StoreWrite,
    /// `index.db` `content_state` could not be written (I/O error, disk full; a busy index
    /// is waited out). The next run reconciles it from `content.db`.
    IndexWrite,
}

impl From<&StartError> for StopReason {
    fn from(e: &StartError) -> StopReason {
        match e {
            StartError::CannotStart(_) => StopReason::WorkerCannotStart,
            StartError::VersionMismatch(_) => StopReason::VersionMismatch,
        }
    }
}

impl RunSummary {
    fn stop(&mut self, reason: StopReason, detail: String) {
        self.stopped = Some(reason);
        self.stop_detail = Some(detail);
    }

    /// Counts one file's outcome.
    fn count(&mut self, resp: &Response) {
        self.worker_ms += resp.ms;
        self.bytes_read += resp.bytes_read;
        let unread = resp.bytes_read == 0
            && matches!(
                resp.outcome,
                Outcome::Skipped(SkipReason::TooLarge | SkipReason::Cloud)
            );
        if !unread {
            self.files_opened += 1;
        }
        match &resp.outcome {
            Outcome::Text { .. } => self.indexed += 1,
            Outcome::Skipped(_) => self.skipped += 1,
            Outcome::Failed { .. } => self.failed += 1,
        }
        *self.reasons.entry((&resp.outcome).into()).or_default() += 1;
    }
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

/// The input-size budget: `meta` key `content_max_mb` of `index.db` (written by VFS-34's
/// setting), else 100 MB (VFS-2 R21); 0 is no limit (VFS-34 R6).
fn input_budget(conn: &Connection) -> Result<u64> {
    Ok(vfs_core::content::content_max_bytes(
        vfs_core::content::content_max_mb(conn)?,
    ))
}

/// Root folders of the index with their paths, and the ids of those whose folder cannot be
/// reached now (a disconnected disk or share, VFS-2 R17).
fn roots(conn: &Connection) -> Result<(HashMap<i64, PathBuf>, HashSet<i64>)> {
    let mut st = conn.prepare("SELECT id, path FROM roots")?;
    let all: HashMap<i64, PathBuf> = st
        .query_map([], |r| {
            Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?)))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let offline = all
        .iter()
        .filter(|(_, p)| std::fs::metadata(p).is_err())
        .map(|(id, _)| *id)
        .collect();
    Ok((all, offline))
}

/// SQL predicate on `files f`: the file's root folder is reachable.
/// SQL predicate on the `files` row named `table` (`f` or `files`): its root folder is
/// reachable. Applied to the row itself, so each statement looks up one folder per file and
/// never rebuilds a list of all files (#178: `mirror` runs it once per window).
fn root_ok_sql(offline: &HashSet<i64>, table: &str) -> String {
    if offline.is_empty() {
        return "1".into();
    }
    let ids: Vec<String> = offline.iter().map(i64::to_string).collect();
    format!(
        "(SELECT dd.root_id FROM dirs dd WHERE dd.id = {table}.dir_id) NOT IN ({})",
        ids.join(",")
    )
}

/// Deletes everything the store holds for `file_id` (VFS-2 R21).
fn forget(conn: &Connection, file_id: i64) -> Result<()> {
    conn.execute("DELETE FROM c.content_fts WHERE rowid = ?1", [file_id])?;
    conn.execute("DELETE FROM c.docs WHERE file_id = ?1", [file_id])?;
    conn.execute("DELETE FROM c.chunks WHERE file_id = ?1", [file_id])?;
    conn.execute("DELETE FROM c.queue WHERE file_id = ?1", [file_id])?;
    Ok(())
}

/// Rebuilds the queue in `content.db` from `files` and `c.docs`. Documents of offline or
/// paused roots (`offline`) are left out and keep their state. Returns the files queued and the counts of documents
/// skipped without reading, by reason.
fn enqueue(
    conn: &mut Connection,
    max_input: u64,
    offline: &HashSet<i64>,
) -> Result<(u64, Vec<(SkipReason, u64)>)> {
    let doc = FileKind::Document.code();
    let skipped = ContentState::Skipped as i64;
    // Deferred on purpose: it only reads `index.db` and writes `content.db`, which no other
    // connection writes, while `Immediate` would hold `index.db`'s write lock for the whole
    // rebuild (R20).
    let tx = conn.transaction()?;
    // Rows whose file left the index or changed identity, size or mtime are stale: drop
    // their text and keys so the file is processed again (R3, R21) and old text is not
    // found. So are skips that no longer hold: a `too_large` file now within the budget,
    // and a `cloud` file whose index flag is clear (the pre-open check decides again).
    let stale: Vec<i64> = {
        let mut st = tx.prepare(
            "SELECT d.file_id FROM c.docs d LEFT JOIN files f ON f.id = d.file_id
             WHERE f.id IS NULL OR f.dir_id <> d.dir_id OR f.name <> d.name
                OR f.size <> d.size OR f.mtime <> d.mtime
                OR (d.state = ?1 AND d.reason = ?2 AND f.size <= ?3)
                OR (d.state = ?1 AND d.reason = ?4 AND f.flags & ?5 = 0)",
        )?;
        let rows = st.query_map(
            params![
                skipped,
                reason_str(&SkipReason::TooLarge),
                max_input as i64,
                reason_str(&SkipReason::Cloud),
                FLAG_CLOUD_ONLY
            ],
            |r| r.get(0),
        )?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for id in &stale {
        forget(&tx, *id)?;
    }
    // Documents that are decided without being read: cloud-only (R12), unsupported, too
    // large.
    let supported = supported_sql();
    let root_ok = root_ok_sql(offline, "f");
    let mut unread = Vec::new();
    for (reason, condition) in [
        (SkipReason::Cloud, "f.flags & ?4 <> 0".to_string()),
        (
            SkipReason::Unsupported,
            format!("f.flags & ?4 = 0 AND NOT ({supported})"),
        ),
        (
            SkipReason::TooLarge,
            format!("f.flags & ?4 = 0 AND ({supported}) AND f.size > ?3"),
        ),
    ] {
        let n = tx.execute(
            &format!(
                "INSERT INTO c.docs(file_id, dir_id, name, size, mtime, state, reason, bytes_read, extract_ms)
                 SELECT f.id, f.dir_id, f.name, f.size, f.mtime, ?1, ?5, 0, 0
                 FROM files f LEFT JOIN c.docs d ON d.file_id = f.id
                 WHERE f.kind = ?2 AND d.file_id IS NULL AND {root_ok} AND {condition}"
            ),
            params![
                skipped,
                doc,
                max_input as i64,
                FLAG_CLOUD_ONLY,
                reason_str(&reason)
            ],
        )? as u64;
        if n > 0 {
            unread.push((reason, n));
        }
    }
    tx.execute("DELETE FROM c.queue", [])?;
    let queued = tx.execute(
        &format!(
            "INSERT INTO c.queue(file_id, dir_id, name, size, mtime)
             SELECT f.id, f.dir_id, f.name, f.size, f.mtime
             FROM files f LEFT JOIN c.docs d ON d.file_id = f.id
             WHERE f.kind = ?1 AND d.file_id IS NULL AND {root_ok} AND ({supported})
               AND f.size <= ?2 AND f.flags & ?3 = 0"
        ),
        params![doc, max_input as i64, FLAG_CLOUD_ONLY],
    )? as u64;
    // Timeouts get one more try (stale rows were deleted above, so these are unchanged).
    let retries = tx.execute(
        &format!(
            "INSERT INTO c.queue(file_id, dir_id, name, size, mtime)
             SELECT d.file_id, d.dir_id, d.name, d.size, d.mtime
             FROM c.docs d JOIN files f ON f.id = d.file_id
             WHERE d.state = ?1 AND d.reason = ?2 AND d.attempts < ?3 AND {root_ok}"
        ),
        params![
            ContentState::Failed as i64,
            reason_str(&FailReason::Timeout),
            TIMEOUT_ATTEMPTS
        ],
    )? as u64;
    tx.commit()?;
    Ok((queued + retries, unread))
}

/// Mirrors the store's states into `index.db` `files.content_state`, the only column the
/// content indexer writes there (VFS-2 R20). A separate commit: WAL does not make attached
/// commits atomic.
fn mirror(conn: &mut Connection, offline: &HashSet<i64>) -> Result<()> {
    let doc = FileKind::Document.code();
    let root_ok = root_ok_sql(offline, "files");
    let max_id: i64 = conn.query_row("SELECT coalesce(max(id), 0) FROM files", [], |r| r.get(0))?;
    // Windows of file ids, each its own short transaction, so a first run over hundreds of
    // thousands of documents never holds `index.db`'s write lock for long (R20).
    let mut lo = 0i64;
    while lo < max_id {
        let hi = lo + MIRROR_WINDOW;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE files SET content_state = (SELECT state FROM c.docs d WHERE d.file_id = files.id)
             WHERE id > ?1 AND id <= ?2 AND id IN (SELECT file_id FROM c.docs)
               AND content_state <> (SELECT state FROM c.docs d WHERE d.file_id = files.id)",
            params![lo, hi],
        )?;
        // After the processed states: a timeout waiting for its retry is in both and shows
        // as queued.
        tx.execute(
            "UPDATE files SET content_state = ?1
             WHERE id > ?2 AND id <= ?3 AND id IN (SELECT file_id FROM c.queue)
               AND content_state <> ?1",
            params![ContentState::Queued as i64, lo, hi],
        )?;
        // Documents the store has no word on go back to `none`, except on offline or paused
        // roots, whose documents keep their state (R17; VFS-27 R8).
        tx.execute(
            &format!(
                "UPDATE files SET content_state = ?1
                 WHERE id > ?3 AND id <= ?4 AND kind = ?2 AND content_state <> ?1
                   AND id NOT IN (SELECT file_id FROM c.docs)
                   AND id NOT IN (SELECT file_id FROM c.queue)
                   AND {root_ok}"
            ),
            params![ContentState::None as i64, doc, lo, hi],
        )?;
        tx.commit()?;
        lo = hi;
    }
    Ok(())
}

struct Done {
    file_id: i64,
    dir_id: i64,
    name: String,
    size: i64,
    mtime: i64,
    resp: Response,
}

/// Writes a batch's text, chunks and keys to `content.db` and drops it from the queue.
fn write_store(
    conn: &mut Connection,
    batch: &[Done],
    analyzer: &Analyzer,
    sum: &mut RunSummary,
) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut ins_doc = tx.prepare_cached(
            "INSERT OR REPLACE INTO c.docs(file_id, dir_id, name, size, mtime, state, reason,
                 bytes_read, extract_ms, truncated, title, text, authors, created, attempts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
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
            let mut meta = None;
            let (state, reason, truncated, title, blob) = match &d.resp.outcome {
                Outcome::Text {
                    title,
                    body,
                    truncated,
                    page_starts,
                    meta: m,
                } => {
                    meta = Some(m);
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
                blob,
                meta.filter(|m| !m.authors.is_empty())
                    .map(|m| serde_json::to_string(&m.authors).expect("authors serialize")),
                meta.and_then(|m| m.created.clone())
            ])?;
            del_q.execute([d.file_id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Mirrors a committed batch's states into `index.db` (R20).
fn write_states(conn: &mut Connection, batch: &[Done]) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
    Ok(())
}

/// Whether `e` is SQLite's busy or locked: another connection, such as the name indexer's
/// compaction, held `index.db`'s write lock past the busy timeout.
fn is_busy(e: &crate::Error) -> bool {
    matches!(e, crate::Error::Db(rusqlite::Error::SqliteFailure(f, _))
        if matches!(f.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
}

/// Runs `write` on `index.db` until it is done, `stop` is set, or it fails for a reason
/// other than a busy index: each try waits out the busy timeout, so a long write by another
/// connection is a wait, never a stop (VFS-2 R20, VFS-43 R2). After a stop the states are left to the
/// next run's [`mirror`].
fn write_index(stop: &AtomicBool, mut write: impl FnMut() -> Result<()>) -> Result<()> {
    loop {
        match write() {
            Err(e) if is_busy(&e) && !stop.load(Ordering::Relaxed) => {
                std::thread::sleep(GATE_POLL);
            }
            Err(e) if is_busy(&e) => return Ok(()),
            r => return r,
        }
    }
}

/// Commits a batch: the content store first, then the states in `index.db`. A failure
/// says which file could not be written (R17).
fn commit_batch(
    conn: &mut Connection,
    batch: &mut Vec<Done>,
    analyzer: &Analyzer,
    sum: &mut RunSummary,
    stop: &AtomicBool,
) -> std::result::Result<(), (StopReason, String)> {
    if batch.is_empty() {
        return Ok(());
    }
    write_store(conn, batch, analyzer, sum).map_err(|e| (StopReason::StoreWrite, e.to_string()))?;
    write_index(stop, || write_states(conn, batch))
        .map_err(|e| (StopReason::IndexWrite, e.to_string()))?;
    batch.clear();
    Ok(())
}

/// What the caller decides before each document (VFS-27 R7, R8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Read the document now.
    Read,
    /// Ask again after this long. The run commits what it has read first, so finished
    /// documents reach the store while it waits, and it watches the stop flag meanwhile.
    Wait(Duration),
    /// End the run here; the document and the rest stay queued.
    Stop,
}

/// How often a gate's wait looks at the stop flag.
const GATE_POLL: Duration = Duration::from_millis(250);

/// A gate's wait commits the batch first once the last commit is this old, so a long run of
/// short waits (a back-off, a name walk) never holds documents already read, while the short
/// gap between documents in use (VFS-27 R7) does not commit each one.
const COMMIT_BEFORE_WAIT: Duration = Duration::from_secs(2);

/// The caller's hooks into a run, called between documents only, so the worker's own read
/// never counts against the pace (VFS-27 R7). The app paces and counts progress through it;
/// [`run`] uses one that admits every document.
pub trait Gate {
    /// Before reading the document at `path`.
    fn before_document(&mut self, path: &Path) -> Admit;
    /// After a document was processed, with its outcome and the worker's time on it.
    fn after_document(&mut self, outcome: OutcomeKind, worker_ms: u64);
}

/// The gate of [`run`]: reads every document as soon as the last one is done.
pub struct NoGate;

impl Gate for NoGate {
    fn before_document(&mut self, _path: &Path) -> Admit {
        Admit::Read
    }
    fn after_document(&mut self, _outcome: OutcomeKind, _worker_ms: u64) {}
}

/// Runs the content indexer over `index_db` until the queue is empty, `stop` is set, or
/// `opts.max_files` extractions are done. Creates the content store if needed (this is what
/// switches content search on). A run that cannot go on (no worker, a store write failing,
/// an unstable worker) ends with `stopped` set and leaves the unprocessed files as they were
/// (VFS-2 R17).
pub fn run(index_db: &Path, opts: &IndexOptions, stop: &AtomicBool) -> Result<RunSummary> {
    run_with(index_db, opts, stop, &mut NoGate)
}

/// [`run`] with the caller's [`Gate`] consulted between documents.
pub fn run_with(
    index_db: &Path,
    opts: &IndexOptions,
    stop: &AtomicBool,
    gate: &mut dyn Gate,
) -> Result<RunSummary> {
    let started = Instant::now();
    drop(store::open_or_create(index_db)?);
    let mut conn = store::open_index_rw(index_db)?;
    if let Some(pages) = opts.max_store_pages {
        conn.pragma_update(Some("c"), "max_page_count", pages)?;
    }
    let max_input = match opts.max_input_bytes {
        Some(b) => b,
        None => input_budget(&conn)?,
    };
    let (root_paths, mut offline) = roots(&conn)?;
    // Paused folders are passed over like offline ones and keep their state (VFS-27 R8).
    offline.extend(vfs_core::db::paused_root_ids(&conn)?);
    let (queued, unread) = enqueue(&mut conn, max_input, &offline)?;
    let unread_total = unread.iter().map(|(_, n)| n).sum();
    let mut sum = RunSummary {
        queued,
        skipped_unread: unread_total,
        skipped: unread_total,
        ..Default::default()
    };
    for (reason, n) in unread {
        *sum.reasons.entry(OutcomeKind::Skipped(reason)).or_default() += n;
    }
    let mut sup = Supervisor::new(opts.worker.clone());
    // Start the worker before any state changes in index.db, so a run that cannot extract
    // changes nothing there.
    if queued > 0 {
        if let Err(e) = sup.start(Supervisor::line_cap(opts.max_text_bytes)) {
            sum.stop(StopReason::from(&e), e.to_string());
            return Ok(finish(sum, &conn, &sup, started));
        }
    }
    if let Err(e) = write_index(stop, || mirror(&mut conn, &offline)) {
        sum.stop(StopReason::IndexWrite, e.to_string());
        return Ok(finish(sum, &conn, &sup, started));
    }
    let analyzer = Analyzer::default();
    let mut batch: Vec<Done> = Vec::new();
    let mut last_commit = Instant::now();
    let mut processed = 0u64;
    let mut crashes = 0u32;
    // Keyset cursor over the queue: files passed over (offline root, gone from the index)
    // are not read again in this run.
    let mut after: (i64, String) = (i64::MIN, String::new());
    'outer: loop {
        let todo: Vec<(i64, i64, String, i64, i64, i64)> = {
            let mut st = conn.prepare_cached(
                "SELECT q.file_id, q.dir_id, q.name, q.size, q.mtime,
                        coalesce((SELECT root_id FROM dirs WHERE id = q.dir_id), -1)
                 FROM c.queue q WHERE (q.dir_id, q.name) > (?1, ?2)
                 ORDER BY q.dir_id, q.name LIMIT 256",
            )?;
            let rows = st.query_map(params![after.0, after.1], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        if todo.is_empty() {
            break;
        }
        for (file_id, dir_id, name, size, mtime, root_id) in todo {
            after = (dir_id, name.clone());
            if stop.load(Ordering::Relaxed) || opts.max_files.is_some_and(|m| processed >= m) {
                break 'outer;
            }
            if offline.contains(&root_id) {
                continue;
            }
            let path = match vfs_core::db::file_abs_path(&conn, file_id) {
                Ok(p) => p,
                Err(vfs_core::Error::NotFound(_)) => {
                    // Removed from the index since the run started (R21).
                    forget(&conn, file_id)?;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let resp = if cloud::is_cloud_only(&path) {
                // Checked just before the open, which would download the file (R12).
                Response {
                    outcome: Outcome::Skipped(SkipReason::Cloud),
                    bytes_read: 0,
                    ms: 0,
                }
            } else {
                // The gate paces reads only: a cloud placeholder, never opened, costs no slot.
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break 'outer;
                    }
                    match gate.before_document(&path) {
                        Admit::Read => break,
                        Admit::Stop => break 'outer,
                        Admit::Wait(d) => {
                            if !batch.is_empty() && last_commit.elapsed() + d >= COMMIT_BEFORE_WAIT
                            {
                                if let Err((reason, detail)) =
                                    commit_batch(&mut conn, &mut batch, &analyzer, &mut sum, stop)
                                {
                                    sum.stop(reason, detail);
                                    break 'outer;
                                }
                                last_commit = Instant::now();
                            }
                            let until = Instant::now() + d;
                            while Instant::now() < until && !stop.load(Ordering::Relaxed) {
                                std::thread::sleep(GATE_POLL.min(until - Instant::now()));
                            }
                        }
                    }
                }
                let req = Request {
                    path,
                    max_input_bytes: max_input,
                    max_text_bytes: opts.max_text_bytes,
                };
                match sup.extract(&req, stop) {
                    Ok(r) => r,
                    Err(Halt::Start(e)) => {
                        sum.stop(StopReason::from(&e), e.to_string());
                        break 'outer;
                    }
                    // Stopped by the caller mid-file: the file stays queued (VFS-27 R3).
                    Err(Halt::Stopped) => break 'outer,
                }
            };
            if matches!(
                resp.outcome,
                Outcome::Failed {
                    reason: FailReason::Open,
                    ..
                }
            ) && root_paths
                .get(&root_id)
                .is_some_and(|p| std::fs::metadata(p).is_err())
            {
                // The disk or share went away: not the document's fault. Leave it queued and
                // pass over the rest of that root (R17).
                offline.insert(root_id);
                continue;
            }
            processed += 1;
            crashes = match resp.outcome {
                Outcome::Failed {
                    reason: FailReason::Crash,
                    ..
                } => crashes + 1,
                _ => 0,
            };
            sum.count(&resp);
            gate.after_document((&resp.outcome).into(), resp.ms);
            batch.push(Done {
                file_id,
                dir_id,
                name,
                size,
                mtime,
                resp,
            });
            if crashes >= UNSTABLE_CRASHES {
                sum.stop(
                    StopReason::WorkerUnstable,
                    format!("{crashes} crashes in a row"),
                );
                break 'outer;
            }
            if batch.len() >= opts.batch_size || last_commit.elapsed() > Duration::from_secs(2) {
                if let Err((reason, detail)) =
                    commit_batch(&mut conn, &mut batch, &analyzer, &mut sum, stop)
                {
                    sum.stop(reason, detail);
                    break 'outer;
                }
                last_commit = Instant::now();
            }
        }
    }
    if let Err((reason, detail)) = commit_batch(&mut conn, &mut batch, &analyzer, &mut sum, stop) {
        if sum.stopped.is_none() {
            sum.stop(reason, detail);
        }
    }
    Ok(finish(sum, &conn, &sup, started))
}

fn finish(
    mut sum: RunSummary,
    conn: &Connection,
    sup: &Supervisor,
    started: Instant,
) -> RunSummary {
    sum.remaining = conn
        .query_row("SELECT count(*) FROM c.queue", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0) as u64;
    sum.worker_starts = sup.spawned;
    sum.elapsed_ms = started.elapsed().as_millis() as u64;
    sum
}

/// Roots whose documents a run passes over now: offline (the folder cannot be reached) or
/// paused (VFS-32), sorted. A change here means documents to read (VFS-27 R8, R9).
pub fn passed_over_roots(index_db: &Path) -> Result<Vec<i64>> {
    let conn = rusqlite::Connection::open_with_flags(
        index_db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let (_, mut offline) = roots(&conn)?;
    offline.extend(vfs_core::db::paused_root_ids(&conn)?);
    let mut ids: Vec<i64> = offline.into_iter().collect();
    ids.sort_unstable();
    Ok(ids)
}

/// Whether a document that timed out waits for its one more try (VFS-2 R3): only then does
/// a look with no document changed have anything to read.
pub fn has_timeout_retries(index_db: &Path) -> bool {
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        store::store_path(index_db),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return false;
    };
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM docs WHERE state = ?1 AND reason = ?2 AND attempts < ?3)",
        params![
            ContentState::Failed as i64,
            reason_str(&FailReason::Timeout),
            TIMEOUT_ATTEMPTS
        ],
        |r| r.get(0),
    )
    .unwrap_or(false)
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
    // Read-only and never repaired here: a damaged store is the user's call (VFS-27 R11).
    let conn = rusqlite::Connection::open_with_flags(
        store::store_path(index_db),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
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
