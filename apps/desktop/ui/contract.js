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

export const MatchKind = Object.freeze({ NAME: 'name', PATH: 'path', FUZZY: 'fuzzy' });

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
  DB: 'db',
  IO: 'io',
});

/** The UI's own rejection code, when no Tauri runtime answers; never sent by Rust. */
export const UiErrorCode = Object.freeze({ BACKEND_MISSING: 'backend_missing' });
