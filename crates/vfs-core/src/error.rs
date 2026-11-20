//! Error type of the core. `code()` is the stable string the UI maps to Spanish copy.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("not a directory: {0}")]
    NotADirectory(String),
    /// The message reads `"<relation> <existing root path>"` so the UI can word each case
    /// (U4).
    #[error("overlaps an indexed folder: {relation} {existing}")]
    OverlappingRoot {
        relation: OverlapRelation,
        existing: String,
    },
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
            Error::OverlappingRoot { .. } => "overlapping_root",
            Error::Unavailable(_) => "unavailable",
            Error::SchemaTooNew => "schema_too_new",
            Error::Db(_) => "db",
            Error::Io(_) => "io",
        }
    }
}

/// How a refused folder relates to the existing root it overlaps (`roots::add_root`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlapRelation {
    /// The same folder, possibly spelled differently (case, accents, NFC/NFD, separator).
    Same,
    /// Inside the existing root.
    Inside,
    /// Contains the existing root.
    Contains,
}

impl std::fmt::Display for OverlapRelation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OverlapRelation::Same => "same",
            OverlapRelation::Inside => "inside",
            OverlapRelation::Contains => "contains",
        })
    }
}

impl Error {
    /// True when `add_root` refused the folder because it is already a root, so a caller
    /// resuming work on it can carry on (VFS-3).
    pub fn is_same_root(&self) -> bool {
        matches!(
            self,
            Error::OverlappingRoot {
                relation: OverlapRelation::Same,
                ..
            }
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;
