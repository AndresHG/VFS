//! One search over several sources (VFS-6 "Matching and ranking"): files from the core,
//! bookmarks from [`Bookmarks`], merged by the shared tiers. Each source keeps its own order;
//! on a tie the file comes first. The merged list is paged like VFS-1 S7.

use std::cmp::Reverse;
use std::time::Instant;

use vfs_core::matching::{name_tier, parse_query};
use vfs_core::{fold_key, Result, SearchHit, SearchQuery, Vfs};

use crate::search::MergeKey;
use crate::types::{Source, SourceHit, SourcesResponse};
use crate::Bookmarks;

/// Largest page the core returns (VFS-1 `search`); the merge reads files in pages this big.
const CORE_PAGE: u32 = 500;

/// Whether `sources` (empty = all) includes `s`.
pub fn covers(sources: &[Source], s: Source) -> bool {
    sources.is_empty() || sources.contains(&s)
}

/// Runs `query` over the chosen sources. Bookmarks are searched only when `bookmarks` is
/// given and its setting is on (R1).
pub fn search_sources(
    vfs: &Vfs,
    bookmarks: Option<&Bookmarks>,
    query: &SearchQuery,
    sources: &[Source],
) -> Result<SourcesResponse> {
    let start = Instant::now();
    let with_files = covers(sources, Source::Files);
    let bookmarks = bookmarks
        .filter(|b| covers(sources, Source::Bookmarks) && b.is_enabled())
        .map(|b| b.search(query));
    let Some(bookmarks) = bookmarks else {
        if !with_files {
            return Ok(empty(start));
        }
        let r = vfs.search(query)?;
        return Ok(SourcesResponse {
            hits: r.hits.into_iter().map(SourceHit::Files).collect(),
            total: r.total,
            truncated: r.truncated,
            truncated_by: r.truncated_by,
            elapsed_ms: r.elapsed_ms,
        });
    };

    let limit = query.limit.clamp(1, CORE_PAGE) as usize;
    let offset = query.offset as usize;
    let want = offset + limit;
    let (mut files, mut total_files, mut truncated_by) = (Vec::new(), 0u32, None);
    if with_files {
        loop {
            let page = SearchQuery {
                limit: CORE_PAGE,
                offset: files.len() as u32,
                ..query.clone()
            };
            let r = vfs.search(&page)?;
            total_files = r.total;
            truncated_by = r.truncated_by;
            let got = r.hits.len();
            files.extend(r.hits);
            if got == 0 || files.len() >= want || files.len() >= r.total as usize {
                break;
            }
        }
    }

    let parsed = parse_query(&query.text);
    let file_key = |h: &SearchHit| -> MergeKey {
        if parsed.terms.is_empty() {
            (0, 0, Reverse(h.modified))
        } else {
            let key = fold_key(&h.name);
            (
                name_tier(&key, h.matched, &parsed),
                key.chars().count(),
                Reverse(h.modified),
            )
        }
    };
    let mut merged = Vec::with_capacity(want.min(files.len() + bookmarks.len()));
    let (mut fi, mut bi) = (0, 0);
    while merged.len() < want && (fi < files.len() || bi < bookmarks.len()) {
        let take_file = match (files.get(fi), bookmarks.get(bi)) {
            (Some(f), Some(b)) => file_key(f) <= b.key,
            (Some(_), None) => true,
            _ => false,
        };
        if take_file {
            merged.push(SourceHit::Files(files[fi].clone()));
            fi += 1;
        } else {
            merged.push(SourceHit::Bookmarks(bookmarks[bi].hit.clone()));
            bi += 1;
        }
    }
    let hits = merged.into_iter().skip(offset).collect();
    Ok(SourcesResponse {
        hits,
        total: total_files + bookmarks.len() as u32,
        truncated: truncated_by.is_some(),
        truncated_by,
        elapsed_ms: start.elapsed().as_millis().min(u32::MAX as u128) as u32,
    })
}

fn empty(start: Instant) -> SourcesResponse {
    SourcesResponse {
        hits: Vec::new(),
        total: 0,
        truncated: false,
        truncated_by: None,
        elapsed_ms: start.elapsed().as_millis() as u32,
    }
}
