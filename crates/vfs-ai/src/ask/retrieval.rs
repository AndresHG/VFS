//! Retrieval for a question (VFS-28 R6.2–R6.4): the rewrite's filters checked and intersected
//! with the filters on screen, up to three `retrieve` rounds, and the passages to send, picked
//! round-robin across documents, expanded with their neighbours and merged.

use chrono::{Local, NaiveDate, TimeZone};
use vfs_content::{ContentHit, PassageText, Query, StructuredQuery};
use vfs_core::{fold_key, FileKind, SearchFilters};

use crate::provider::Rewrite;

/// Documents asked of each `retrieve` round (R6.3).
pub const ROUND_K: u32 = 20;
/// Fewer documents than this in a round runs the next round (R6.3).
pub const MIN_DOCS: usize = 5;
/// Passages picked before merging (R6.4).
pub const MAX_PASSAGES: usize = 10;

/// What the ask flow needs from the archive: content retrieval (VFS-2 R15) and the folders of
/// the name index. [`super::LocalCorpus`] is the app's; tests use fakes.
pub trait Corpus {
    /// Ranked documents for `query` under `filters`, at most `k`.
    fn retrieve(
        &self,
        query: &Query,
        filters: &SearchFilters,
        k: u32,
    ) -> vfs_content::Result<Vec<ContentHit>>;

    /// Chunk `ordinal` of `file_id` with `before` and `after` neighbours (VFS-2 R15).
    fn passage_text(
        &self,
        file_id: i64,
        ordinal: u32,
        before: u32,
        after: u32,
    ) -> vfs_content::Result<Option<PassageText>>;

    /// Absolute paths of the indexed folders, under the user's folders, that `hint` names: a
    /// folder name or the end of a folder path, compared after the accent fold (R6.2).
    fn folders_named(&self, hint: &str) -> vfs_content::Result<Vec<String>>;
}

/// The filters a question retrieves under (R6.2): the screen's, narrowed by what the rewrite
/// asked for wherever the two intersect. Returns the filters and whether the rewrite added any.
pub fn question_filters(
    rewrite: &Rewrite,
    screen: &SearchFilters,
    corpus: &dyn Corpus,
) -> vfs_content::Result<(SearchFilters, bool)> {
    let mut f = screen.clone();
    let mut narrowed = false;

    if let Some(kind) = rewrite.kind.as_deref().and_then(kind_of) {
        if screen.kinds.is_empty() || screen.kinds.contains(&kind) {
            narrowed |= f.kinds != [kind];
            f.kinds = vec![kind];
        }
    }

    let from = rewrite
        .date_from
        .as_deref()
        .and_then(|d| day_start(d, false));
    let to = rewrite.date_to.as_deref().and_then(|d| day_start(d, true));
    let new_from = max_opt(screen.modified_from, from);
    let new_to = min_opt(screen.modified_to, to);
    let empty = matches!((new_from, new_to), (Some(a), Some(b)) if a >= b);
    if !empty && (new_from, new_to) != (screen.modified_from, screen.modified_to) {
        f.modified_from = new_from;
        f.modified_to = new_to;
        narrowed = true;
    }

    if let Some(hint) = rewrite
        .folder_hint
        .as_deref()
        .filter(|h| !h.trim().is_empty())
    {
        let found = corpus.folders_named(hint.trim())?;
        // A hint naming no folder, or several, is dropped: the filter takes one folder.
        if let [folder] = found.as_slice() {
            match screen.folder.as_deref() {
                None => {
                    f.folder = Some(folder.clone());
                    narrowed = true;
                }
                Some(on_screen) if is_within(folder, on_screen) => {
                    narrowed |= path_key(folder) != path_key(on_screen);
                    f.folder = Some(folder.clone());
                }
                // The screen's folder is inside the hint, or the two are disjoint: the
                // intersection is the screen's folder, or nothing; keep the screen's.
                Some(_) => {}
            }
        }
    }
    Ok((f, narrowed))
}

/// The rewrite's `kind`, read as a [`FileKind`] by its own serde name; `other` and `folder`
/// name no kind a question can ask for.
fn kind_of(kind: &str) -> Option<FileKind> {
    let value = serde_json::Value::String(kind.trim().to_ascii_lowercase());
    match serde_json::from_value(value).ok()? {
        FileKind::Other | FileKind::Folder => None,
        k => Some(k),
    }
}

/// Local midnight of `YYYY-MM-DD`, `YYYY-MM` or `YYYY`, as Unix seconds. As an end date
/// (`end`), a month or a year means the start of the next one.
fn day_start(text: &str, end: bool) -> Option<i64> {
    let parts: Vec<&str> = text.trim().split('-').collect();
    let num = |i: usize| parts.get(i).and_then(|p| p.parse::<u32>().ok());
    let year = parts.first()?.parse::<i32>().ok()?;
    let date = match parts.len() {
        1 if end => NaiveDate::from_ymd_opt(year + 1, 1, 1),
        1 => NaiveDate::from_ymd_opt(year, 1, 1),
        2 => {
            let m = num(1)?;
            match (end, m) {
                (true, 12) => NaiveDate::from_ymd_opt(year + 1, 1, 1),
                (true, _) => NaiveDate::from_ymd_opt(year, m + 1, 1),
                (false, _) => NaiveDate::from_ymd_opt(year, m, 1),
            }
        }
        3 => NaiveDate::from_ymd_opt(year, num(1)?, num(2)?),
        _ => None,
    }?;
    let midnight = date.and_hms_opt(0, 0, 0)?;
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map(|t| t.timestamp())
}

fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    a.max(b)
}

fn min_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// A folder path as a comparable key: folded, `/` separators, no trailing separator.
fn path_key(path: &str) -> String {
    fold_key(&path.replace('\\', "/"))
        .trim_end_matches('/')
        .to_owned()
}

/// `inner` is `outer` or a folder below it.
pub(crate) fn is_within(inner: &str, outer: &str) -> bool {
    let (i, o) = (path_key(inner), path_key(outer));
    i == o || i.strip_prefix(&o).is_some_and(|rest| rest.starts_with('/'))
}

/// The documents of the last round, and how many each round found (R6.3).
#[derive(Debug, Clone)]
pub struct Retrieved {
    pub hits: Vec<ContentHit>,
    pub rounds: Vec<usize>,
}

/// Runs the retrieval rounds of R6.3: `{all: keywords, phrases}`, then `{any: keywords +
/// synonyms}`, then that again without the rewrite's filters. A round runs only if the one
/// before found fewer than [`MIN_DOCS`]; the third is skipped when the rewrite added no
/// filter, since it would repeat the second.
pub fn retrieve_rounds(
    corpus: &dyn Corpus,
    rewrite: &Rewrite,
    screen: &SearchFilters,
    filters: &SearchFilters,
    narrowed: bool,
) -> vfs_content::Result<Retrieved> {
    let strict = Query::Structured(StructuredQuery {
        all: rewrite.keywords.clone(),
        any: Vec::new(),
        phrases: rewrite.phrases.clone(),
    });
    let mut any: Vec<String> = rewrite.keywords.clone();
    for s in &rewrite.synonyms {
        if !any.contains(s) {
            any.push(s.clone());
        }
    }
    let loose = Query::Structured(StructuredQuery {
        all: Vec::new(),
        any,
        phrases: Vec::new(),
    });

    let mut rounds = Vec::new();
    let mut hits = corpus.retrieve(&strict, filters, ROUND_K)?;
    rounds.push(hits.len());
    if hits.len() < MIN_DOCS {
        hits = corpus.retrieve(&loose, filters, ROUND_K)?;
        rounds.push(hits.len());
        if hits.len() < MIN_DOCS && narrowed {
            hits = corpus.retrieve(&loose, screen, ROUND_K)?;
            rounds.push(hits.len());
        }
    }
    Ok(Retrieved { hits, rounds })
}

/// A passage chosen to send: a run of chunks of one document (R6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub file_id: i64,
    pub name: String,
    /// Absolute path of the containing folder. Never sent (D11).
    pub dir: String,
    /// Pages of the chunks picked for it, ascending; empty when the format has none.
    pub pages: Vec<u32>,
    pub first_ordinal: u32,
    pub last_ordinal: u32,
    pub text: String,
}

impl Source {
    /// What the model sees as the passage's title (D11): the file name and pages, no folder.
    pub fn title(&self) -> String {
        match self.pages.as_slice() {
            [] => self.name.clone(),
            [p] => format!("{} · p. {p}", self.name),
            [a, .., b] => format!("{} · pp. {a}–{b}", self.name),
        }
    }
}

/// Picks up to [`MAX_PASSAGES`] passages from `hits`: the best of each document in document
/// order, then each one's second best, and so on. Each is expanded with its previous and next
/// chunk, and passages of one document whose chunk ranges overlap or touch are merged into one,
/// at the place of the first (R6.4).
pub fn pick_passages(corpus: &dyn Corpus, hits: &[ContentHit]) -> vfs_content::Result<Vec<Source>> {
    let mut picked: Vec<(&ContentHit, &vfs_content::Passage)> = Vec::new();
    let depth = hits.iter().map(|h| h.passages.len()).max().unwrap_or(0);
    'pick: for i in 0..depth {
        for h in hits {
            if let Some(p) = h.passages.get(i) {
                picked.push((h, p));
                if picked.len() == MAX_PASSAGES {
                    break 'pick;
                }
            }
        }
    }

    let mut sources: Vec<Source> = Vec::new();
    for (hit, p) in picked {
        let Some(t) = corpus.passage_text(hit.id, p.ordinal, 1, 1)? else {
            continue;
        };
        sources.push(Source {
            file_id: hit.id,
            name: hit.name.clone(),
            dir: hit.dir.clone(),
            pages: p.page.into_iter().collect(),
            first_ordinal: t.first_ordinal,
            last_ordinal: t.last_ordinal,
            text: t.text,
        });
    }

    // Merge until no two runs of one document overlap or touch; a merge can join a third run.
    let mut merged_any = vec![false; sources.len()];
    loop {
        let mut joined = None;
        'find: for a in 0..sources.len() {
            for b in a + 1..sources.len() {
                let (x, y) = (&sources[a], &sources[b]);
                if x.file_id == y.file_id
                    && x.first_ordinal <= y.last_ordinal + 1
                    && y.first_ordinal <= x.last_ordinal + 1
                {
                    joined = Some((a, b));
                    break 'find;
                }
            }
        }
        let Some((a, b)) = joined else { break };
        let y = sources.remove(b);
        merged_any.remove(b);
        let x = &mut sources[a];
        x.first_ordinal = x.first_ordinal.min(y.first_ordinal);
        x.last_ordinal = x.last_ordinal.max(y.last_ordinal);
        x.pages.extend(y.pages);
        merged_any[a] = true;
    }
    for (s, merged) in sources.iter_mut().zip(merged_any) {
        s.pages.sort_unstable();
        s.pages.dedup();
        if merged {
            let span = s.last_ordinal - s.first_ordinal;
            if let Some(t) = corpus.passage_text(s.file_id, s.first_ordinal, 0, span)? {
                s.text = t.text;
            }
        }
    }
    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_parse_as_local_midnights() {
        let jan = day_start("2021-01-01", false).unwrap();
        assert_eq!(day_start("2021", false), Some(jan));
        assert_eq!(day_start("2020", true), Some(jan));
        assert_eq!(day_start("2020-12", true), Some(jan));
        assert!(day_start("2021-02-30", false).is_none());
        assert!(day_start("ayer", false).is_none());
    }

    #[test]
    fn folder_containment_folds_case_accents_and_separators() {
        assert!(is_within("/Docs/Casa/Facturas", "/docs/casa"));
        assert!(is_within("C:\\Docs\\Año", "c:/docs/ano/"));
        assert!(!is_within("/Docs/Casas", "/Docs/Casa"));
        assert!(!is_within("/Docs", "/Docs/Casa"));
    }
}
