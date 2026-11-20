# VFS-3 — Robustness harness: crashes, hostile trees, properties, concurrency, soak

- **Status**: implemented (PR #85)
- **Size**: M
- **Links**: VFS-1 (`specs/vfs-1-core-search/spec.md`, AC-4..AC-11)

## Problem

The end user switches the PC off with programs open, has one busy HDD with a sync client
touching files, and an archive full of odd names. Phase 1 has unit and integration tests,
but nothing had killed the real indexer mid-write, fed it a hostile tree, fuzzed the query
path, raced two processes on one index or churned a tree for half an hour. This spec adds
those harnesses and makes every defect they find a test.

## Requirements (EARS)

- R1. IF the indexer process is killed at any point (scan, batch commit, re-walk, in-place
  folder rename, index recovery, schema migration), THEN the next start SHALL open the index,
  keep the root list, pass every structural invariant, and converge to the disk.
- R2. IF the database, its WAL or the session marker is truncated or overwritten, THEN the
  next open SHALL NOT fail, and the index SHALL converge to the disk.
- R3. WHEN several processes or threads use one index, THE core SHALL never move a healthy
  index aside, return a search error, or lose committed rows.
- R4. WHILE a tree is churned under the running app, THE indexer SHALL keep resident memory
  bounded and, once the churn stops, reach "up to date" with the index equal to the disk.

Structural invariants: `PRAGMA integrity_check` is ok; each FTS5 table passes its
`integrity-check` and holds exactly one entry per base row (no orphan, none missing); every
file and folder is found by its exact folded name; no duplicate or orphan rows; nothing left
pending after convergence.

## Acceptance scenarios

Process-level tests live in `crates/vfs-cli/tests/` (`crash.rs`, `concurrency.rs`,
`soak.rs`, shared `support/`); hostile trees and properties in `crates/vfs-core/tests/`
(`hostile.rs`, `props.rs`), citing the VFS-1 criteria they stress.

- AC-1 (R1) Given a first scan, When `vfs index` or `vfs run` is killed at a random moment
  (and sometimes the resume too), Then the index is consistent and converges.
- AC-2 (R1) Given a complete index and renamed, moved, deleted and new folders, When the app
  is killed during the catch-up re-walk or while the watcher applies a rename in place, Then
  the index converges.
- AC-3 (R1) Given a damaged index, When recovery is killed after any of its steps, Then the
  next open still holds the root list.
- AC-4 (R1) Given a version-1 index, When the migration is killed, Then the index is wholly
  at version 1 or wholly at the current version, and converges.
- AC-5 (R2) Given a stale marker and a WAL, When the database, WAL or marker is truncated,
  overwritten or has a page zeroed at a random offset, Then open succeeds, searches never
  fail and the index converges; when SQLite detects the damage (recovery) or only the marker
  is hit, every structural invariant holds too. Damage SQLite cannot detect may leave stale
  FTS entries (#103).
- AC-6 (R3) Given the indexer scanning and a flood of file events, When four threads search
  on two handles, Then no search fails and the index settles on the disk.
- AC-7 (R3) Given racing queries on one handle, Then every call returns `Ok` and a later
  query answers the same as a fresh handle.
- AC-8 (R1, R3) Given a running indexer, When it is shut down at a random moment, Then
  `shutdown` returns within 10 s and the next start converges.
- AC-9 (R3) Given the app and the command line opening, creating and indexing one index at
  the same time, Then neither recovers (moves aside) a healthy index and both roots converge.
- AC-10 (R4) Given `VFS_SOAK_SECS` of churn under `vfs run`, Then the index reaches up to date,
  equals the disk, and the second half's peak RSS is within 64 MiB of the first half's.

## Running

`cargo test` runs every scenario at a small size. `VFS_STRESS=1` runs hundreds of kill
iterations, 300,000-entry folders, paths past 32,000 characters and 50,000-file bursts;
`VFS_SOAK_SECS=1800` sets the soak length. `.github/workflows/stress.yml` runs both nightly
on Windows and macOS, in release.

## Non-goals

- Real power loss (SIGKILL keeps the OS page cache; torn writes are simulated instead).
- Hardware that lies about fsync.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Slow variants | Same tests, sized by `VFS_STRESS`, never `#[ignore]` | lead | 2026-11-20 |
| D2 | Recovery trigger | Only SQLite corrupt / not-a-database errors and a failed quick check; supersedes VFS-1 C5's "cannot be opened" | lead | 2026-11-20 |
| D3 | Start-up catch-up | Never deferred, not even behind an interrupted walk, and idle is not reported while it is owed; supersedes VFS-1 I10's rule for start-up requests (#95) | lead | 2026-11-20 |
