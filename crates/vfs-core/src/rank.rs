//! Result ordering (VFS-1 AC-10). Owned by task T2b.

use crate::types::SearchHit;

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
#[allow(dead_code)]
pub(crate) fn rank(_candidates: &mut [Candidate], _folded_query: &str, _terms: &[String]) {
    todo!("T2b")
}
