//! Search by meaning inside `retrieve` (VFS-29 R11–R13, D7, D17): when the caller gives the
//! [`Searcher`] a [`Meaning`] ranker, `retrieve` asks it for the chunks closest to the query,
//! keeps the documents inside the filters, and fuses them with BM25 by reciprocal rank fusion
//! (k = 60). Every caller of `retrieve` (the «Documentos» tab, questions) gets the fused list
//! with no change of its own. The ranker lives in `vfs-semantic`; this crate only knows the
//! trait, so it never depends on vectors or models.

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use vfs_core::SearchFilters;

use crate::search::{
    collapse_snippet, filter_clause, ContentHit, ContentResponse, Passage, Query, Searcher,
};
use crate::Result;

/// Reciprocal rank fusion constant (R11, D7).
pub const RRF_K: f64 = 60.0;

/// Chunks asked of the ranker per document wanted: several chunks of one document may come
/// first, and filters drop some.
const CHUNKS_PER_DOC: usize = 4;

/// One chunk found by meaning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeaningHit {
    pub file_id: i64,
    pub ordinal: u32,
    /// Similarity to the query; higher is closer.
    pub score: f32,
}

/// Why meaning search could not answer this query (R13): the caller falls back to BM25 and
/// says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeaningUnavailable {
    /// The model is still loading, or the vectors are not complete yet.
    NotReady,
    /// The query embedding failed or took longer than allowed.
    Failed(String),
}

/// Ranks chunks by meaning (implemented in `vfs-semantic`).
pub trait Meaning: Send + Sync {
    /// Up to `n` chunks closest to `query`, best first.
    fn rank(
        &self,
        query: &str,
        n: usize,
    ) -> std::result::Result<Vec<MeaningHit>, MeaningUnavailable>;
}

/// Whether meaning search took part in a response, for the «Documentos» tab (R13).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeaningUse {
    /// Meaning search is off: BM25 alone, as always.
    #[default]
    Off,
    /// The hits are the fused list.
    Used,
    /// Meaning search is on but could not answer now: BM25 alone, with the notice.
    Unavailable,
}

/// The text a query is embedded from: the user's words, or a structured query's terms.
pub fn query_text(query: &Query) -> String {
    match query {
        Query::Text(t) => t.clone(),
        Query::Structured(s) => s
            .all
            .iter()
            .chain(&s.phrases)
            .chain(&s.any)
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Reciprocal rank fusion of ranked id lists: each list adds `1 / (RRF_K + rank)`; ties
/// break on the order of the first list, then the id.
pub fn rrf(lists: &[&[i64]]) -> Vec<i64> {
    let mut score: HashMap<i64, (f64, usize)> = HashMap::new();
    let mut seen = 0usize;
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            let e = score.entry(*id).or_insert_with(|| {
                seen += 1;
                (0.0, seen)
            });
            e.0 += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
    }
    let mut ids: Vec<i64> = score.keys().copied().collect();
    ids.sort_by(|a, b| {
        let (sa, oa) = score[a];
        let (sb, ob) = score[b];
        sb.total_cmp(&sa).then(oa.cmp(&ob))
    });
    ids
}

impl Searcher {
    /// `retrieve` with meaning (R11): BM25's top `offset + k` and the ranker's documents,
    /// fused. BM25's list is left out when no document has every query word (`partial`,
    /// D17). When the ranker cannot answer, BM25's own response, marked [`MeaningUse::Unavailable`].
    pub(crate) fn retrieve_fused(
        &self,
        meaning: &dyn Meaning,
        bm25: ContentResponse,
        query: &Query,
        filters: &SearchFilters,
        k: u32,
        offset: u32,
    ) -> Result<ContentResponse> {
        let want = (offset + k) as usize;
        let chunks = match meaning.rank(&query_text(query), want * CHUNKS_PER_DOC) {
            Ok(c) => c,
            Err(_) => {
                let mut r = bm25;
                r.hits = r.hits.into_iter().skip(offset as usize).collect();
                r.meaning = MeaningUse::Unavailable;
                return Ok(r);
            }
        };
        // Documents in the order of their best chunk, with their matching chunks.
        let mut by_doc: Vec<(i64, Vec<(u32, f32)>)> = Vec::new();
        for c in chunks {
            match by_doc.iter_mut().find(|(id, _)| *id == c.file_id) {
                Some((_, v)) => v.push((c.ordinal, c.score)),
                None => by_doc.push((c.file_id, vec![(c.ordinal, c.score)])),
            }
        }
        let allowed =
            self.inside_filters(&by_doc.iter().map(|d| d.0).collect::<Vec<_>>(), filters)?;
        by_doc.retain(|(id, _)| allowed.contains(id));
        by_doc.truncate(want);

        let dense: Vec<i64> = by_doc.iter().map(|d| d.0).collect();
        if dense.is_empty() {
            // Nothing found by meaning inside the filters: the word hits, partial or not.
            let mut r = bm25;
            r.hits = r.hits.into_iter().skip(offset as usize).collect();
            r.meaning = MeaningUse::Used;
            return Ok(r);
        }
        let lexical: Vec<i64> = if bm25.partial {
            Vec::new()
        } else {
            bm25.hits.iter().map(|h| h.id).collect()
        };
        let fused = rrf(&[&lexical, &dense]);
        let dense_only = dense.iter().filter(|id| !lexical.contains(id)).count() as u64;

        let mut bm_hits: HashMap<i64, ContentHit> = if bm25.partial {
            HashMap::new()
        } else {
            bm25.hits.into_iter().map(|h| (h.id, h)).collect()
        };
        let mut hits = Vec::new();
        for id in fused.into_iter().skip(offset as usize).take(k as usize) {
            if let Some(h) = bm_hits.remove(&id) {
                hits.push(h);
            } else if let Some((_, chunks)) = by_doc.iter().find(|d| d.0 == id) {
                if let Some(h) = self.meaning_hit(id, chunks)? {
                    hits.push(h);
                }
            }
        }
        let base = if bm25.partial { 0 } else { bm25.total };
        Ok(ContentResponse {
            hits,
            partial: false,
            total: base + dense_only,
            total_capped: !bm25.partial && bm25.total_capped,
            elapsed_ms: 0,
            meaning: MeaningUse::Used,
        })
    }

    /// The ids among `ids` that are stored documents inside `filters`.
    fn inside_filters(&self, ids: &[i64], filters: &SearchFilters) -> Result<Vec<i64>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let Some((filter_sql, filter_params)) = filter_clause(&self.conn, filters)? else {
            return Ok(Vec::new());
        };
        let marks = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT f.id FROM files f
             JOIN c.docs dc ON dc.file_id = f.id AND dc.dir_id = f.dir_id AND dc.name = f.name
             JOIN dirs d ON d.id = f.dir_id
             WHERE f.id IN ({marks}){filter_sql}"
        );
        let mut params: Vec<Value> = ids.iter().map(|&i| Value::Integer(i)).collect();
        params.extend(filter_params);
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A document found by meaning alone (R12): its matching chunks as passages, best first,
    /// and the best one's text as the snippet, with nothing highlighted.
    fn meaning_hit(&self, file_id: i64, chunks: &[(u32, f32)]) -> Result<Option<ContentHit>> {
        let Some(mut hit) = self.document(file_id, 0)? else {
            return Ok(None);
        };
        let text = self.doc_text(file_id)?;
        let mut st = self.conn.prepare_cached(
            "SELECT page, start_byte, end_byte, start_char, end_char FROM c.chunks
             WHERE file_id = ?1 AND ordinal = ?2",
        )?;
        for &(ordinal, score) in chunks.iter().take(crate::search::MAX_PASSAGES) {
            let row: Option<(Option<u32>, i64, i64, i64, i64)> = st
                .query_row(rusqlite::params![file_id, ordinal], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .optional()?;
            let Some((page, sb, eb, sc, ec)) = row else {
                continue;
            };
            let piece = text
                .get(sb.max(0) as usize..eb.max(0) as usize)
                .unwrap_or("");
            hit.passages.push(Passage {
                file_id,
                ordinal,
                page,
                start_char: sc.max(0) as usize,
                end_char: ec.max(0) as usize,
                score: score as f64,
                snippet: collapse_snippet(piece),
                highlights: Vec::new(),
            });
        }
        hit.snippet = hit
            .passages
            .first()
            .map(|p| p.snippet.clone())
            .unwrap_or_default();
        hit.score = chunks.first().map(|c| c.1 as f64).unwrap_or(0.0);
        hit.by_meaning = true;
        Ok(Some(hit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::StructuredQuery;

    /// VFS-29 R11 (D7): reciprocal rank fusion adds 1/(60 + rank) per list; a document in
    /// both lists beats one first in a single list; ties keep the first list's order.
    #[test]
    fn rrf_fuses_ranks() {
        assert_eq!(rrf(&[&[1, 2, 3], &[3, 4]]), vec![3, 1, 2, 4]);
        assert_eq!(rrf(&[&[], &[7, 8]]), vec![7, 8]);
        assert_eq!(rrf(&[&[5, 6], &[6, 5]]), vec![5, 6]);
    }

    /// VFS-29 R11: a structured query is embedded from all its terms.
    #[test]
    fn structured_queries_embed_their_terms() {
        let q = Query::Structured(StructuredQuery {
            all: vec!["gastos".into()],
            any: vec!["dentista".into()],
            phrases: vec!["seguro médico".into()],
        });
        assert_eq!(query_text(&q), "gastos seguro médico dentista");
        assert_eq!(query_text(&Query::Text("hola".into())), "hola");
    }
}
