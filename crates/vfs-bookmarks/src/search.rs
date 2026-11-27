//! Bookmark and bookmark-folder matching and ranking in memory (VFS-6 "Matching and
//! ranking"), with the file rules of `vfs_core::matching`: a term matches a folded key as a
//! substring or, when it tolerates a typo and the exact matches are few, within one edit.
//! A folder matches as a file folder does (VFS-1 D1): every term in its path and name, at
//! least one in its name.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use vfs_core::matching::{
    contains_within_one, name_tier, tolerates_typo, ParsedQuery, FUZZY_TRIGGER,
};
use vfs_core::{fold_key, FileKind, MatchKind, SearchFilters};

use crate::store::Row;
use crate::types::{BookmarkHit, BookmarkPlace, BookmarkRoot, Browser};

/// Bookmark hits kept per query, like the file list (VFS-1 S7).
pub const MAX_BOOKMARK_HITS: usize = 2000;
/// Folders deeper than this are not followed when finding ancestors (a broken tree).
const MAX_DEPTH: usize = 128;

/// A stored bookmark or folder with its folded keys.
pub(crate) struct Entry {
    pub id: i64,
    pub is_folder: bool,
    pub profile_id: i64,
    pub browser: Browser,
    /// The profile name, only when its browser has several profiles with bookmarks.
    pub profile: Option<String>,
    pub root: BookmarkRoot,
    pub folder: Vec<String>,
    pub title: String,
    pub url: String,
    pub added: i64,
    title_key: String,
    /// Bookmark: address without scheme and `www.`, folder path, tags, keyword.
    /// Folder: its whole path, own name included.
    other_keys: Vec<String>,
    /// Ids of the folders it is inside, the root included, innermost first.
    ancestors: Vec<i64>,
    /// Profile, then the browser's position at every level from the root down: sorting by
    /// it gives the browser's own order and nesting.
    tree_key: Vec<i64>,
}

/// Only plain web addresses are opened in a browser (VFS-6 R10): `http`/`https`, with no
/// space, quote or control character (Firefox stores addresses already percent-encoded), so
/// the address is one command-line argument that starts with the scheme.
pub fn is_openable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let plain = !url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '"');
    plain && (lower.starts_with("https://") || lower.starts_with("http://"))
}

/// The address as matched: no scheme, no leading `www.`, folded.
fn url_key(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    fold_key(rest)
}

/// Entries for every stored row, with ancestors and tree order resolved through the
/// folders' guids.
pub(crate) fn entries(rows: Vec<Row>) -> Vec<Entry> {
    // A profile is named only when its browser has more than one with bookmarks (R7).
    let mut with_bookmarks: HashMap<Browser, HashSet<i64>> = HashMap::new();
    for r in rows.iter().filter(|r| !r.is_folder) {
        with_bookmarks
            .entry(r.browser)
            .or_default()
            .insert(r.profile_id);
    }
    let named = |r: &Row| with_bookmarks.get(&r.browser).is_some_and(|p| p.len() > 1);
    let by_guid: HashMap<(i64, &str), &Row> = rows
        .iter()
        .filter(|r| r.is_folder)
        .map(|r| ((r.profile_id, r.guid.as_str()), r))
        .collect();
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let mut ancestors = Vec::new();
        let mut positions = vec![r.position];
        let mut parent = r.parent_guid.as_deref();
        while let Some(g) = parent {
            let Some(p) = by_guid.get(&(r.profile_id, g)) else {
                break;
            };
            if ancestors.len() == MAX_DEPTH || ancestors.contains(&p.id) {
                break;
            }
            ancestors.push(p.id);
            positions.push(p.position);
            parent = p.parent_guid.as_deref();
        }
        positions.push(r.profile_id);
        positions.reverse();
        let mut path = r.folder.clone();
        let other_keys = if r.is_folder {
            path.push(r.title.clone());
            vec![fold_key(&path.join("/"))]
        } else {
            let mut keys = vec![url_key(&r.url), fold_key(&r.folder.join("/"))];
            keys.extend(r.tags.iter().map(|t| fold_key(t)));
            keys.extend(r.keyword.iter().map(|k| fold_key(k)));
            keys
        };
        out.push(Entry {
            id: r.id,
            is_folder: r.is_folder,
            profile_id: r.profile_id,
            browser: r.browser,
            profile: named(r).then(|| r.profile.clone()),
            root: r.root,
            folder: r.folder.clone(),
            title: r.title.clone(),
            url: r.url.clone(),
            added: r.added,
            title_key: fold_key(&r.title),
            other_keys,
            ancestors,
            tree_key: positions,
        });
    }
    out
}

/// The roots that hold something, by browser, then in the browser's order (the "Carpeta"
/// filter's choices).
pub(crate) fn places(entries: &[Entry]) -> Vec<BookmarkPlace> {
    let mut roots: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.is_folder && e.ancestors.is_empty())
        .filter(|r| entries.iter().any(|e| e.ancestors.contains(&r.id)))
        .collect();
    roots.sort_by(|a, b| (a.browser, &a.tree_key).cmp(&(b.browser, &b.tree_key)));
    roots
        .into_iter()
        .map(|r| BookmarkPlace {
            id: r.id,
            root: r.root,
            browser: r.browser,
            profile: r.profile.clone(),
        })
        .collect()
}

impl Entry {
    fn hit(&self, matched: MatchKind) -> BookmarkHit {
        BookmarkHit {
            id: self.id,
            is_folder: self.is_folder,
            title: self.title.clone(),
            url: self.url.clone(),
            root: self.root,
            folder: self.folder.clone(),
            browser: self.browser,
            also_in: Vec::new(),
            profile: self.profile.clone(),
            added: self.added,
            matched,
            openable: !self.is_folder && is_openable(&self.url),
        }
    }

    /// Every term in a key; with `fuzzy`, terms that tolerate a typo may be within one edit.
    /// A folder also needs one term in its own name (VFS-1 D1); a root has none. With
    /// `names`, only the title counts (VFS-38 R3, D5).
    fn matches(&self, terms: &[String], fuzzy: bool, names: bool) -> Option<MatchKind> {
        if self.is_folder && self.title_key.is_empty() {
            return None;
        }
        let others: &[String] = if names { &[] } else { &self.other_keys };
        let near = |k: &str, t: &str| fuzzy && tolerates_typo(t) && contains_within_one(k, t);
        let mut elsewhere = false;
        let mut typo = false;
        let mut in_name = false;
        for t in terms {
            if self.title_key.contains(t.as_str()) {
                in_name = true;
            } else if others.iter().any(|k| k.contains(t.as_str())) {
                elsewhere = true;
            } else if near(&self.title_key, t) {
                typo = true;
                in_name = true;
            } else if others.iter().any(|k| near(k, t)) {
                typo = true;
            } else {
                return None;
            }
        }
        if self.is_folder && !in_name {
            return None;
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

/// A bookmark hit with the keys both sources merge and sort on (`crate::sources`).
pub(crate) struct Ranked {
    pub hit: BookmarkHit,
    pub key: MergeKey,
    pub tree_key: Vec<i64>,
}

/// Tier, then folded name or title length, then newer first. A filter-only query has no
/// tiers: newest first (VFS-1 S6).
pub(crate) type MergeKey = (u8, usize, Reverse<i64>);

/// No bookmark is a document, photo, video or audio, has a size, or lies in a folder on
/// disk; bookmark folders count as folders for the type filter (VFS-6 R7).
fn kept_by_filters(f: &SearchFilters, is_folder: bool) -> bool {
    let kind_ok = f.kinds.is_empty() || (is_folder && f.kinds.contains(&FileKind::Folder));
    kind_ok && f.size_min.is_none() && f.size_max.is_none() && f.folder.is_none()
}

/// True when the filters leave no bookmark or bookmark folder.
pub fn filters_exclude_bookmarks(f: &SearchFilters) -> bool {
    !kept_by_filters(f, true)
}

fn in_dates(f: &SearchFilters, added: i64) -> bool {
    f.modified_from.is_none_or(|from| added >= from) && f.modified_to.is_none_or(|to| added < to)
}

/// One result per address across browsers (VFS-44 R8): a bookmark whose address an
/// earlier, better ranked result from another browser has is dropped, and its browser is
/// named on that result. Profiles of one browser keep one row each (VFS-6 D8).
fn fold_other_browsers(ranked: Vec<Ranked>) -> Vec<Ranked> {
    let mut out: Vec<Ranked> = Vec::with_capacity(ranked.len());
    let mut first: HashMap<String, usize> = HashMap::new();
    for r in ranked {
        if !r.hit.is_folder {
            if let Some(&i) = first.get(&r.hit.url) {
                let kept = &mut out[i].hit;
                if kept.browser != r.hit.browser {
                    if !kept.also_in.contains(&r.hit.browser) {
                        kept.also_in.push(r.hit.browser);
                    }
                    continue;
                }
            } else {
                first.insert(r.hit.url.clone(), out.len());
            }
        }
        out.push(r);
    }
    out
}

pub(crate) fn search(
    entries: &[Entry],
    q: &ParsedQuery,
    f: &SearchFilters,
    inside: Option<i64>,
    names: bool,
) -> (Vec<Ranked>, bool) {
    let has_date = f.modified_from.is_some() || f.modified_to.is_some();
    if q.terms.is_empty() && !has_date && inside.is_none() {
        return (Vec::new(), false);
    }
    let candidates: Vec<&Entry> = entries
        .iter()
        .filter(|e| kept_by_filters(f, e.is_folder) && in_dates(f, e.added))
        .filter(|e| inside.is_none_or(|id| e.ancestors.contains(&id)))
        .collect();
    let mut out: Vec<Ranked> = Vec::new();
    let rank = |e: &Entry, m: MatchKind, key: MergeKey| Ranked {
        hit: e.hit(m),
        key,
        tree_key: e.tree_key.clone(),
    };
    if q.terms.is_empty() {
        out.extend(
            candidates
                .iter()
                .map(|e| rank(e, MatchKind::Name, (0, 0, Reverse(e.added)))),
        );
    } else {
        let key = |e: &Entry, m: MatchKind| -> MergeKey {
            (
                name_tier(&e.title_key, m, q),
                e.title_key.chars().count(),
                Reverse(e.added),
            )
        };
        for e in &candidates {
            if let Some(m) = e.matches(&q.terms, false, names) {
                out.push(rank(e, m, key(e, m)));
            }
        }
        if out.len() < FUZZY_TRIGGER {
            for e in &candidates {
                if e.matches(&q.terms, false, names).is_none() {
                    if let Some(m) = e.matches(&q.terms, true, names) {
                        out.push(rank(e, m, key(e, m)));
                    }
                }
            }
        }
    }
    out.sort_by_key(|r| (r.key, r.hit.id));
    let mut out = fold_other_browsers(out);
    let cut = out.len() > MAX_BOOKMARK_HITS;
    out.truncate(MAX_BOOKMARK_HITS);
    (out, cut)
}
