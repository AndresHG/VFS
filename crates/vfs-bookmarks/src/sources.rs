//! One search over several sources (VFS-6 "Matching and ranking", "Sorting"): files from the
//! core, bookmarks and bookmark folders from [`Bookmarks`]. By relevance both lists merge by
//! the shared tiers, each keeping its own order, the file first on a tie; inside a bookmark
//! folder with no words, Firefox's own order. Newest first and by source are the other
//! orders. The merged list is paged like VFS-1 S7.

use std::cmp::Reverse;
use std::time::Instant;

use vfs_core::matching::{name_tier, parse_query};
use vfs_core::{fold_key, Result, SearchHit, SearchQuery, TruncatedBy, Vfs};

use crate::search::{MergeKey, Ranked};
use crate::types::{SortOrder, Source, SourceHit, SourceOptions, SourcesResponse};
use crate::Bookmarks;

/// Largest page the core returns (VFS-1 `search`); the merge reads files in pages this big.
const CORE_PAGE: u32 = 500;

/// Whether `sources` (empty = all) includes `s`.
pub fn covers(sources: &[Source], s: Source) -> bool {
    sources.is_empty() || sources.contains(&s)
}

/// Runs `query` over the chosen sources. Bookmarks are searched only when `bookmarks` is
/// given and its setting is on (R1); a bookmark folder filter leaves files out, as a file
/// folder filter leaves bookmarks out.
pub fn search_sources(
    vfs: &Vfs,
    bookmarks: Option<&Bookmarks>,
    query: &SearchQuery,
    options: &SourceOptions,
) -> Result<SourcesResponse> {
    let start = Instant::now();
    let inside = options.bookmark_folder;
    let with_files = covers(&options.sources, Source::Files) && inside.is_none();
    let marks = bookmarks
        .filter(|b| covers(&options.sources, Source::Bookmarks) && b.is_enabled())
        .map(|b| b.search(query, inside));
    let limit = query.limit.clamp(1, CORE_PAGE) as usize;
    let offset = query.offset as usize;
    let Some((marks, marks_cut)) = marks else {
        if !with_files {
            return Ok(respond(Vec::new(), 0, None, start));
        }
        if options.sort == SortOrder::Relevance || options.sort == SortOrder::Source {
            // Files alone: the core's own list and paging.
            let r = vfs.search(query)?;
            return Ok(SourcesResponse {
                hits: r.hits.into_iter().map(SourceHit::Files).collect(),
                total: r.total,
                truncated: r.truncated,
                truncated_by: r.truncated_by,
                elapsed_ms: r.elapsed_ms,
            });
        }
        let (files, total, cut) = file_hits(vfs, query, usize::MAX)?;
        let mut all: Vec<SourceHit> = files.into_iter().map(SourceHit::Files).collect();
        sort_newest(&mut all);
        let hits = all.into_iter().skip(offset).take(limit).collect();
        return Ok(respond(hits, total, cut, start));
    };

    // Newest first needs every file hit; the other orders only the first `want`.
    let want = if options.sort == SortOrder::Newest {
        usize::MAX
    } else {
        offset + limit
    };
    let (files, total_files, files_cut) = if with_files {
        file_hits(vfs, query, want)?
    } else {
        (Vec::new(), 0, None)
    };
    let total = total_files + marks.len() as u32;
    let cut = files_cut.or(marks_cut.then_some(TruncatedBy::Cap));

    let merged: Vec<SourceHit> = match options.sort {
        SortOrder::Source => files
            .into_iter()
            .map(SourceHit::Files)
            .chain(marks.into_iter().map(|m| SourceHit::Bookmarks(m.hit)))
            .skip(offset)
            .take(limit)
            .collect(),
        SortOrder::Newest => {
            let mut all: Vec<SourceHit> = files
                .into_iter()
                .map(SourceHit::Files)
                .chain(marks.into_iter().map(|m| SourceHit::Bookmarks(m.hit)))
                .collect();
            sort_newest(&mut all);
            all.into_iter().skip(offset).take(limit).collect()
        }
        SortOrder::Relevance => {
            let parsed = parse_query(&query.text);
            let mut marks = marks;
            if parsed.terms.is_empty() && inside.is_some() {
                // A folder's contents, no words: as Firefox shows them.
                marks.sort_by(|a, b| a.tree_key.cmp(&b.tree_key));
            }
            merge_by_relevance(files, marks, parsed.terms.is_empty(), &|h: &SearchHit| {
                let key = fold_key(&h.name);
                (
                    name_tier(&key, h.matched, &parsed),
                    key.chars().count(),
                    Reverse(h.modified),
                )
            })
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect()
        }
    };
    Ok(respond(merged, total, cut, start))
}

/// Up to `want` hits of the file list from the core, its total and its truncation.
fn file_hits(
    vfs: &Vfs,
    query: &SearchQuery,
    want: usize,
) -> Result<(Vec<SearchHit>, u32, Option<TruncatedBy>)> {
    let mut files = Vec::new();
    loop {
        let page = SearchQuery {
            limit: CORE_PAGE,
            offset: files.len() as u32,
            ..query.clone()
        };
        let r = vfs.search(&page)?;
        let got = r.hits.len();
        files.extend(r.hits);
        if got == 0 || files.len() >= want || files.len() >= r.total as usize {
            return Ok((files, r.total, r.truncated_by));
        }
    }
}

/// Both ranked lists merged: each keeps its order, the better merge key goes first, the file
/// on a tie. With no words the keys are newest first (VFS-1 S6).
fn merge_by_relevance(
    files: Vec<SearchHit>,
    marks: Vec<Ranked>,
    no_terms: bool,
    file_key: &dyn Fn(&SearchHit) -> MergeKey,
) -> Vec<SourceHit> {
    let mut out = Vec::with_capacity(files.len() + marks.len());
    let mut files = files.into_iter().peekable();
    let mut marks = marks.into_iter().peekable();
    loop {
        let take_file = match (files.peek(), marks.peek()) {
            (Some(f), Some(b)) => {
                let fk = if no_terms {
                    (0, 0, Reverse(f.modified))
                } else {
                    file_key(f)
                };
                fk <= b.key
            }
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if take_file {
            out.extend(files.next().map(SourceHit::Files));
        } else {
            out.extend(marks.next().map(|m| SourceHit::Bookmarks(m.hit)));
        }
    }
    out
}

/// Newest first: a file's modification date, a bookmark's or folder's date saved; on a tie
/// the order before sorting stays.
fn sort_newest(all: &mut [SourceHit]) {
    all.sort_by_key(|h| {
        Reverse(match h {
            SourceHit::Files(f) => f.modified,
            SourceHit::Bookmarks(b) => b.added,
        })
    });
}

fn respond(
    hits: Vec<SourceHit>,
    total: u32,
    truncated_by: Option<TruncatedBy>,
    start: Instant,
) -> SourcesResponse {
    SourcesResponse {
        hits,
        total,
        truncated: truncated_by.is_some(),
        truncated_by,
        elapsed_ms: start.elapsed().as_millis().min(u32::MAX as u128) as u32,
    }
}
