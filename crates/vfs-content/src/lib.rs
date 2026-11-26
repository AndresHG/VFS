//! Document content search for VFS (VFS-2, prototype): text extraction in an isolated worker
//! process, a resumable queue driven by `files.content_state`, a Spanish BM25 full-text index
//! in a separate `content.db`, and a query function with snippets.
//!
//! Name and path search (`vfs-core`) never depend on this crate; nothing here runs unless the
//! caller runs [`indexer::run`] or switches the [`service`] on.
//!
//! The `extractor` feature (on by default) adds Xberg, the extraction itself and the worker
//! entry point used by the `vfs-extract` binary. A caller that only indexes and searches, as
//! the app does, depends on this crate without it, so Xberg stays in the worker (VFS-2 D3,
//! VFS-27 D1). The app's background indexer is [`service`].

pub mod analyze;
pub mod chunk;
pub mod cloud;
#[cfg(feature = "extractor")]
pub mod extract;
pub mod indexer;
pub mod pace;
pub mod paths;
pub mod process;
pub mod protocol;
pub mod reader;
pub mod search;
pub mod service;
pub mod store;
pub mod supervisor;
#[cfg(feature = "test-support")]
pub mod testing;
#[cfg(feature = "extractor")]
pub mod worker;

pub use indexer::{
    run as index, run_with as index_with, Admit, Gate, IndexOptions, RunSummary, StoreStats,
};
pub use reader::ChunkReader;
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
