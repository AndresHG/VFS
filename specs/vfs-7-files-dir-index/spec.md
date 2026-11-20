# VFS-7 — Files indexed by folder only

- **Status**: implemented (this PR)
- **Size**: L (index schema migration, small)
- **Links**: VFS-4 (study `docs/research/index-footprint.md`, "Owner decisions"), VFS-1 AC-9

## Problem

`UNIQUE(dir_id, name)` on `files` stores every file name a second time (85 MiB of the
722 MiB 2 M-file index). Nothing looks files up by folder and name: the indexer lists a
folder's files by `dir_id` and diffs them in Rust. The owner decided (2026-11-20) to drop
it, accepting that when a word matches more than 2,000 equally ranked files, a different set
of 2,000 is shown (study, "Owner decisions").

## Requirements (EARS)

- R1. THE index SHALL find a folder's files through an index on `files(dir_id)` and keep no
  other index on `files`.
- R2. WHEN `db::open_rw` opens a version 1, 2 or 3 index, THE index SHALL be migrated to
  version 4 keeping every file id (the FTS rowids), then compacted as in VFS-4.
- R3. WHILE there is no uniqueness constraint, THE indexer SHALL still store each name once
  per folder.

## Acceptance scenarios

- AC-1 (R1) Given a new index, Then `files_dir` is the only index on `files` and serves the
  lookup by folder.
- AC-2 (R2) Given a version-2 index, When it is opened, Then it is at version 4 with its
  files, ids and FTS rows intact and no name index.
- AC-3 (R3) Given a tree re-walked after renames, moves and changes, Then no folder holds
  the same name twice.
- AC-4 Given the 2 M-file index, Then only queries capped at 2,000 with more equally ranked
  matches return a different subset (measured: 3 of 59 bench queries, same totals).

## Non-goals

- Changing ranking, caps, or the order within a page.

## Pre-mortem

- Slow PC / HDD: rebuilding `files` took 31 s at 2 M files (load 61) plus the `VACUUM`;
  once, at the first start after the update.
- Long paths, NFC/NFD, permissions, links, user files: n/a because only the index's own
  layout changes; names are stored as before.
- Killed mid-write: the migration is one transaction; the `VACUUM` is atomic and optional.
- A million-hit query: same caps; a different, equally ranked subset may fill them.
- Antivirus: as VFS-4, the one-time `VACUUM` writes a temporary copy.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Drop `UNIQUE(dir_id, name)` although tied capped results change | Yes | owner | 2026-11-20 |
