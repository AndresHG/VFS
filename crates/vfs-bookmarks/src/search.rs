//! Bookmark matching and ranking in memory (VFS-6 "Matching and ranking"), with the file
//! rules of `vfs_core::matching`: a term matches a folded key as a substring or, when it
//! tolerates a typo and the exact matches are few, within one edit.

use std::cmp::Reverse;

use vfs_core::matching::{
    contains_within_one, name_tier, tolerates_typo, ParsedQuery, FUZZY_TRIGGER,
};
use vfs_core::{fold_key, MatchKind, SearchFilters};

use crate::store::Row;
use crate::types::{BookmarkHit, BookmarkRoot};

/// Bookmark hits kept per query, like the file list (VFS-1 S7).
pub const MAX_BOOKMARK_HITS: usize = 2000;

/// A stored bookmark with its folded keys.
pub(crate) struct Entry {
    pub id: i64,
    pub profile: String,
    pub root: BookmarkRoot,
    pub folder: Vec<String>,
    pub title: String,
    pub url: String,
    pub added: i64,
    title_key: String,
    /// Address without scheme and `www.`, folder path, tags, keyword.
    other_keys: Vec<String>,
}

/// Only web addresses are handed to the browser (VFS-6 R10).
pub fn is_openable(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// The address as matched: no scheme, no leading `www.`, folded.
fn url_key(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    fold_key(rest)
}

impl Entry {
    pub fn new(r: Row) -> Entry {
        let mut other_keys = vec![url_key(&r.url), fold_key(&r.folder.join("/"))];
        other_keys.extend(r.tags.iter().map(|t| fold_key(t)));
        other_keys.extend(r.keyword.iter().map(|k| fold_key(k)));
        Entry {
            title_key: fold_key(&r.title),
            other_keys,
            id: r.id,
            profile: r.profile,
            root: r.root,
            folder: r.folder,
            title: r.title,
            url: r.url,
            added: r.added,
        }
    }

    fn hit(&self, matched: MatchKind, multi_profile: bool) -> BookmarkHit {
        BookmarkHit {
            id: self.id,
            title: self.title.clone(),
            url: self.url.clone(),
            root: self.root,
            folder: self.folder.clone(),
            profile: multi_profile.then(|| self.profile.clone()),
            added: self.added,
            matched,
            openable: is_openable(&self.url),
        }
    }

    /// Every term in a key; with `fuzzy`, terms that tolerate a typo may be within one edit.
    fn matches(&self, terms: &[String], fuzzy: bool) -> Option<MatchKind> {
        let mut elsewhere = false;
        let mut typo = false;
        for t in terms {
            if self.title_key.contains(t.as_str()) {
            } else if self.other_keys.iter().any(|k| k.contains(t.as_str())) {
                elsewhere = true;
            } else if fuzzy
                && tolerates_typo(t)
                && (contains_within_one(&self.title_key, t)
                    || self.other_keys.iter().any(|k| contains_within_one(k, t)))
            {
                typo = true;
            } else {
                return None;
            }
        }
        Some(if typo {
            MatchKind::Fuzzy
        } else if elsewhere {
            MatchKind::Path
        } else {
            MatchKind::Name
        })
    }
}

/// A bookmark hit with the key both sources merge on (`crate::sources`).
pub(crate) struct Ranked {
    pub hit: BookmarkHit,
    pub key: MergeKey,
}

/// Tier, then folded name or title length, then newer first. A filter-only query has no
/// tiers: newest first (VFS-1 S6).
pub(crate) type MergeKey = (u8, usize, Reverse<i64>);

/// No bookmark is a document, photo, video, audio or folder, has a size, or lies in a
/// folder on disk (VFS-6 R7).
pub fn filters_exclude_bookmarks(f: &SearchFilters) -> bool {
    !f.kinds.is_empty() || f.size_min.is_some() || f.size_max.is_some() || f.folder.is_some()
}

fn in_dates(f: &SearchFilters, added: i64) -> bool {
    f.modified_from.is_none_or(|from| added >= from) && f.modified_to.is_none_or(|to| added < to)
}

pub(crate) fn search(
    entries: &[Entry],
    q: &ParsedQuery,
    f: &SearchFilters,
    multi_profile: bool,
) -> Vec<Ranked> {
    let has_date = f.modified_from.is_some() || f.modified_to.is_some();
    if filters_exclude_bookmarks(f) || (q.terms.is_empty() && !has_date) {
        return Vec::new();
    }
    let candidates = entries.iter().filter(|e| in_dates(f, e.added));
    let mut out: Vec<Ranked> = Vec::new();
    if q.terms.is_empty() {
        out.extend(candidates.map(|e| Ranked {
            hit: e.hit(MatchKind::Name, multi_profile),
            key: (0, 0, Reverse(e.added)),
        }));
    } else {
        let rank = |e: &Entry, m: MatchKind| Ranked {
            key: (
                name_tier(&e.title_key, m, q),
                e.title_key.chars().count(),
                Reverse(e.added),
            ),
            hit: e.hit(m, multi_profile),
        };
        let candidates: Vec<&Entry> = candidates.collect();
        for e in &candidates {
            if let Some(m) = e.matches(&q.terms, false) {
                out.push(rank(e, m));
            }
        }
        if out.len() < FUZZY_TRIGGER {
            for e in &candidates {
                if e.matches(&q.terms, false).is_none() {
                    if let Some(m) = e.matches(&q.terms, true) {
                        out.push(rank(e, m));
                    }
                }
            }
        }
    }
    out.sort_by_key(|r| (r.key, r.hit.id));
    out.truncate(MAX_BOOKMARK_HITS);
    out
}
