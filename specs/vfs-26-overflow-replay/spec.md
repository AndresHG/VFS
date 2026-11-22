# VFS-26 — Watcher overflow: replay the change history instead of a full re-walk

- **Status**: implemented (PR #145); approved after an independent architect review
- **Size**: M (indexer and watcher only; no schema change; uses VFS-9's change history)
- **Links**: issue 47; R6 in `docs/research/windows-recommendations.md`; VFS-9 (change
  cursor, `indexer/cursor.rs`); VFS-1 I10 (lost-events walks wait for a one-hour cooldown)

## Problem

When the live watcher loses events (Windows: `ReadDirectoryChangesW` returns 0 bytes or
`ERROR_NOTIFY_ENUM_DIR` because its 64 KB buffer overflowed; macOS: FSEvents asks for a
rescan), VFS owes the root a full re-walk, deferred until its last full scan is an hour old
(VFS-1 I10). On the target PC a full walk of the archive is hours of seeking, and Synology's
sync client produces exactly the bursts that overflow the buffer. Since VFS-9 each root keeps
a change cursor, so the changes the watcher dropped can be read back from the NTFS journal or
the FSEvents store in seconds.

## Requirements (EARS)

- R1. THE watcher SHALL report each loss of events as an overflow or a failed watch:
  - overflow (the watch stays armed): Windows `ReadDirectoryChangesW` completing with 0 bytes
    or with `ERROR_NOTIFY_ENUM_DIR`; macOS a `notify` event that needs a rescan (for the root
    holding its path, or every root when none does, as today);
  - failed watch: Windows the event cannot be created, the call cannot be armed, the
    completion fails with any other error, or the handle is reopened after closing; macOS a
    `notify` error.
- R2. WHEN a root overflows, THE indexer SHALL drop that root's R6 candidate cursor (VFS-9)
  and mark the root "replay due" 10 s after this overflow, pushed back by every further
  overflow of that root but never beyond 60 s after the first overflow of the burst, so a
  burst gives one replay; at most one replay per root is due at a time. WHILE a root's
  replay is due, THE cursor checks (VFS-9 R6) SHALL take no candidate and store no cursor
  for it, as for a held root, without owing a walk. Both delays are parameters of the
  indexer, so tests need no real waiting.
- R3. WHEN a root's replay is due and the root has a stored cursor, is reachable and is not
  being removed, THE indexer SHALL first write the pending watcher hints, then replay the
  root's history from the stored cursor (VFS-9 `History::replay`), mark the changed folders
  pending shallow (filtered as VFS-9 R7) and store the new cursor, in one transaction, and
  owe the root no walk for this overflow. If the root was held (VFS-9 R6: an earlier loss),
  the hold is released by the new cursor and a walk owed for that earlier loss stays owed.
- R4. IF the replay cannot run or fails (no cursor, history lost, root unreachable or being
  removed, a database error), THEN THE indexer SHALL do what it did before VFS-26: hold the
  cursor (VFS-9 R6) and owe the deferred lost-events walk (VFS-1 I10).
- R5. WHEN a watch fails (R1), THE indexer SHALL cancel any replay due for that root and do
  what it did before VFS-26 (hold the cursor and owe the deferred lost-events walk),
  whatever the cursor. A later overflow of that root (its watch is armed again) may replay
  and release the hold: the replay starts at the held cursor, so it covers the gap.
- R6. THE replay SHALL run on the indexer thread between steps, wrapped like a listing in
  the scheduler's background mode while the indexer is in incremental mode, and SHALL be
  bounded by VFS-9's own limits (the journal read since the cursor on Windows, the 60 s
  `HistoryDone` timeout on macOS). Each replay SHALL write a diagnostic line: root, folders
  marked, milliseconds, and replayed or lost.

## Acceptance scenarios

Tests cite these ids (`VFS-26 AC-<m>`) in their doc comments.

- AC-1 (R2, R3; fake history, short delays) Given a watched root with a stored cursor, When
  it overflows three times within the quiet delay, and a cursor check runs before the
  replay, Then that check leaves the stored cursor unchanged and takes no candidate; the
  fake is replayed once, exactly the two folders it names are pending shallow, the new
  cursor is stored and no walk is owed; after two more cursor checks the stored cursor
  equals the candidate taken at the first of them. Overflows that keep coming still give a
  replay by the maximum delay.
- AC-2 (R3; fake history) Given a held root (a walk owed for an earlier loss), When it
  overflows and the replay succeeds, Then the stored cursor is the replay's, the hold is
  released, and the earlier walk is still owed.
- AC-3 (R4; fake history) Given an overflow with no cursor, with a replay reporting the
  history lost, or with an unreachable root, Then a deferred walk is owed and the cursor
  holds, as before VFS-26.
- AC-4 (R5; fake history) Given a root with a stored cursor, When its watch fails, Then no
  replay runs and a deferred walk is owed; Given an overflow and then a failed watch before
  the replay is due, Then the replay never runs, the cursor holds and the walk is owed.
- AC-5 (R1) Unit tests: the Windows completion classifier maps 0 bytes and
  `ERROR_NOTIFY_ENUM_DIR` to an overflow and other errors to a failed watch; the macOS
  `notify` handler reports a rescan event as an overflow of its root (of every root when no
  root holds its path) and a `notify` error as a failed watch.
- AC-6 (R3; real OS, Windows and macOS CI) Given a root indexed with a stored cursor, When a
  folder is created and an overflow is injected through the watch sink (a real buffer
  overflow cannot be provoked reliably), Then the replay of the real history marks its
  parent and the new folder is indexed without a walk. Where the volume keeps no usable
  history (no NTFS journal), the test says so and checks the walk fallback instead.

## Non-goals

- Replaying after a failed watch once it is armed again: sound (a replay never leaves a hole
  in what the cursor promises), but it adds a second trigger; kept for a later change, the
  lost-events walk remains its fallback.
- Narrowing the watch filter to names while a root is walked (issue 47's second idea): a
  size or time change in a folder the walk has already listed would then be missed until
  the next start; it also needs the measurement of issue 47 on the target PC first.
- `ReadDirectoryChangesExW` with extended information (issue 47's third idea): a later
  change, if the measurement shows size events dominate.
- No change to the buffer size (64 KB is the limit for network paths and enough locally).

## Pre-mortem

- Slow PC, multi-TB HDD: a Windows replay reads the volume's journal since the cursor and
  opens every changed parent folder on the volume by id before keeping those under the root
  (VFS-9), so it is not free: coalescing (R2) gives one replay per burst, and the cursor it
  stores makes the next one short. A clean root's cursor is 5 to 10 minutes old (VFS-9 R6);
  a held root's can be older, so its first replay may read longer, once. Still seconds to
  minutes against an hours-long walk.
- Long paths and case: as VFS-9 (paths from `GetFinalPathNameByHandleW` / FSEvents).
- NFC/NFD: n/a because nothing is folded; paths are matched as in VFS-9.
- Antivirus: Defender's own changes only add shallow re-lists, as in VFS-9.
- Killed mid-write: marks and cursor are stored in one transaction; a crash before commit
  leaves the old cursor, which the next start replays.
- Million-file match: n/a because search is unchanged.
- Permission-denied, links, cloud placeholders: as VFS-9 (folders opened by id with no
  access; links never followed).
- User files opened or written: none.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Which of issue 47's three ideas? | Only the replay: it removes the hours-long walk with the history VFS-9 already keeps; the other two need measurement and carry a miss risk | Claude | 2026-11-21 |
| 2 | Failed watch too? | Not in this change: a replay when the watch is re-armed would be sound, but it is a second trigger to build and test; the lost-events walk stays its fallback | Claude (architect review) | 2026-11-21 |
| 3 | A burst of overflows | One replay per root, 10 s after its last overflow and at most 60 s after its first (R2); the root's cursor is frozen meanwhile, so no check stores a cursor past the dropped changes | Claude (architect review) | 2026-11-21 |

## Open questions

None.
