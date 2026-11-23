// Closed sets of the command contract (VFS-1 spec, "Tauri commands and events"; VFS-6): the
// serde values of `crates/vfs-core/src/types.rs`, `api.rs`, `error.rs` and
// `crates/vfs-bookmarks/src/types.rs`, named once for the UI and the development fake backend. The Rust test `contract_js_matches_the_rust_enums`
// (apps/desktop/src-tauri/tests/contract.rs) fails when a set here and its Rust enum differ.

export const FileKind = Object.freeze({
  OTHER: 'other',
  DOCUMENT: 'document',
  PHOTO: 'photo',
  VIDEO: 'video',
  AUDIO: 'audio',
  FOLDER: 'folder',
});

export const RootStatus = Object.freeze({
  PENDING: 'pending',
  SCANNING: 'scanning',
  READY: 'ready',
  UNAVAILABLE: 'unavailable',
  REMOVING: 'removing',
});

export const IndexerState = Object.freeze({ IDLE: 'idle', SCANNING: 'scanning', PAUSED: 'paused' });

export const TruncatedBy = Object.freeze({ CAP: 'cap', DEADLINE: 'deadline' });

export const MatchKind = Object.freeze({ NAME: 'name', PATH: 'path', FUZZY: 'fuzzy', TARGET: 'target' });

/** What kind of link a file result is (VFS-18): `SearchHit.link.kind`. */
export const LinkKind = Object.freeze({ SHORTCUT: 'shortcut', INTERNET: 'internet', SYMLINK: 'symlink' });

// Why a folder could not be listed (VFS-10).
export const UnreadableReason = Object.freeze({
  NO_PERMISSION: 'no_permission', UNAVAILABLE: 'unavailable', CLOUD_ONLY: 'cloud_only', DISK_ERROR: 'disk_error',
});

export const DriveKind = Object.freeze({
  LOCAL: 'local',
  NETWORK: 'network',
  REMOVABLE: 'removable',
  UNKNOWN: 'unknown',
});

export const RevealOutcome = Object.freeze({ REVEALED: 'revealed', OPENED_PARENT: 'opened_parent' });

/** What a search covers, and the `source` of each result (VFS-6). */
export const Source = Object.freeze({ FILES: 'files', BOOKMARKS: 'bookmarks' });

/** The Firefox folder a bookmark is under (VFS-6). */
export const BookmarkRoot = Object.freeze({
  MENU: 'menu',
  TOOLBAR: 'toolbar',
  UNFILED: 'unfiled',
  MOBILE: 'mobile',
});

/** `BookmarkStatus.state`: how the last read of Firefox bookmarks went (VFS-6). */
export const ReadState = Object.freeze({
  NEVER: 'never',
  READING: 'reading',
  OK: 'ok',
  FROM_BACKUP: 'from_backup',
  NO_FIREFOX: 'no_firefox',
  FAILED: 'failed',
});

/** `Suggestion.kind`: what a suggestion for a search with no hits changes (VFS-13). */
export const SuggestionKind = Object.freeze({
  DROP_FILTERS: 'dropFilters', SPELLING: 'spelling', DROP_TERM: 'dropTerm',
});

/** `SearchQuery.scope`: what the words are matched against (VFS-38); left out means `all`. */
export const Scope = Object.freeze({ ALL: 'all', NAMES: 'names' });

/** Order of a search over several sources (VFS-6). */
export const SortOrder = Object.freeze({
  RELEVANCE: 'relevance', NEWEST: 'newest', OLDEST: 'oldest', NAME: 'name', SIZE: 'size', SOURCE: 'source',
});

/** `ApiError.code`: every code a command can reject with (`Error::code` in error.rs). */
export const ErrorCode = Object.freeze({
  NOT_FOUND: 'not_found',
  NOT_A_DIRECTORY: 'not_a_directory',
  OVERLAPPING_ROOT: 'overlapping_root',
  UNAVAILABLE: 'unavailable',
  SCHEMA_TOO_NEW: 'schema_too_new',
  INVALID_PATTERN: 'invalid_pattern', // VFS-34: the message is the reason
  DB: 'db',
  IO: 'io',
});

/** Why a file-filter pattern is refused: the message of an `invalid_pattern` error (VFS-34 R2). */
export const PatternReason = Object.freeze({
  EMPTY: 'empty',
  SEPARATOR: 'separator',
  NO_LITERAL: 'no_literal',
  DUPLICATE: 'duplicate',
});

/** The UI's own rejection code, when no Tauri runtime answers; never sent by Rust. */
export const UiErrorCode = Object.freeze({ BACKEND_MISSING: 'backend_missing' });

/** `ContentStatus.phase.kind`: what the content run is doing (VFS-27 R8, R11, R15). */
export const ContentPhase = Object.freeze({
  OFF: 'off',
  PAUSED: 'paused',
  CHECKING: 'checking',
  DAMAGED: 'damaged',
  READING: 'reading',
  WAITING: 'waiting',
  FINISHED: 'finished',
  STOPPED: 'stopped',
});

/** `phase.reason` of a waiting content run (VFS-27 R6, R7). */
export const ContentWait = Object.freeze({
  NAME_INDEXING: 'nameIndexing',
  DISK_BUSY: 'diskBusy',
  CPU_BUSY: 'cpuBusy',
  IN_USE: 'inUse',
  LOW_SPACE: 'lowSpace',
});

/** `phase.reason` of a content run stopped for a run-level reason (VFS-27 R15, VFS-2 R17). */
export const ContentStop = Object.freeze({
  WORKER_CANNOT_START: 'worker_cannot_start',
  VERSION_MISMATCH: 'version_mismatch',
  WORKER_UNSTABLE: 'worker_unstable',
  STORE_WRITE: 'store_write',
  INDEX_WRITE: 'index_write',
});

/** Why a document's text was not read (VFS-2 R15, VFS-27 R14). */
export const UnreadReason = Object.freeze({
  CLOUD: 'cloud',
  SCANNED: 'scanned',
  PASSWORD: 'password',
  TOO_LARGE: 'too_large',
  UNSUPPORTED: 'unsupported',
  FAILED: 'failed',
});
