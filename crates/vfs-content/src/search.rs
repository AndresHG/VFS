//! Content search (VFS-2 R6, R7): words and quoted phrases through the Spanish analyzer,
//! matched in FTS5 and ranked by BM25, filtered with the same type, date, size and folder
//! filters as name search, each hit with a snippet and the byte offsets of the matched words.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use vfs_core::content::ContentState;
use vfs_core::{fold_key, FileKind, SearchFilters, SearchQuery};

use crate::analyze::{Analyzer, Token};
use crate::protocol::SkipReason;
use crate::store;
use crate::Result;

/// `total` counts matches up to this many (VFS-2 R15).
const TOTAL_CAP: i64 = 1_000;
/// Candidates ranked when every query word is common (VFS-2 R14).
const CANDIDATE_CAP: i64 = 1_000;
/// Most hits one call returns.
const MAX_K: u32 = 500;
/// BM25 weights of the `title` and `body` columns.
const TITLE_WEIGHT: f64 = 3.0;
const BODY_WEIGHT: f64 = 1.0;
/// Words of context before the first highlighted word, and the snippet length in words.
const SNIPPET_LEAD: usize = 8;
const SNIPPET_WORDS: usize = 32;
/// Passages returned per document hit (VFS-46 R8, superseding VFS-2 R11's three).
const MAX_PASSAGES: usize = 6;

/// A matching chunk of a document (VFS-2 R11): where it is, so a caller can cite or fetch
/// it, and a highlighted snippet of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Passage {
    /// The document (`files.id`).
    pub file_id: i64,
    /// Chunk ordinal in the document (`chunks.ordinal`).
    pub ordinal: u32,
    /// 1-based page, when the format has pages.
    pub page: Option<u32>,
    /// Character offsets of the chunk in the document's stored text.
    pub start_char: usize,
    pub end_char: usize,
    /// BM25 of the chunk for the query keys; passages are ranked by it within a document.
    pub score: f64,
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
    /// Metadata as the extractor reported it; empty when the format has none (VFS-2 R16).
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub created: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentResponse {
    pub hits: Vec<ContentHit>,
    /// No document had every word, so these hits have any of them (VFS-2 R13).
    pub partial: bool,
    /// Matching documents after filters, counted up to 1,000 (VFS-2 R15).
    pub total: u64,
    pub total_capped: bool,
    pub elapsed_ms: u64,
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

/// The best chunks of a document, each with a BM25 score computed over the chunk (term
/// frequency in the chunk, document frequency from the index), best first, at most
/// `MAX_PASSAGES` (VFS-2 R11, R15).
fn passages(
    file_id: i64,
    text: &str,
    tokens: &[Token],
    chunks: &[ChunkRow],
    keys: &HashSet<&str>,
    stats: &Stats,
) -> Vec<Passage> {
    const K1: f64 = 1.2;
    const B: f64 = 0.75;
    let avg_len = (tokens.len() as f64 / chunks.len().max(1) as f64).max(1.0);
    let mut scored: Vec<(f64, &ChunkRow, &[Token])> = Vec::new();
    for c in chunks {
        let a = tokens.partition_point(|t| t.start < c.start_byte);
        let b = tokens.partition_point(|t| t.start < c.end_byte);
        let toks = &tokens[a..b];
        let mut tf: HashMap<&str, f64> = HashMap::new();
        for t in toks.iter().filter(|t| keys.contains(t.key.as_str())) {
            *tf.entry(t.key.as_str()).or_default() += 1.0;
        }
        if tf.is_empty() {
            continue;
        }
        let norm = K1 * (1.0 - B + B * toks.len() as f64 / avg_len);
        let score = tf
            .iter()
            .map(|(k, f)| stats.idf(k) * f * (K1 + 1.0) / (f + norm))
            .sum();
        scored.push((score, c, toks));
    }
    scored.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.ordinal.cmp(&y.1.ordinal)));
    scored
        .into_iter()
        .take(MAX_PASSAGES)
        .map(|(score, c, toks)| {
            let (snippet, highlights) = snippet(text, toks, keys);
            Passage {
                file_id,
                ordinal: c.ordinal,
                page: c.page,
                start_char: c.start_char,
                end_char: c.end_char,
                score,
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

/// A content query (VFS-2 R15): the user's text, or a structured query built by a caller
/// such as the questions step (VFS-28).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Query {
    /// Parsed as in R6 and R13: quoted runs are phrases, other words are AND-ed, with an OR
    /// fallback when that finds nothing.
    Text(String),
    Structured(StructuredQuery),
}

/// Every `all` word and every phrase must match; `any` words are optional and needed (one
/// at least) only when `all` and `phrases` are empty. All of them rank.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredQuery {
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub any: Vec<String>,
    #[serde(default)]
    pub phrases: Vec<String>,
}

/// A parsed query: each group must match (AND); a group of several keys is a phrase.
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
            if i % 2 == 1 {
                push_group(&mut groups, part, analyzer);
            } else {
                for word in part.split_whitespace() {
                    push_group(&mut groups, word, analyzer);
                }
            }
        }
        ParsedQuery { groups }
    }

    /// FTS5 MATCH expression requiring every group.
    pub fn fts_expr(&self) -> String {
        join_groups(&self.groups, " AND ")
    }
}

fn push_group(groups: &mut Vec<Vec<String>>, text: &str, analyzer: &Analyzer) {
    let keys: Vec<String> = analyzer.tokens(text).into_iter().map(|t| t.key).collect();
    if !keys.is_empty() && !groups.contains(&keys) {
        groups.push(keys);
    }
}

/// FTS5 expression of groups joined by `op`. Keys hold only letters and digits, so quoting
/// is safe.
fn join_groups(groups: &[Vec<String>], op: &str) -> String {
    groups
        .iter()
        .map(|g| format!("\"{}\"", g.join(" ")))
        .collect::<Vec<_>>()
        .join(op)
}

/// What to match and what to rank by.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    required: Vec<Vec<String>>,
    optional: Vec<Vec<String>>,
}

impl Plan {
    fn from_query(q: &Query, analyzer: &Analyzer) -> Plan {
        match q {
            Query::Text(t) => Plan {
                required: ParsedQuery::parse(t, analyzer).groups,
                optional: Vec::new(),
            },
            Query::Structured(s) => {
                let mut required = Vec::new();
                for w in s.all.iter().chain(&s.phrases) {
                    push_group(&mut required, w, analyzer);
                }
                let mut optional = Vec::new();
                for w in &s.any {
                    push_group(&mut optional, w, analyzer);
                }
                optional.retain(|g| !required.contains(g));
                Plan { required, optional }
            }
        }
    }

    /// R13's fallback: any of the words.
    fn any_of_all(&self) -> Plan {
        Plan {
            required: Vec::new(),
            optional: self
                .required
                .iter()
                .chain(&self.optional)
                .cloned()
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.required.is_empty() && self.optional.is_empty()
    }

    fn keys(&self) -> HashSet<&str> {
        self.required
            .iter()
            .chain(&self.optional)
            .flatten()
            .map(String::as_str)
            .collect()
    }
}

/// Indexed documents and documents per key, for the frequency guard (R14) and passage
/// scores.
struct Stats {
    docs: u64,
    df: HashMap<String, u64>,
}

impl Stats {
    fn load(conn: &Connection, docs: u64, keys: &HashSet<&str>) -> Result<Stats> {
        let mut st = conn.prepare_cached("SELECT doc FROM c.content_vocab WHERE term = ?1")?;
        let mut df = HashMap::new();
        for k in keys {
            let n: Option<i64> = st.query_row([k], |r| r.get(0)).optional()?;
            df.insert(k.to_string(), n.unwrap_or(0).max(0) as u64);
        }
        let docs = at_least_every_df(docs, &df);
        Ok(Stats { docs, df })
    }

    /// R14: a word in more than 5 % of the indexed documents filters but does not rank.
    /// Phrases always rank.
    fn common(&self, group: &[String]) -> bool {
        group.len() == 1 && self.df.get(&group[0]).copied().unwrap_or(0) * 100 > self.docs * 5
    }

    fn idf(&self, key: &str) -> f64 {
        let n = self.docs as f64;
        let df = self.df.get(key).copied().unwrap_or(0) as f64;
        ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
    }
}

/// FTS5 expressions for one plan: `filter` decides which documents match, `rank` which
/// keys score them, and `cap` limits the candidates ranked (R14).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Exprs {
    filter: String,
    rank: String,
    cap: Option<i64>,
}

fn exprs(plan: &Plan, stats: &Stats) -> Exprs {
    let rare = |gs: &[Vec<String>]| -> Vec<Vec<String>> {
        gs.iter().filter(|g| !stats.common(g)).cloned().collect()
    };
    if plan.required.is_empty() {
        let filter = join_groups(&plan.optional, " OR ");
        let all_common = rare(&plan.optional).is_empty();
        return Exprs {
            rank: filter.clone(),
            filter,
            cap: all_common.then_some(CANDIDATE_CAP),
        };
    }
    let filter = join_groups(&plan.required, " AND ");
    let rare_req = rare(&plan.required);
    if rare_req.is_empty() {
        // Every required word is common: rank by all of them over the first candidates.
        let all: Vec<Vec<String>> = plan
            .required
            .iter()
            .chain(&plan.optional)
            .cloned()
            .collect();
        return Exprs {
            filter,
            rank: join_groups(&all, " OR "),
            cap: Some(CANDIDATE_CAP),
        };
    }
    if rare_req.len() == plan.required.len() && plan.optional.is_empty() {
        return Exprs {
            rank: filter.clone(),
            filter,
            cap: None,
        };
    }
    let mut rank_groups = rare_req;
    rank_groups.extend(rare(&plan.optional));
    Exprs {
        filter,
        rank: join_groups(&rank_groups, " OR "),
        cap: None,
    }
}

/// The text of one stored chunk joined with its neighbours (VFS-2 R15 `passage_text`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassageText {
    pub file_id: i64,
    /// Ordinals of the first and last chunk joined.
    pub first_ordinal: u32,
    pub last_ordinal: u32,
    /// Character range of `text` in the document's stored text.
    pub start_char: usize,
    pub end_char: usize,
    pub text: String,
}

/// Why a document's content is not searchable (VFS-2 R15 `unread`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreadReason {
    Cloud,
    Scanned,
    Password,
    TooLarge,
    Unsupported,
    Failed,
}

impl UnreadReason {
    /// The reason for a processed document, following the table in VFS-2 R15. `None` for an
    /// indexed one.
    fn of(state: i64, reason: Option<&str>) -> Option<UnreadReason> {
        if state == ContentState::Indexed as i64 {
            return None;
        }
        if state != ContentState::Skipped as i64 {
            return Some(UnreadReason::Failed);
        }
        let skip: Option<SkipReason> =
            reason.and_then(|r| serde_json::from_value(serde_json::Value::String(r.into())).ok());
        Some(match skip {
            Some(SkipReason::Cloud) => UnreadReason::Cloud,
            Some(SkipReason::Scanned) => UnreadReason::Scanned,
            Some(SkipReason::Password) => UnreadReason::Password,
            Some(SkipReason::TooLarge) => UnreadReason::TooLarge,
            Some(SkipReason::Unsupported) => UnreadReason::Unsupported,
            Some(SkipReason::NoText) | None => UnreadReason::Failed,
        })
    }
}

/// An open read connection for repeated content calls (the app keeps one, so SQLite's page
/// cache stays warm between queries). These calls are the only way into `content.db` for
/// every other crate (VFS-2 R15).
pub struct Searcher {
    conn: Connection,
    /// The indexed-document count and when it was read: counted at most every
    /// [`DOC_COUNT_TTL`], not on every query (VFS-27 hand-off from VFS-2).
    doc_count: std::sync::Mutex<Option<(Instant, u64)>>,
}

/// How long a [`Searcher`] reuses its indexed-document count. The count only weighs the
/// frequency guard's 5 % (R14); while the store fills, a count up to this old can mark a few
/// more words common than a fresh one. A count of 0 is never reused.
const DOC_COUNT_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// A cached count can trail the store while it fills; no key is in more documents than there
/// are, so the count is raised to the largest document frequency and `idf` stays positive.
fn at_least_every_df(docs: u64, df: &HashMap<String, u64>) -> u64 {
    df.values().copied().fold(docs, u64::max)
}

/// The cached count, when it may be reused at `now`: not older than [`DOC_COUNT_TTL`], and
/// never 0 (a store still filling would make every word common).
fn reuse_count(cached: Option<(Instant, u64)>, now: Instant) -> Option<u64> {
    let (at, n) = cached?;
    (n > 0 && now.saturating_duration_since(at) < DOC_COUNT_TTL).then_some(n)
}

/// Indexed documents in the store.
fn count_indexed(conn: &Connection) -> Result<u64> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM c.docs WHERE state = ?1",
        [ContentState::Indexed as i64],
        |r| r.get(0),
    )?;
    Ok(n.max(0) as u64)
}

impl Searcher {
    /// `Err(Error::Off)` when content search is not switched on.
    pub fn open(index_db: &Path) -> Result<Searcher> {
        if !store::exists(index_db) {
            return Err(crate::Error::Off);
        }
        Ok(Searcher {
            conn: store::open_index_ro(index_db)?,
            doc_count: std::sync::Mutex::new(None),
        })
    }

    /// Ranked documents for `query` under `filters`, from `offset`, at most `k` (R15).
    pub fn retrieve(
        &self,
        query: &Query,
        filters: &SearchFilters,
        k: u32,
        offset: u32,
    ) -> Result<ContentResponse> {
        let docs = self.doc_count()?;
        retrieve_conn(&self.conn, docs, query, filters, None, k, offset)
    }

    fn doc_count(&self) -> Result<u64> {
        let mut cached = self.doc_count.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = reuse_count(*cached, Instant::now()) {
            return Ok(n);
        }
        let n = count_indexed(&self.conn)?;
        *cached = Some((Instant::now(), n));
        Ok(n)
    }

    /// [`Searcher::retrieve`] inside one document, `file_id`: at most one hit, with its
    /// best-matching passages (VFS-49 R7).
    pub fn retrieve_file(&self, query: &Query, file_id: i64, k: u32) -> Result<ContentResponse> {
        let docs = self.doc_count()?;
        retrieve_conn(
            &self.conn,
            docs,
            query,
            &SearchFilters::default(),
            Some(file_id),
            k,
            0,
        )
    }

    /// Document `file_id` as a hit with no score, whose passages are its first `n` chunks in
    /// order: what a chat limited to that document sends when its question matches none of
    /// it (VFS-49 R7). `None` when the store does not hold the document.
    pub fn document(&self, file_id: i64, n: u32) -> Result<Option<ContentHit>> {
        let row = self
            .conn
            .query_row(
                "SELECT f.id, f.name, f.kind, f.size, f.mtime, f.dir_id,
                        dc.title, dc.authors, dc.created
                 FROM files f
                 JOIN c.docs dc ON dc.file_id = f.id AND dc.dir_id = f.dir_id AND dc.name = f.name
                 WHERE f.id = ?1",
                [file_id],
                |r| {
                    Ok((
                        ContentHit {
                            id: r.get(0)?,
                            name: r.get(1)?,
                            dir: String::new(),
                            kind: FileKind::from_code(r.get(2)?),
                            size: r.get::<_, i64>(3)?.max(0) as u64,
                            modified: r.get(4)?,
                            score: 0.0,
                            snippet: String::new(),
                            highlights: Vec::new(),
                            passages: Vec::new(),
                            title: r.get(6)?,
                            authors: Vec::new(),
                            created: r.get(8)?,
                        },
                        r.get::<_, i64>(5)?,
                        r.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((mut hit, dir_id, authors)) = row else {
            return Ok(None);
        };
        hit.authors = authors
            .and_then(|a| serde_json::from_str(&a).ok())
            .unwrap_or_default();
        hit.dir = vfs_core::db::dir_abs_path(&self.conn, dir_id)?
            .to_string_lossy()
            .into_owned();
        let mut st = self.conn.prepare_cached(
            "SELECT ordinal, page, start_char, end_char FROM c.chunks
             WHERE file_id = ?1 ORDER BY ordinal LIMIT ?2",
        )?;
        hit.passages = st
            .query_map(rusqlite::params![file_id, n], |r| {
                Ok(Passage {
                    file_id,
                    ordinal: r.get(0)?,
                    page: r.get(1)?,
                    start_char: r.get::<_, i64>(2)? as usize,
                    end_char: r.get::<_, i64>(3)? as usize,
                    score: 0.0,
                    snippet: String::new(),
                    highlights: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(hit))
    }

    /// The user's text query, as the CLI and the app run it.
    pub fn search(&self, query: &SearchQuery) -> Result<ContentResponse> {
        self.retrieve(
            &Query::Text(query.text.clone()),
            &query.filters,
            query.limit,
            query.offset,
        )
    }

    /// Chunk `ordinal` of `file_id` joined with up to `before` preceding and `after`
    /// following chunks of the same document. `None` when the document or chunk is unknown.
    pub fn passage_text(
        &self,
        file_id: i64,
        ordinal: u32,
        before: u32,
        after: u32,
    ) -> Result<Option<PassageText>> {
        let first = ordinal.saturating_sub(before);
        let last = ordinal.saturating_add(after);
        let rows: Vec<(u32, usize, usize, usize, usize)> = {
            let mut st = self.conn.prepare_cached(
                "SELECT ordinal, start_byte, end_byte, start_char, end_char FROM c.chunks
                 WHERE file_id = ?1 AND ordinal BETWEEN ?2 AND ?3 ORDER BY ordinal",
            )?;
            let rows = st.query_map(rusqlite::params![file_id, first, last], |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, i64>(1)? as usize,
                    r.get::<_, i64>(2)? as usize,
                    r.get::<_, i64>(3)? as usize,
                    r.get::<_, i64>(4)? as usize,
                ))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        if !rows.iter().any(|r| r.0 == ordinal) {
            return Ok(None);
        }
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT text FROM c.docs WHERE file_id = ?1",
                [file_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let Some(blob) = blob else { return Ok(None) };
        let text = store::decompress(&blob);
        let (a, b) = (rows[0], rows[rows.len() - 1]);
        Ok(Some(PassageText {
            file_id,
            first_ordinal: a.0,
            last_ordinal: b.0,
            start_char: a.3,
            end_char: b.4,
            text: text.get(a.1..b.2).unwrap_or_default().to_string(),
        }))
    }

    /// Every chunk of `file_id` with its 1-based page (`None` for formats without pages), in
    /// ordinal order; empty when the document is unknown. VFS-28 uses it to tell which pages a
    /// passage covers.
    pub fn chunk_pages(&self, file_id: i64) -> Result<Vec<(u32, Option<u32>)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT ordinal, page FROM c.chunks WHERE file_id = ?1 ORDER BY ordinal",
        )?;
        let rows = st.query_map([file_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Each file's unread reason (R15): `None` for an indexed file and for one not processed
    /// yet (queued, or not seen).
    pub fn unread(&self, file_ids: &[i64]) -> Result<Vec<(i64, Option<UnreadReason>)>> {
        let mut st = self
            .conn
            .prepare_cached("SELECT state, reason FROM c.docs WHERE file_id = ?1")?;
        let mut out = Vec::with_capacity(file_ids.len());
        for id in file_ids {
            let row: Option<(i64, Option<String>)> = st
                .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            out.push((
                *id,
                row.and_then(|(s, r)| UnreadReason::of(s, r.as_deref())),
            ));
        }
        Ok(out)
    }

    /// Documents per unread reason (R15).
    pub fn unread_counts(&self) -> Result<BTreeMap<UnreadReason, u64>> {
        let mut st = self.conn.prepare_cached(
            "SELECT state, reason, count(*) FROM c.docs WHERE state <> ?1 GROUP BY state, reason",
        )?;
        let rows = st.query_map([ContentState::Indexed as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (state, reason, n) = row?;
            if let Some(u) = UnreadReason::of(state, reason.as_deref()) {
                *out.entry(u).or_default() += n.max(0) as u64;
            }
        }
        Ok(out)
    }
}

/// [`Searcher::search`] once (opens and closes a connection). `Err(Error::Off)` when content
/// search is not switched on.
pub fn search(index_db: &Path, query: &SearchQuery) -> Result<ContentResponse> {
    Searcher::open(index_db)?.search(query)
}

/// [`Searcher::retrieve`] once.
pub fn retrieve(
    index_db: &Path,
    query: &Query,
    filters: &SearchFilters,
    k: u32,
    offset: u32,
) -> Result<ContentResponse> {
    Searcher::open(index_db)?.retrieve(query, filters, k, offset)
}

/// `file`, when set, limits the search to that one document (VFS-49 R7).
fn retrieve_conn(
    conn: &Connection,
    docs: u64,
    query: &Query,
    filters: &SearchFilters,
    file: Option<i64>,
    k: u32,
    offset: u32,
) -> Result<ContentResponse> {
    let started = Instant::now();
    let analyzer = Analyzer::default();
    let plan = Plan::from_query(query, &analyzer);
    let empty = ContentResponse {
        hits: Vec::new(),
        partial: false,
        total: 0,
        total_capped: false,
        elapsed_ms: 0,
    };
    if plan.is_empty() {
        return Ok(ContentResponse {
            elapsed_ms: started.elapsed().as_millis() as u64,
            ..empty
        });
    }
    let Some(mut filter) = filter_clause(conn, filters)? else {
        return Ok(ContentResponse {
            elapsed_ms: started.elapsed().as_millis() as u64,
            ..empty
        });
    };
    if let Some(id) = file {
        filter.0.push_str(" AND f.id = ?");
        filter.1.push(Value::Integer(id));
    }
    let stats = Stats::load(conn, docs, &plan.keys())?;
    let mut resp = run_plan(conn, &plan, &stats, &filter, k, offset, &analyzer)?;
    // R13: two or more words that are not all in one document: any of them, marked partial.
    let words = plan.required.len() + plan.optional.len();
    if resp.total == 0 && matches!(query, Query::Text(_)) && words >= 2 {
        let fallback = plan.any_of_all();
        let stats = Stats::load(conn, docs, &fallback.keys())?;
        resp = run_plan(conn, &fallback, &stats, &filter, k, offset, &analyzer)?;
        resp.partial = true;
    }
    resp.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(resp)
}

fn run_plan(
    conn: &Connection,
    plan: &Plan,
    stats: &Stats,
    (filter_sql, filter_params): &(String, Vec<Value>),
    k: u32,
    offset: u32,
    analyzer: &Analyzer,
) -> Result<ContentResponse> {
    let mut e = exprs(plan, stats);
    let unfiltered = filter_sql.is_empty();
    if !unfiltered {
        // The filters bound the candidates; a cap before them could drop every document
        // inside the filter (R7, R14).
        e.cap = None;
    }
    // Total: documents matching the filter expression and the filters, up to TOTAL_CAP.
    let (count_sql, mut count_params) = if unfiltered {
        (
            "SELECT count(*) FROM (SELECT 1 FROM c.content_fts WHERE content_fts MATCH ? LIMIT ?)"
                .to_string(),
            vec![Value::Text(e.filter.clone())],
        )
    } else {
        let mut p = vec![Value::Text(e.filter.clone())];
        p.extend(filter_params.iter().cloned());
        (
            format!(
                "SELECT count(*) FROM (SELECT 1 FROM c.content_fts
                   JOIN files f ON f.id = content_fts.rowid
                   JOIN c.docs dc ON dc.file_id = f.id AND dc.dir_id = f.dir_id AND dc.name = f.name
                   JOIN dirs d ON d.id = f.dir_id
                 WHERE content_fts MATCH ?{filter_sql} LIMIT ?)"
            ),
            p,
        )
    };
    count_params.push(Value::Integer(TOTAL_CAP + 1));
    let total: i64 = conn
        .prepare_cached(&count_sql)?
        .query_row(rusqlite::params_from_iter(count_params.iter()), |r| {
            r.get(0)
        })?;
    // Candidates scored by the rank expression; restricted to the filter expression's
    // matches when the two differ (R14, structured `any`).
    let mut params = vec![Value::Text(e.rank.clone())];
    let bm25 = format!("bm25(content_fts, {TITLE_WEIGHT}, {BODY_WEIGHT})");
    let ranked = match (e.rank == e.filter, e.cap) {
        (true, None) => format!(
            "SELECT rowid AS id, {bm25} AS score FROM c.content_fts WHERE content_fts MATCH ?"
        ),
        // One scan: the first `cap` matches, each scored.
        (true, Some(cap)) => format!(
            "SELECT * FROM (SELECT rowid AS id, {bm25} AS score FROM c.content_fts
                            WHERE content_fts MATCH ? LIMIT {cap})"
        ),
        (false, cap) => {
            params.push(Value::Text(e.filter.clone()));
            let limit = cap.map(|c| format!(" LIMIT {c}")).unwrap_or_default();
            format!(
                "SELECT rowid AS id, {bm25} AS score FROM c.content_fts
                 WHERE content_fts MATCH ?
                   AND rowid IN (SELECT rowid FROM c.content_fts WHERE content_fts MATCH ?{limit})"
            )
        }
    };
    let limit = Value::Integer(k.clamp(1, MAX_K) as i64);
    let skip = Value::Integer(offset as i64);
    let cols = "f.id, f.name, f.kind, f.size, f.mtime, f.dir_id, t.score, dc.text,
                dc.title, dc.authors, dc.created";
    let page_sql = if unfiltered {
        // Rank inside FTS5 and join only the page: the joins over every match of a common
        // word were most of the latency (VFS-2 measurements). Ties break on file id.
        params.extend([limit, skip]);
        format!(
            "SELECT {cols}
             FROM ({ranked} ORDER BY score, id LIMIT ? OFFSET ?) t
             JOIN files f ON f.id = t.id
             JOIN c.docs dc ON dc.file_id = f.id AND dc.dir_id = f.dir_id AND dc.name = f.name
             ORDER BY t.score, t.id"
        )
    } else {
        params.extend(filter_params.iter().cloned());
        params.extend([limit, skip]);
        format!(
            "SELECT {cols}
             FROM ({ranked}) t
             JOIN files f ON f.id = t.id
             JOIN c.docs dc ON dc.file_id = f.id AND dc.dir_id = f.dir_id AND dc.name = f.name
             JOIN dirs d ON d.id = f.dir_id
             WHERE 1{filter_sql}
             ORDER BY t.score, t.id LIMIT ? OFFSET ?"
        )
    };
    type Row = (ContentHit, i64, Option<Vec<u8>>, Option<String>);
    let rows: Vec<Row> = {
        let mut st = conn.prepare_cached(&page_sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(params.iter()), |r| {
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
                title: r.get(8)?,
                authors: Vec::new(),
                created: r.get(10)?,
            };
            Ok((hit, r.get(5)?, r.get(7)?, r.get(9)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let keys = plan.keys();
    let mut hits = Vec::with_capacity(rows.len());
    let mut chunk_st = conn.prepare_cached(
        "SELECT ordinal, start_byte, end_byte, start_char, end_char, page
         FROM c.chunks WHERE file_id = ?1 ORDER BY ordinal",
    )?;
    for (mut hit, dir_id, blob, authors) in rows {
        hit.authors = authors
            .and_then(|a| serde_json::from_str(&a).ok())
            .unwrap_or_default();
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
        hit.passages = passages(hit.id, &text, &tokens, &chunk_rows, &keys, stats);
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
        partial: false,
        total: total.min(TOTAL_CAP) as u64,
        total_capped: total > TOTAL_CAP,
        elapsed_ms: 0,
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
        let keys: HashSet<&str> = q.groups.iter().flatten().map(String::as_str).collect();
        let (s, hl) = snippet(&text, &toks, &keys);
        assert!(s.starts_with("… ") && s.ends_with(" …"));
        let words: Vec<&str> = hl.iter().map(|[b, e]| &s[*b..*e]).collect();
        assert_eq!(words, ["facturas", "calefacción"]);
        assert!(s.contains("señor Muñoz pagó las facturas"));
    }

    /// VFS-2 AC-15 (R14): a `Searcher` reuses its indexed-document count for at most
    /// `DOC_COUNT_TTL`, and never a count of 0.
    #[test]
    fn document_count_reuse() {
        let t = Instant::now();
        assert_eq!(reuse_count(None, t), None);
        assert_eq!(reuse_count(Some((t, 0)), t), None);
        assert_eq!(reuse_count(Some((t, 40)), t + DOC_COUNT_TTL / 2), Some(40));
        assert_eq!(reuse_count(Some((t, 40)), t + DOC_COUNT_TTL), None);
    }

    /// VFS-2 AC-15 (R14): a count that trails the store (cached while it fills) is raised to
    /// the largest document frequency, so `idf` stays positive and a word in every document
    /// counts as common, not as rare.
    #[test]
    fn trailing_count_keeps_idf_positive() {
        let df: HashMap<String, u64> = [("factura".to_string(), 100), ("pena".to_string(), 3)]
            .into_iter()
            .collect();
        let docs = at_least_every_df(5, &df);
        assert_eq!(docs, 100);
        let stats = Stats { docs, df };
        assert!(stats.idf("factura") > 0.0);
        assert!(stats.idf("pena") > stats.idf("factura"));
        assert!(stats.common(&["factura".to_string()]));
        assert!(!stats.common(&["pena".to_string()]));
        assert_eq!(at_least_every_df(500, &stats.df), 500);
    }
}
