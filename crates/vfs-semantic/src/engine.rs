//! The embedding run (VFS-29 R5–R7, R10, D10): embeds every distinct chunk text of
//! `content.db` that the current vector set lacks, in batches stored one transaction each, so
//! a run stopped at any point resumes where it was and sends no text twice. Once the content
//! indexer has nothing queued, texts no chunk has any more are dropped, the set is complete and
//! it replaces any older one.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde::Serialize;

use crate::store::{text_hash, Store, TextHash};
use crate::{ModelId, Result};

/// Why an embedder returned no vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedError {
    /// The stop flag was raised; the batch in flight was abandoned.
    Stopped,
    /// The worker failed, hung or could not start; the message is for the log.
    Failed(String),
}

/// Something that turns texts into unit-length vectors: the local worker, or a fake in tests.
pub trait Embedder {
    fn model(&self) -> ModelId;
    /// One vector per text, in order. `query` marks a search query (VFS-29 R11).
    fn embed(
        &mut self,
        texts: &[String],
        query: bool,
        stop: &AtomicBool,
    ) -> std::result::Result<Vec<Vec<f32>>, EmbedError>;
}

/// How a run works.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Texts per batch, stored in one transaction.
    pub batch: usize,
}

impl Default for RunOptions {
    fn default() -> RunOptions {
        RunOptions { batch: 16 }
    }
}

/// Where a run is, for the progress line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// Texts embedded so far in this run.
    pub done: u64,
    /// Texts this run had to embed when it started.
    pub total: u64,
}

/// What a run did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    /// Distinct chunk texts in the archive.
    pub texts: u64,
    /// Texts without a vector when the run started.
    pub pending: u64,
    pub embedded: u64,
    /// Vectors dropped because no chunk has their text any more.
    pub dropped: u64,
    /// Every chunk text of the whole archive (nothing queued) has a vector in the set.
    pub complete: bool,
    pub stopped: bool,
    /// The worker's failure, when the run ended on one.
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

/// Embeds what the vector set of `embedder`'s model lacks (see the module docs). `progress`
/// is called after every stored batch; it may wait (the pace, VFS-27 R7) and returns whether
/// to go on.
pub fn run(
    index_db: &Path,
    embedder: &mut dyn Embedder,
    opts: &RunOptions,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<RunSummary> {
    let started = Instant::now();
    let content = vfs_content::ChunkReader::open(index_db)?;
    let mut store = Store::open(index_db)?;
    let set = store.set_id(&embedder.model())?;
    let stored = store.hashes(set)?;

    // Pass 1: hash every chunk; note the first chunk of each text the set lacks.
    let mut seen: HashSet<TextHash> = HashSet::new();
    let mut pending: Vec<(i64, u32)> = Vec::new();
    let mut locations: Vec<(TextHash, i64, u32)> = Vec::new();
    for file_id in content.doc_ids()? {
        for (ordinal, text) in content.chunk_texts(file_id)? {
            let h = text_hash(&text);
            locations.push((h, file_id, ordinal));
            if seen.insert(h) && !stored.contains(&h) {
                pending.push((file_id, ordinal));
            }
        }
    }
    // Rewritten only when a chunk moved, appeared or went: an unchanged archive writes
    // nothing (VFS-50).
    locations.sort_unstable();
    if store.locations()? != locations {
        store.set_locations(&locations)?;
    }
    drop(locations);
    // A new set's first batch fixes its mean (store::MEAN_SAMPLE).
    let mut batch_size = if store.mean(set)?.is_none() {
        opts.batch.max(crate::store::MEAN_SAMPLE)
    } else {
        opts.batch.max(1)
    };
    let mut sum = RunSummary {
        texts: seen.len() as u64,
        pending: pending.len() as u64,
        ..RunSummary::default()
    };
    // A set that once covered the archive keeps serving searches while new texts are
    // embedded (D20): until then those documents are found by their words alone.

    // Pass 2: embed the pending texts, one document's text read at a time.
    let mut done_now: HashSet<TextHash> = HashSet::new();
    let mut batch: Vec<(TextHash, String)> = Vec::new();
    let mut flush = |batch: &mut Vec<(TextHash, String)>,
                     store: &mut Store,
                     sum: &mut RunSummary|
     -> Result<bool> {
        if batch.is_empty() {
            return Ok(true);
        }
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        match embedder.embed(&texts, false, stop) {
            Ok(vectors) if vectors.len() == texts.len() => {
                let rows: Vec<(TextHash, Vec<f32>)> =
                    batch.drain(..).map(|(h, _)| h).zip(vectors).collect();
                store.insert(set, &rows)?;
                sum.embedded += rows.len() as u64;
                let go_on = progress(Progress {
                    done: sum.embedded,
                    total: sum.pending,
                });
                if !go_on {
                    sum.stopped = true;
                }
                Ok(go_on)
            }
            Ok(_) => {
                sum.error = Some("the worker returned a wrong number of vectors".into());
                Ok(false)
            }
            Err(EmbedError::Stopped) => {
                sum.stopped = true;
                Ok(false)
            }
            Err(EmbedError::Failed(e)) => {
                sum.error = Some(e);
                Ok(false)
            }
        }
    };
    let mut going = true;
    let mut i = 0;
    while going && i < pending.len() {
        let file_id = pending[i].0;
        let mut wanted: HashSet<u32> = HashSet::new();
        while i < pending.len() && pending[i].0 == file_id {
            wanted.insert(pending[i].1);
            i += 1;
        }
        for (ordinal, text) in content.chunk_texts(file_id)? {
            if !wanted.contains(&ordinal) {
                continue;
            }
            // The document may have changed since pass 1: key on the text as it is now.
            let h = text_hash(&text);
            if stored.contains(&h) || !done_now.insert(h) {
                continue;
            }
            batch.push((h, text));
            if batch.len() >= batch_size {
                batch_size = opts.batch.max(1);
                if stop.load(Ordering::Relaxed) {
                    sum.stopped = true;
                    going = false;
                    break;
                }
                going = flush(&mut batch, &mut store, &mut sum)?;
                if !going {
                    break;
                }
            }
        }
    }
    if going && !stop.load(Ordering::Relaxed) {
        going = flush(&mut batch, &mut store, &mut sum)?;
    } else if going {
        sum.stopped = true;
        going = false;
    }

    // The set covers the archive only when no document waits to be read and the store holds
    // text: while the store is rebuilt or still filling, missing texts are just not there yet.
    // Only then are vectors of vanished texts dropped and an older set replaced (R6, R10).
    if going && content.queue_empty()? && !seen.is_empty() {
        let gone: Vec<TextHash> = stored.difference(&seen).copied().collect();
        store.delete(set, &gone)?;
        sum.dropped = gone.len() as u64;
        sum.complete = true;
        store.set_complete(set, true)?;
        store.drop_other_sets(set)?;
    }
    sum.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(sum)
}
