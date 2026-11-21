# VFS-9 — design

## Fit

- `crates/vfs-core/src/indexer/cursor.rs`: the `History` trait (`current`, `replay`), the OS
  implementations, and `meta` storage (`cursor/<root id>`). `OsHistory` in the app and CLI,
  a fake in unit tests.
- `indexer/mod.rs`: `queue_rewalk` and a first scan's start store `history.current()` with
  the marks (R1); `Worker::init` arms the watcher, then `catch_up` replays (R2, R3, R7);
  `Worker::advance_cursors` runs every `CURSOR_EVERY` (R6); `finish_removing` deletes the
  cursor (R8). `mark_dirty`'s marking is shared with the replay (`mark_paths`), which marks
  the changed folder itself, not its parent.
- `indexer/watch.rs`: `Filter` is shared with the replay (R7).

## Windows

`GetVolumePathNameW` → refuse unless `GetDriveTypeW` is fixed/removable and the file system
is NTFS → open the volume root with `FILE_TRAVERSE` → `FSCTL_QUERY_USN_JOURNAL` (journal
id, first and next USN) → `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` from the saved USN up to the
next USN, 64 KB per call → distinct parent file references → `OpenFileById` (no access) +
`GetFinalPathNameByHandleW` → folder paths. Lost: other serial, other journal id, saved USN
below first or above next, any read error, a record not version 2.

## macOS

`FSEventsCopyUUIDForDevice` (no UUID → no history) → `FSEventsGetCurrentEventId` taken
before the stream → a stream on the root with `sinceWhen` = saved id, `WatchRoot`, on a
private dispatch queue → collect folder paths until `HistoryDone`. Lost: other UUID, saved
id ahead of now, a lost-history flag, no `HistoryDone` in 60 s. The live watcher stays
`notify`.

## Failure modes

- Crash between replay and commit: nothing stored; the next start replays again.
- Event ids assigned late (FSEvents): a change just before a cursor may be replayed once
  more; never skipped, since ids only grow.
- Watcher latency: R6 stores a cursor one interval (5 min) old, so every change before it
  has long reached `dirs`.
- Many roots on one volume: each reads the journal on its own; acceptable for a handful.

## Tests

`indexer/cursor.rs` (AC-6), `indexer/tests.rs` (AC-4 with `FakeHistory`, AC-5 on the real
OS), `tests/cursor.rs` (AC-1..AC-3 on the real OS, Windows and macOS CI),
`tests/indexer.rs::startup_catches_up_changes_made_while_closed` (no cursor → walk).
