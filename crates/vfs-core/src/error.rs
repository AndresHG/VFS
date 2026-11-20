//! Error type of the core. `code()` is the stable string the UI maps to Spanish copy.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("not a directory: {0}")]
    NotADirectory(String),
    #[error("overlaps an indexed folder: {0}")]
    OverlappingRoot(String),
    #[error("folder unavailable: {0}")]
    Unavailable(String),
    #[error("index was created by a newer version")]
    SchemaTooNew,
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// One of `not_found`, `not_a_directory`, `overlapping_root`, `unavailable`,
    /// `schema_too_new`, `db`, `io` (the `ErrorCode` type of the UI contract).
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "not_found",
            Error::NotADirectory(_) => "not_a_directory",
            Error::OverlappingRoot(_) => "overlapping_root",
            Error::Unavailable(_) => "unavailable",
            Error::SchemaTooNew => "schema_too_new",
            Error::Db(_) => "db",
            Error::Io(_) => "io",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
