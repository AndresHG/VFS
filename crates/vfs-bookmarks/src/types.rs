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

/// The browser a bookmark was read from (VFS-6, VFS-44).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Browser {
    Firefox,
    Chrome,
    Brave,
}

impl Browser {
    /// Every browser, in the order the screen lists them.
    pub const ALL: [Browser; 3] = [Browser::Firefox, Browser::Chrome, Browser::Brave];

    /// Value stored in `profiles.browser`.
    pub fn code(self) -> i64 {
        match self {
            Browser::Firefox => 0,
            Browser::Chrome => 1,
            Browser::Brave => 2,
        }
    }

    /// Inverse of [`Browser::code`]; unknown values read as Firefox.
    pub fn from_code(code: i64) -> Browser {
        match code {
            1 => Browser::Chrome,
            2 => Browser::Brave,
            _ => Browser::Firefox,
        }
    }
}

/// The top-level folder a bookmark is under; the UI names them. Chromium's roots map to
/// `Toolbar` (`bookmark_bar`), `Unfiled` (`other`) and `Mobile` (`synced`) (VFS-44 R5).
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
    /// The browser's own stable id of the bookmark (`guid`).
    pub guid: String,
    /// The `guid` of the folder it is in (a root's own guid at the top).
    pub parent_guid: String,
    /// Its place among the folder's children, as Firefox orders them.
    pub position: i64,
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

/// One bookmark folder, or one of the four roots (`parent_guid` `None`, empty title).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub guid: String,
    pub parent_guid: Option<String>,
    pub root: BookmarkRoot,
    /// Folder names from the root down to its parent, outermost first.
    pub path: Vec<String>,
    pub title: String,
    pub position: i64,
    pub added: i64,
}

/// Everything read from one profile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    pub folders: Vec<Folder>,
    pub bookmarks: Vec<Bookmark>,
}

/// Order of a search over several sources (VFS-6 "Sorting").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    /// The shared ranking tiers (VFS-1 AC-10); inside a bookmark folder with no words, the
    /// folder's own order.
    #[default]
    Relevance,
    /// Newest first: a file's modification date, a bookmark's date saved; unknown dates last.
    Newest,
    /// Oldest first, by the same dates; unknown dates last (VFS-12).
    Oldest,
    /// A file's or folder's name, a bookmark's title, A to Z (VFS-12).
    Name,
    /// Largest file first; folders, then bookmarks (no size), after every file (VFS-12).
    Size,
    /// Files first, then bookmarks, each by relevance.
    Source,
}

impl SortOrder {
    /// The order the core applies to the file list (VFS-12): by source, files keep their
    /// relevance order.
    pub fn files(self) -> vfs_core::SortOrder {
        match self {
            SortOrder::Relevance | SortOrder::Source => vfs_core::SortOrder::Relevance,
            SortOrder::Newest => vfs_core::SortOrder::Newest,
            SortOrder::Oldest => vfs_core::SortOrder::Oldest,
            SortOrder::Name => vfs_core::SortOrder::Name,
            SortOrder::Size => vfs_core::SortOrder::Size,
        }
    }
}

/// What a search over several sources covers and how it is ordered.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceOptions {
    /// Empty = every source.
    pub sources: Vec<Source>,
    /// Only inside this bookmark folder (`BookmarkHit.id` of a folder or `BookmarkPlace.id`);
    /// files are then left out, as bookmarks are under a file folder filter.
    pub bookmark_folder: Option<i64>,
    pub sort: SortOrder,
}

/// A root folder of a profile, offered as a "Carpeta" filter choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkPlace {
    pub id: i64,
    pub root: BookmarkRoot,
    /// The screen names it only when more than one browser has bookmarks (VFS-44 R7).
    pub browser: Browser,
    /// Only when its browser has more than one profile with bookmarks.
    pub profile: Option<String>,
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
    /// No profile of the browser with bookmarks was found.
    NotFound,
    /// At least one profile could not be read at all; its earlier bookmarks are kept.
    Failed,
}

/// One browser's switch and last read («Incluir marcadores de Chrome», VFS-44 R1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub browser: Browser,
    pub enabled: bool,
    pub state: ReadState,
    /// Bookmarks stored from this browser.
    pub count: u32,
    /// Its profiles with at least one stored bookmark.
    pub profiles: u32,
    /// When its bookmarks were last read successfully.
    pub last_read: Option<i64>,
}

/// What the bookmark settings show: every browser together, then each on its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkStatus {
    /// At least one browser's switch is on.
    pub enabled: bool,
    /// The switched-on browsers' states together: reading, else failed, else from backup,
    /// else ok, else not found.
    pub state: ReadState,
    /// Bookmarks stored.
    pub count: u32,
    /// Profiles with at least one stored bookmark.
    pub profiles: u32,
    /// When bookmarks were last read successfully.
    pub last_read: Option<i64>,
    /// The roots that hold something, by browser, then in the browser's order.
    pub roots: Vec<BookmarkPlace>,
    /// Firefox, then each Chromium browser found on this computer or switched on.
    pub browsers: Vec<BrowserStatus>,
}

/// A bookmark or bookmark folder search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkHit {
    /// Stable while the bookmark or folder exists. Bookmarks and folders are one id space.
    pub id: i64,
    /// A folder result (VFS-6 R11): its action shows what it contains.
    pub is_folder: bool,
    pub title: String,
    /// Empty for a folder.
    pub url: String,
    pub root: BookmarkRoot,
    pub folder: Vec<String>,
    /// The browser it was read from; its action opens the address there (VFS-44 R6). The
    /// screen names it only when more than one browser has bookmarks (R7).
    pub browser: Browser,
    /// Other browsers that keep the same address, folded into this result (VFS-44 R8).
    pub also_in: Vec<Browser>,
    /// The profile name, only when its browser has more than one profile with bookmarks
    /// (VFS-6 D8, VFS-44 R7).
    pub profile: Option<String>,
    pub added: i64,
    pub matched: MatchKind,
    /// The address can be opened (`http` or `https`, VFS-6 R10).
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
    /// The file list's order covers only the matches read (`SearchResponse`, VFS-12 AC-3).
    pub approximate_order: bool,
}
