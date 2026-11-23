//! One suggestion for a search with no hits (VFS-13): the same text without filters (and,
//! for a names-only search, over names and paths: VFS-38 R13), a
//! short word with two adjacent letters swapped or one letter removed, or the query without
//! one word; the first try that finds something wins. Every try only counts, within one
//! budget, and stops when a newer search starts.

use std::time::Instant;

use rusqlite::Connection;

use crate::error::Result;
use crate::fold::fold_key;
use crate::matching::parse_query;
use crate::search::{self, has_filter, Cancel};
use crate::types::{
    Scope, SearchConfig, SearchFilters, SearchQuery, SortOrder, Suggestion, SuggestionKind,
};

/// Terms of this many characters get spelling tries (shorter ones have 2-letter deletions,
/// below the trigram index; from 6 on the search already tolerates a typo).
const SPELLING_LEN: std::ops::RangeInclusive<usize> = 4..=5;
/// Only terms of this length also get deletions (a 4-letter term's 3-letter deletions match
/// far too broadly).
const DELETION_LEN: usize = 5;

/// What a try changes, for the suggestion it becomes.
struct Try {
    kind: SuggestionKind,
    text: String,
    clear_filters: bool,
    term: Option<String>,
    replacement: Option<String>,
}

/// Runs the tries for `query` in order on the search connection. `cancel` tells whether a
/// newer search started, which ends the call with no suggestion (R4), between tries and
/// inside a running one.
pub(crate) fn suggest(
    conn: &mut Connection,
    config: &SearchConfig,
    query: &SearchQuery,
    cancel: &Cancel,
) -> Result<Option<Suggestion>> {
    let end = Instant::now() + config.suggest_budget;
    for t in tries(&query.text, &query.filters, query.scope) {
        if cancel.is_set() || Instant::now() >= end {
            return Ok(None);
        }
        let q = SearchQuery {
            text: t.text.clone(),
            filters: if t.clear_filters {
                SearchFilters::default()
            } else {
                query.filters.clone()
            },
            limit: 1,
            offset: 0,
            sort: SortOrder::Relevance,
            // VFS-38 R13: dropping the filters drops the scope too; the other tries keep it.
            scope: if t.clear_filters {
                Scope::All
            } else {
                query.scope
            },
        };
        if let Some((count, capped)) = search::count(conn, &q, end, cancel)? {
            if cancel.is_set() {
                return Ok(None);
            }
            return Ok(Some(Suggestion {
                kind: t.kind,
                text: t.text,
                clear_filters: t.clear_filters,
                term: t.term,
                replacement: t.replacement,
                count: count as u32,
                capped,
            }));
        }
    }
    Ok(None)
}

/// Every try for a query, in the order R2 gives. A names-only scope counts as a filter
/// (VFS-38 R13).
fn tries(text: &str, filters: &SearchFilters, scope: Scope) -> Vec<Try> {
    let terms = parse_query(text).terms;
    let mut out = Vec::new();
    if !terms.is_empty() && (has_filter(filters) || scope == Scope::Names) {
        out.push(Try {
            kind: SuggestionKind::DropFilters,
            text: text.to_string(),
            clear_filters: true,
            term: None,
            replacement: None,
        });
    }
    for term in &terms {
        let Some(typed) = typed_word(text, term) else {
            continue;
        };
        for variant in spelling_variants(term) {
            out.push(Try {
                kind: SuggestionKind::Spelling,
                text: replace_word(text, term, &variant),
                clear_filters: false,
                term: Some(typed.clone()),
                replacement: Some(variant),
            });
        }
    }
    if terms.len() >= 2 {
        for term in terms.iter().rev() {
            let Some(typed) = typed_word(text, term) else {
                continue;
            };
            out.push(Try {
                kind: SuggestionKind::DropTerm,
                text: remove_word(text, term),
                clear_filters: false,
                term: Some(typed),
                replacement: None,
            });
        }
    }
    out
}

/// Adjacent transpositions, then (5 letters only) single-letter deletions, of a term made
/// only of 4 or 5 letters; none for any other term.
fn spelling_variants(term: &str) -> Vec<String> {
    let chars: Vec<char> = term.chars().collect();
    if !SPELLING_LEN.contains(&chars.len()) || !chars.iter().all(|c| c.is_alphabetic()) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut push = |v: String| {
        if v != term && !out.contains(&v) {
            out.push(v);
        }
    };
    for i in 0..chars.len() - 1 {
        let mut c = chars.clone();
        c.swap(i, i + 1);
        push(c.into_iter().collect());
    }
    if chars.len() == DELETION_LEN {
        for i in 0..chars.len() {
            let mut c = chars.clone();
            c.remove(i);
            push(c.into_iter().collect());
        }
    }
    out
}

/// How the search folds one typed word (S6: `\` reads as `/`).
fn fold_word(word: &str) -> String {
    fold_key(&word.replace('\\', "/"))
}

/// The first typed word that folds to `term`.
fn typed_word(text: &str, term: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| fold_word(w) == term)
        .map(str::to_string)
}

/// `text` with every word that folds to `term` replaced by `with`; spacing kept.
fn replace_word(text: &str, term: &str, with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            out.push_str(if fold_word(word) == term { with } else { word });
            word.clear();
        }
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// `text` without the words that fold to `term`, the rest joined by single spaces.
fn remove_word(text: &str, term: &str) -> String {
    text.split_whitespace()
        .filter(|w| fold_word(w) != term)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-13 AC-2: transpositions first, deletions only for 5-letter terms, and nothing
    /// for terms of other lengths or with a digit or punctuation.
    #[test]
    fn spelling_variants_follow_the_rules() {
        assert_eq!(spelling_variants("bdoa"), ["dboa", "boda", "bdao"]);
        assert_eq!(
            spelling_variants("bodda"),
            ["obdda", "bdoda", "bodad", "odda", "bdda", "boda", "bodd"]
        );
        for none in ["bod", "fotos2", "2019", "ab.cd", "facturas"] {
            assert!(spelling_variants(none).is_empty(), "{none}");
        }
    }

    /// VFS-13 R2: the text keeps its other words and spacing; a removed word leaves single
    /// spaces; every typed form of the term changes.
    #[test]
    fn words_are_replaced_and_removed_in_place() {
        assert_eq!(replace_word("Bdoa  de Ana", "bdoa", "boda"), "boda  de Ana");
        assert_eq!(replace_word("bdoa BDOA", "bdoa", "boda"), "boda boda");
        assert_eq!(remove_word("boda  2019 Ávila", "2019"), "boda Ávila");
        assert_eq!(typed_word("Bóda 2019", "boda").as_deref(), Some("Bóda"));
    }

    /// VFS-13 R2: filters first (only with text and a filter), then spelling, then dropping
    /// words from the last.
    #[test]
    fn tries_come_in_order() {
        let photos = SearchFilters {
            kinds: vec![crate::types::FileKind::Photo],
            ..SearchFilters::default()
        };
        let kinds: Vec<_> = tries("boda 2019", &photos, Scope::All)
            .iter()
            .map(|t| t.kind)
            .collect();
        assert_eq!(kinds[0], SuggestionKind::DropFilters);
        assert!(kinds[1..4].iter().all(|k| *k == SuggestionKind::Spelling));
        let drops: Vec<_> = tries("boda 2019", &photos, Scope::All)
            .into_iter()
            .filter(|t| t.kind == SuggestionKind::DropTerm)
            .map(|t| t.text)
            .collect();
        assert_eq!(drops, ["boda", "2019"]);
        assert!(tries("", &photos, Scope::All).is_empty());
        assert!(tries("boda", &SearchFilters::default(), Scope::All)
            .iter()
            .all(|t| t.kind == SuggestionKind::Spelling));
    }

    /// VFS-38 AC-11 (R13): under names only, dropping the filters is tried first even with
    /// no type, date, size or folder filter; with no words it is not.
    #[test]
    fn names_only_counts_as_a_filter() {
        let none = SearchFilters::default();
        let kinds: Vec<_> = tries("boda", &none, Scope::Names)
            .iter()
            .map(|t| t.kind)
            .collect();
        assert_eq!(kinds[0], SuggestionKind::DropFilters);
        assert!(kinds[1..].iter().all(|k| *k == SuggestionKind::Spelling));
        assert!(tries("", &none, Scope::Names).is_empty());
    }
}
