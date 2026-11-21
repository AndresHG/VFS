# VFS-9 — Resume from a change cursor instead of re-walking at every start

- **Status**: approved
- **Size**: L (a new platform-specific path on each system: the USN journal on Windows, an
  FSEvents history stream on macOS)
- **Links**: prior-art proposal P1 (`docs/research/prior-art-proposals.md`), prior art §2.8,
  §4, §5 (`docs/research/prior-art.md`), USN journal §4 (`docs/research/windows-io.md`), R5
  (`docs/research/windows-recommendations.md`), VFS-1 I4, I10, I11; issues 32, 34, 65

## Problem

Every time VFS starts it re-walks every root in full (VFS-1 I4), because changes made while
it was closed are not otherwise seen. On the target PC (a slow Windows machine, one 8 TB HDD,
Synology sync always running) a full walk of about 5 M files is hours of seeking at each
launch, competing with Synology, with "Comprobando cambios…" on screen most of the time.
Everything and Cardinal never re-walk at start: they replay the change history the operating
system already keeps (the NTFS USN journal, the FSEvents event store) from the position they
saved. VFS should do the same, and fall back to today's walk whenever that history cannot be
trusted.

## Requirements (EARS)

Terms. A **cursor** is a position in a volume's change history: on Windows the volume serial
number, the USN journal id and a USN; on macOS the FSEvents event-store UUID of the device and
an event id. A root's stored cursor means: every change made on disk before this position is
already in the index or marked pending in `dirs` (VFS-1 AC-4). Replaying a cursor turns every
change after it into "re-list this directory" marks, the same marks a watcher hint makes.

- R1. WHEN a full walk of a root starts (first scan, catch-up, manual, periodic, overflow,
  back online), THE indexer SHALL store for that root the cursor taken just before the root
  directory is marked pending, in the same transaction.
- R2. WHEN the indexer starts, THE indexer SHALL first arm the watcher, then, for each root
  with a stored cursor, replay the history from that cursor to now, mark the directories that
  changed pending shallow (a directory not indexed marks its nearest indexed ancestor), and
  store the new cursor in the same transaction as the marks; it SHALL NOT queue a full walk
  for that root.
- R3. IF a root has no stored cursor, or the stored cursor cannot be read, belongs to another
  volume, journal or event store, or the history no longer reaches it, THEN THE indexer SHALL
  owe that root the full catch-up walk exactly as today (VFS-1 I4, I10).
- R4. IF, on Windows, the root is not on a local NTFS volume (FAT, exFAT, ReFS, a network
  drive), the volume has no active journal, the journal id changed, the saved USN is below the
  journal's first USN or above its next USN, the unprivileged read
  (`FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`) fails, or a record is not version 2, THEN THE replay
  SHALL report the history as lost (R3).
- R5. IF, on macOS, the device has no persistent event store (no UUID), the UUID changed, the
  stored id is above the current event id, the history stream delivers `MustScanSubDirs`,
  `UserDropped`, `KernelDropped`, `EventIdsWrapped`, `RootChanged`, `Mount` or `Unmount` for
  the root, or `HistoryDone` does not arrive within 60 s, THEN THE replay SHALL report the
  history as lost (R3).
- R6. WHILE the indexer runs, THE indexer SHALL advance a watched root's cursor every 5 min to
  the position it took at the previous check, after the pending watcher hints are written,
  provided the root was watched without a lost-events report (overflow, watch failure) since
  that position was taken.
- R7. WHEN a replay yields paths, THE replay SHALL drop the paths outside every root, under the
  app data directory, and with a component the walker excludes (VFS-1 I3, I8).
- R8. WHEN a root is removed, THE indexer SHALL delete its cursor with its rows.
- R9. THE replay SHALL need no administrator rights, open no user file (on Windows it opens
  changed directories by file id with no access rights, to learn their path), and read the
  history in one stream at the indexer's background priority.

## Acceptance scenarios

Tests cite these ids (`VFS-9 AC-<m>`) in their doc comments. AC-1, AC-2, AC-3 and AC-5 run on
the real OS in CI (Windows and macOS); AC-4 runs everywhere through a fake history source.

- AC-1 (R1, R2) Given a root fully indexed with a stored cursor and the app closed, when files
  are added, removed and renamed in two folders and the indexer starts again, then after it is
  idle the index matches the disk; the root stayed ready with the same `last_full_scan_at`; and
  a row deleted from the index by the test in an untouched folder is still missing (that
  folder was not re-listed, so no full walk ran).
- AC-2 (R1, R2) Given a first scan killed halfway (the root still scanning, a cursor stored at
  its start), when a file is added to an already-listed folder while closed and the indexer
  starts again, then the walk resumes, the new file is indexed, and no second full walk is
  owed.
- AC-3 (R3, R4, R5) Given a root with a stored cursor that is unparsable, from the other
  platform, or names another journal id / event-store UUID, when the indexer starts, then the
  root gets the full catch-up walk (the deleted row of AC-1 comes back).
- AC-4 (R2, R3, R6, R8) With a fake history source: a "lost" replay owes a full walk and keeps
  no new cursor until that walk starts; a replay with paths marks exactly their folders and
  stores the new cursor; a lost-events report stops the cursor advancing until the next full
  walk stores one; removing a root deletes its cursor.
- AC-5 (R6) Given a running indexer watching a root, when two checks pass, then the stored
  cursor has moved forward from the one stored by the first scan.
- AC-6 (R4, R5, R7) Unit tests: USN record parsing (version 2 accepted, other versions lost);
  the volume check refuses non-NTFS file systems and network drives; FSEvents flags that mean
  lost history are classified as lost; excluded names and the data directory are dropped.

## Non-goals

- No change to the first scan's speed (enumeration without admin stays a directory walk;
  prior art §4).
- No per-file id map and no schema change: the cursor lives in the existing `meta` table
  (`cursor/<root id>`), and Windows records are keyed on their parent directory, resolved by
  file id when replayed (windows-io §4.2, the "simpler use").
- No replacement of the live watchers (Windows `ReadDirectoryChangesW`, macOS `notify`), and
  no journal replay on a watcher overflow (R6 of the Windows study; a later change).
- No leaf-directory mtime skip as a no-journal fallback (issue 34 stays open).
- The weekly safety re-walk (VFS-1 I4) and the one-hour cooldown of deferred walks (I10) are
  unchanged.
- No change to the back-off and priority policy of VFS-5.

## Pre-mortem

- Slow PC, multi-TB HDD: replay reads the journal (32 MB by default) sequentially and resolves
  one MFT record per changed directory, then shallow re-lists only those directories: seconds
  to minutes instead of hours. It runs on the indexer thread at background priority. Killed
  halfway, nothing is stored and the next start replays again from the same cursor.
- Long paths and case: Windows paths come from `GetFinalPathNameByHandleW` (the same call
  `canonicalize` uses for root paths, so prefixes match) and are simplified with `dunce`;
  FSEvents reports canonical paths, as root paths are stored.
- NFC/NFD: paths are only matched against root prefixes and `dirs.rel_path` from the same OS
  APIs; no folding involved.
- Antivirus: the journal records Defender's own changes (attributes, streams); they only add
  shallow re-lists. The index database lives outside roots (VFS-1 I3, R7).
- Killed mid-write: the cursor is stored in the same transaction as the marks or the walk
  start (R1, R2); a crash before commit replays again.
- Million-file match: n/a because search is unchanged.
- Permission-denied, links, cloud placeholders: a directory the user cannot open by id is
  skipped (its parent's record still marks the parent when entries are added or removed).
  Opening a directory by id with no access does not hydrate a placeholder (directories are
  populated on enumeration, which the walk does anyway). Links are never followed (walk
  unchanged).
- User files opened or written: only directories are opened, with no access rights, to read
  their path (R9). Nothing is written.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Where does the cursor live? | `meta` key `cursor/<root id>`; no schema version | Claude (code: `meta` is unused) | 2026-11-20 |
| 2 | Windows: map every file id, or key on the parent directory? | Parent directory, resolved by `OpenFileById` at replay; no map, no extra work during the walk | Claude (windows-io §4.2, R5) | 2026-11-20 |
| 3 | macOS: replace `notify` with an own FSEvents loop? | No: a one-shot history stream at start, `notify` stays the live watcher; the watcher is armed first so there is no gap | Claude | 2026-11-20 |
| 4 | Do folders synced to a Synology fall back to the full walk? | No. The archive is a local folder on the user's PC that Synology backs up; local NTFS folders are replayed from the journal. Only network drives (a mapped NAS share), non-NTFS disks and lost history fall back | owner (answered: the big local folder is backed up to the Synology) | 2026-11-20 |
| 5 | Approve the spec | Approved; the weekly safety re-walk stays | owner | 2026-11-20 |

## Open questions

None blocking. Known risk: `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` is undocumented and CI runs
as administrator, so the standard-user path is proven only by the probe on the real PC
(windows-io §4.6); every failure falls back to the walk (R3).
