//! Document content search for VFS (VFS-2, prototype): text extraction in an isolated worker
//! process, a resumable queue driven by `files.content_state`, a Spanish BM25 full-text index
//! in a separate `content.db`, and a query function with snippets.
//!
//! Name and path search (`vfs-core`) never depend on this crate; nothing here runs unless the
//! caller switches content search on by calling [`indexer::run`].

pub mod analyze;
pub mod chunk;
pub mod cloud;
pub mod extract;
pub mod indexer;
pub mod protocol;
pub mod search;
pub mod store;
pub mod supervisor;
pub mod worker;

pub use indexer::{run as index, IndexOptions, RunSummary, StoreStats};
pub use search::{
    retrieve, search, ContentHit, ContentResponse, Passage, PassageText, Query, Searcher,
    StructuredQuery, UnreadReason,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Content search is not switched on for this index (no content store).
    #[error("content search is off for this index")]
    Off,
    #[error("content store has unknown version {0}")]
    StoreVersion(i64),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Core(#[from] vfs_core::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Switches content search off (VFS-2 R9): deletes the content store and resets
/// `files.content_state` to `None`. The name index is otherwise untouched.
pub fn switch_off(index_db: &std::path::Path) -> Result<()> {
    store::delete(index_db)?;
    let conn = rusqlite::Connection::open_with_flags(
        index_db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute(
        "UPDATE files SET content_state = ?1 WHERE content_state <> ?1",
        [vfs_core::content::ContentState::None as i64],
    )?;
    Ok(())
}
