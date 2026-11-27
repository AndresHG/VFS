//! «Relacionados» (VFS-53): document vectors built from the chunk vectors search by meaning
//! already holds, and the documents closest to one document. Document vectors are updated
//! after each completed embedding run, for changed documents only (R1, R2); related
//! documents are computed on each request and nothing about them is kept (R5).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::quant;
use crate::store::{Store, TextHash};
use crate::Result;

/// Lowest cosine of two centred document vectors shown as related (D2, measured on the 29b
/// corpus: it lets 0.47 % of random document pairs through).
pub const MIN_SIMILARITY: f32 = 0.35;
/// Related documents shown per document (R5).
pub const MAX_RELATED: usize = 5;
/// Candidates of the 1-bit scan rescored with their int8 rows.
const CANDIDATES: usize = 50;
/// Documents computed per transaction when document vectors are updated: their chunk rows
/// (about 9 per document, 1.5 KB each as floats) stay in memory only for one batch.
const DOC_BATCH: usize = 512;

/// One related document: the closest of its chunks to the asked document is its passage (R6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Related {
    pub file_id: i64,
    pub ordinal: u32,
    /// Cosine of the centred document vectors.
    pub score: f32,
}

/// What an update of document vectors did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DocsUpdate {
    pub written: u64,
    pub dropped: u64,
}

/// The signature of a document's list of chunk texts (R2).
fn signature(hashes: &[TextHash]) -> TextHash {
    let mut h = blake3::Hasher::new();
    for x in hashes {
        h.update(x);
    }
    h.finalize().as_bytes()[..16].try_into().expect("16 bytes")
}

/// A chunk's int8 row as floats.
fn dequantise(q8: &[i8], scale: f32) -> Vec<f32> {
    q8.iter().map(|&x| x as f32 * scale).collect()
}

/// `v - mean`, scaled to unit length.
fn centred_unit(mut v: Vec<f32>, mean: &[f32]) -> Vec<f32> {
    v.iter_mut().zip(mean).for_each(|(x, m)| *x -= m);
    quant::normalise(&mut v);
    v
}

/// Brings the document vectors of `set` up to date with its chunk vectors and the stored
/// locations (R1, R2): documents whose chunk texts changed are computed again, documents gone
/// are dropped, and nothing is written when nothing changed. Stops between batches when `stop`
/// is raised; what is stored stays valid and the rest waits for the next run.
pub fn update_docs(store: &mut Store, set: i64, stop: &AtomicBool) -> Result<DocsUpdate> {
    let Some(mean) = store.mean(set)? else {
        return Ok(DocsUpdate::default());
    };
    let mut by_doc: Vec<(i64, u32, TextHash)> = store
        .locations()?
        .into_iter()
        .map(|(h, f, o)| (f, o, h))
        .collect();
    by_doc.sort_unstable_by_key(|&(f, o, _)| (f, o));
    let stored = store.doc_sigs(set)?;
    let mut present: HashSet<i64> = HashSet::new();
    let mut changed: Vec<(i64, TextHash, Vec<TextHash>)> = Vec::new();
    for group in by_doc.chunk_by(|a, b| a.0 == b.0) {
        let file_id = group[0].0;
        present.insert(file_id);
        let hashes: Vec<TextHash> = group.iter().map(|x| x.2).collect();
        let sig = signature(&hashes);
        if stored.get(&file_id) != Some(&sig) {
            changed.push((file_id, sig, hashes));
        }
    }
    drop(by_doc);
    let gone: Vec<i64> = stored
        .keys()
        .filter(|id| !present.contains(id))
        .copied()
        .collect();
    let mut out = DocsUpdate::default();
    for batch in changed.chunks(DOC_BATCH) {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        // Each distinct text read once, in hash order (near-sequential in the WITHOUT ROWID
        // table, which matters on a spinning disk).
        let wanted: BTreeSet<TextHash> = batch.iter().flat_map(|d| d.2.iter().copied()).collect();
        let mut rows: HashMap<TextHash, Vec<f32>> = HashMap::with_capacity(wanted.len());
        for h in wanted {
            if let Some((q8, scale)) = store.q8(set, &h)? {
                rows.insert(h, dequantise(&q8, scale));
            }
        }
        let mut docs = Vec::with_capacity(batch.len());
        for (file_id, sig, hashes) in batch {
            let mut sum = vec![0f32; mean.len()];
            let mut n = 0usize;
            for h in hashes {
                if let Some(v) = rows.get(h) {
                    sum.iter_mut().zip(v).for_each(|(a, x)| *a += x);
                    n += 1;
                }
            }
            // A document with a chunk text not embedded yet waits for the next run.
            if n == hashes.len() && n > 0 {
                sum.iter_mut().for_each(|x| *x /= n as f32);
                docs.push((*file_id, *sig, centred_unit(sum, &mean)));
            }
        }
        store.put_docs(set, &docs)?;
        out.written += docs.len() as u64;
    }
    store.delete_docs(set, &gone)?;
    out.dropped = gone.len() as u64;
    Ok(out)
}

/// Up to `n` documents related to `file_id`, best first (R5–R7): a scan of the 1-bit
/// document rows, a rescore of the closest with their int8 rows, the threshold, and each one's
/// chunk closest to the asked document. Empty when search by meaning has no complete set, the
/// document has no document vector yet, or nothing passes the threshold.
pub fn related(index_db: &Path, file_id: i64, n: usize) -> Result<Vec<Related>> {
    let Some(store) = Store::open_ro(index_db)? else {
        return Ok(Vec::new());
    };
    let Some((set, dim)) = store.search_set()? else {
        return Ok(Vec::new());
    };
    let Some((q8, scale)) = store.doc_q8(set, file_id)? else {
        return Ok(Vec::new());
    };
    let target = dequantise(&q8, scale);
    let (ids, bits) = store.doc_bits(set)?;
    let width = dim.div_ceil(8);
    let tbits = quant::bits(&target);
    let mut by_distance: Vec<(u32, usize)> = bits
        .chunks_exact(width)
        .enumerate()
        .filter(|&(i, _)| ids[i] != file_id)
        .map(|(i, b)| (quant::hamming(b, &tbits), i))
        .collect();
    let take = CANDIDATES.min(by_distance.len());
    if take < by_distance.len() {
        by_distance.select_nth_unstable(take);
        by_distance.truncate(take);
    }
    let mut scored: Vec<(f32, i64)> = Vec::with_capacity(take);
    for (_, i) in by_distance {
        if let Some((q8, scale)) = store.doc_q8(set, ids[i])? {
            let s = quant::dot_int8(&target, &q8, scale);
            if s >= MIN_SIMILARITY {
                scored.push((s, ids[i]));
            }
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(n);
    let mean = store.mean(set)?.unwrap_or_else(|| vec![0.0; dim]);
    let mut out = Vec::with_capacity(scored.len());
    for (score, id) in scored {
        let mut best: Option<(f32, u32)> = None;
        for (ordinal, h) in store.chunk_hashes(id)? {
            if let Some((q8, scale)) = store.q8(set, &h)? {
                let v = centred_unit(dequantise(&q8, scale), &mean);
                let s: f32 = v.iter().zip(&target).map(|(a, b)| a * b).sum();
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, ordinal));
                }
            }
        }
        out.push(Related {
            file_id: id,
            ordinal: best.map(|b| b.1).unwrap_or(0),
            score,
        });
    }
    Ok(out)
}
