//! Content search (VFS-2 R6, R7): words and quoted phrases through the Spanish analyzer,
//! matched in FTS5 and ranked by BM25, filtered with the same type, date, size and folder
//! filters as name search, each hit with a snippet and the byte offsets of the matched words.

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use vfs_core::{fold_key, FileKind, SearchFilters, SearchQuery};

use crate::analyze::{Analyzer, Token};
use crate::store;
use crate::Result;

/// `total` counts matches up to this many.
const TOTAL_CAP: i64 = 10_000;
/// BM25 weights of the `title` and `body` columns.
const TITLE_WEIGHT: f64 = 3.0;
const BODY_WEIGHT: f64 = 1.0;
/// Words of context before the first highlighted word, and the snippet length in words.
const SNIPPET_LEAD: usize = 8;
const SNIPPET_WORDS: usize = 32;
/// Passages returned per document hit.
const MAX_PASSAGES: usize = 3;

/// A matching chunk of a document (VFS-2 R11): where it is, so a caller can cite or fetch
/// it, and a highlighted snippet of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Passage {
    /// Chunk ordinal in the document (`chunks.ordinal`).
    pub ordinal: u32,
    /// 1-based page, when the format has pages.
    pub page: Option<u32>,
    /// Character offsets of the chunk in the document's stored text.
    pub start_char: usize,
    pub end_char: usize,
    pub snippet: String,
    /// `[start, end)` byte offsets of matched words inside `snippet`.
    pub highlights: Vec<[usize; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentHit {
    /// `files.id`.
    pub id: i64,
    pub name: String,
    /// Absolute path of the containing directory, native separators.
    pub dir: String,
    pub kind: FileKind,
    pub size: u64,
    pub modified: i64,
    /// BM25 score (higher is better).
    pub score: f64,
    /// A fragment of the document text around the best match, whitespace collapsed.
    pub snippet: String,
    /// `[start, end)` byte offsets of matched words inside `snippet`.
    pub highlights: Vec<[usize; 2]>,
    /// Up to three best-matching chunks, best first; `snippet` is the first one's.
    pub passages: Vec<Passage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentResponse {
    pub hits: Vec<ContentHit>,
    /// Matching documents after filters, counted up to 10,000.
    pub total: u64,
    pub total_capped: bool,
    pub elapsed_ms: u64,
}

/// A parsed query: each element must match (AND); an element of several keys is a phrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    pub groups: Vec<Vec<String>>,
}

impl ParsedQuery {
    /// Splits on whitespace outside double quotes; a quoted run is a phrase, as is an
    /// unquoted word that the analyzer splits (`120€`, `Peñalara-2`).
    pub fn parse(text: &str, analyzer: &Analyzer) -> ParsedQuery {
        let mut groups = Vec::new();
        for (i, part) in text.split('"').enumerate() {
            let quoted = i % 2 == 1;
            if quoted {
                let keys: Vec<String> = analyzer.tokens(part).into_iter().map(|t| t.key).collect();
                if !keys.is_empty() {
                    groups.push(keys);
                }
            } else {
                for word in part.split_whitespace() {
                    let keys: Vec<String> =
                        analyzer.tokens(word).into_iter().map(|t| t.key).collect();
                    if !keys.is_empty() {
                        groups.push(keys);
                    }
                }
            }
        }
        ParsedQuery { groups }
    }

    /// FTS5 MATCH expression. Keys hold only letters and digits, so quoting is safe.
    pub fn fts_expr(&self) -> String {
        self.groups
            .iter()
            .map(|g| format!("\"{}\"", g.join(" ")))
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    fn keys(&self) -> HashSet<&str> {
        self.groups.iter().flatten().map(String::as_str).collect()
    }
}

/// Snippet around the window with the most distinct query keys, with highlight offsets.
pub fn snippet(text: &str, tokens: &[Token], keys: &HashSet<&str>) -> (String, Vec<[usize; 2]>) {
    if tokens.is_empty() {
        return (String::new(), Vec::new());
    }
    let hit = |t: &Token| keys.contains(t.key.as_str());
    let mut best = (0usize, 0usize); // (distinct keys, first matching token index)
    let mut found = false;
    for (i, t) in tokens.iter().enumerate() {
        if !hit(t) {
            continue;
        }
        let window = &tokens[i..(i + SNIPPET_WORDS - SNIPPET_LEAD).min(tokens.len())];
        let distinct: HashSet<&str> = window
            .iter()
            .filter(|w| hit(w))
            .map(|w| w.key.as_str())
            .collect();
        if !found || distinct.len() > best.0 {
            best = (distinct.len(), i);
            found = true;
            if distinct.len() == keys.len() {
                break;
            }
        }
    }
    let first = best.1.saturating_sub(SNIPPET_LEAD);
    let last = (first + SNIPPET_WORDS).min(tokens.len()) - 1;
    let mut out = String::new();
    let mut highlights = Vec::new();
    if first > 0 {
        out.push_str("… ");
    }
    for i in first..=last {
        let t = &tokens[i];
        if i > first {
            // The gap between two words, whitespace runs collapsed to one space.
            let gap = &text[tokens[i - 1].end..t.start];
            let mut prev_space = false;
            for c in gap.chars() {
                if c.is_whitespace() {
                    if !prev_space {
                        out.push(' ');
                    }
                    prev_space = true;
                } else {
                    out.push(c);
                    prev_space = false;
                }
            }
        }
        let start = out.len();
        out.push_str(&text[t.start..t.end]);
        if hit(t) {
            highlights.push([start, out.len()]);
        }
    }
    if last + 1 < tokens.len() {
        out.push_str(" …");
    }
    (out, highlights)
}

struct ChunkRow {
    ordinal: u32,
    start_byte: usize,
    end_byte: usize,
    start_char: usize,
    end_char: usize,
    page: Option<u32>,
}

/// The best chunks by (distinct query keys, matched words), each with its snippet.
fn passages(
    text: &str,
    tokens: &[Token],
    chunks: &[ChunkRow],
    keys: &HashSet<&str>,
) -> Vec<Passage> {
    let mut scored: Vec<(usize, usize, &ChunkRow, &[Token])> = Vec::new();
    for c in chunks {
        let a = tokens.partition_point(|t| t.start < c.start_byte);
        let b = tokens.partition_point(|t| t.start < c.end_byte);
        let toks = &tokens[a..b];
        let hits: Vec<&str> = toks
            .iter()
            .filter(|t| keys.contains(t.key.as_str()))
            .map(|t| t.key.as_str())
            .collect();
        if hits.is_empty() {
            continue;
        }
        let distinct: HashSet<&str> = hits.iter().copied().collect();
        scored.push((distinct.len(), hits.len(), c, toks));
    }
    scored.sort_by(|x, y| {
        (y.0, y.1)
            .cmp(&(x.0, x.1))
            .then(x.2.ordinal.cmp(&y.2.ordinal))
    });
    scored
        .into_iter()
        .take(MAX_PASSAGES)
        .map(|(_, _, c, toks)| {
            let (snippet, highlights) = snippet(text, toks, keys);
            Passage {
                ordinal: c.ordinal,
                page: c.page,
                start_char: c.start_char,
                end_char: c.end_char,
                snippet,
                highlights,
            }
        })
        .collect()
}

struct RootRow {
    id: i64,
    path: String,
    path_key: String,
}

/// Where a folder filter lets hits come from (`rel = None`: the whole root).
struct Scope {
    root_id: i64,
    rel: Option<String>,
}

/// Same meaning as name search's folder filter (`vfs-core` `search::resolve_scopes`):
/// the root the folder lies in plus the directory below it, or every root inside the folder;
/// compared on folded keys, so case, accents and `\` vs `/` do not matter.
fn resolve_scopes(conn: &Connection, folder: &str) -> Result<Vec<Scope>> {
    let roots: Vec<RootRow> = {
        let mut st = conn.prepare("SELECT id, path, path_key FROM roots")?;
        let rows = st.query_map([], |r| {
            Ok(RootRow {
                id: r.get(0)?,
                path: r.get(1)?,
                path_key: r.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let raw = match folder.trim_end_matches(['/', '\\']) {
        "" => folder,
        t => t,
    };
    let fkey = fold_key(&raw.replace('\\', "/"));
    let fbase = fkey.trim_end_matches('/');
    let mut scopes = Vec::new();
    for r in &roots {
        let rbase = r.path_key.trim_end_matches('/');
        if fbase == rbase
            || rbase
                .strip_prefix(fbase)
                .is_some_and(|rest| rest.starts_with('/'))
        {
            scopes.push(Scope {
                root_id: r.id,
                rel: None,
            });
        } else if let Some(rel_key) = fkey.strip_prefix(rbase).and_then(|s| s.strip_prefix('/')) {
            let raw_rel = raw
                .strip_prefix(r.path.trim_end_matches(['/', '\\']))
                .map(|s| s.trim_start_matches(['/', '\\']).replace('\\', "/"));
            let mut rel: Option<String> = None;
            if let Some(raw_rel) = raw_rel {
                rel = conn
                    .query_row(
                        "SELECT rel_path FROM dirs WHERE root_id = ?1 AND rel_path = ?2",
                        rusqlite::params![r.id, raw_rel],
                        |row| row.get(0),
                    )
                    .optional()?;
            }
            if rel.is_none() {
                rel = conn
                    .query_row(
                        "SELECT rel_path FROM dirs WHERE root_id = ?1 AND path_key = ?2 LIMIT 1",
                        rusqlite::params![r.id, rel_key],
                        |row| row.get(0),
                    )
                    .optional()?;
            }
            if let Some(rel) = rel {
                scopes.push(Scope {
                    root_id: r.id,
                    rel: Some(rel),
                });
            }
        }
    }
    Ok(scopes)
}

/// SQL predicate (starting with ` AND`) for the filters, with its parameters; `None` when
/// the filters exclude every document. Same semantics as name search's `file_clause`.
fn filter_clause(conn: &Connection, f: &SearchFilters) -> Result<Option<(String, Vec<Value>)>> {
    let mut sql = String::new();
    let mut params = Vec::new();
    if !f.kinds.is_empty() && !f.kinds.contains(&FileKind::Document) {
        return Ok(None);
    }
    if let Some(v) = f.modified_from {
        sql.push_str(" AND f.mtime >= ?");
        params.push(Value::Integer(v));
    }
    if let Some(v) = f.modified_to {
        sql.push_str(" AND f.mtime < ?");
        params.push(Value::Integer(v));
    }
    if let Some(v) = f.size_min {
        sql.push_str(" AND f.size >= ?");
        params.push(Value::Integer(i64::try_from(v).unwrap_or(i64::MAX)));
    }
    if let Some(v) = f.size_max {
        sql.push_str(" AND f.size <= ?");
        params.push(Value::Integer(i64::try_from(v).unwrap_or(i64::MAX)));
    }
    if let Some(folder) = &f.folder {
        let scopes = resolve_scopes(conn, folder)?;
        if scopes.is_empty() {
            return Ok(None);
        }
        let mut parts = Vec::new();
        for s in scopes {
            match s.rel {
                // Descendants of `p` are exactly the rel_paths in [p + '/', p + '0').
                Some(p) => {
                    parts.push(
                        "(d.root_id = ? AND (d.rel_path = ? OR (d.rel_path >= ? AND d.rel_path < ?)))",
                    );
                    params.extend([
                        Value::Integer(s.root_id),
                        Value::Text(p.clone()),
                        Value::Text(format!("{p}/")),
                        Value::Text(format!("{p}0")),
                    ]);
                }
                None => {
                    parts.push("d.root_id = ?");
                    params.push(Value::Integer(s.root_id));
                }
            }
        }
        sql.push_str(&format!(" AND ({})", parts.join(" OR ")));
    }
    Ok(Some((sql, params)))
}

/// An open read connection for repeated content searches (the app keeps one, so SQLite's
/// page cache stays warm between queries).
pub struct Searcher {
    conn: Connection,
}

impl Searcher {
    /// `Err(Error::Off)` when content search is not switched on.
    pub fn open(index_db: &Path) -> Result<Searcher> {
        if !store::exists(index_db) {
            return Err(crate::Error::Off);
        }
        Ok(Searcher {
            conn: store::open_index_ro(index_db)?,
        })
    }

    pub fn search(&self, query: &SearchQuery) -> Result<ContentResponse> {
        search_conn(&self.conn, query)
    }
}

/// Searches document content once (opens and closes a connection). `Err(Error::Off)` when
/// content search is not switched on.
pub fn search(index_db: &Path, query: &SearchQuery) -> Result<ContentResponse> {
    Searcher::open(index_db)?.search(query)
}

/// [`search`] on an already opened `index.db` connection with the store attached as `c`.
pub fn search_conn(conn: &Connection, query: &SearchQuery) -> Result<ContentResponse> {
    let started = Instant::now();
    let analyzer = Analyzer::default();
    let parsed = ParsedQuery::parse(&query.text, &analyzer);
    let empty = |started: Instant| ContentResponse {
        hits: Vec::new(),
        total: 0,
        total_capped: false,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    if parsed.groups.is_empty() {
        return Ok(empty(started));
    }
    let Some((filter_sql, filter_params)) = filter_clause(conn, &query.filters)? else {
        return Ok(empty(started));
    };
    let from = format!(
        "FROM c.content_fts
         JOIN files f ON f.id = content_fts.rowid
         JOIN c.docs dc ON dc.file_id = f.id AND dc.dir_id = f.dir_id AND dc.name = f.name
         JOIN dirs d ON d.id = f.dir_id
         WHERE content_fts MATCH ?{filter_sql}"
    );
    let unfiltered = filter_sql.is_empty();
    let mut params = vec![Value::Text(parsed.fts_expr())];
    params.extend(filter_params);
    // Without filters, count and rank inside FTS5 and join only the page: the joins over
    // every match of a common word were most of the latency (VFS-2 measurements).
    let count_sql = if unfiltered {
        "SELECT count(*) FROM (SELECT 1 FROM c.content_fts WHERE content_fts MATCH ? LIMIT ?)"
            .to_string()
    } else {
        format!("SELECT count(*) FROM (SELECT 1 {from} LIMIT ?)")
    };
    let mut count_params = params.clone();
    count_params.push(Value::Integer(TOTAL_CAP + 1));
    let total: i64 = conn
        .prepare_cached(&count_sql)?
        .query_row(rusqlite::params_from_iter(count_params.iter()), |r| {
            r.get(0)
        })?;
    let limit = query.limit.clamp(1, 500) as i64;
    let mut page_params = params.clone();
    page_params.push(Value::Integer(limit));
    page_params.push(Value::Integer(query.offset as i64));
    let page_sql = if unfiltered {
        format!(
            "SELECT f.id, f.name, f.kind, f.size, f.mtime, f.dir_id, t.score, dc.text
             FROM (SELECT rowid AS id, bm25(content_fts, {TITLE_WEIGHT}, {BODY_WEIGHT}) AS score
                   FROM c.content_fts WHERE content_fts MATCH ? ORDER BY score LIMIT ? OFFSET ?) t
             JOIN files f ON f.id = t.id
             JOIN c.docs dc ON dc.file_id = f.id AND dc.dir_id = f.dir_id AND dc.name = f.name
             ORDER BY t.score"
        )
    } else {
        format!(
            "SELECT f.id, f.name, f.kind, f.size, f.mtime, f.dir_id,
                    bm25(content_fts, {TITLE_WEIGHT}, {BODY_WEIGHT}) AS score, dc.text
             {from} ORDER BY score LIMIT ? OFFSET ?"
        )
    };
    // (hit without dir and snippet, dir_id, compressed text)
    let rows: Vec<(ContentHit, i64, Option<Vec<u8>>)> = {
        let mut st = conn.prepare_cached(&page_sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(page_params.iter()), |r| {
            let hit = ContentHit {
                id: r.get(0)?,
                name: r.get(1)?,
                dir: String::new(),
                kind: FileKind::from_code(r.get(2)?),
                size: r.get::<_, i64>(3)?.max(0) as u64,
                modified: r.get(4)?,
                // FTS5's bm25() is negative, more negative for better matches.
                score: -r.get::<_, f64>(6)?,
                snippet: String::new(),
                highlights: Vec::new(),
                passages: Vec::new(),
            };
            Ok((hit, r.get(5)?, r.get(7)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let keys = parsed.keys();
    let mut hits = Vec::with_capacity(rows.len());
    let mut chunk_st = conn.prepare_cached(
        "SELECT ordinal, start_byte, end_byte, start_char, end_char, page
         FROM c.chunks WHERE file_id = ?1 ORDER BY ordinal",
    )?;
    for (mut hit, dir_id, blob) in rows {
        let text = blob.as_deref().map(store::decompress).unwrap_or_default();
        let tokens = analyzer.tokens(&text);
        let chunk_rows: Vec<ChunkRow> = chunk_st
            .query_map([hit.id], |r| {
                Ok(ChunkRow {
                    ordinal: r.get(0)?,
                    start_byte: r.get::<_, i64>(1)? as usize,
                    end_byte: r.get::<_, i64>(2)? as usize,
                    start_char: r.get::<_, i64>(3)? as usize,
                    end_char: r.get::<_, i64>(4)? as usize,
                    page: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        hit.passages = passages(&text, &tokens, &chunk_rows, &keys);
        (hit.snippet, hit.highlights) = match hit.passages.first() {
            Some(p) => (p.snippet.clone(), p.highlights.clone()),
            // The match is only in the title, or the store predates chunks.
            None => snippet(&text, &tokens, &keys),
        };
        hit.dir = vfs_core::db::dir_abs_path(conn, dir_id)?
            .to_string_lossy()
            .into_owned();
        hits.push(hit);
    }
    Ok(ContentResponse {
        hits,
        total: total.min(TOTAL_CAP) as u64,
        total_capped: total > TOTAL_CAP,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-2 AC-8: quoted runs are phrases, other words are separate AND terms.
    #[test]
    fn parses_words_and_phrases() {
        let a = Analyzer::default();
        let q = ParsedQuery::parse(r#"facturas "contrato de alquiler" PEÑALARA"#, &a);
        assert_eq!(q.groups.len(), 3);
        assert_eq!(q.groups[1].len(), 3);
        assert_eq!(q.groups[0], vec![a.key("factura")]);
        assert!(q.fts_expr().contains(" AND "));
        assert!(ParsedQuery::parse("  \"\" ¿? ", &a).groups.is_empty());
    }

    /// VFS-2 AC-8: the snippet window holds the matches and the offsets point at them.
    #[test]
    fn snippet_highlights_matches() {
        let a = Analyzer::default();
        let mut text = "palabra ".repeat(40);
        text.push_str("El señor   Muñoz pagó\n\nlas facturas de la calefacción. ");
        text.push_str(&"final ".repeat(40));
        let q = ParsedQuery::parse("factura calefaccion", &a);
        let toks = a.tokens(&text);
        let (s, hl) = snippet(&text, &toks, &q.keys());
        assert!(s.starts_with("… ") && s.ends_with(" …"));
        let words: Vec<&str> = hl.iter().map(|[b, e]| &s[*b..*e]).collect();
        assert_eq!(words, ["facturas", "calefacción"]);
        assert!(s.contains("señor Muñoz pagó las facturas"));
    }
}
