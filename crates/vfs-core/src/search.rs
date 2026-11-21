//! Query parsing, candidate generation, verification and paging (VFS-1 AC-10, AC-11).
//! Owned by task T2a; ranking is `rank.rs` and typo tolerance `fuzzy.rs` (T2b).
//!
//! Semantics: the query is split on whitespace into at most 8 folded terms (`\` folded to
//! `/`). A file matches when every term is a substring of its `name_key` or of its
//! directory's `path_key`; a folder (decision D1) matches when every term is in its own
//! `path_key` (the root directory: its name) and at least one term is in its name. Terms
//! already contained in a root's `path_key` are ignored for that root, unless that would
//! leave no term (S1). Folders are results; empty folders included.
//! NUL is read as whitespace; typo tolerance covers terms of 6 to 64 folded characters.
//!
//! Evaluation is candidate generation in SQL plus verification in Rust, in ranking-tier
//! order (S2) so that a full list never pushes out a better tier: names starting with the
//! first term, names holding every term (one `files_fts` intersection each, read up to
//! `NAME_READ_CAP` and ranked before the cut to `MAX_HITS`), then path matches (one
//! intersection in `file_paths_fts`, the folded relative path and name of every file), then
//! typos (near spellings in the name, then split-term candidates). A query with no hit
//! after the path tier skips the typo tiers when one probe of the path index proves that no
//! row holds every term exactly or within one edit (S8). Filters are SQL
//! predicates of every stage, so a filtered query never spends the cap on rows the filter
//! drops. Terms shorter than 3 characters cannot use the trigram index (S5): they are
//! verified in Rust, and when only short terms remain the group is a filtered scan. The
//! wall-clock deadline applies to scans; FTS-driven stages run to completion, bounded at
//! about 2 s (S3). A newer query interrupts this one through the connection's interrupt
//! handle (S6); both interruptions return what was found with `truncated_by = deadline`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection, ErrorCode, OptionalExtension, Row};

use crate::db::join_rel;
use crate::error::{Error, Result};
use crate::fold::fold_key;
use crate::fuzzy;
use crate::matching::FUZZY_TRIGGER;
use crate::rank::{self, Candidate};
use crate::types::{
    FileCopy, FileKind, MatchKind, RootStatus, SearchConfig, SearchFilters, SearchHit, SearchQuery,
    SearchResponse, SortOrder, TruncatedBy,
};

/// Hits per query after ranking; more sets `truncated_by = cap`.
const MAX_HITS: usize = 2000;
/// Candidates read by the name tiers (every term in the name). They are all ranked before
/// the list is cut to `MAX_HITS`, so a better-ranked name match is never lost to rowid
/// order while its tier has at most this many matches.
const NAME_READ_CAP: usize = 4_000;
/// Candidates read by every tier when the order is not relevance: all of them are sorted
/// before the cut to `MAX_HITS`, so the order is exact while a query has at most this many
/// matches; past it the response says `approximate_order` (VFS-12 AC-2, AC-3). 3× the cap
/// keeps every order under the AC-9 budget on 2 M files; 20,000 did not.
const SORT_READ_CAP: usize = 6_000;
/// Directories read by a directory-driven stage.
const SCAN_CAP_DIR: usize = 20_000;
/// Rows read by the fuzzy pass (best effort: reaching it does not mark truncation).
const SCAN_CAP_FUZZY: usize = 5_000;
/// Bound on work that is not subject to the configured deadline (S3).
const UNDER_CAP_BUDGET: Duration = Duration::from_secs(2);
const MAX_LIMIT: u32 = 500;
/// VM steps between deadline checks.
const PROGRESS_OPS: i32 = 1000;
/// Stored before every name in `files_fts` (schema v3): the trigrams of `NAME_START` + term
/// find the names starting with the term without positions.
pub(crate) const NAME_START: char = '\u{1}';

const FILE_COLS: &str =
    "f.id, f.name, f.name_key, f.kind, f.size, f.mtime, d.root_id, d.rel_path, d.path_key";
const DIR_COLS: &str = "d.id, d.root_id, d.rel_path, d.path_key, d.name_key, d.mtime";
/// File columns without the directory row (`Ctx::lazy_dirs`).
const LAZY_FILE_COLS: &str = "f.id, f.name, f.name_key, f.kind, f.size, f.mtime, f.dir_id";

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
    /// Per hit: the directory still to be written into `hit.dir` (see `resolve_dirs`).
    pending: Vec<Option<i64>>,
    /// Per hit, per copy: the directory still to be written into its `dir` (VFS-19).
    copy_pending: Vec<Vec<Option<i64>>>,
    truncated_by: Option<TruncatedBy>,
    approximate_order: bool,
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
    // One read transaction per search: the candidates, the kept list's `data_version` and
    // the folders read afterwards for the returned page (`resolve_dirs`) all see one
    // snapshot, so a folder the indexer deletes meanwhile is still there (VFS-4 AC-3).
    let begin = match conn.is_autocommit() {
        true => conn.execute_batch("BEGIN"),
        false => conn.execute_batch("ROLLBACK; BEGIN"),
    };
    match begin {
        Ok(()) => {}
        Err(e) if is_interrupt(&e) => {
            let start = Instant::now();
            let limit = query.limit.clamp(1, MAX_LIMIT) as usize;
            return Ok(interrupted(query.offset as usize, limit, start));
        }
        Err(e) => return Err(e.into()),
    }
    let response = search_snapshot(conn, state, config, query);
    if conn.execute_batch("COMMIT").is_err() {
        conn.execute_batch("ROLLBACK")?;
    }
    response
}

fn search_snapshot(
    conn: &mut Connection,
    state: &mut SearchState,
    config: &SearchConfig,
    query: &SearchQuery,
) -> Result<SearchResponse> {
    let start = Instant::now();
    let limit = query.limit.clamp(1, MAX_LIMIT) as usize;
    let offset = query.offset as usize;
    let crate::matching::ParsedQuery { folded, terms } = crate::matching::parse_query(&query.text);
    let f = &query.filters;
    let has_filter = !f.kinds.is_empty()
        || f.modified_from.is_some()
        || f.modified_to.is_some()
        || f.size_min.is_some()
        || f.size_max.is_some()
        || f.folder.is_some();
    if terms.is_empty() && !has_filter {
        return Ok(respond(&[], None, false, offset, limit, start));
    }

    let key = format!("{folded}\u{1f}{f:?}\u{1f}{:?}", query.sort);
    let data_version: i64 = match conn.query_row("PRAGMA data_version", [], |r| r.get(0)) {
        Ok(v) => v,
        Err(e) if is_interrupt(&e) => return Ok(interrupted(offset, limit, start)),
        Err(e) => return Err(e.into()),
    };
    if let Some(c) = state
        .cache
        .as_mut()
        .filter(|c| c.key == key && c.data_version == data_version)
    {
        let page = offset.min(c.hits.len())..(offset + limit).min(c.hits.len());
        return Ok(
            match resolve_page(conn, &mut c.hits, &mut c.pending, &mut c.copy_pending, page) {
                Ok(()) => respond(
                    &c.hits,
                    c.truncated_by,
                    c.approximate_order,
                    offset,
                    limit,
                    start,
                ),
                Err(Error::Db(e)) if is_interrupt(&e) => interrupted(offset, limit, start),
                Err(e) => return Err(e),
            },
        );
    }

    let mut run = Run {
        soft: start + config.deadline,
        hard: start + config.deadline.max(UNDER_CAP_BUDGET),
        hits: Vec::new(),
        seen_files: HashSet::new(),
        seen_dirs: HashSet::new(),
        truncated: None,
        stopped: false,
        limit: MAX_HITS,
    };
    // VFS-12: an order other than relevance reads up to `SORT_READ_CAP` candidates and sorts
    // them all before the cut. A filter-only query under relevance keeps the hit cap (its
    // scan has no index to stop early; S6 lists newest first among what it read).
    let sorted = query.sort != SortOrder::Relevance;
    let outcome = run_query(conn, &mut run, &terms, f, sorted);
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
    rank::sort_by_order(&mut candidates, query.sort);
    let approximate_order = (sorted || terms.is_empty()) && run.truncated.is_some();
    let (mut hits, mut pending, mut copy_pending) = group_copies(candidates);
    if hits.len() > MAX_HITS {
        hits.truncate(MAX_HITS);
        pending.truncate(MAX_HITS);
        copy_pending.truncate(MAX_HITS);
        run.truncated.get_or_insert(TruncatedBy::Cap);
    }
    let page = offset.min(hits.len())..(offset + limit).min(hits.len());
    match resolve_page(conn, &mut hits, &mut pending, &mut copy_pending, page) {
        Ok(()) => {}
        Err(Error::Db(e)) if is_interrupt(&e) => return Ok(interrupted(offset, limit, start)),
        Err(e) => return Err(e),
    }
    let response = respond(
        &hits,
        run.truncated,
        approximate_order,
        offset,
        limit,
        start,
    );
    // A deadline-cut list is not cached: trying again must search again (U1).
    if run.truncated != Some(TruncatedBy::Deadline) {
        state.cache = Some(Cached {
            key,
            data_version,
            hits,
            pending,
            copy_pending,
            truncated_by: run.truncated,
            approximate_order,
        });
    }
    Ok(response)
}

/// VFS-19: the ranked candidates as hits, each file folded into the first (best-ranked)
/// file with the same folded name and size as one of its `copies`; folders are never
/// grouped. Returns the hits with, per hit, its pending directory and its copies' ones.
#[allow(clippy::type_complexity)]
fn group_copies(
    candidates: Vec<Candidate>,
) -> (Vec<SearchHit>, Vec<Option<i64>>, Vec<Vec<Option<i64>>>) {
    let mut hits: Vec<SearchHit> = Vec::with_capacity(candidates.len());
    let mut pending = Vec::with_capacity(candidates.len());
    let mut copy_pending: Vec<Vec<Option<i64>>> = Vec::with_capacity(candidates.len());
    let mut first: HashMap<(String, u64), usize> = HashMap::new();
    for c in candidates {
        if !c.hit.is_dir {
            if let Some(&i) = first.get(&(c.name_key.clone(), c.hit.size)) {
                hits[i].copies.push(FileCopy {
                    id: c.hit.id,
                    dir: c.hit.dir,
                    modified: c.hit.modified,
                });
                copy_pending[i].push(c.dir_id);
                continue;
            }
            first.insert((c.name_key, c.hit.size), hits.len());
        }
        hits.push(c.hit);
        pending.push(c.dir_id);
        copy_pending.push(Vec::new());
    }
    (hits, pending, copy_pending)
}

/// Fills the folders still pending on the hits of `page` and on their copies.
fn resolve_page(
    conn: &Connection,
    hits: &mut [SearchHit],
    pending: &mut [Option<i64>],
    copy_pending: &mut [Vec<Option<i64>>],
    page: std::ops::Range<usize>,
) -> Result<()> {
    resolve_dirs(conn, &mut hits[page.clone()], &mut pending[page.clone()])?;
    // Copies often share a folder (a backup folder holds many): each is read once per page.
    let mut folders: HashMap<i64, String> = HashMap::new();
    for (hit, waiting) in hits[page.clone()].iter_mut().zip(&mut copy_pending[page]) {
        for (copy, dir) in hit.copies.iter_mut().zip(waiting.iter_mut()) {
            let Some(id) = *dir else { continue };
            copy.dir = match folders.get(&id) {
                Some(path) => path.clone(),
                None => {
                    let path = folder_of(conn, id)?;
                    folders.insert(id, path.clone());
                    path
                }
            };
            *dir = None;
        }
    }
    Ok(())
}

/// Absolute path of a directory, native separators.
fn folder_of(conn: &Connection, dir_id: i64) -> Result<String> {
    let (root, rel): (String, String) = conn
        .prepare_cached(
            "SELECT r.path, d.rel_path FROM dirs d JOIN roots r ON r.id = d.root_id \
             WHERE d.id = ?1",
        )?
        .query_row([dir_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(join_rel(&root, &rel).to_string_lossy().into_owned())
}

/// Writes the directory of the file hits read without their directory row (`dir_id`
/// pending) into `hit.dir`, for the hits of the page being returned only: ranking never
/// reads it, and a cold disk pays about one seek per directory row (VFS-1 AC-9).
fn resolve_dirs(
    conn: &Connection,
    hits: &mut [SearchHit],
    pending: &mut [Option<i64>],
) -> Result<()> {
    for (hit, dir) in hits.iter_mut().zip(pending.iter_mut()) {
        // Cleared only once written: an interrupted lookup leaves it to the next request
        // for the page, which the kept list serves.
        let Some(id) = *dir else { continue };
        hit.dir = folder_of(conn, id)?;
        *dir = None;
    }
    Ok(())
}

/// The response of a search a newer one interrupted outside the candidate loops.
fn interrupted(offset: usize, limit: usize, start: Instant) -> SearchResponse {
    respond(
        &[],
        Some(TruncatedBy::Deadline),
        false,
        offset,
        limit,
        start,
    )
}

fn respond(
    hits: &[SearchHit],
    truncated_by: Option<TruncatedBy>,
    approximate_order: bool,
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
        approximate_order,
    }
}

/// Accumulated hits and the reason the search was cut short, if any.
struct Run {
    /// Deadline for scans.
    soft: Instant,
    /// Deadline for everything else.
    hard: Instant,
    hits: Vec<Candidate>,
    seen_files: HashSet<i64>,
    seen_dirs: HashSet<i64>,
    truncated: Option<TruncatedBy>,
    /// Set by the deadline or an interruption: nothing more runs.
    stopped: bool,
    /// Candidates the current tier may bring the list up to.
    limit: usize,
}

impl Run {
    fn stop(&mut self, why: TruncatedBy) {
        self.stopped = true;
        self.truncated.get_or_insert(why);
    }

    /// Adds a verified hit unless already present; returns false once the current tier's
    /// limit is reached (more matches exist: `truncated_by = cap`).
    fn accept(&mut self, c: Candidate) -> bool {
        let seen = if c.hit.is_dir {
            &self.seen_dirs
        } else {
            &self.seen_files
        };
        if seen.contains(&c.hit.id) {
            return true;
        }
        if self.hits.len() >= self.limit {
            self.truncated.get_or_insert(TruncatedBy::Cap);
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
    F: FnMut(&mut Run, &Connection, &Row<'_>) -> rusqlite::Result<bool>,
{
    // A full list already marked `cap` cannot change: every further hit would be refused
    // (VFS-1 AC-9; a folder scan after a full file scan read every directory for nothing).
    if run.stopped || (run.hits.len() >= run.limit && run.truncated.is_some()) {
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
            if !f(run, conn, row)? {
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
        match i64::try_from(v) {
            Ok(v) => c.push(" AND f.size >= ?", v),
            // Above any stored size (`files.size` is an i64).
            Err(_) => c.sql.push_str(" AND 0"),
        }
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
    /// With a folder filter on one folder below a root: the trigrams of its folded path, a
    /// `file_paths_fts` condition every file inside it meets, added to the path tier's
    /// intersection so it reads only rows from that subtree.
    scope_grams: Option<String>,
    /// A folder filter is set.
    scoped: bool,
    /// No root is being removed: every directory belongs to a root in `roots`.
    all_roots_active: bool,
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
                copies: Vec::new(),
            },
            name_key: r.name_key,
            dir_id: None,
        })
    }

    /// Whether the files of `g` can be read without their directory row: no condition on
    /// the directory (every root searched, none being removed, no folder filter). The row
    /// is then read only to verify a term missing from the name, and for the returned page.
    fn lazy_dirs(&self, g: &Group) -> bool {
        !self.scoped && self.all_roots_active && g.root_ids.len() == self.roots.len()
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
                copies: Vec::new(),
            },
            name_key: r.name_key,
            dir_id: None,
        })
    }
}

/// Terms that tolerate a typo (`matching::tolerates_typo`).
fn is_long_for_fuzzy(t: &str) -> bool {
    crate::matching::tolerates_typo(t)
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
    sorted: bool,
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
    let all_roots: i64 = conn
        .prepare_cached("SELECT COUNT(*) FROM roots")?
        .query_row([], |r| r.get(0))?;
    let all_roots_active = all_roots == roots.len() as i64;
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

    let scope_grams = match scopes.as_deref() {
        Some([Scope { rel: Some(rel), .. }]) => {
            let key = fold_key(rel);
            (key.chars().count() >= 3).then(|| all_trigrams(&[&key]))
        }
        _ => None,
    };
    let ctx = Ctx {
        files: file_clause(f, scopes.as_deref()),
        dirs: dir_clause(f, scopes.as_deref()),
        roots: roots.into_iter().map(|r| (r.id, r)).collect(),
        scope_grams,
        scoped: scopes.is_some(),
        all_roots_active,
    };
    // Retrieval in ranking-tier order (rank.rs), so a filled list never pushes out a better
    // tier: names starting with the first term, then names holding every term (both read up
    // to NAME_READ_CAP and ranked before the cut), then path matches, then typos.
    // Date and size have no index: every extra name hit under them costs reading the rows
    // the filter drops, so their name tiers stop at the hit cap.
    let unindexed = f.modified_from.is_some()
        || f.modified_to.is_some()
        || f.size_min.is_some()
        || f.size_max.is_some();
    run.limit = if sorted {
        SORT_READ_CAP
    } else if unindexed {
        MAX_HITS
    } else {
        NAME_READ_CAP
    };
    for prefix in [true, false] {
        for g in &groups {
            name_tier(conn, run, &ctx, g, prefix)?;
        }
    }
    run.limit = if sorted { SORT_READ_CAP } else { MAX_HITS };
    if run.hits.len() < run.limit {
        for g in &groups {
            exact_group(conn, run, &ctx, g)?;
        }
    }
    // S8: with nothing found yet, both typo tiers would run to exhaustion; one probe of the
    // path index proves first whether any file or folder could match at all.
    if !terms.is_empty() && !run.stopped && run.hits.is_empty() {
        let mut possible = false;
        for g in &groups {
            if may_match(conn, g)? {
                possible = true;
                break;
            }
        }
        if !possible {
            return Ok(());
        }
    }
    if !terms.is_empty() && !run.stopped && run.hits.len() < FUZZY_TRIGGER {
        for prefix in [true, false] {
            for g in &groups {
                fuzzy_variants(conn, run, &ctx, g, prefix)?;
            }
        }
    }
    // The split-term pass reads many candidates per term: only when nothing matched at all
    // (substitutions and missing letters, which the near spellings do not cover).
    if !terms.is_empty() && !run.stopped && run.hits.is_empty() {
        fuzzy_pass(conn, run, &ctx, &groups)?;
    }
    Ok(())
}

/// False only when no file or folder can be a hit of the group in any tier (S8). A hit holds
/// in its name or path, for every term of 3 or more characters, the term itself or, within
/// one edit, one alternative of `fuzzy::candidate_parts` (each contains the term too). One
/// probe of those alternatives, ORed per term and ANDed across terms, in `file_paths_fts`
/// (as trigrams) and in `dirs_fts`, ignoring filters and roots, so a row found proves
/// nothing and none found proves there is no hit.
fn may_match(conn: &Connection, g: &Group) -> Result<bool> {
    let mut paths = Vec::new();
    let mut dirs = Vec::new();
    for t in g.terms.iter().filter(|t| t.chars().count() >= 3) {
        let alts = if is_long_for_fuzzy(t) {
            fuzzy::candidate_parts(t)
        } else {
            vec![vec![t.clone()]]
        };
        let any = |each: &dyn Fn(&Vec<String>) -> String| {
            let alts: Vec<String> = alts.iter().map(|a| format!("({})", each(a))).collect();
            format!("({})", alts.join(" OR "))
        };
        paths.push(any(&|a| all_trigrams(a)));
        dirs.push(any(&|a| {
            a.iter()
                .map(|p| phrase(p))
                .collect::<Vec<_>>()
                .join(" AND ")
        }));
    }
    if paths.is_empty() {
        return Ok(true);
    }
    for (table, expr) in [("file_paths_fts", paths), ("dirs_fts", dirs)] {
        let sql = format!("SELECT 1 FROM {table} WHERE {table} MATCH ?1 LIMIT 1");
        if conn.prepare_cached(&sql)?.exists([expr.join(" AND ")])? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Files and folders whose own name holds every term of 3 or more characters, as one FTS
/// intersection (shorter terms are verified). With `prefix`, the name must also start with
/// the group's first term (rank tiers 0 and 1).
fn name_tier(
    conn: &mut Connection,
    run: &mut Run,
    ctx: &Ctx,
    g: &Group,
    prefix: bool,
) -> Result<()> {
    let long: Vec<&String> = g.terms.iter().filter(|t| t.chars().count() >= 3).collect();
    if long.is_empty() || (prefix && long[0] != &g.terms[0]) {
        return Ok(());
    }
    let cond = NameCond {
        alts: vec![long[0].clone()],
        anchored: prefix,
        all: long[1..].iter().map(|t| t.to_string()).collect(),
        // The prefix stage already read these.
        not_prefix: (!prefix && long[0] == &g.terms[0]).then(|| long[0].clone()),
    };
    if let Some(scope) = &ctx.scope_grams {
        // Inside one folder: the files holding every term anywhere in their path within
        // that folder, as one intersection (read once, in the prefix stage, and ranked);
        // a common name would otherwise read every match in the index to keep a few.
        if prefix {
            scoped_files(
                conn,
                run,
                ctx,
                g,
                &format!("{} AND {scope}", all_trigrams(&long)),
            )?;
        }
        return name_matches(conn, run, ctx, g, &cond, false, false);
    }
    name_matches(conn, run, ctx, g, &cond, false, true)
}

/// Typo tiers: one term of at least `FUZZY_MIN_LEN` replaced by its deletions and adjacent
/// transpositions (`fuzzy::near_variants`), the other terms exact, all in the name; with
/// `prefix` only for the first term and anchored at the start of the name. Substitutions
/// and insertions are left to `fuzzy_pass`.
fn fuzzy_variants(
    conn: &mut Connection,
    run: &mut Run,
    ctx: &Ctx,
    g: &Group,
    prefix: bool,
) -> Result<()> {
    let long: Vec<&String> = g.terms.iter().filter(|t| t.chars().count() >= 3).collect();
    for (i, t) in long.iter().enumerate() {
        if prefix && (i > 0 || *t != &g.terms[0]) {
            break;
        }
        let variants = fuzzy::near_variants(t);
        if variants.is_empty() {
            continue;
        }
        let cond = NameCond {
            alts: variants,
            anchored: prefix,
            all: long
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, o)| o.to_string())
                .collect(),
            not_prefix: None,
        };
        name_matches(conn, run, ctx, g, &cond, true, true)?;
    }
    Ok(())
}

/// Files matching a `file_paths_fts` expression, with the filters in SQL, verified.
fn scoped_files(
    conn: &mut Connection,
    run: &mut Run,
    ctx: &Ctx,
    g: &Group,
    expr: &str,
) -> Result<()> {
    let Some(clause) = &ctx.files else {
        return Ok(());
    };
    let roots = g.roots_sql();
    let sql = format!(
        "SELECT {FILE_COLS} FROM (SELECT rowid AS id FROM file_paths_fts \
         WHERE file_paths_fts MATCH ?) m CROSS JOIN files f ON f.id = m.id \
         CROSS JOIN dirs d ON d.id = f.dir_id WHERE 1{roots}{}",
        clause.sql
    );
    let params = with_params(Some(Value::Text(expr.to_string())), clause);
    let deadline = run.hard;
    each_row(conn, run, &sql, &params, deadline, |run, _, row| {
        let r = FileRow::read(row)?;
        Ok(
            match verify_file(&g.terms, &r.name_key, &r.path_key, false) {
                Some(m) => ctx.file_hit(r, m).is_none_or(|c| run.accept(c)),
                None => true,
            },
        )
    })
}

/// A condition on a name: it holds one of `alts` (at its start when `anchored`) and every
/// term of `all`, and does not start with `not_prefix`. All strings have 3 or more
/// characters.
struct NameCond {
    alts: Vec<String>,
    anchored: bool,
    all: Vec<String>,
    not_prefix: Option<String>,
}

impl NameCond {
    /// FTS5 expression for `dirs_fts.name_key` (positions kept): exact.
    fn phrases(&self) -> String {
        let anchor = if self.anchored { "^" } else { "" };
        let alts: Vec<String> = self
            .alts
            .iter()
            .map(|v| format!("{anchor}{}", phrase(v)))
            .collect();
        let mut parts = vec![format!("({})", alts.join(" OR "))];
        parts.extend(self.all.iter().map(|t| phrase(t)));
        let expr = parts.join(" AND ");
        match &self.not_prefix {
            Some(p) => format!("({expr}) NOT ^{}", phrase(p)),
            None => expr,
        }
    }

    /// FTS5 expression for `files_fts` (detail=none, every name stored after `NAME_START`):
    /// covering trigrams, a superset of the names meeting the condition, in rowid order.
    fn trigrams(&self) -> String {
        let alts: Vec<String> = self
            .alts
            .iter()
            .map(|v| {
                let v = if self.anchored {
                    format!("{NAME_START}{v}")
                } else {
                    v.clone()
                };
                format!("({})", all_trigrams(&[v]))
            })
            .collect();
        let mut parts = vec![format!("({})", alts.join(" OR "))];
        if !self.all.is_empty() {
            parts.push(all_trigrams(&self.all));
        }
        parts.join(" AND ")
    }

    /// Exactly the names `phrases` matches: applied to the `trigrams` candidates, it keeps
    /// the same rows in the same order, so caps and ranking see what positions gave.
    fn holds(&self, name: &str) -> bool {
        self.alts.iter().any(|v| {
            if self.anchored {
                name.starts_with(v.as_str())
            } else {
                name.contains(v.as_str())
            }
        }) && self.all.iter().all(|t| name.contains(t.as_str()))
            && self
                .not_prefix
                .as_ref()
                .is_none_or(|p| !name.starts_with(p.as_str()))
    }
}

/// Runs a name condition on folder names and, with `files`, on file names, with the
/// filters in SQL, verifying every candidate against all terms of the group.
fn name_matches(
    conn: &mut Connection,
    run: &mut Run,
    ctx: &Ctx,
    g: &Group,
    cond: &NameCond,
    fuzzy: bool,
    files: bool,
) -> Result<()> {
    let roots = g.roots_sql();
    let deadline = run.hard;
    if let Some(clause) = ctx.files.as_ref().filter(|_| files && ctx.lazy_dirs(g)) {
        let sql = format!(
            "SELECT {LAZY_FILE_COLS} FROM files_fts CROSS JOIN files f \
             ON f.id = files_fts.rowid WHERE files_fts MATCH ?{}",
            clause.sql
        );
        let params = with_params(Some(Value::Text(cond.trigrams())), clause);
        each_row(conn, run, &sql, &params, deadline, |run, conn, row| {
            let dir_id: i64 = row.get(6)?;
            let name_key: String = row.get(2)?;
            if !cond.holds(&name_key) {
                return Ok(true);
            }
            let matched = if g.terms.iter().all(|t| name_key.contains(t.as_str())) {
                Some(MatchKind::Name)
            } else {
                let path: String = conn
                    .prepare_cached("SELECT path_key FROM dirs WHERE id = ?1")?
                    .query_row([dir_id], |r| r.get(0))?;
                verify_file(&g.terms, &name_key, &path, fuzzy)
            };
            let Some(matched) = matched else {
                return Ok(true);
            };
            Ok(run.accept(Candidate {
                hit: SearchHit {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    dir: String::new(),
                    kind: FileKind::from_code(row.get(3)?),
                    size: row.get::<_, i64>(4)?.max(0) as u64,
                    modified: row.get(5)?,
                    matched,
                    is_dir: false,
                    copies: Vec::new(),
                },
                name_key,
                dir_id: Some(dir_id),
            }))
        })?;
    } else if let Some(clause) = ctx.files.as_ref().filter(|_| files) {
        let sql = format!(
            "SELECT {FILE_COLS} FROM files_fts CROSS JOIN files f ON f.id = files_fts.rowid \
             CROSS JOIN dirs d ON d.id = f.dir_id WHERE files_fts MATCH ?{roots}{}",
            clause.sql
        );
        let params = with_params(Some(Value::Text(cond.trigrams())), clause);
        each_row(conn, run, &sql, &params, deadline, |run, _, row| {
            let r = FileRow::read(row)?;
            if !cond.holds(&r.name_key) {
                return Ok(true);
            }
            Ok(
                match verify_file(&g.terms, &r.name_key, &r.path_key, fuzzy) {
                    Some(m) => ctx.file_hit(r, m).is_none_or(|c| run.accept(c)),
                    None => true,
                },
            )
        })?;
    }
    if let Some(clause) = &ctx.dirs {
        let sql = format!(
            "SELECT {DIR_COLS} FROM (SELECT rowid AS id FROM dirs_fts WHERE dirs_fts MATCH ?) m \
             CROSS JOIN dirs d ON d.id = m.id WHERE 1{roots}{}",
            clause.sql
        );
        let params = with_params(
            Some(Value::Text(format!("name_key : ({})", cond.phrases()))),
            clause,
        );
        each_row(conn, run, &sql, &params, deadline, |run, _, row| {
            let d = DirRow::read(row)?;
            Ok(match verify_dir(&g.terms, &d, fuzzy) {
                Some(m) => ctx.dir_hit(d, m).is_none_or(|c| run.accept(c)),
                None => true,
            })
        })?;
    }
    Ok(())
}

fn with_params(first: Option<Value>, clause: &Clause) -> Vec<Value> {
    first
        .into_iter()
        .chain(clause.params.iter().cloned())
        .collect()
}

/// FTS5 expression for `file_paths_fts` (detail=none, so no phrases): for every term, the
/// trigrams that cover it without overlap plus its last one, ANDed. A superset of the files
/// whose folded path and name contain every term; candidates are verified. The overlapping
/// trigrams would add few constraints and read a doclist each: on the 2 M-file index the
/// covering set halves the time of an intersection for a few percent more candidates.
fn all_trigrams<S: AsRef<str>>(terms: &[S]) -> String {
    let mut grams: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for t in terms {
        let c: Vec<char> = t.as_ref().chars().collect();
        let Some(last) = c.len().checked_sub(3) else {
            continue;
        };
        for i in (0..=last).step_by(3).chain([last]) {
            let g = phrase(&c[i..i + 3].iter().collect::<String>());
            if seen.insert(g.clone()) {
                grams.push(g);
            }
        }
    }
    grams.join(" AND ")
}

/// Path tier (S2): files whose relative path and name together hold every term of 3 or
/// more characters, as one intersection in `file_paths_fts` (no table row is read before
/// the intersection), and folders whose path holds every term with one in their own name.
/// Groups whose terms are all shorter than 3 characters are a filtered scan instead.
fn exact_group(conn: &mut Connection, run: &mut Run, ctx: &Ctx, g: &Group) -> Result<()> {
    let long: Vec<&String> = g.terms.iter().filter(|t| t.chars().count() >= 3).collect();
    if long.is_empty() {
        return scan_group(conn, run, ctx, g);
    }
    let roots = g.roots_sql();
    // S3: FTS-driven generation is bounded by the hit cap, not by the configured deadline.
    let deadline = run.hard;
    if let Some(clause) = &ctx.files {
        // One term: its exact phrase in the folder paths, then their files (precise; the
        // position-less path index would admit names holding its trigrams anywhere).
        // Several terms: one intersection in `file_paths_fts`.
        let (sql, expr) = if long.len() == 1 {
            (
                format!(
                    "SELECT {FILE_COLS} FROM {} CROSS JOIN files f ON f.dir_id = d.id \
                     WHERE d.rel_path <> ''{roots}{}",
                    matching_dirs(SCAN_CAP_DIR),
                    clause.sql
                ),
                phrase(long[0]),
            )
        } else {
            (
                format!(
                    "SELECT {FILE_COLS} FROM (SELECT rowid AS id FROM file_paths_fts \
                     WHERE file_paths_fts MATCH ?) m CROSS JOIN files f ON f.id = m.id \
                     CROSS JOIN dirs d ON d.id = f.dir_id WHERE 1{roots}{}",
                    clause.sql
                ),
                match &ctx.scope_grams {
                    Some(grams) => format!("{} AND {grams}", all_trigrams(&long)),
                    None => all_trigrams(&long),
                },
            )
        };
        let params = with_params(Some(Value::Text(expr)), clause);
        each_row(conn, run, &sql, &params, deadline, |run, _, row| {
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
        let mut expr = long
            .iter()
            .map(|t| phrase(t))
            .collect::<Vec<_>>()
            .join(" AND ");
        // D1: a folder needs a term in its own name. When every term can use the index,
        // FTS applies that too, so the subfolders of a matching folder are not read.
        if long.len() == g.terms.len() {
            let in_name: Vec<String> = long
                .iter()
                .map(|t| format!("name_key : {}", phrase(t)))
                .collect();
            expr = format!("{expr} AND ({})", in_name.join(" OR "));
        }
        let sql = format!(
            "SELECT {DIR_COLS} FROM {} WHERE 1{roots}{}",
            matching_dirs(SCAN_CAP_DIR),
            clause.sql
        );
        let params = with_params(Some(Value::Text(expr)), clause);
        each_row(conn, run, &sql, &params, deadline, |run, _, row| {
            let d = DirRow::read(row)?;
            Ok(match verify_dir(&g.terms, &d, false) {
                Some(m) => ctx.dir_hit(d, m).is_none_or(|c| run.accept(c)),
                None => true,
            })
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
        each_row(conn, run, &sql, &params, deadline, |run, _, row| {
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
        each_row(conn, run, &sql, &params, deadline, |run, _, row| {
            let d = DirRow::read(row)?;
            Ok(match verify_dir(&g.terms, &d, false) {
                Some(m) => ctx.dir_hit(d, m).is_none_or(|c| run.accept(c)),
                None => true,
            })
        })?;
    }
    Ok(())
}

/// Where a `fuzzy_pass` statement reads its candidates.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    FileNames,
    Dirs,
    DirFiles,
}

/// One-typo pass (T2b's `fuzzy` helpers): candidates from the split-term `MATCH`
/// expressions, at most `SCAN_CAP_FUZZY` rows read in all, verified within one edit.
fn fuzzy_pass(conn: &mut Connection, run: &mut Run, ctx: &Ctx, groups: &[Group]) -> Result<()> {
    let deadline = run.soft;
    let mut budget = SCAN_CAP_FUZZY;
    for g in groups {
        let roots = g.roots_sql();
        for t in g.terms.iter().filter(|t| is_long_for_fuzzy(t)) {
            for (expr, parts) in fuzzy::candidate_queries(t)
                .into_iter()
                .zip(fuzzy::candidate_parts(t))
            {
                if budget == 0 || run.stopped {
                    return Ok(());
                }
                // `files_fts` keeps no positions: its candidates are the parts' trigrams,
                // narrowed in Rust to the names holding every part (what `expr` matches).
                let grams = all_trigrams(&parts);
                let matching_dirs = matching_dirs(budget);
                let queries = [
                    ctx.files.as_ref().map(|c| {
                        (
                            format!(
                                "SELECT {FILE_COLS} FROM files_fts JOIN files f ON f.id = files_fts.rowid \
                                 JOIN dirs d ON d.id = f.dir_id WHERE files_fts MATCH ?{roots}{}",
                                c.sql
                            ),
                            c,
                            Source::FileNames,
                        )
                    }),
                    ctx.dirs.as_ref().map(|c| {
                        (
                            format!(
                                "SELECT {DIR_COLS} FROM {matching_dirs} WHERE 1{roots}{}",
                                c.sql
                            ),
                            c,
                            Source::Dirs,
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
                            Source::DirFiles,
                        )
                    }),
                ];
                for (sql, clause, source) in queries.into_iter().flatten() {
                    let text = match source {
                        Source::FileNames => grams.clone(),
                        Source::Dirs | Source::DirFiles => expr.clone(),
                    };
                    let params = with_params(Some(Value::Text(text)), clause);
                    each_row(conn, run, &sql, &params, deadline, |run, _, row| {
                        if source == Source::FileNames {
                            let name_key: String = row.get(2)?;
                            if !parts.iter().all(|p| name_key.contains(p.as_str())) {
                                return Ok(true);
                            }
                        }
                        budget = budget.saturating_sub(1);
                        let hit = if source == Source::Dirs {
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

    use rusqlite::params;

    use super::*;
    use crate::db::{self, ingest};

    /// VFS-1 AC-10 (S6), VFS-4 AC-3: a newer query interrupting a running one makes it
    /// return what it has with `truncated_by = deadline` instead of an error, also while it
    /// reads the folders of the returned page, and the connection stays usable.
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
        // A text query, its folders read for the returned page only (VFS-4 AC-3): an
        // interruption at any point, the folder lookups included, is a response, never an
        // error, and never leaves the search's read transaction open.
        let text = SearchQuery {
            text: "archivo".into(),
            filters: SearchFilters::default(),
            limit: 10,
            offset: 0,
            sort: Default::default(),
        };
        for _ in 0..200 {
            search(&mut conn, &mut SearchState::default(), &config, &text).unwrap();
            assert!(conn.is_autocommit());
        }
        // Filter-only, matching nothing: a full scan that only an interruption can cut.
        let mut q = SearchQuery {
            text: String::new(),
            filters: SearchFilters {
                size_min: Some(1 << 40),
                ..SearchFilters::default()
            },
            limit: 10,
            offset: 0,
            sort: Default::default(),
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

    /// VFS-1 AC-9, AC-10 (S8): the no-hit probe says "impossible" only when no file or
    /// folder holds every term exactly or within one edit, in its name or its path.
    #[test]
    fn no_hit_probe_never_rules_out_a_possible_hit() {
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
        let sub = ingest::insert_dir(&tx, root, top, "Notaría Pérez", 0).unwrap();
        let file = ingest::NewFile {
            name: "escritura 2019.pdf",
            size: 1,
            mtime: 1,
            lossy: false,
        };
        ingest::insert_file(&tx, sub, &file).unwrap();
        ingest::insert_dir(&tx, root, top, "Viaje Cádiz", 0).unwrap();
        tx.commit().unwrap();
        drop(w);

        let conn = db::open_ro(&path).unwrap();
        let possible = |q: &str| {
            let g = Group {
                terms: q.split(' ').map(str::to_string).collect(),
                root_ids: vec![root],
            };
            may_match(&conn, &g).unwrap()
        };
        // Exact, split between folder and name, typos, folder only, short terms only.
        for q in [
            "escritura",
            "notaria escritura",
            "escirtura perez",
            "notaira 2019",
            "viaje cadiz",
            "viaje cadizz",
            "es 20",
        ] {
            assert!(possible(q), "{q}");
        }
        // Terms under 6 characters (`cadzi`) get no typo tolerance.
        for q in [
            "xilofono marciano",
            "escritura cadiz",
            "zzqxj",
            "notaria marciano",
            "cadzi",
        ] {
            assert!(!possible(q), "{q}");
        }
    }

    /// VFS-4 AC-3: a folder lookup that fails (an interruption, here a missing row) keeps
    /// the hits not yet resolved pending, so the next request for the page, served from the
    /// kept list, still fills their folder; resolved hits are not looked up again.
    #[test]
    fn unresolved_folders_stay_pending_after_a_failed_lookup() {
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
        let sub = ingest::insert_dir(&tx, root, top, "Gestoría", 0).unwrap();
        tx.commit().unwrap();
        let hit = || SearchHit {
            id: 1,
            name: "a.pdf".into(),
            dir: String::new(),
            kind: FileKind::Document,
            size: 0,
            modified: 0,
            matched: MatchKind::Name,
            is_dir: false,
            copies: Vec::new(),
        };
        let mut hits = vec![hit(), hit(), hit()];
        let mut pending = vec![Some(top), Some(sub + 1000), Some(sub)];
        assert!(resolve_dirs(&w, &mut hits, &mut pending).is_err());
        assert_eq!(hits[0].dir, "/datos");
        assert_eq!(pending, vec![None, Some(sub + 1000), Some(sub)]);
        pending[1] = Some(sub);
        resolve_dirs(&w, &mut hits, &mut pending).unwrap();
        assert_eq!(pending, vec![None, None, None]);
        let expected = join_rel("/datos", "Gestoría")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            (hits[1].dir.as_str(), hits[2].dir.as_str()),
            (expected.as_str(), expected.as_str())
        );
    }

    /// VFS-4 AC-5 (R5): once the list is full and marked truncated by the cap, no further
    /// statement is even prepared (here an invalid one), and a list not yet capped still
    /// runs its statements.
    #[test]
    fn a_full_capped_list_runs_no_further_statement() {
        let mut conn = Connection::open_in_memory().unwrap();
        let now = Instant::now();
        let mut run = Run {
            soft: now + Duration::from_secs(60),
            hard: now + Duration::from_secs(60),
            hits: Vec::new(),
            seen_files: HashSet::new(),
            seen_dirs: HashSet::new(),
            truncated: Some(TruncatedBy::Cap),
            stopped: false,
            limit: 0,
        };
        let deadline = run.hard;
        let mut calls = 0;
        each_row(&mut conn, &mut run, "NOT SQL", &[], deadline, |_, _, _| {
            calls += 1;
            Ok(true)
        })
        .unwrap();
        assert_eq!(calls, 0);
        run.truncated = None;
        assert!(
            each_row(&mut conn, &mut run, "NOT SQL", &[], deadline, |_, _, _| Ok(
                true
            ))
            .is_err()
        );
    }

    /// VFS-4 AC-1: on `files_fts` (detail=none, names after `NAME_START`) a name condition's
    /// trigram candidates narrowed by `holds` are exactly, in the same order, the rows its
    /// phrase expression matches with positions, for prefixes, excluded prefixes, typo
    /// alternatives and the typo pass's split parts; so caps and ranking see the same rows.
    #[test]
    fn name_condition_matches_exactly_what_phrases_matched() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE full USING fts5(n, tokenize='trigram case_sensitive 1');
             CREATE VIRTUAL TABLE none USING fts5(n, content='', columnsize=0, detail='none',
               tokenize='trigram case_sensitive 1');",
        )
        .unwrap();
        let names = [
            "notaria perez.pdf",
            "perez notaria.pdf",
            "xnotaria.txt",
            "notxaria.txt",
            "aria not.txt",
            "notarianotaria",
            "nota",
            "notari",
            "ntaria",
            "escritura 2019 notaria",
            "aaaa",
            "aaa",
            "ab",
            "riano tar ia",
            "\"notaria\".doc",
        ];
        for (i, n) in (0_i64..).zip(names) {
            conn.execute("INSERT INTO full(rowid, n) VALUES (?1, ?2)", params![i, n])
                .unwrap();
            conn.execute(
                "INSERT INTO none(rowid, n) VALUES (?1, ?2)",
                params![i, format!("{NAME_START}{n}")],
            )
            .unwrap();
        }
        let ids = |table: &str, expr: &str| -> Vec<i64> {
            let sql = format!("SELECT rowid FROM {table} WHERE {table} MATCH ?1 ORDER BY rowid");
            let mut stmt = conn.prepare(&sql).unwrap();
            let rows = stmt.query_map([expr], |r| r.get(0)).unwrap();
            rows.collect::<rusqlite::Result<_>>().unwrap()
        };
        let strings = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let conds = [
            NameCond {
                alts: strings(&["notaria"]),
                anchored: true,
                all: vec![],
                not_prefix: None,
            },
            NameCond {
                alts: strings(&["notaria"]),
                anchored: false,
                all: vec![],
                not_prefix: None,
            },
            NameCond {
                alts: strings(&["notaria"]),
                anchored: false,
                all: strings(&["perez"]),
                not_prefix: Some("notaria".into()),
            },
            NameCond {
                alts: strings(&["aaa"]),
                anchored: true,
                all: vec![],
                not_prefix: None,
            },
            NameCond {
                alts: strings(&["\"notaria\""]),
                anchored: false,
                all: vec![],
                not_prefix: None,
            },
            NameCond {
                alts: fuzzy::near_variants("notaira"),
                anchored: true,
                all: vec![],
                not_prefix: None,
            },
            NameCond {
                alts: fuzzy::near_variants("notaira"),
                anchored: false,
                all: strings(&["2019"]),
                not_prefix: None,
            },
        ];
        for c in &conds {
            let kept: Vec<i64> = ids("none", &c.trigrams())
                .into_iter()
                .filter(|&i| c.holds(names[i as usize]))
                .collect();
            assert_eq!(kept, ids("full", &c.phrases()), "{}", c.phrases());
        }
        for (expr, parts) in fuzzy::candidate_queries("notaira")
            .into_iter()
            .zip(fuzzy::candidate_parts("notaira"))
        {
            let kept: Vec<i64> = ids("none", &all_trigrams(&parts))
                .into_iter()
                .filter(|&i| parts.iter().all(|p| names[i as usize].contains(p.as_str())))
                .collect();
            assert_eq!(kept, ids("full", &expr), "{expr}");
        }
    }

    /// VFS-1 AC-9: the path-index expression covers every character of every term, so a row
    /// holding all the terms always matches it.
    #[test]
    fn covering_trigrams_cover_every_character() {
        for t in ["abc", "abcd", "notaria", "gestoria/ano", "ñandúes"] {
            let expr = all_trigrams(&[t]);
            let chars: Vec<char> = t.chars().collect();
            let mut covered = vec![false; chars.len()];
            for g in expr.split(" AND ") {
                let g: Vec<char> = g.trim_matches('"').chars().collect();
                let at = chars.windows(3).position(|w| w == g.as_slice()).unwrap();
                covered[at..at + 3].iter_mut().for_each(|c| *c = true);
            }
            assert!(covered.iter().all(|&c| c), "{t}: {expr}");
        }
        assert!(all_trigrams(&["ab"]).is_empty());
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
