//! Contract types of the bookmark source (VFS-6). Structs serialize camelCase, enums
//! snake_case; times are unix seconds. The closed sets are mirrored in
//! `apps/desktop/ui/contract.js` and checked by `apps/desktop/src-tauri/tests/contract.rs`.

use serde::{Deserialize, Serialize};
use vfs_core::{MatchKind, SearchHit, TruncatedBy};

/// What a search covers (VFS-6 "Source choice"). An empty list means every source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Files,
    Bookmarks,
}

/// The browser a bookmark was read from. Only Firefox today (VFS-6 non-goals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Browser {
    Firefox,
}

impl Browser {
    /// Value stored in `profiles.browser`.
    pub fn code(self) -> i64 {
        match self {
            Browser::Firefox => 0,
        }
    }
}

/// The top-level Firefox folder a bookmark is under; the UI names them in Spanish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkRoot {
    /// "Menú de marcadores" (`menu________`).
    Menu,
    /// "Barra de marcadores" (`toolbar_____`).
    Toolbar,
    /// "Otros marcadores" (`unfiled_____`): where unsorted bookmarks go.
    Unfiled,
    /// "Marcadores del móvil" (`mobile______`): synced from a phone.
    Mobile,
}

impl BookmarkRoot {
    /// Value stored in `bookmarks.root`.
    pub fn code(self) -> i64 {
        match self {
            BookmarkRoot::Menu => 0,
            BookmarkRoot::Toolbar => 1,
            BookmarkRoot::Unfiled => 2,
            BookmarkRoot::Mobile => 3,
        }
    }

    /// Inverse of [`BookmarkRoot::code`]; unknown values read as `Unfiled`.
    pub fn from_code(code: i64) -> BookmarkRoot {
        match code {
            0 => BookmarkRoot::Menu,
            1 => BookmarkRoot::Toolbar,
            3 => BookmarkRoot::Mobile,
            _ => BookmarkRoot::Unfiled,
        }
    }
}

/// One bookmark as read from a browser profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    /// The browser's own stable id of the bookmark (Firefox `guid`).
    pub guid: String,
    pub root: BookmarkRoot,
    /// Folder names below the root, outermost first.
    pub folder: Vec<String>,
    /// The bookmark's title, or its address when it has none.
    pub title: String,
    pub url: String,
    pub tags: Vec<String>,
    pub keyword: Option<String>,
    /// When it was saved.
    pub added: i64,
    pub modified: i64,
}

/// Where the last read of a profile came from, or why there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadState {
    /// Never read (the setting was never on, or it was just turned on).
    Never,
    /// A read is running.
    Reading,
    /// Every profile was read from its live database.
    Ok,
    /// At least one profile was read from its daily backup (database unreadable).
    FromBackup,
    /// No Firefox profile with bookmarks was found.
    NoFirefox,
    /// At least one profile could not be read at all; its earlier bookmarks are kept.
    Failed,
}

/// What the "Incluir marcadores de Firefox" setting shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkStatus {
    pub enabled: bool,
    pub state: ReadState,
    /// Bookmarks stored.
    pub count: u32,
    /// Profiles with at least one stored bookmark.
    pub profiles: u32,
    /// When bookmarks were last read successfully.
    pub last_read: Option<i64>,
}

/// A bookmark search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkHit {
    /// Stable while the bookmark exists (`bookmarks.id`).
    pub id: i64,
    pub title: String,
    pub url: String,
    pub root: BookmarkRoot,
    pub folder: Vec<String>,
    /// The profile name, only when more than one profile has bookmarks (VFS-6 D8).
    pub profile: Option<String>,
    pub added: i64,
    pub matched: MatchKind,
    /// The address can be opened in the browser (`http` or `https`, VFS-6 R10).
    pub openable: bool,
}

/// One result of a search over several sources; `source` tells them apart.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum SourceHit {
    Files(SearchHit),
    Bookmarks(BookmarkHit),
}

impl SourceHit {
    pub fn source(&self) -> Source {
        match self {
            SourceHit::Files(_) => Source::Files,
            SourceHit::Bookmarks(_) => Source::Bookmarks,
        }
    }
}

/// `SearchResponse` (VFS-1) over several sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcesResponse {
    pub hits: Vec<SourceHit>,
    pub total: u32,
    pub truncated: bool,
    pub truncated_by: Option<TruncatedBy>,
    pub elapsed_ms: u32,
}
