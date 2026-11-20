//! Result ordering (VFS-1 AC-10). Owned by task T2b.

use std::cmp::Reverse;

use crate::types::{MatchKind, SearchHit};

/// A verified hit plus what ranking needs. Built by `search.rs` (T2a).
#[allow(dead_code)] // T2a builds candidates; T2b ranks them.
pub(crate) struct Candidate {
    pub hit: SearchHit,
    /// `fold_key` of the file or folder name.
    pub name_key: String,
}

/// Sorts `candidates` best first: whole-query or stem match on the name, all terms in the
/// name with the first as prefix, all terms in the name, some term only in the path, fuzzy;
/// ties by shorter name, newer `modified`, lower id. `folded_query` is the whole folded
/// query text and `terms` its folded terms.
///
/// Folders (VFS-1 D1) rank by the same rules on their own name; on a full tie a file sorts
/// before a folder (their id spaces overlap). With no terms (a filter-only query, addendum
/// S6) the order is newest first, then lower id.
#[allow(dead_code)]
pub(crate) fn rank(candidates: &mut [Candidate], folded_query: &str, terms: &[String]) {
    let whole = terms.join(" ");
    let trimmed = folded_query.trim();
    candidates.sort_by_cached_key(|c| {
        // A filter-only query skips the tiers and the name length: newest first.
        let (tier, len) = if terms.is_empty() {
            (0, 0)
        } else {
            (tier(c, trimmed, &whole, terms), c.name_key.chars().count())
        };
        (tier, len, Reverse(c.hit.modified), c.hit.id, c.hit.is_dir)
    });
}

/// 0 = whole query or stem, 1 = all terms in the name with the first as prefix, 2 = all
/// terms in the name, 3 = some term only in the path, 4 = fuzzy.
fn tier(c: &Candidate, trimmed: &str, whole: &str, terms: &[String]) -> u8 {
    if c.hit.matched == MatchKind::Fuzzy {
        return 4;
    }
    let name = c.name_key.as_str();
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    if [name, stem].iter().any(|n| *n == trimmed || *n == whole) {
        return 0;
    }
    if !terms.iter().all(|t| name.contains(t.as_str())) {
        return 3;
    }
    if name.starts_with(terms[0].as_str()) {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::fold_key;
    use crate::types::FileKind;

    fn cand(id: i64, name: &str, modified: i64, matched: MatchKind, is_dir: bool) -> Candidate {
        Candidate {
            hit: SearchHit {
                id,
                name: name.to_string(),
                dir: "/r/gestoria".to_string(),
                kind: if is_dir {
                    FileKind::Folder
                } else {
                    FileKind::Document
                },
                size: 0,
                modified,
                matched,
                is_dir,
            },
            name_key: fold_key(name),
        }
    }

    fn ranked(mut c: Vec<Candidate>, query: &str) -> Vec<(i64, bool)> {
        let q = fold_key(query);
        let terms: Vec<String> = q.split_whitespace().map(str::to_string).collect();
        rank(&mut c, &q, &terms);
        c.iter().map(|c| (c.hit.id, c.hit.is_dir)).collect()
    }

    /// VFS-1 AC-10: hits sort by the five tiers of plan 2.4.
    #[test]
    fn tiers_in_order() {
        use MatchKind::*;
        let c = vec![
            cand(1, "notria.pdf", 50, Fuzzy, false),            // fuzzy
            cand(2, "acta.pdf", 50, Path, false),               // path only
            cand(3, "la notaria perez.pdf", 50, Name, false),   // all terms, no prefix
            cand(4, "notaria perez 2019.pdf", 50, Name, false), // all terms, first prefix
            cand(5, "Notaría Pérez.pdf", 50, Name, false),      // stem equals query
            cand(6, "notaria perez", 50, Name, false),          // name equals query
        ];
        assert_eq!(
            ranked(c, "notaria  perez"),
            vec![
                (6, false),
                (5, false),
                (4, false),
                (3, false),
                (2, false),
                (1, false)
            ]
        );
    }

    /// VFS-1 AC-10: ties break by shorter name, then newer, then lower id; a folder ranks
    /// by its own name (D1) and sorts after a file only on a full tie.
    #[test]
    fn ties_and_folders() {
        use MatchKind::*;
        let c = vec![
            cand(7, "gestoria2", 10, Name, false),
            cand(3, "gestoria1", 10, Name, false),
            cand(9, "gestoria1", 20, Name, false),
            cand(1, "gestoria-long", 99, Name, false),
            cand(3, "gestoria1", 10, Name, true),
            cand(2, "Gestoría", 1, Name, true), // folder, exact name
        ];
        assert_eq!(
            ranked(c, "gestoria"),
            vec![
                (2, true),
                (9, false),
                (3, false),
                (3, true),
                (7, false),
                (1, false)
            ]
        );
    }

    /// VFS-1 AC-10: ranking is deterministic regardless of input order.
    #[test]
    fn order_is_stable_across_input_permutations() {
        use MatchKind::*;
        let make = || {
            vec![
                cand(1, "factura.pdf", 5, Name, false),
                cand(2, "factura.pdf", 5, Name, true),
                cand(3, "factura 2019.pdf", 8, Name, false),
                cand(4, "facturas", 8, Name, true),
                cand(5, "x.pdf", 8, Path, false),
                cand(6, "x.pdf", 9, Path, false),
                cand(7, "fatcura.pdf", 9, Fuzzy, false),
                cand(8, "mi factura.pdf", 1, Name, false),
            ]
        };
        let expected = ranked(make(), "factura");
        for rot in 0..8 {
            let mut c = make();
            c.rotate_left(rot);
            if rot % 2 == 1 {
                c.reverse();
            }
            assert_eq!(ranked(c, "factura"), expected, "rotation {rot}");
        }
        assert_eq!(expected[0], (1, false));
        assert_eq!(expected[1], (2, true));
        assert_eq!(*expected.last().unwrap(), (7, false));
    }

    /// VFS-1 AC-10: with no terms (filter-only query) results are newest first.
    #[test]
    fn filter_only_is_newest_first() {
        use MatchKind::*;
        let c = vec![
            cand(1, "a", 10, Name, false),
            cand(2, "bbbbbb", 30, Name, false),
            cand(3, "cc", 20, Name, false),
        ];
        assert_eq!(ranked(c, "  "), vec![(2, false), (3, false), (1, false)]);
    }
}
