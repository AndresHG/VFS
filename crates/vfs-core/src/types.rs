//! Serde contract types shared by the core, the CLI and the Tauri app (VFS-1).
//! Structs serialize camelCase and enums snake_case; ids are `i64`, times unix seconds.

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Other,
    Document,
    Photo,
    Video,
    Audio,
    /// A directory hit (VFS-1 decision 13: folders are results). Never stored in `files.kind`.
    Folder,
}

impl FileKind {
    /// Value stored in `files.kind` (`Folder` = 5 is never stored).
    pub fn code(self) -> i64 {
        match self {
            FileKind::Other => 0,
            FileKind::Document => 1,
            FileKind::Photo => 2,
            FileKind::Video => 3,
            FileKind::Audio => 4,
            FileKind::Folder => 5,
        }
    }

    /// Inverse of [`FileKind::code`]; unknown values read as `Other`.
    pub fn from_code(code: i64) -> FileKind {
        match code {
            1 => FileKind::Document,
            2 => FileKind::Photo,
            3 => FileKind::Video,
            4 => FileKind::Audio,
            5 => FileKind::Folder,
            _ => FileKind::Other,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchFilters {
    /// Empty = every kind, folders included. `Folder` selects directory hits.
    pub kinds: Vec<FileKind>,
    /// Inclusive.
    pub modified_from: Option<i64>,
    /// Exclusive.
    pub modified_to: Option<i64>,
    /// Bytes, inclusive.
    pub size_min: Option<u64>,
    /// Bytes, inclusive.
    pub size_max: Option<u64>,
    /// Absolute path; hits must be inside it at any depth.
    pub folder: Option<String>,
}

fn default_limit() -> u32 {
    100
}

/// Order of a search's results (VFS-12). Every order other than `Relevance` applies to all
/// matches read, before the list is cut to the hit cap.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    /// The ranking tiers (VFS-1 AC-10); a filter-only query lists newest first.
    #[default]
    Relevance,
    /// Newest modification first; an unknown date (0) last.
    Newest,
    /// Oldest modification first; an unknown date (0) last.
    Oldest,
    /// Folded name, A to Z.
    Name,
    /// Largest first; folders (size 0) last.
    Size,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub text: String,
    #[serde(default)]
    pub filters: SearchFilters,
    /// Default 100, clamped to 1..=500 by the search.
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
    /// Default `Relevance` (VFS-12).
    #[serde(default)]
    pub sort: SortOrder,
}

/// Why a hit matched. Document content search (VFS-1 AC-15) adds its own variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    Name,
    Path,
    Fuzzy,
    /// A link found only with the help of its target (VFS-18 R4).
    Target,
}

/// What kind of link a file result is (VFS-18): its `links.kind` code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// A Windows shortcut (`.lnk`).
    Shortcut,
    /// An Internet Shortcut (`.url`) holding a web address.
    Internet,
    /// A symbolic link or junction listed in its folder, never followed.
    Symlink,
}

impl LinkKind {
    /// Value stored in `links.kind`.
    pub fn code(self) -> i64 {
        match self {
            LinkKind::Shortcut => 0,
            LinkKind::Internet => 1,
            LinkKind::Symlink => 2,
        }
    }

    /// Inverse of [`LinkKind::code`]; unknown values read as `Shortcut`.
    pub fn from_code(code: i64) -> LinkKind {
        match code {
            1 => LinkKind::Internet,
            2 => LinkKind::Symlink,
            _ => LinkKind::Shortcut,
        }
    }
}

/// Where a link result points (VFS-18 D2), as read from the link itself; nothing about the
/// target is checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkTarget {
    pub kind: LinkKind,
    /// The target path or web address; `None` while unread or when the link records none.
    pub target: Option<String>,
    /// An Internet Shortcut whose address may be opened in Firefox (VFS-18 R7).
    pub openable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// `files.id`, or `dirs.id` when `is_dir` (the two id spaces overlap).
    pub id: i64,
    /// Original file or folder name.
    pub name: String,
    /// Absolute path of the containing directory, native separators.
    pub dir: String,
    /// `Folder` when `is_dir`.
    pub kind: FileKind,
    /// 0 for folders.
    pub size: u64,
    pub modified: i64,
    pub matched: MatchKind,
    /// The hit is a folder (VFS-1 decision 13).
    pub is_dir: bool,
    /// Other files with the same folded name and size in the ranked list, best first: the
    /// same file kept in other places (VFS-19). Empty for folders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<FileCopy>,
    /// Set when the file is a shortcut or link (VFS-18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkTarget>,
}

/// Another place of a file hit (VFS-19).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCopy {
    /// `files.id`, what `reveal` takes.
    pub id: i64,
    /// Absolute path of the containing directory, native separators.
    pub dir: String,
    pub modified: i64,
}

/// What cut a search short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruncatedBy {
    /// A row or hit cap was reached.
    Cap,
    /// The wall-clock deadline expired.
    Deadline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    /// The slice `[offset, offset + limit)` of the ranked list.
    pub hits: Vec<SearchHit>,
    /// Length of the ranked list, <= 2000.
    pub total: u32,
    /// A cap or the deadline cut the search short (`truncated_by.is_some()`).
    pub truncated: bool,
    /// `null`, `"cap"` or `"deadline"`.
    pub truncated_by: Option<TruncatedBy>,
    pub elapsed_ms: u32,
    /// The order (anything but relevance, or a filter-only query's newest first) covers only
    /// the matches read before a cap or the deadline cut the search (VFS-12 AC-3).
    pub approximate_order: bool,
}

/// Default wall-clock budget of a search; filtered scans and the split-term typo pass stop
/// at it and the response says `truncatedBy = deadline` (VFS-1 AC-9, S3).
pub const DEFAULT_SEARCH_DEADLINE: Duration = Duration::from_millis(1000);

/// Default wall-clock budget of one `Vfs::suggest` call (VFS-13 R4).
pub const DEFAULT_SUGGEST_BUDGET: Duration = Duration::from_millis(1500);

/// Search tuning; not part of the UI contract.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Wall-clock budget for scans and the typo pass ([`DEFAULT_SEARCH_DEADLINE`]).
    pub deadline: Duration,
    /// Wall-clock budget of every try of one suggestion ([`DEFAULT_SUGGEST_BUDGET`]).
    pub suggest_budget: Duration,
}

impl Default for SearchConfig {
    fn default() -> SearchConfig {
        SearchConfig {
            deadline: DEFAULT_SEARCH_DEADLINE,
            suggest_budget: DEFAULT_SUGGEST_BUDGET,
        }
    }
}

/// What a suggestion for a search with no hits changes (VFS-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SuggestionKind {
    /// The same text without the type, date, size and folder filters.
    DropFilters,
    /// A short word with two adjacent letters swapped or one letter removed.
    Spelling,
    /// The query without one of its words.
    DropTerm,
}

/// One search that finds something where the asked one found nothing (VFS-13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub kind: SuggestionKind,
    /// The search-box text to run: the typed text with the word replaced or removed.
    pub text: String,
    /// True only for `DropFilters`; sources and the bookmark folder are not filters here.
    pub clear_filters: bool,
    /// The typed word that changes (`Spelling`, `DropTerm`).
    pub term: Option<String>,
    /// The word that replaces it (`Spelling`).
    pub replacement: Option<String>,
    /// Hits of the suggested search as the list shows them, at most the hit cap.
    pub count: u32,
    /// The hit cap was reached.
    pub capped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootStatus {
    Pending,
    Scanning,
    Ready,
    Unavailable,
    Removing,
}

impl RootStatus {
    /// Value stored in `roots.status`.
    pub fn code(self) -> i64 {
        match self {
            RootStatus::Pending => 0,
            RootStatus::Scanning => 1,
            RootStatus::Ready => 2,
            RootStatus::Unavailable => 3,
            RootStatus::Removing => 4,
        }
    }

    /// Inverse of [`RootStatus::code`]; unknown values read as `Pending`.
    pub fn from_code(code: i64) -> RootStatus {
        match code {
            1 => RootStatus::Scanning,
            2 => RootStatus::Ready,
            3 => RootStatus::Unavailable,
            4 => RootStatus::Removing,
            _ => RootStatus::Pending,
        }
    }
}

/// Where a root folder lives, so the UI can warn about network and removable drives, which
/// v1 does not support (VFS-1 non-goals): their files read as unavailable when disconnected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveKind {
    Local,
    Network,
    Removable,
    /// The system could not tell (also for a folder that is not reachable now).
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    pub id: i64,
    pub path: String,
    pub status: RootStatus,
    pub files: u64,
    pub dirs: u64,
    pub last_full_scan: Option<i64>,
    /// Asked of the system each time roots are read.
    pub drive: DriveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexerState {
    Idle,
    Scanning,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootProgress {
    pub root_id: i64,
    pub path: String,
    pub status: RootStatus,
    /// Directories listed in the current pass.
    pub dirs_done: u64,
    pub dirs_pending: u64,
    /// Files of this root currently in the index.
    pub files_indexed: u64,
    pub current_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexProgress {
    pub state: IndexerState,
    /// All roots.
    pub files_indexed: u64,
    pub roots: Vec<RootProgress>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub files: u64,
    pub dirs: u64,
    pub roots: u32,
    pub db_bytes: u64,
    pub schema_version: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub dirs_listed: u64,
    pub files_added: u64,
    pub files_updated: u64,
    pub files_removed: u64,
    pub errors: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// VFS-1 AC-11: the query shape the frontend sends (camelCase filters, omitted fields
    /// defaulted) deserializes into the contract types.
    #[test]
    fn search_query_accepts_frontend_shape() {
        let q: SearchQuery = serde_json::from_value(json!({
            "text": "notaria",
            "filters": { "kinds": ["document", "photo"], "modifiedFrom": 10, "sizeMax": null }
        }))
        .unwrap();
        assert_eq!(q.limit, default_limit());
        assert_eq!(q.offset, 0);
        assert_eq!(q.filters.kinds, vec![FileKind::Document, FileKind::Photo]);
        assert_eq!(q.filters.modified_from, Some(10));
        assert_eq!(q.filters.size_max, None);
        assert!(q.filters.folder.is_none());

        let folders: SearchFilters =
            serde_json::from_value(json!({ "kinds": ["folder"] })).unwrap();
        assert_eq!(folders.kinds, vec![FileKind::Folder]);

        let bare: SearchQuery = serde_json::from_value(json!({ "text": "" })).unwrap();
        assert!(bare.filters.kinds.is_empty());
    }

    /// Responses serialize with the field and enum spellings of the command contract
    /// (VFS-19: `copies`, omitted when empty).
    #[test]
    fn search_response_serializes_camel_case() {
        let r = SearchResponse {
            hits: vec![SearchHit {
                id: 7,
                name: "Notaría.pdf".into(),
                dir: "/a/b".into(),
                kind: FileKind::Document,
                size: 3,
                modified: 4,
                matched: MatchKind::Fuzzy,
                is_dir: false,
                copies: vec![FileCopy {
                    id: 9,
                    dir: "/c".into(),
                    modified: 5,
                }],
                link: None,
            }],
            total: 1,
            truncated: true,
            truncated_by: Some(TruncatedBy::Deadline),
            elapsed_ms: 2,
            approximate_order: false,
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({
                "hits": [{ "id": 7, "name": "Notaría.pdf", "dir": "/a/b", "kind": "document",
                           "size": 3, "modified": 4, "matched": "fuzzy", "isDir": false,
                           "copies": [{ "id": 9, "dir": "/c", "modified": 5 }] }],
                "total": 1, "truncated": true, "truncatedBy": "deadline", "elapsedMs": 2,
                "approximateOrder": false
            })
        );
    }

    /// Progress and roots serialize with the command contract's spellings.
    #[test]
    fn progress_and_roots_serialize_camel_case() {
        let p = IndexProgress {
            state: IndexerState::Scanning,
            files_indexed: 5,
            roots: vec![RootProgress {
                root_id: 1,
                path: "/r".into(),
                status: RootStatus::Unavailable,
                dirs_done: 2,
                dirs_pending: 3,
                files_indexed: 5,
                current_dir: None,
            }],
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({ "state": "scanning", "filesIndexed": 5, "roots": [{
                "rootId": 1, "path": "/r", "status": "unavailable", "dirsDone": 2,
                "dirsPending": 3, "filesIndexed": 5, "currentDir": null }] })
        );
        let root = Root {
            id: 1,
            path: "/r".into(),
            status: RootStatus::Ready,
            files: 1,
            dirs: 1,
            last_full_scan: Some(9),
            drive: DriveKind::Removable,
        };
        let v = serde_json::to_value(&root).unwrap();
        assert_eq!(v["lastFullScan"], json!(9));
        assert_eq!(v["drive"], json!("removable"));
        let s = IndexStats {
            files: 1,
            dirs: 2,
            roots: 3,
            db_bytes: 4,
            schema_version: 1,
        };
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            json!({ "files": 1, "dirs": 2, "roots": 3, "dbBytes": 4, "schemaVersion": 1 })
        );
    }

    /// Stored integer codes round-trip for every kind and root status.
    #[test]
    fn integer_codes_round_trip() {
        for k in [
            FileKind::Other,
            FileKind::Document,
            FileKind::Photo,
            FileKind::Video,
            FileKind::Audio,
            FileKind::Folder,
        ] {
            assert_eq!(FileKind::from_code(k.code()), k);
        }
        for s in [
            RootStatus::Pending,
            RootStatus::Scanning,
            RootStatus::Ready,
            RootStatus::Unavailable,
            RootStatus::Removing,
        ] {
            assert_eq!(RootStatus::from_code(s.code()), s);
        }
    }

    /// Unknown stored codes read as the safe defaults. (What the search deadline does is
    /// tested in `tests/search.rs`: `ac9_truncation_by_deadline_is_reported`.)
    #[test]
    fn unknown_codes_read_as_safe_defaults() {
        assert_eq!(FileKind::from_code(42), FileKind::Other);
        assert_eq!(RootStatus::from_code(-1), RootStatus::Pending);
    }
}
