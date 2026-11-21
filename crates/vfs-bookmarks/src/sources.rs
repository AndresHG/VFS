//! One search over several sources (VFS-6 "Matching and ranking", "Sorting"): files from the
//! core, bookmarks and bookmark folders from [`Bookmarks`]. By relevance both lists merge by
//! the shared tiers, each keeping its own order, the file first on a tie; inside a bookmark
//! folder with no words, Firefox's own order. By date, name or size (VFS-12) the core sorts
//! the file list and the bookmarks merge into it; by source, files come first. The merged list is paged like VFS-1 S7.

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
    // VFS-12: the core sorts the file list before its cap; the merge below keeps that order.
    let query = &SearchQuery {
        sort: options.sort.files(),
        ..query.clone()
    };
    let inside = options.bookmark_folder;
    let with_files = covers(&options.sources, Source::Files) && inside.is_none();
    let marks = bookmarks
        .filter(|b| covers(&options.sources, Source::Bookmarks) && b.is_enabled())
        .map(|b| b.search(query, inside));
    let limit = query.limit.clamp(1, CORE_PAGE) as usize;
    let offset = query.offset as usize;
    let Some((marks, marks_cut)) = marks else {
        if !with_files {
            return Ok(respond(Vec::new(), 0, None, false, start));
        }
        // Files alone: the core's own list, order and paging.
        let r = vfs.search(query)?;
        return Ok(SourcesResponse {
            hits: r.hits.into_iter().map(SourceHit::Files).collect(),
            total: r.total,
            truncated: r.truncated,
            truncated_by: r.truncated_by,
            elapsed_ms: r.elapsed_ms,
            approximate_order: r.approximate_order,
        });
    };

    // Every order merges the file list, already in order, with the bookmarks: only the
    // first `offset + limit` files can reach the page.
    let (files, total_files, files_cut, approximate) = if with_files {
        file_hits(vfs, query, offset + limit)?
    } else {
        (Vec::new(), 0, None, false)
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
        SortOrder::Newest | SortOrder::Oldest | SortOrder::Name | SortOrder::Size => {
            let mut marks: Vec<SourceHit> = marks
                .into_iter()
                .map(|m| SourceHit::Bookmarks(m.hit))
                .collect();
            marks.sort_by_cached_key(|h| order_key(options.sort, h));
            let files = files.into_iter().map(SourceHit::Files).collect();
            merge_in_order(files, marks, options.sort)
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect()
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
    Ok(respond(merged, total, cut, approximate, start))
}

/// Up to `want` hits of the file list from the core, its total, its truncation and whether
/// its order is approximate.
fn file_hits(
    vfs: &Vfs,
    query: &SearchQuery,
    want: usize,
) -> Result<(Vec<SearchHit>, u32, Option<TruncatedBy>, bool)> {
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
            return Ok((files, r.total, r.truncated_by, r.approximate_order));
        }
    }
}

/// Sort key of a hit under a date, name or size order (VFS-12): an unknown date (0) last in
/// both date orders; by size, everything without one (folders, bookmarks) after the files.
fn order_key(order: SortOrder, h: &SourceHit) -> (bool, i64, String) {
    let date = match h {
        SourceHit::Files(f) => f.modified,
        SourceHit::Bookmarks(b) => b.added,
    };
    match (order, h) {
        (SortOrder::Newest, _) => (date == 0, -date, String::new()),
        (SortOrder::Oldest, _) => (date == 0, date, String::new()),
        (SortOrder::Name, SourceHit::Files(f)) => (false, 0, fold_key(&f.name)),
        (SortOrder::Name, SourceHit::Bookmarks(b)) => (false, 0, fold_key(&b.title)),
        (SortOrder::Size, SourceHit::Files(f)) => (f.is_dir, -(f.size as i64), String::new()),
        _ => (true, 0, String::new()),
    }
}

/// Both lists, each already in `order`, merged; the file first on a tie.
fn merge_in_order(
    files: Vec<SourceHit>,
    marks: Vec<SourceHit>,
    order: SortOrder,
) -> Vec<SourceHit> {
    let mut out = Vec::with_capacity(files.len() + marks.len());
    let mut files = files.into_iter().peekable();
    let mut marks = marks.into_iter().peekable();
    loop {
        let take_file = match (files.peek(), marks.peek()) {
            (Some(f), Some(b)) => order_key(order, f) <= order_key(order, b),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        out.extend(if take_file {
            files.next()
        } else {
            marks.next()
        });
    }
    out
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

fn respond(
    hits: Vec<SourceHit>,
    total: u32,
    truncated_by: Option<TruncatedBy>,
    approximate_order: bool,
    start: Instant,
) -> SourcesResponse {
    SourcesResponse {
        hits,
        total,
        truncated: truncated_by.is_some(),
        truncated_by,
        elapsed_ms: start.elapsed().as_millis().min(u32::MAX as u128) as u32,
        approximate_order,
    }
}
