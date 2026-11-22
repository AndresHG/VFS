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
    /// The message is the [`PatternError`] code, which the UI words (VFS-34 R2).
    #[error("{0}")]
    InvalidPattern(PatternError),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// One of `not_found`, `not_a_directory`, `overlapping_root`, `unavailable`,
    /// `schema_too_new`, `invalid_pattern`, `db`, `io` (the `ErrorCode` type of the UI
    /// contract).
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "not_found",
            Error::NotADirectory(_) => "not_a_directory",
            Error::OverlappingRoot { .. } => "overlapping_root",
            Error::Unavailable(_) => "unavailable",
            Error::SchemaTooNew => "schema_too_new",
            Error::InvalidPattern(_) => "invalid_pattern",
            Error::Db(_) => "db",
            Error::Io(_) => "io",
        }
    }
}

/// Why a file-filter pattern is refused (VFS-34 R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternError {
    Empty,
    /// It holds `/` or `\`: a pattern matches a name, never a path.
    Separator,
    /// Only `*`, `?` and `.`, so it would hide (nearly) every file.
    NoLiteral,
    Duplicate,
}

impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PatternError::Empty => "empty",
            PatternError::Separator => "separator",
            PatternError::NoLiteral => "no_literal",
            PatternError::Duplicate => "duplicate",
        })
    }
}

/// How a refused folder relates to the existing root it overlaps (`roots::add_root`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlapRelation {
    /// The same folder, possibly spelled differently (case, accents, NFC/NFD, separator).
    Same,
    /// Inside the existing root. A folder containing a root is not refused: it takes the
    /// root in (VFS-31 R2).
    Inside,
}

impl std::fmt::Display for OverlapRelation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OverlapRelation::Same => "same",
            OverlapRelation::Inside => "inside",
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
