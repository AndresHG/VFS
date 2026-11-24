# VFS-40 — The index gives back space after large deletions

- **Status**: implemented (PR pending)
- **Size**: M (indexer idle work and one `meta` key; no schema change)
- **Links**: VFS-4 (schema v3: `files_fts` and `file_paths_fts` without `contentless_delete`,
  `docs/research/index-footprint.md`); VFS-1 AC-6 (integrity check after an unclean exit);
  VFS-27 T2 and rubric R10 (background work paced by measured cost)

## Problem

The owner's Mac index was 1.87 GB (`index_mb=1874`) for about 255 k remaining rows (228 k
files, 27 k folders). A first scan of a whole home folder indexed about 4.9 M files; about
3.6 M were later dropped (a new exclusion) and then the remaining 1.24 M-file root was removed
(`root_removed id=1 … files=1244237`). Measured on a copy of that index:

| | pages (16 KB) | free pages | `file_paths_fts_data` | `files_fts_data` | file |
|---|---|---|---|---|---|
| as found | 122,283 | 5,973 | 1,560 MB | 217 MB | 2,003 MB |
| after FTS merge | 122,283 | 118,115 | 21 MB | 4 MB | 2,003 MB |
| after merge + `VACUUM` | 3,794 | 0 | 21 MB | 4 MB | **62 MB** |

The space is not in the freelist: it is held by FTS5 segments. `files_fts` and
`file_paths_fts` are contentless without `contentless_delete` (VFS-4), so a deleted row is
removed by FTS5's `'delete'` command, which writes a tombstone as large as the original entry
into a new small segment. The original entry stays in a large old segment until both are
merged. Automerge only merges segments of the same level, and the indexer's
`('merge', 500)` at the end of a full walk only acts on a level that already has
`automerge` segments, so the small tombstone segments never meet the big ones: a removal
makes the index grow (1,595 → 1,874 MB here), and nothing ever shrinks it. Every query and the
integrity check after an unclean exit (VFS-1 AC-6) read through the dead entries; the file
keeps 1.9 GB of disk.

## Requirements (EARS)

- R1. WHEN rows of `files_fts` / `file_paths_fts` are deleted (walks, exclusions, file
  filter purges, folder renames, root removal), THE index SHALL add the number of files
  whose entries were deleted to the `meta` counter `fts_deleted`, in the same transaction.
- R2. WHILE the indexer is idle (no walk, purge, link pass or change flush pending) and `fts_deleted` is
  at least max(50,000, a tenth of the indexed files), THE indexer SHALL merge the FTS
  segments of `file_paths_fts`, `files_fts` and `dirs_fts` with FTS5's `('merge', -64)`, one
  slice per write transaction, at background priority, resting three times each slice's
  duration before the next (at most a quarter of the time busy). Any pending work, a pause
  or a stop ends the slice loop; it goes on at the next idle.
- R3. WHEN no table has merge work left, THE indexer SHALL subtract from `fts_deleted` the
  value it had when the merging began, then run `VACUUM` (temporary data in a file, a 2 MB
  page cache) only if the free pages are at least 64 MiB, at least as many as the used pages,
  and the used pages are at most 256 MiB. It SHALL write one diagnostic record:
  `compact deleted=<n> slices=<n> busy_ms=<n> vacuum=<done|skipped> index_mb=<before>-><after>`.
- R4. WHEN an index has no `fts_deleted` key (built by 0.3.0 or earlier), THE indexer SHALL
  set it to the threshold if the used pages exceed 1 KiB per indexed file plus 64 MiB, and to
  0 otherwise.

## Acceptance scenarios

Tests cite these ids (`VFS-40 AC-<m>`) in their doc comments.

- AC-1 (R1, R2, R3) Given an index with two roots, When the larger root is removed and the
  indexer goes idle, Then the FTS data of the removed files is merged away, the file is
  vacuumed to the size of an index built from the remaining root alone (within 25 %), every
  search returns what it returned before the removal for the remaining root, and
  `fts_deleted` is 0.
- AC-2 (R2) Given deletions below the threshold, When the indexer goes idle, Then no merge
  runs (the FTS data rows are unchanged).
- AC-3 (R2) Given compaction in progress, When the indexer is woken, paused or stopped, Then
  it leaves the slice loop at the next slice boundary, and the next idle finishes the work.
- AC-4 (R3) The `VACUUM` decision follows the three limits of R3: a large free share with a
  small used part vacuums; less than 64 MiB free, fewer free than used pages, or more than
  256 MiB used does not.
- AC-5 (R4) Given an index without `fts_deleted`, Then a bloated one gets the threshold and a
  healthy one gets 0.

## Non-goals

- Restoring `contentless_delete=1` on the per-file indexes: it refuses `columnsize=0`, costs
  about 50 MB at 2 M files (VFS-4) and needs a migration that rebuilds both indexes (L).
- Vacuuming a large index: above 256 MiB used, a `VACUUM` in WAL mode writes the whole index
  three times (temporary copy, WAL, checkpoint) while holding the write lock. The merge alone
  already removes the dead entries from every read and from the integrity check, which reads
  used pages only; the free pages are reused as the index grows.
- `auto_vacuum`: incremental vacuum moves pages to the end of the file and scatters the
  b-trees, more seeks on a spinning disk (`docs/research/index-footprint.md`).
- The content store (`content.db`, VFS-2): its FTS table already uses `contentless_delete`.

## Decisions

- D1. A counter, not a size probe: measuring the FTS data needs a scan of the whole table
  (1.5 GB here). Every file FTS deletion goes through `ingest::delete_file_paths_fts`, the one
  place that counts.
- D2. Threshold max(50,000, files / 10), like FTS5's own `deletemerge` default of 10 %: a full
  merge rewrites the whole FTS data, so it must not run for everyday churn.
- D3. `('merge', -64)`: a negative argument merges regardless of `automerge`. On the copy
  above (Apple Silicon SSD), 76 slices, p50 0.45 s, max 0.92 s, WAL at most 32 MB per slice,
  peak RSS 58 MB; with the R2 rest the owner's index compacts in about 2.5 minutes. On the
  target PC (CPU about 3–4× slower, HDD shared with Synology) slices are expected around 2–5 s
  and the whole job around 15–25 minutes, once. `optimize` does the same in one statement but
  peaked at 380 MB RSS and holds the write lock for 51 s here; rejected.
- D4. `VACUUM` with `temp_store=FILE` and a 2 MB page cache for that statement. The temporary
  database takes the connection's cache size, so with the writer's 32 MB cache and the
  index's `temp_store=MEMORY` the `VACUUM` of the merged copy peaked at 141 MB RSS; with a
  file and 2 MB, at 29 MB, in 1.6 s against 2.9 s. `PRAGMA integrity_check` and the search
  results were unchanged.

## Pre-mortem

- Slow PC + HDD: one-off, sliced, background priority, 25 % duty cycle, resumable (each slice
  commits; a partial merge is kept by FTS5).
- Killed mid-slice or mid-`VACUUM`: the transaction rolls back (WAL); the counter is only
  reduced after the last slice, so the work restarts at the next idle.
- Antivirus / long paths / NFC-NFD / cloud placeholders / user files: n/a, the work is inside
  the index only.
- A million-file match: n/a, no query change.
- Already shipped? The end-of-walk `('merge', 500)` (`indexer/mod.rs`) and `compact()` after
  migrations below v4 (`db.rs`); neither runs after deletions.
