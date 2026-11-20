// Closed sets of the command contract (VFS-1 spec, "Tauri commands and events"): the serde
// values of `crates/vfs-core/src/types.rs` and `api.rs`, named once for the whole UI.

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

/** `ApiError.code` values the UI branches on; `BACKEND_MISSING` is the UI's own (no Tauri). */
export const ErrorCode = Object.freeze({
  OVERLAPPING_ROOT: 'overlapping_root',
  BACKEND_MISSING: 'backend_missing',
});
