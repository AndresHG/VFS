//! Query parsing, candidate generation, verification and paging (VFS-1 AC-10, AC-11).
//! Owned by task T2a; ranking is `rank.rs` and typo tolerance `fuzzy.rs` (T2b).
//!
//! Semantics: the query is split on whitespace into at most 8 folded terms (`\` folded to
//! `/`). A file matches when every term is a substring of its `name_key` or of its
//! directory's `path_key`; a folder (decision D1) matches when every term is in its own
//! `path_key` (the root directory: its name) and at least one term is in its name. Terms
//! already contained in a root's `path_key` are ignored for that root, unless that would
//! leave no term (S1). Folders are results; empty folders included.
//!
//! Evaluation is candidate generation in SQL plus verification in Rust. Per group of roots
//! with the same effective terms, the generator term is the one with the fewest matches in
//! a capped count probe of both FTS tables (S2). Candidates: files whose name contains it
//! (G1), and folders whose path contains it with their files (G2). Terms shorter than 3
//! characters cannot use the trigram index (S5): they are verified in Rust, and when only
//! short terms remain the group is a filtered scan. The wall-clock deadline applies to scans
//! and to generators over their cap; a generator under its cap runs to completion, bounded
//! at about 2 s (S3). A newer query interrupts this one through the connection's interrupt
//! handle (S6); both interruptions return what was found with `truncated_by = deadline`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection, ErrorCode, OptionalExtension, Row};

use crate::db::join_rel;
use crate::error::{Error, Result};
use crate::fold::fold_key;
use crate::fuzzy::{self, FUZZY_MIN_LEN};
use crate::rank::{self, Candidate};
use crate::types::{
    FileKind, MatchKind, RootStatus, SearchConfig, SearchFilters, SearchHit, SearchQuery,
    SearchResponse, TruncatedBy,
};

/// Accepted hits per query; one more sets `truncated_by = cap`.
const MAX_HITS: usize = 2000;
const MAX_TERMS: usize = 8;
/// Directories read by the directory-driven generator.
const SCAN_CAP_DIR: usize = 20_000;
/// Rows read by the fuzzy pass (best effort: reaching it does not mark truncation).
const SCAN_CAP_FUZZY: usize = 5_000;
/// A generator term with more matches than this is "over cap" (S3): the configured deadline
/// applies to it.
const PROBE_CAP: i64 = 5_000;
/// Count probes (S2) stop here. Counting FTS rowids is cheap, so the counts stay exact far
/// beyond `PROBE_CAP` and still tell a common generator term from a very common one.
const COUNT_CAP: i64 = 50_000;
/// The fuzzy pass runs only when the exact pass found fewer hits.
const FUZZY_TRIGGER: usize = 20;
/// Bound on work that is not subject to the configured deadline (S3).
const UNDER_CAP_BUDGET: Duration = Duration::from_secs(2);
const MAX_LIMIT: u32 = 500;
/// VM steps between deadline checks.
const PROGRESS_OPS: i32 = 1000;

const FILE_COLS: &str =
    "f.id, f.name, f.name_key, f.kind, f.size, f.mtime, d.root_id, d.rel_path, d.path_key";
const DIR_COLS: &str = "d.id, d.root_id, d.rel_path, d.path_key, d.name_key, d.mtime";

/// State kept between searches on the search connection: the last ranked list keyed by
/// (query, filters, `PRAGMA data_version`), so paging is consistent (S7).
#[derive(Default)]
pub(crate) struct SearchState {
    cache: Option<Cached>,
}

struct Cached {
    key: String,
    data_version: i64,
    hits: Vec<SearchHit>,
    truncated_by: Option<TruncatedBy>,
}

/// Runs one query on the dedicated search connection. The facade calls
/// `InterruptHandle::interrupt` on this connection before each new query, so a running
/// query sees `SQLITE_INTERRUPT` and returns what it has with `truncated_by` set.
pub(crate) fn search(
    conn: &mut Connection,
    state: &mut SearchState,
    config: &SearchConfig,
    query: &SearchQuery,
) -> Result<SearchResponse> {
    let start = Instant::now();
    let limit = query.limit.clamp(1, MAX_LIMIT) as usize;
    let offset = query.offset as usize;
    let folded = fold_key(&query.text.replace('\\', "/"));
    let folded = folded.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut terms: Vec<String> = Vec::new();
    for t in folded.split(' ').filter(|t| !t.is_empty()) {
        if terms.len() == MAX_TERMS {
            break;
        }
        if !terms.iter().any(|x| x == t) {
            terms.push(t.to_string());
        }
    }
    let f = &query.filters;
    let has_filter = !f.kinds.is_empty()
        || f.modified_from.is_some()
        || f.modified_to.is_some()
        || f.size_min.is_some()
        || f.size_max.is_some()
        || f.folder.is_some();
    if terms.is_empty() && !has_filter {
        return Ok(respond(&[], None, offset, limit, start));
    }

    let key = format!("{folded}\u{1f}{f:?}");
    let data_version: i64 = match conn.query_row("PRAGMA data_version", [], |r| r.get(0)) {
        Ok(v) => v,
        Err(e) if is_interrupt(&e) => {
            return Ok(respond(
                &[],
                Some(TruncatedBy::Deadline),
                offset,
                limit,
                start,
            ))
        }
        Err(e) => return Err(e.into()),
    };
    if let Some(c) = state
        .cache
        .as_ref()
        .filter(|c| c.key == key && c.data_version == data_version)
    {
        return Ok(respond(&c.hits, c.truncated_by, offset, limit, start));
    }

    let mut run = Run {
        soft: start + config.deadline,
        hard: start + config.deadline.max(UNDER_CAP_BUDGET),
        hits: Vec::new(),
        seen_files: HashSet::new(),
        seen_dirs: HashSet::new(),
        truncated: None,
        stopped: false,
    };
    let outcome = run_query(conn, &mut run, &terms, f);
    conn.progress_handler(0, None::<fn() -> bool>)?;
    match outcome {
        // Statements outside `each_row` (roots, folder lookup) can be interrupted too.
        Err(Error::Db(e)) if is_interrupt(&e) => run.stop(TruncatedBy::Deadline),
        other => other?,
    }

    let mut candidates = run.hits;
    if terms.is_empty() {
        // S6: filter-only queries list newest first.
        candidates.sort_by(|a, b| {
            (b.hit.modified, a.hit.is_dir, a.hit.id).cmp(&(a.hit.modified, b.hit.is_dir, b.hit.id))
        });
    } else if candidates.len() > 1 {
        rank::rank(&mut candidates, &folded, &terms);
    }
    let hits: Vec<SearchHit> = candidates.into_iter().map(|c| c.hit).collect();
    let response = respond(&hits, run.truncated, offset, limit, start);
    // A deadline-cut list is not cached: trying again must search again (U1).
    if run.truncated != Some(TruncatedBy::Deadline) {
        state.cache = Some(Cached {
            key,
            data_version,
            hits,
            truncated_by: run.truncated,
        });
    }
    Ok(response)
}

fn respond(
    hits: &[SearchHit],
    truncated_by: Option<TruncatedBy>,
    offset: usize,
    limit: usize,
    start: Instant,
) -> SearchResponse {
    SearchResponse {
        hits: hits.iter().skip(offset).take(limit).cloned().collect(),
        total: hits.len() as u32,
        truncated: truncated_by.is_some(),
        truncated_by,
        elapsed_ms: start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
    }
}

/// Accumulated hits and the reason the search was cut short, if any.
struct Run {
    /// Deadline for scans and over-cap generators.
    soft: Instant,
    /// Deadline for everything else.
    hard: Instant,
    hits: Vec<Candidate>,
    seen_files: HashSet<i64>,
    seen_dirs: HashSet<i64>,
    truncated: Option<TruncatedBy>,
    stopped: bool,
}

impl Run {
    fn stop(&mut self, why: TruncatedBy) {
        self.stopped = true;
        self.truncated.get_or_insert(why);
    }

    /// Adds a verified hit unless already present; returns false once the hit cap stops the
    /// search.
    fn accept(&mut self, c: Candidate) -> bool {
        let seen = if c.hit.is_dir {
            &self.seen_dirs
        } else {
            &self.seen_files
        };
        if seen.contains(&c.hit.id) {
            return true;
        }
        if self.hits.len() >= MAX_HITS {
            self.stop(TruncatedBy::Cap);
            return false;
        }
        if c.hit.is_dir {
            self.seen_dirs.insert(c.hit.id);
        } else {
            self.seen_files.insert(c.hit.id);
        }
        self.hits.push(c);
        true
    }
}

/// Runs `sql` with a progress-handler `deadline`, feeding rows to `f` until it returns
/// false. An interruption (deadline or a newer query) stops the run with `Deadline`.
fn each_row<F>(
    conn: &mut Connection,
    run: &mut Run,
    sql: &str,
    params: &[Value],
    deadline: Instant,
    mut f: F,
) -> Result<()>
where
    F: FnMut(&mut Run, &Row<'_>) -> rusqlite::Result<bool>,
{
    if run.stopped {
        return Ok(());
    }
    conn.progress_handler(PROGRESS_OPS, Some(move || Instant::now() >= deadline))?;
    let conn: &Connection = conn;
    let outcome = (|| -> rusqlite::Result<()> {
        let mut stmt = conn.prepare_cached(sql)?;
        let mut rows = stmt.query(params_from_iter(params))?;
        while let Some(row) = rows.next()? {
            if Instant::now() >= deadline {
                run.stop(TruncatedBy::Deadline);
                break;
            }
            if !f(run, row)? {
                break;
            }
        }
        Ok(())
    })();
    match outcome {
        Ok(()) => Ok(()),
        Err(e) if is_interrupt(&e) => {
            run.stop(TruncatedBy::Deadline);
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn is_interrupt(e: &rusqlite::Error) -> bool {
    e.sqlite_error_code() == Some(ErrorCode::OperationInterrupted)
}

struct RootRow {
    id: i64,
    path: String,
    path_key: String,
}

/// Where a folder filter lets hits come from.
struct Scope {
    root_id: i64,
    /// `None`: the whole root.
    rel: Option<String>,
    /// The root directory itself may be a folder hit (the filter folder contains the root).
    include_self: bool,
}

/// SQL predicate fragment (starting with ` AND`) and its parameters, in order.
#[derive(Default)]
struct Clause {
    sql: String,
    params: Vec<Value>,
}

impl Clause {
    fn push(&mut self, sql: &str, value: i64) {
        self.sql.push_str(sql);
        self.params.push(Value::Integer(value));
    }
}

/// Filters pushed into SQL for file rows; `None` when the kind filter excludes files.
fn file_clause(f: &SearchFilters, scopes: Option<&[Scope]>) -> Option<Clause> {
    let kinds: Vec<String> = f
        .kinds
        .iter()
        .filter(|k| **k != FileKind::Folder)
        .map(|k| k.code().to_string())
        .collect();
    if !f.kinds.is_empty() && kinds.is_empty() {
        return None;
    }
    let mut c = Clause::default();
    if !kinds.is_empty() {
        c.sql
            .push_str(&format!(" AND f.kind IN ({})", kinds.join(",")));
    }
    if let Some(v) = f.modified_from {
        c.push(" AND f.mtime >= ?", v);
    }
    if let Some(v) = f.modified_to {
        c.push(" AND f.mtime < ?", v);
    }
    if let Some(v) = f.size_min {
        c.push(" AND f.size >= ?", i64::try_from(v).unwrap_or(i64::MAX));
    }
    if let Some(v) = f.size_max {
        c.push(" AND f.size <= ?", i64::try_from(v).unwrap_or(i64::MAX));
    }
    if let Some(scopes) = scopes {
        let mut parts = Vec::new();
        for s in scopes {
            match &s.rel {
                // Descendants of `p` are exactly the rel_paths in [p + '/', p + '0').
                Some(p) => {
                    parts.push("(d.root_id = ? AND (d.rel_path = ? OR (d.rel_path >= ? AND d.rel_path < ?)))");
                    c.params.extend([
                        Value::Integer(s.root_id),
                        Value::Text(p.clone()),
                        Value::Text(format!("{p}/")),
                        Value::Text(format!("{p}0")),
                    ]);
                }
                None => {
                    parts.push("d.root_id = ?");
                    c.params.push(Value::Integer(s.root_id));
                }
            }
        }
        c.sql.push_str(&format!(" AND ({})", parts.join(" OR ")));
    }
    Some(c)
}

/// Filters pushed into SQL for folder rows; `None` when folders are excluded: by the kind
/// filter, or by a size filter (folders have no size). The date filter uses the folder's
/// own modification time; the filter folder itself is not a hit.
fn dir_clause(f: &SearchFilters, scopes: Option<&[Scope]>) -> Option<Clause> {
    let kinds_ok = f.kinds.is_empty() || f.kinds.contains(&FileKind::Folder);
    if !kinds_ok || f.size_min.is_some() || f.size_max.is_some() {
        return None;
    }
    let mut c = Clause::default();
    if let Some(v) = f.modified_from {
        c.push(" AND d.mtime >= ?", v);
    }
    if let Some(v) = f.modified_to {
        c.push(" AND d.mtime < ?", v);
    }
    if let Some(scopes) = scopes {
        let mut parts = Vec::new();
        for s in scopes {
            match (&s.rel, s.include_self) {
                (Some(p), _) => {
                    parts.push("(d.root_id = ? AND d.rel_path >= ? AND d.rel_path < ?)");
                    c.params.extend([
                        Value::Integer(s.root_id),
                        Value::Text(format!("{p}/")),
                        Value::Text(format!("{p}0")),
                    ]);
                }
                (None, false) => {
                    parts.push("(d.root_id = ? AND d.rel_path <> '')");
                    c.params.push(Value::Integer(s.root_id));
                }
                (None, true) => {
                    parts.push("d.root_id = ?");
                    c.params.push(Value::Integer(s.root_id));
                }
            }
        }
        c.sql.push_str(&format!(" AND ({})", parts.join(" OR ")));
    }
    Some(c)
}

/// Maps the folder filter (an absolute path) to scopes: the root it lies in plus the
/// directory below, or every root it contains. Compared on folded keys, so case,
/// accents, NFC/NFD and `\` vs `/` do not matter.
fn resolve_scopes(conn: &Connection, roots: &[RootRow], folder: &str) -> Result<Vec<Scope>> {
    let raw = match folder.trim_end_matches(['/', '\\']) {
        "" => folder,
        trimmed => trimmed,
    };
    let fkey = fold_key(&raw.replace('\\', "/"));
    let fbase = fkey.trim_end_matches('/');
    let mut scopes = Vec::new();
    for r in roots {
        let rbase = r.path_key.trim_end_matches('/');
        if fbase == rbase {
            scopes.push(Scope {
                root_id: r.id,
                rel: None,
                include_self: false,
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
                    include_self: false,
                });
            }
        } else if rbase
            .strip_prefix(fbase)
            .is_some_and(|rest| rest.starts_with('/'))
        {
            scopes.push(Scope {
                root_id: r.id,
                rel: None,
                include_self: true,
            });
        }
    }
    Ok(scopes)
}

/// Roots that share the same effective terms (S1).
struct Group {
    terms: Vec<String>,
    root_ids: Vec<i64>,
}

impl Group {
    /// The unary `+` keeps SQLite from driving the query by the `(root_id, rel_path)`
    /// index, which would read every directory of the root.
    fn roots_sql(&self) -> String {
        let ids: Vec<String> = self.root_ids.iter().map(i64::to_string).collect();
        format!(" AND +d.root_id IN ({})", ids.join(","))
    }
}

struct Ctx {
    roots: HashMap<i64, RootRow>,
    files: Option<Clause>,
    dirs: Option<Clause>,
}

struct FileRow {
    id: i64,
    name: String,
    name_key: String,
    kind: i64,
    size: i64,
    mtime: i64,
    root_id: i64,
    rel_path: String,
    path_key: String,
}

impl FileRow {
    fn read(r: &Row<'_>) -> rusqlite::Result<FileRow> {
        Ok(FileRow {
            id: r.get(0)?,
            name: r.get(1)?,
            name_key: r.get(2)?,
            kind: r.get(3)?,
            size: r.get(4)?,
            mtime: r.get(5)?,
            root_id: r.get(6)?,
            rel_path: r.get(7)?,
            path_key: r.get(8)?,
        })
    }
}

struct DirRow {
    id: i64,
    root_id: i64,
    rel_path: String,
    path_key: String,
    name_key: String,
    mtime: i64,
}

impl DirRow {
    fn read(r: &Row<'_>) -> rusqlite::Result<DirRow> {
        Ok(DirRow {
            id: r.get(0)?,
            root_id: r.get(1)?,
            rel_path: r.get(2)?,
            path_key: r.get(3)?,
            name_key: r.get(4)?,
            mtime: r.get(5)?,
        })
    }
}

impl Ctx {
    fn file_hit(&self, r: FileRow, matched: MatchKind) -> Option<Candidate> {
        let root = self.roots.get(&r.root_id)?;
        Some(Candidate {
            hit: SearchHit {
                id: r.id,
                name: r.name,
                dir: join_rel(&root.path, &r.rel_path)
                    .to_string_lossy()
                    .into_owned(),
                kind: FileKind::from_code(r.kind),
                size: r.size.max(0) as u64,
                modified: r.mtime,
                matched,
                is_dir: false,
            },
            name_key: r.name_key,
        })
    }

    fn dir_hit(&self, r: DirRow, matched: MatchKind) -> Option<Candidate> {
        let root = self.roots.get(&r.root_id)?;
        let (name, dir) = if r.rel_path.is_empty() {
            let p = Path::new(&root.path);
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.path.clone());
            let dir = p
                .parent()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.path.clone());
            (name, dir)
        } else {
            let (parent, name) = r.rel_path.rsplit_once('/').unwrap_or(("", &r.rel_path));
            (
                name.to_string(),
                join_rel(&root.path, parent).to_string_lossy().into_owned(),
            )
        };
        Some(Candidate {
            hit: SearchHit {
                id: r.id,
                name,
                dir,
                kind: FileKind::Folder,
                size: 0,
                modified: r.mtime,
                matched,
                is_dir: true,
            },
            name_key: r.name_key,
        })
    }
}

fn is_long_for_fuzzy(t: &str) -> bool {
    t.chars().count() >= FUZZY_MIN_LEN
}

/// File verification: every term in the name or the directory path; with `fuzzy`, terms
/// long enough may instead be within one edit (`MatchKind::Fuzzy`).
fn verify_file(terms: &[String], name: &str, path: &str, fuzzy: bool) -> Option<MatchKind> {
    let mut in_path = false;
    let mut used_fuzzy = false;
    for t in terms {
        if name.contains(t.as_str()) {
        } else if path.contains(t.as_str()) {
            in_path = true;
        } else if fuzzy
            && is_long_for_fuzzy(t)
            && (fuzzy::contains_within_one(name, t) || fuzzy::contains_within_one(path, t))
        {
            used_fuzzy = true;
        } else {
            return None;
        }
    }
    Some(if used_fuzzy {
        MatchKind::Fuzzy
    } else if in_path {
        MatchKind::Path
    } else {
        MatchKind::Name
    })
}

/// Folder verification (D1): every term in the folder's own path (the root directory: its
/// name) and, unless the query is filter-only, at least one term in its name.
fn verify_dir(terms: &[String], d: &DirRow, fuzzy: bool) -> Option<MatchKind> {
    let hay = if d.rel_path.is_empty() {
        &d.name_key
    } else {
        &d.path_key
    };
    let near = |h: &str, t: &str| fuzzy && is_long_for_fuzzy(t) && fuzzy::contains_within_one(h, t);
    let mut used_fuzzy = false;
    for t in terms {
        if hay.contains(t.as_str()) {
        } else if near(hay, t) {
            used_fuzzy = true;
        } else {
            return None;
        }
    }
    if terms.is_empty() {
        return Some(MatchKind::Name);
    }
    let any_in_name = terms
        .iter()
        .any(|t| d.name_key.contains(t.as_str()) || near(&d.name_key, t));
    if !any_in_name {
        return None;
    }
    Some(if used_fuzzy {
        MatchKind::Fuzzy
    } else if terms.iter().all(|t| d.name_key.contains(t.as_str())) {
        MatchKind::Name
    } else {
        MatchKind::Path
    })
}

/// `FROM` source of the directories whose path matches the next `?` (at most `cap`), as
/// `d`. `CROSS JOIN` fixes the join order: the FTS match drives and each directory is read
/// by rowid. Left to itself, SQLite estimates the subquery at `cap` rows and reads every
/// directory of the root through the `(root_id, rel_path)` index instead (hundreds of
/// milliseconds at 2 M files, VFS-1 AC-9).
fn matching_dirs(cap: usize) -> String {
    format!(
        "(SELECT rowid AS id FROM dirs_fts WHERE dirs_fts MATCH ? LIMIT {cap}) m \
         CROSS JOIN dirs d ON d.id = m.id"
    )
}

/// FTS5 phrase for a folded term: quoted, inner quotes doubled.
fn phrase(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

fn run_query(
    conn: &mut Connection,
    run: &mut Run,
    terms: &[String],
    f: &SearchFilters,
) -> Result<()> {
    let roots: Vec<RootRow> = {
        let mut stmt = conn.prepare_cached(
            "SELECT id, path, path_key FROM roots WHERE status <> ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([RootStatus::Removing.code()], |r| {
            Ok(RootRow {
                id: r.get(0)?,
                path: r.get(1)?,
                path_key: r.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let scopes = match &f.folder {
        Some(folder) => Some(resolve_scopes(conn, &roots, folder)?),
        None => None,
    };
    let in_scope = |id: i64| {
        scopes
            .as_ref()
            .is_none_or(|s| s.iter().any(|s| s.root_id == id))
    };

    // S1: per root, drop terms its own path already satisfies, unless none would remain.
    let mut by_terms: BTreeMap<Vec<String>, Vec<i64>> = BTreeMap::new();
    for r in roots.iter().filter(|r| in_scope(r.id)) {
        let mut effective: Vec<String> = terms
            .iter()
            .filter(|t| !r.path_key.contains(t.as_str()))
            .cloned()
            .collect();
        if effective.is_empty() {
            effective = terms.to_vec();
        }
        by_terms.entry(effective).or_default().push(r.id);
    }
    let groups: Vec<Group> = by_terms
        .into_iter()
        .map(|(terms, root_ids)| Group { terms, root_ids })
        .collect();

    let ctx = Ctx {
        files: file_clause(f, scopes.as_deref()),
        dirs: dir_clause(f, scopes.as_deref()),
        roots: roots.into_iter().map(|r| (r.id, r)).collect(),
    };
    for g in &groups {
        exact_group(conn, run, &ctx, g)?;
    }
    if !terms.is_empty() && !run.stopped && run.hits.len() < FUZZY_TRIGGER {
        fuzzy_pass(conn, run, &ctx, &groups)?;
    }
    Ok(())
}

fn with_params(first: Option<Value>, clause: &Clause) -> Vec<Value> {
    first
        .into_iter()
        .chain(clause.params.iter().cloned())
        .collect()
}

/// Number of rows of `table` matching `phrase`, counted up to `COUNT_CAP` + 1 (S2).
fn probe(conn: &mut Connection, run: &mut Run, table: &str, phrase: &str) -> Result<i64> {
    let sql = format!(
        "SELECT count(*) FROM (SELECT 1 FROM {table} WHERE {table} MATCH ? LIMIT {})",
        COUNT_CAP + 1
    );
    let mut n = 0;
    let deadline = run.hard;
    each_row(
        conn,
        run,
        &sql,
        &[Value::Text(phrase.to_string())],
        deadline,
        |_, row| {
            n = row.get(0)?;
            Ok(false)
        },
    )?;
    Ok(n)
}

fn exact_group(conn: &mut Connection, run: &mut Run, ctx: &Ctx, g: &Group) -> Result<()> {
    let long: Vec<&String> = g.terms.iter().filter(|t| t.chars().count() >= 3).collect();
    if long.is_empty() {
        return scan_group(conn, run, ctx, g);
    }
    let roots = g.roots_sql();

    // S2: the generator term is the one with the fewest matches; ties go to the longer.
    let mut best: Option<(i64, i64, &String)> = None;
    for t in long {
        let p = phrase(t);
        let fc = probe(conn, run, "files_fts", &p)?;
        let dc = probe(conn, run, "dirs_fts", &p)?;
        let better = match best {
            None => true,
            Some((bf, bd, bt)) => {
                (fc + dc, std::cmp::Reverse(t.len())) < (bf + bd, std::cmp::Reverse(bt.len()))
            }
        };
        if better {
            best = Some((fc, dc, t));
        }
    }
    let Some((fc, dc, gen)) = best else {
        return Ok(());
    };
    if run.stopped {
        return Ok(());
    }
    let p = phrase(gen);

    // G1: files whose name contains the generator term.
    if let Some(clause) = &ctx.files {
        let sql = format!(
            "SELECT {FILE_COLS} FROM files_fts JOIN files f ON f.id = files_fts.rowid \
             JOIN dirs d ON d.id = f.dir_id WHERE files_fts MATCH ?{roots}{}",
            clause.sql
        );
        let deadline = if fc > PROBE_CAP { run.soft } else { run.hard };
        let params = with_params(Some(Value::Text(p.clone())), clause);
        each_row(conn, run, &sql, &params, deadline, |run, row| {
            let r = FileRow::read(row)?;
            Ok(
                match verify_file(&g.terms, &r.name_key, &r.path_key, false) {
                    Some(m) => ctx.file_hit(r, m).is_none_or(|c| run.accept(c)),
                    None => true,
                },
            )
        })?;
    }

    // G2: folders whose path contains the generator term, then their files.
    let deadline = if dc > PROBE_CAP { run.soft } else { run.hard };
    if dc > PROBE_CAP {
        let sql = format!(
            "SELECT count(*) FROM (SELECT 1 FROM dirs_fts WHERE dirs_fts MATCH ? LIMIT {})",
            SCAN_CAP_DIR + 1
        );
        let mut n = 0usize;
        each_row(
            conn,
            run,
            &sql,
            &[Value::Text(p.clone())],
            deadline,
            |_, row| {
                n = row.get::<_, i64>(0)? as usize;
                Ok(false)
            },
        )?;
        if n > SCAN_CAP_DIR {
            run.truncated.get_or_insert(TruncatedBy::Cap);
        }
    }
    let matching_dirs = matching_dirs(SCAN_CAP_DIR);
    if let Some(clause) = &ctx.dirs {
        let sql = format!(
            "SELECT {DIR_COLS} FROM {matching_dirs} WHERE 1{roots}{}",
            clause.sql
        );
        let params = with_params(Some(Value::Text(p.clone())), clause);
        each_row(conn, run, &sql, &params, deadline, |run, row| {
            let d = DirRow::read(row)?;
            Ok(match verify_dir(&g.terms, &d, false) {
                Some(m) => ctx.dir_hit(d, m).is_none_or(|c| run.accept(c)),
                None => true,
            })
        })?;
    }
    if let Some(clause) = &ctx.files {
        // The root directory's path is empty: its files are covered by G1.
        let sql = format!(
            "SELECT {FILE_COLS} FROM {matching_dirs} CROSS JOIN files f ON f.dir_id = d.id \
             WHERE d.rel_path <> ''{roots}{}",
            clause.sql
        );
        let params = with_params(Some(Value::Text(p)), clause);
        each_row(conn, run, &sql, &params, deadline, |run, row| {
            let r = FileRow::read(row)?;
            Ok(
                match verify_file(&g.terms, &r.name_key, &r.path_key, false) {
                    Some(m) => ctx.file_hit(r, m).is_none_or(|c| run.accept(c)),
                    None => true,
                },
            )
        })?;
    }
    Ok(())
}

/// Filter-only queries and groups whose terms are all shorter than 3 characters: a
/// filtered scan under the deadline (S5), terms checked with `instr` in SQL.
fn scan_group(conn: &mut Connection, run: &mut Run, ctx: &Ctx, g: &Group) -> Result<()> {
    let roots = g.roots_sql();
    let deadline = run.soft;
    if let Some(clause) = &ctx.files {
        let mut sql = format!(
            "SELECT {FILE_COLS} FROM files f JOIN dirs d ON d.id = f.dir_id WHERE 1{roots}{}",
            clause.sql
        );
        let mut params = with_params(None, clause);
        for t in &g.terms {
            sql.push_str(" AND (instr(f.name_key, ?) > 0 OR instr(d.path_key, ?) > 0)");
            params.extend([Value::Text(t.clone()), Value::Text(t.clone())]);
        }
        each_row(conn, run, &sql, &params, deadline, |run, row| {
            let r = FileRow::read(row)?;
            Ok(
                match verify_file(&g.terms, &r.name_key, &r.path_key, false) {
                    Some(m) => ctx.file_hit(r, m).is_none_or(|c| run.accept(c)),
                    None => true,
                },
            )
        })?;
    }
    if let Some(clause) = &ctx.dirs {
        let mut sql = format!("SELECT {DIR_COLS} FROM dirs d WHERE 1{roots}{}", clause.sql);
        let mut params = with_params(None, clause);
        for t in &g.terms {
            sql.push_str(
                " AND instr(CASE WHEN d.rel_path = '' THEN d.name_key ELSE d.path_key END, ?) > 0",
            );
            params.push(Value::Text(t.clone()));
        }
        each_row(conn, run, &sql, &params, deadline, |run, row| {
            let d = DirRow::read(row)?;
            Ok(match verify_dir(&g.terms, &d, false) {
                Some(m) => ctx.dir_hit(d, m).is_none_or(|c| run.accept(c)),
                None => true,
            })
        })?;
    }
    Ok(())
}

/// One-typo pass (T2b's `fuzzy` helpers): candidates from the split-term `MATCH`
/// expressions, at most `SCAN_CAP_FUZZY` rows read in all, verified within one edit.
fn fuzzy_pass(conn: &mut Connection, run: &mut Run, ctx: &Ctx, groups: &[Group]) -> Result<()> {
    let deadline = run.soft;
    let mut budget = SCAN_CAP_FUZZY;
    for g in groups {
        let roots = g.roots_sql();
        for t in g.terms.iter().filter(|t| is_long_for_fuzzy(t)) {
            for expr in fuzzy::candidate_queries(t) {
                if budget == 0 || run.stopped {
                    return Ok(());
                }
                let matching_dirs = matching_dirs(budget);
                let queries = [
                    ctx.files.as_ref().map(|c| {
                        (
                            format!(
                                "SELECT {FILE_COLS} FROM files_fts JOIN files f ON f.id = files_fts.rowid \
                                 JOIN dirs d ON d.id = f.dir_id WHERE files_fts MATCH ?{roots}{} LIMIT {budget}",
                                c.sql
                            ),
                            c,
                            false,
                        )
                    }),
                    ctx.dirs.as_ref().map(|c| {
                        (
                            format!(
                                "SELECT {DIR_COLS} FROM {matching_dirs} WHERE 1{roots}{}",
                                c.sql
                            ),
                            c,
                            true,
                        )
                    }),
                    ctx.files.as_ref().map(|c| {
                        (
                            format!(
                                "SELECT {FILE_COLS} FROM {matching_dirs} CROSS JOIN files f \
                                 ON f.dir_id = d.id WHERE d.rel_path <> ''{roots}{} LIMIT {budget}",
                                c.sql
                            ),
                            c,
                            false,
                        )
                    }),
                ];
                for (sql, clause, is_dir) in queries.into_iter().flatten() {
                    let params = with_params(Some(Value::Text(expr.clone())), clause);
                    each_row(conn, run, &sql, &params, deadline, |run, row| {
                        budget = budget.saturating_sub(1);
                        let hit = if is_dir {
                            let d = DirRow::read(row)?;
                            verify_dir(&g.terms, &d, true).and_then(|m| ctx.dir_hit(d, m))
                        } else {
                            let r = FileRow::read(row)?;
                            verify_file(&g.terms, &r.name_key, &r.path_key, true)
                                .and_then(|m| ctx.file_hit(r, m))
                        };
                        Ok(hit.is_none_or(|c| run.accept(c)) && budget > 0)
                    })?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::db::{self, ingest};

    /// VFS-1 AC-10 (S6): a newer query interrupting a running one makes it return what it
    /// has with `truncated_by = deadline` instead of an error, and the connection stays
    /// usable for the next query.
    #[test]
    fn interrupted_query_returns_partial_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let mut w = db::open_rw(&path).unwrap();
        let tx = w.transaction().unwrap();
        let root = ingest::insert_root(&tx, "/", "/datos").unwrap();
        let top: i64 = tx
            .query_row("SELECT id FROM dirs WHERE root_id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        for i in 0..20_000 {
            let name = format!("archivo {i}.pdf");
            let file = ingest::NewFile {
                name: &name,
                size: 10,
                mtime: 1,
                lossy: false,
            };
            ingest::insert_file(&tx, top, &file).unwrap();
        }
        tx.commit().unwrap();

        let mut conn = db::open_ro(&path).unwrap();
        let handle = conn.get_interrupt_handle();
        let done = Arc::new(AtomicBool::new(false));
        let spammer = {
            let done = done.clone();
            std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    handle.interrupt();
                    std::thread::sleep(Duration::from_micros(200));
                }
            })
        };
        let config = SearchConfig {
            deadline: Duration::from_secs(60),
        };
        // Filter-only, matching nothing: a full scan that only an interruption can cut.
        let mut q = SearchQuery {
            text: String::new(),
            filters: SearchFilters {
                size_min: Some(1 << 40),
                ..SearchFilters::default()
            },
            limit: 10,
            offset: 0,
        };
        let mut state = SearchState::default();
        let mut cut = None;
        for _ in 0..50 {
            let r = search(&mut conn, &mut state, &config, &q).unwrap();
            if r.truncated_by.is_some() {
                cut = Some(r);
                break;
            }
            // A complete result was cached; drop it so the next try searches again.
            state = SearchState::default();
        }
        done.store(true, Ordering::Relaxed);
        spammer.join().unwrap();
        let cut = cut.expect("the scan was never interrupted");
        assert_eq!(cut.truncated_by, Some(TruncatedBy::Deadline));
        assert!(cut.truncated);
        assert!(cut.hits.is_empty());

        q.filters.size_min = Some(10);
        q.filters.kinds = vec![FileKind::Document];
        let r = search(&mut conn, &mut state, &config, &q).unwrap();
        assert_eq!(r.truncated_by, Some(TruncatedBy::Cap));
        assert_eq!(r.total as usize, MAX_HITS);
    }

    /// VFS-1 AC-9: directory-driven generation reads only the directories the FTS match
    /// returns, by rowid, never every directory of the root through the `(root_id,
    /// rel_path)` index (what SQLite picked before, hundreds of ms at 2 M files).
    #[test]
    fn matching_dirs_is_driven_by_the_fts_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let mut w = db::open_rw(&path).unwrap();
        let tx = w.transaction().unwrap();
        let root = ingest::insert_root(&tx, "/", "/datos").unwrap();
        let top: i64 = tx
            .query_row("SELECT id FROM dirs WHERE root_id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        for i in 0..200 {
            ingest::insert_dir(&tx, root, top, &format!("carpeta {i}"), 0).unwrap();
        }
        tx.commit().unwrap();
        let conn = db::open_ro(&path).unwrap();
        let group = Group {
            terms: vec!["carpeta".into()],
            root_ids: vec![root],
        };
        let roots = group.roots_sql();
        let m = matching_dirs(SCAN_CAP_DIR);
        for sql in [
            format!("SELECT {DIR_COLS} FROM {m} WHERE 1{roots}"),
            format!(
                "SELECT {FILE_COLS} FROM {m} CROSS JOIN files f ON f.dir_id = d.id \
                 WHERE d.rel_path <> ''{roots}"
            ),
        ] {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let plan: Vec<String> = stmt
                .query_map(["\"carpeta\""], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert!(
                plan.iter()
                    .any(|l| l.contains("SEARCH d USING INTEGER PRIMARY KEY")),
                "{plan:?}"
            );
            assert!(
                !plan.iter().any(|l| l.contains("sqlite_autoindex_dirs_1")),
                "{plan:?}"
            );
        }
    }

    /// VFS-1 AC-10, decision D1: file hits need every term in the name or the directory
    /// path; folder hits need every term in their path and one in their own name.
    #[test]
    fn verification_rules() {
        let t = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            verify_file(&t(&["nota", "2019"]), "nota.pdf", "ano 2019", false),
            Some(MatchKind::Path)
        );
        assert_eq!(
            verify_file(&t(&["nota"]), "nota.pdf", "", false),
            Some(MatchKind::Name)
        );
        assert_eq!(verify_file(&t(&["zz"]), "nota.pdf", "a", false), None);
        let d = DirRow {
            id: 1,
            root_id: 1,
            rel_path: "Gestoría/Año 2019".into(),
            path_key: "gestoria/ano 2019".into(),
            name_key: "ano 2019".into(),
            mtime: 0,
        };
        assert_eq!(
            verify_dir(&t(&["gestoria", "2019"]), &d, false),
            Some(MatchKind::Path)
        );
        assert_eq!(verify_dir(&t(&["2019"]), &d, false), Some(MatchKind::Name));
        // A subfolder of a matching folder is not a hit unless a term is in its own name.
        assert_eq!(verify_dir(&t(&["gestoria"]), &d, false), None);
        assert_eq!(verify_dir(&[], &d, false), Some(MatchKind::Name));
        assert_eq!(phrase("a\"b"), "\"a\"\"b\"");
    }
}
