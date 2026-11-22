# VFS-37 — Why a cloud-synced root looks small, in the diagnostic report

- **Status**: approved
- **Size**: L (index schema v7 with a migration)
- **Links**: issue 170, PR 174, VFS-20 (feedback report, R6), VFS-10 (unreadable folders),
  VFS-18 (links), VFS-25 (cloud-only files)

## Problem

Issue 170: a OneDrive root on Windows was indexed with 121 files and 30 folders and no
errors. PR 174 proved on real Cloud Files placeholders that VFS lists placeholders by name,
that a folder whose provider fails to fill it in lists as empty with no error from Windows,
and that a listing failing partway is now retried. What happened on the user's PC is still
unproven: folders OneDrive never filled in, OneDrive shortcuts to unsynced shared libraries
(links), or the truncation PR 174 fixed. The report's per-root line carries only files and
folders, which cannot tell these apart. The next report from such a PC must.

## Requirements (EARS)

- R1. WHEN the diagnostic report is built, THE report SHALL add to each root's line the
  root's cloud-only files (`files.flags` bit 2), links (bit 1), unreadable folders
  (`Root::unreadable`, VFS-10) and unpopulated empty folders (R2), as
  `cloud_only=<n> links=<n> unreadable=<n> unpopulated=<n>`; a count not yet known is `?`.
- R2. WHEN a folder lists with no entries on Windows and its own attributes carry
  `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (its contents are not all present locally), THE
  indexer SHALL mark that folder unpopulated; WHEN it next lists with entries, or without
  that attribute, or is recorded unreadable (VFS-10), THE indexer SHALL clear the mark. On macOS no folder is ever marked.
- R3. WHEN a full walk of a root ends, THE indexer SHALL store that root's cloud-only file
  and link counts on the root row; until a root's first full walk under schema 7 they are
  unknown (`?`). They are not refreshed by incremental passes.
- R4. THE root's unreadable and unpopulated counts SHALL be read live, each from a partial
  index that holds only the marked folders, so reading roots stays cheap at 5 M files.
- R5. THE added counts SHALL carry no path or name, so they go into the public summary with
  the rest of the root line (VFS-20 R6); the root's path stays only in the saved report.
- R6. THE indexer SHALL never open, read or hydrate a cloud-only file to mark a folder: the
  folder's attributes are read from its metadata, and only for a folder that listed empty.

## Acceptance scenarios

Tests cite these ids (`VFS-37 AC-<m>`) in their doc comments.

- AC-1 (R1, R5) Given a root with 7 cloud-only files, 2 links, 3 unreadable and 4
  unpopulated folders, When the report is built, Then its line reads
  `… cloud_only=7 links=2 unreadable=3 unpopulated=4 …`, the summary carries that line and
  not the root's path.
- AC-2 (R1, R3) Given a root whose cloud-only and link counts are unknown, When the report is
  built, Then the line reads `cloud_only=? links=?`.
- AC-3 (R3) Given a root with one cloud-only file and one symbolic link, When its full walk
  ends, Then the root reads cloud-only 1 and links 1; and a schema-6 index opens as schema 7
  with those counts unknown and every folder unmarked.
- AC-4 (R2, R6, Windows) Given a Cloud Files sync root whose provider fails every fill-in
  request, with an unpopulated empty placeholder folder, a populated empty one and an
  unpopulated one holding a placeholder file, When it is scanned, Then only the first is
  marked unpopulated (the root reads unpopulated 1), and no placeholder file is hydrated.
- AC-5 (R2) Given a folder marked unpopulated, When it lists again with entries, or is
  recorded unreadable, Then the mark is cleared.

## Non-goals

- Showing unpopulated folders in the app or treating them as unreadable: this change only
  reports them. Whether to tell the user is a product decision for later.
- Asking the provider to fill folders in, or retrying them: listing already asks once.
- Counting at report time across all files: too slow on a 5 M-file HDD.
- Live cloud-only and link counts between full walks (would need a partial index over files,
  up to 100 MB on an archive that is mostly cloud-only, or counters kept by every write).
- Unpopulated detection on macOS (File Provider): VFS-25 non-goal.

## Pre-mortem

- Slow PC, multi-TB HDD: the cloud-only and link counts are one pass over the root's file
  rows at the end of each full walk, which itself lists every folder (minutes to hours);
  report and root reads use partial indexes of marked folders only. Resumable: counts are
  written in the transaction that marks the root ready.
- Paths over 260 characters: the folder attribute is read through the same `std::fs` path
  the listing used; n/a beyond what listing already handles.
- NFC/NFD: n/a because no name is compared or stored.
- Antivirus: one metadata read per empty folder, Windows only; no file opened.
- Kill mid-write: the mark is set in the listing's transaction; the migration adds nullable
  columns in one transaction and rewrites no row.
- Million-hit queries: n/a because search is unchanged.
- Permission-denied, links, placeholders: a folder that fails to list stays unreadable
  (VFS-10), never unpopulated; links are counted, never followed; placeholders are the
  subject of R2 and R6.
- Opens or writes user files: no; R6.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Is the unpopulated count worth a schema change? | Yes: nullable `dirs.unpopulated` with a partial index; it is the only signal for the leading hypothesis of issue 170, and the column rewrites no row | agent, delegated by the issue-170 session | 2026-11-22 |
| 2 | Live cloud-only/link counts (partial index over files) or stamped at each full walk? | Stamped at each full walk on `roots`: no index growth on cloud-heavy archives; the root line already carries `last_full_scan` | agent, delegated (footprint first on the target PC) | 2026-11-22 |
| 3 | Read the folder's attributes from its parent's listing or after its own listing? | After its own listing, and only when it came back empty: reflects whether the provider filled it in, needs no change to how folder rows are inserted | agent | 2026-11-22 |

## Open questions

None.
