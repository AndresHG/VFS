//! Query parsing, candidate generation, verification and paging (VFS-1 AC-10, AC-11).
//! Owned by task T2a; ranking is `rank.rs` and typo tolerance `fuzzy.rs` (T2b).

use rusqlite::Connection;

use crate::error::Result;
use crate::types::{SearchConfig, SearchQuery, SearchResponse};

/// State kept between searches on the search connection, e.g. the last ranked list keyed by
/// (query, index generation from `PRAGMA data_version`) so paging is consistent. T2a fills it.
#[derive(Default)]
pub(crate) struct SearchState {
    _private: (),
}

/// Runs one query on the dedicated search connection. The facade calls
/// `InterruptHandle::interrupt` on this connection before each new query, so a running
/// query can see `SQLITE_INTERRUPT` and must return what it has with `truncated_by` set.
pub(crate) fn search(
    _conn: &Connection,
    _state: &mut SearchState,
    _config: &SearchConfig,
    _query: &SearchQuery,
) -> Result<SearchResponse> {
    todo!("T2a")
}
