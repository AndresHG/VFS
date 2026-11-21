//! Query and match rules shared by every search source (VFS-1 S1–S9 for files; VFS-6 reuses
//! them for bookmarks), so a word matches a bookmark exactly as it matches a file name.

use crate::fold::fold_key;
use crate::fuzzy::{self, FUZZY_MAX_LEN, FUZZY_MIN_LEN};
use crate::types::MatchKind;

/// Distinct terms read from a query.
const MAX_TERMS: usize = 8;
/// Bytes of query text read; the rest is ignored (the search box allows no more).
const MAX_QUERY_BYTES: usize = 1024;
/// Characters kept of each term; a longer one is cut, which only widens what it matches.
const MAX_TERM_CHARS: usize = 128;

/// The typo pass runs only when the exact matches are fewer than this (VFS-1 S4).
pub const FUZZY_TRIGGER: usize = 20;

/// A query as every source sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    /// The folded text, terms cut to `MAX_TERM_CHARS`, joined by single spaces.
    pub folded: String,
    /// At most `MAX_TERMS` distinct folded terms, in query order.
    pub terms: Vec<String>,
}

/// Bounded input: at most `MAX_QUERY_BYTES` of text and `MAX_TERM_CHARS` per term, so no
/// query costs more than a typed one, whatever is pasted. NUL reads as a space, and `\\` as
/// `/` on Windows only: a macOS name can hold a backslash (VFS-3 AC-12).
pub fn parse_query(text: &str) -> ParsedQuery {
    let mut text_end = text.len().min(MAX_QUERY_BYTES);
    while !text.is_char_boundary(text_end) {
        text_end -= 1;
    }
    // NUL cannot occur in a file name and would end an FTS5 expression early: whitespace.
    let text = text[..text_end].replace('\0', " ");
    let text = if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    };
    let folded = fold_key(&text);
    let folded = folded
        .split_whitespace()
        .map(|t| t.chars().take(MAX_TERM_CHARS).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ");
    let mut terms: Vec<String> = Vec::new();
    for t in folded.split(' ').filter(|t| !t.is_empty()) {
        if terms.len() == MAX_TERMS {
            break;
        }
        if !terms.iter().any(|x| x == t) {
            terms.push(t.to_string());
        }
    }
    ParsedQuery { folded, terms }
}

/// Terms that tolerate a typo: from `FUZZY_MIN_LEN` up to `FUZZY_MAX_LEN` characters.
pub fn tolerates_typo(term: &str) -> bool {
    (FUZZY_MIN_LEN..=FUZZY_MAX_LEN).contains(&term.chars().count())
}

/// True when some substring of `haystack` is within one edit of `needle` (folded keys).
pub fn contains_within_one(haystack: &str, needle: &str) -> bool {
    fuzzy::contains_within_one(haystack, needle)
}

/// The ranking tier of a hit whose folded name (a bookmark: its title) is `name_key`
/// (VFS-1 AC-10): 0 whole query or stem, 1 all terms in the name with the first as prefix,
/// 2 all terms in the name, 3 some term only elsewhere, 4–5 typos. `terms` is not empty.
pub fn name_tier(name_key: &str, matched: MatchKind, q: &ParsedQuery) -> u8 {
    let whole = q.terms.join(" ");
    crate::rank::tier(name_key, matched, q.folded.trim(), &whole, &q.terms)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-6 AC-6: both sources read a query the same way (folded, deduplicated, bounded).
    #[test]
    fn parse_folds_dedups_and_bounds() {
        let q = parse_query("  Recetas  RECETAS año/2019 ");
        assert_eq!(q.terms, vec!["recetas", "ano/2019"]);
        assert_eq!(q.folded, "recetas recetas ano/2019");
        let many = (0..20)
            .map(|i| format!("t{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(parse_query(&many).terms.len(), MAX_TERMS);
        let long = "é".repeat(2000);
        assert!(parse_query(&long).folded.chars().count() <= MAX_TERM_CHARS);
    }

    /// VFS-3 AC-12: a backslash is a separator only on Windows, where no name holds one; on
    /// macOS a name can, so the query keeps it.
    #[test]
    fn backslash_is_a_separator_only_on_windows() {
        let q = parse_query("año\\2019 dos\\barras\\x");
        if cfg!(windows) {
            assert_eq!(q.terms, vec!["ano/2019", "dos/barras/x"]);
        } else {
            assert_eq!(q.terms, vec!["ano\\2019", "dos\\barras\\x"]);
        }
    }

    /// VFS-6 AC-6: the tier of a title follows the file-name tiers.
    #[test]
    fn name_tier_matches_file_rules() {
        let q = parse_query("recetas postres");
        assert_eq!(name_tier("recetas postres", MatchKind::Name, &q), 0);
        assert_eq!(name_tier("recetas de postres", MatchKind::Name, &q), 1);
        assert_eq!(name_tier("mis recetas postres", MatchKind::Name, &q), 2);
        assert_eq!(name_tier("cocina", MatchKind::Path, &q), 3);
        assert!(name_tier("recteas", MatchKind::Fuzzy, &q) >= 4);
        assert!(tolerates_typo("recetas") && !tolerates_typo("pan"));
        assert!(contains_within_one("mis recteas", "recetas"));
    }
}
