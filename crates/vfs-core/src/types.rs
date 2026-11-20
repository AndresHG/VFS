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
}

/// Why a hit matched. Document content search (VFS-1 AC-15) adds its own variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    Name,
    Path,
    Fuzzy,
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
}

/// Default wall-clock budget of a search; filtered scans and the split-term typo pass stop
/// at it and the response says `truncatedBy = deadline` (VFS-1 AC-9, S3).
pub const DEFAULT_SEARCH_DEADLINE: Duration = Duration::from_millis(1000);

/// Search tuning; not part of the UI contract.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Wall-clock budget for scans and the typo pass ([`DEFAULT_SEARCH_DEADLINE`]).
    pub deadline: Duration,
}

impl Default for SearchConfig {
    fn default() -> SearchConfig {
        SearchConfig {
            deadline: DEFAULT_SEARCH_DEADLINE,
        }
    }
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

    /// Responses serialize with the field and enum spellings of the command contract.
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
            }],
            total: 1,
            truncated: true,
            truncated_by: Some(TruncatedBy::Deadline),
            elapsed_ms: 2,
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({
                "hits": [{ "id": 7, "name": "Notaría.pdf", "dir": "/a/b", "kind": "document",
                           "size": 3, "modified": 4, "matched": "fuzzy", "isDir": false }],
                "total": 1, "truncated": true, "truncatedBy": "deadline", "elapsedMs": 2
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
