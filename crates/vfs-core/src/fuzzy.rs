//! One-typo tolerance (VFS-1 AC-10). Owned by task T2b.

/// Minimum folded term length for typo tolerance; shorter terms match exactly.
#[allow(dead_code)] // T2a/T2b wire these into search.rs.
pub(crate) const FUZZY_MIN_LEN: usize = 6;

/// FTS5 `MATCH` expressions whose union contains every string within one insertion,
/// deletion, substitution or transposition of the folded `term`: for each edit position i,
/// `"t[..i]" AND "t[i+1..]"` (plus `t[i+2..]` for transpositions), parts shorter than 3
/// characters dropped. Empty when `term` is shorter than [`FUZZY_MIN_LEN`].
#[allow(dead_code)]
pub(crate) fn candidate_queries(_term: &str) -> Vec<String> {
    todo!("T2b")
}

/// True when some substring of `haystack` is within optimal-string-alignment distance 1 of
/// `needle` (Sellers' approximate substring search). Both arguments are folded keys.
#[allow(dead_code)]
pub(crate) fn contains_within_one(_haystack: &str, _needle: &str) -> bool {
    todo!("T2b")
}
