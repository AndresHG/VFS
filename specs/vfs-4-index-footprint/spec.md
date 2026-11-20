# VFS-4 — A smaller index that reads fewer pages per query

- **Status**: implemented (this PR)
- **Size**: L (index schema migration)
- **Links**: VFS-1 (AC-9, S2, S8), study `docs/research/index-footprint.md`

## Problem

The reference machine keeps both the 4–5 TB archive and the index on one slow, contended 8 TB
spinning disk, with little memory for the OS file cache. A cold query there pays about one
disk seek per database page it reads. On the 2 M-file synthetic index (schema version 2,
889 MB), a single-word query reads about 5,000 pages from a cold file: tens of seconds on
such a disk. Most of those pages are `files` and `dirs` rows, not the FTS indexes; the FTS
indexes are still most of the file. The scope was set by the owner's brief: make the index
and search as small and cheap as possible without losing correctness, measured, and
implement what does not change results.

## Requirements (EARS)

- R1. WHEN a query runs on schema version 3, THE search SHALL return the same hits, in the
  same order, with the same totals and truncation, as on version 2.
- R2. WHEN `db::open_rw` opens a version 1 or 2 index, THE index SHALL be migrated to
  version 3 from its rows (no rescan) and then compacted at `PAGE_SIZE`; IF compaction
  fails, THEN the migrated index SHALL stay valid and open.
- R3. WHEN a file hit needs no directory condition (no folder filter, every root searched,
  none being removed) and its name holds every term, THE search SHALL not read its
  directory row except to fill `dir` for the page it returns.
- R4. WHEN a file or a directory is deleted, renamed or moved, THE per-file FTS rows SHALL
  be removed by the values they were indexed under, leaving no stale rows.
- R5. WHEN the hit list is full and already marked truncated by the cap, THE search SHALL
  run no further statement.

## Acceptance scenarios

- AC-1 (R1) Given the AC-9 query set on the 2 M-file index, When it runs on version 2 with
  the old code and on version 3 with the new code (4, 8 and 16 KB pages), Then every page of
  every query is identical (measured in the study); and the name conditions on the
  position-less `files_fts` keep exactly the rows the phrase expressions matched (unit test).
- AC-2 (R2) Given a version-2 index, When it is opened, Then it is at version 3, in WAL mode,
  with no free pages, at `PAGE_SIZE`, and its migrated rows are found and deleted.
- AC-3 (R3) Given hits in several folders, When pages are requested one by one, Then each
  hit names its folder; and a root being removed keeps its files out.
- AC-4 (R4) Given a deleted file, a deleted subtree, a removed root or a renamed folder,
  When the FTS tables are queried, Then the old names and paths match nothing (existing
  indexer and db tests, now on the delete-by-value path).
- AC-5 (R1–R5) Given the 2 M-file index, When the study harness runs, Then on-disk size and
  pages read per query fall, peak RSS stays under AC-9's 300 MB, and the bench's AC-9 query
  set and thresholds are unchanged.

## Non-goals

- Changing which results a query returns, the 2,000-hit cap or the 4,000-candidate name
  read cap.
- Dropping `UNIQUE(dir_id, name)` (63 MB): it changes which of more than 2,000 equally
  ranked path hits are returned; left to the owner (study, "Owner decisions").
- `detail=none` on `dirs_fts` (55 MB): its capped directory reads would need an exact
  re-check before the cap; not done here.
- An in-memory directory table or name buffer.

## Pre-mortem

- Slow PC / HDD: the migration rewrites the two per-file FTS tables and then the whole file
  (`VACUUM`): about 65 s + 11 s at 2 M files on the development Mac; several minutes at
  5 M files on the target HDD, once, at the first start after the update.
- Long paths: n/a because paths are not touched.
- NFC/NFD: n/a because keys are folded as before; only how they are indexed changes.
- Antivirus: the `VACUUM` writes a full temporary copy of the index next to it; an
  antivirus scanning it slows the one-time compaction, nothing else.
- Killed mid-write: the migration is one transaction; the `VACUUM` is atomic; a failed
  compaction leaves a valid, uncompacted version-3 index.
- A million-hit query: unchanged caps; it reads fewer pages (lazy directories, early exit).
- Permission-denied, links, placeholders: n/a because the walk is unchanged.
- User files: n/a because only the index file is written.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Change results for size? | No: only result-identical changes ship; the others are listed for the owner | owner's brief | 2026-11-20 |
| 2 | Page size | 16 KB: fewest pages per query and least CPU; costs a larger hot set in bytes (see the study) | measured | 2026-11-20 |

## Open questions

- Which page size and pre-read pay off on the real PC: only measurable there (study).
