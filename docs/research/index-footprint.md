# How small and how cheap can the index be? (VFS-4)

Study of the name-and-path index on the 2,000,000-file synthetic archive, for the real target:
a slow, memory-starved Windows PC whose one 8 TB spinning disk holds the archive, the index
and a NAS sync client that keeps it busy. There, the cost of a query is the number of disk
seeks it pays, and a cold seek is roughly one database page read. Everything below is
**measured** unless marked otherwise; nothing was measured on the real PC or on any HDD.

## Machine, load and method

- **Machine**: Apple M1 Pro, 10 cores, 16 GB, internal APFS SSD, macOS 26.6.2. SQLite 3.54.1
  (bundled by rusqlite 0.43). Release builds.
- **Load**: other sessions shared the machine throughout; load average 28–90 during the runs
  (each figure is noted where it matters). Wall-clock latencies are therefore not comparable
  with AC-9's 150 ms threshold or with each other; the bench's own PASS/FAIL lines from these
  runs mean nothing either way. The load-robust warm figure is **thread CPU time per query**.
- **Database**: `vfs gen-db --files 2000000` (seed 42): 2,000,000 files, 153,846 folders,
  888.7 MiB, built in 193 s, peak RSS 151 MB. Each variant is a copy of that file, transformed
  outside the repository; sizes are MiB of database + WAL.
- **Query set**: the unchanged AC-9 set (59 queries, 10 classes) from `vfs bench`.
- **Pages read per query** (the seek proxy): a SQLite VFS shim logs every `xRead` on the
  database and WAL files. Each query runs on a freshly opened `Vfs` (empty page caches), so
  every distinct page is one read from the file; on a cold HDD each is about one seek, since
  the reads are almost never contiguous (time-ordered discontinuities ≈ pages in every run).
  "Distinct pages" is the union over the 59 queries: the hot set a cache would need.
- **First query**: a fresh process opens a freshly cloned file (APFS clone: a new file with no
  cached pages) and runs one query. On this SSD a page costs about 50–80 µs; on a 7,200 rpm
  disk with competing I/O it is closer to 8–12 ms [estimate].
- **Warm**: one `Vfs`, every query once to warm, then 10 rounds over the set, thread CPU time
  and wall time per search (`vfsx warm`), plus `vfs bench` for its peak RSS.
- **Correctness**: every page of every query (all hits, order, totals, truncation, folder,
  match kind) dumped by the old code on the old index and by the new code on each new index,
  compared byte for byte.

The harness (VFS shim, `pages`/`first`/`warm`/`dump` modes, scripts) is kept outside the
repository with the session notes; it needs nothing from the app beyond `vfs_core`.

## Where the pages go

On the version-2 index, the 59 queries read 178,178 pages from cold files. By table:

| Table | Pages | Share |
|---|---|---|
| `files` rows | 80,026 | 45% |
| `dirs` rows | 67,273 | 38% |
| `files_fts` | 11,731 + 897 | 7% |
| `dirs_fts` | 9,653 + 992 | 6% |
| `file_paths_fts` | 6,237 + 549 | 4% |
| indexes, roots, config | ~900 | <1% |

So 83% of the cold reads are table rows, one random page per candidate: each candidate file
read its row and its folder's row, alternating between the two tables. The FTS indexes are
most of the file (554 of 889 MiB) but under a fifth of the reads. Two consequences shape the
recommendations: reading fewer rows matters more than smaller FTS indexes for seeks, and
larger pages help because neighbouring candidates then share pages.

## Results

Main variants (the full gate runs on the last three rows' code; "v3" is the implemented
schema version 3 with the new search code, at three page sizes):

| variant | on disk (MiB) | bench peak RSS (MB) | warm CPU p50 / p95 (ms) | warm wall p95 (ms, load) | pages/query p50 / p95 | pages, 59 queries | distinct pages (MiB) | first query `iberdrola`: ms, pages | first `factura iberdrola 2015`: ms, pages |
|---|---|---|---|---|---|---|---|---|---|
| base (v2, 4 KB) | 889 | 123 | 26.4 / 97.4 | 439 (40) | 1759 / 12524 | 178178 | 47324 (185) | 748, 5130 | 1389, 4169 |
| + lazy folder rows, early exit | 889 | 108 | 19.9 / 97.7 | 430 (82) | 1514 / 6027 | 129471 | 41363 (162) | 466, 3538 | 732, 4170 |
| base, `VACUUM` 4 KB | 936 | 118 | — | — | 1895 / 12551 | 184521 | 50474 (197) | 193, 5227 | 478, 4525 |
| base, FTS `optimize` + `VACUUM` 4 KB | 935 | 118 | — | — | 1528 / 12475 | 168517 | 45674 (178) | 165, 4953 | 390, 3860 |
| base, `VACUUM` 8 KB | 869 | 144 | 16.1 / 58.7 | 81 (34) | 1260 / 6700 | 119093 | 26930 (210) | 277, 3874 | 819, 3170 |
| base, `VACUUM` 16 KB | 867 | 156 | 16.5 / 60.2 | 87 (39) | 864 / 3576 | 79534 | 16011 (250) | 250, 2573 | 645, 2589 |
| **v3, 4 KB** | 729 | 109 | 13.5 / 58.2 | 226 (83) | 1507 / 6027 | 129304 | 40352 (158) | 244, 3388 | 443, 3512 |
| **v3, 8 KB** | 724 | 125 | 10.7 / 47.6 | 134 (76) | 1127 / 3793 | 92743 | 25294 (198) | 172, 2717 | 473, 2831 |
| **v3, 16 KB (implemented)** | 722 | 138 | 9.8 / 42.8 | 107 (55) | 793 / 3132 | 66041 | 15695 (245) | 128, 1970 | 373, 2406 |

Per class (warm CPU p95 in ms, then cold pages p95):

| variant | single | multi | path | accent | typo | short | filter_only | text_filter | broad | no_hit |
|---|---|---|---|---|---|---|---|---|---|---|
| base CPU | 47.7 | 117.2 | 50.4 | 85.4 | 53.0 | 92.9 | 33.9 | 97.4 | 29.0 | 52.7 |
| v3 16 KB CPU | 29.6 | 105.2 | 25.9 | 35.1 | 32.3 | 9.5 | 13.4 | 36.8 | 7.0 | 34.8 |
| base pages | 6020 | 4245 | 1426 | 6797 | 3301 | 14955 | 6026 | 12524 | 1759 | 4604 |
| lazy rows | 4225 | 4246 | 1427 | 4129 | 3226 | 926 | 6027 | 11805 | 1562 | 4605 |
| v3 4 KB pages | 4212 | 3509 | 1405 | 4212 | 3963 | 926 | 6027 | 11783 | 1883 | 4677 |
| v3 8 KB pages | 3442 | 2830 | 981 | 3793 | 3534 | 462 | 3128 | 6073 | 1226 | 3240 |
| v3 16 KB pages | 2573 | 2405 | 735 | 3339 | 3132 | 234 | 1588 | 4468 | 744 | 1989 |

The real migration (`db::open_rw` on a copy of the version-2 file: migrate, then compact at
16 KB) took 70 s (load 33) and produced a 722 MiB file, no free pages, WAL mode, and results
identical to the old code on the old file. After rebasing on the latest phase-1 branch the
comparison was repeated: identical again, with the same page counts (178,178 → 66,041).

Summary of version 3 at 16 KB against version 2: on disk −19% (889 → 722 MiB; about 2.2 →
1.8 GB at 5 M files [extrapolated, linear]); pages read per query −55% at the median and −75%
at p95; −63% over the whole set; warm CPU −63% p50, −56% p95; peak RSS 123 → 138 MB (still
under half of AC-9's 300 MB); results identical.

### FTS5 options, sized one by one

Each index rebuilt alone from the base rows, then `optimize`d, 4 KB pages (`run.py`):

| index | variant | MiB | note |
|---|---|---|---|
| `files_fts` | today: positions, `contentless_delete=1` | 172 | 148 data + 24 per-row sizes |
| | `columnsize=0` (needs plain contentless) | 149 | −24 |
| | `detail=column` | 148 | positions in one column cost almost nothing more |
| | **`detail=none` + start marker, `columnsize=0`** | **58** | −114; implemented |
| `dirs_fts` | today | 75 | |
| | `detail=column`, `columnsize=0` | 57 | |
| | `detail=none`, `columnsize=0` | 19 | −56; not implemented (below) |
| `file_paths_fts` | today: `detail=none`, `contentless_delete=1` | 307 | |
| | **`columnsize=0`** | **282** | −25; implemented |
| one per-file index | path and name as two columns, `detail=column` | 818 | replaces two indexes totalling 479; rejected |

- `detail=none` cannot match phrases or `^` (start of column). Trigram phrases are replaced
  by their covering trigrams, and the start by the trigrams of U+0001 + term (every name is
  stored after U+0001); the search keeps of those candidates exactly the names the phrase
  would have matched, in the same rowid order, so caps and ranking see the same rows
  (`NameCond::holds`; unit test against a positional table).
- `contentless_delete=1` refuses `columnsize=0`; without it, rows are deleted with FTS5's
  `'delete'` command and the indexed value, which `files` and `dirs` still hold.
- `prefix=` indexes do not apply: trigram tokens are all exactly 3 characters.
- The tokenizer's `case_sensitive 0` / `remove_diacritics` cannot replace our folding: it
  does no NFKD and leaves the combining marks of NFD names (macOS) as characters, so `Len͂a`
  would not match `leña`. The keys stay folded by `fold_key`; the size is the same either way.
- `optimize` after the first scan: 21 s at 2 M files, no size change, −9% pages per query at
  4 KB (one segment instead of several per term). Its effect is folded into version 3, whose
  migration rebuilds both per-file indexes in one pass. `automerge`/`crisismerge` only change
  how fast the segments of an incrementally built index converge to that state; not measured
  separately.

### Table layout

- **Page size** is the largest single lever for seeks: at the same content, 8 KB reads 28%
  fewer pages per query than 4 KB and 16 KB 51% fewer, because candidates close in rowid
  order share pages and b-trees are shallower. The price is bytes: the hot set of the query
  set is 185 MiB at 4 KB and 250 MiB at 16 KB, and peak RSS rises by 15–30 MB (the 64 MB
  cache is the same in bytes). Warm CPU does not rise. 16 KB is implemented.
- **`VACUUM` alone at 4 KB** makes the file 5% *larger* (FTS leaf blobs of about 4,050 bytes
  spill into overflow pages when repacked) and does not reduce pages read. It only pays as
  part of a page-size change or after the migration frees pages.
- **`auto_vacuum`**: not recommended; incremental vacuum moves pages to the end of the file
  and fragments tables, the opposite of what a spinning disk needs. A one-time `VACUUM`
  after the migration does the compaction.
- **`UNIQUE(dir_id, name)`** stores every name a second time (85 MiB). Files are only ever
  looked up by folder, so a plain `files(dir_id)` index (22 MiB) would do: −63 MiB, and fewer
  pages for folder-driven reads. Not implemented: see "Owner decisions".
- **`WITHOUT ROWID`** for `files`: not possible, the FTS rowid is the file id.
- **Column order / integer packing**: `kind`, `size`, `mtime`, `flags`, `content_state` take
  about 12 of a row's ~80 bytes; packing them saves at most ~10 MiB at 2 M files. Not worth
  a migration. Dropping `name_key` (26 bytes per file, ~50 MiB) would need folding on every
  scanned row in the short-term scan; not measured.

### Directory paths

Folders store `rel_path` and `path_key`, 155 characters each on average (mean depth 8.4):
46 of the 56 MiB of `dirs`, plus 26 MiB in `UNIQUE(root_id, rel_path)`. A parent pointer +
name (18.6 characters) would save about 65 MiB at 2 M files [estimate from the column
sizes], but verification and display need the full path, rebuilt by walking up to 8 parents
(8 random reads per folder on a cold disk) unless the whole tree is kept in memory: about
5 MB at 2 M files and 13 MB at 5 M [estimate], loaded by one sequential read of a ~12 MiB
table. With the tree in memory, no query would read a `dirs` row at all (still 18% of the
reads after version 3). A rename would then update one row, but `dirs_fts` and
`file_paths_fts` still index full paths and must be rewritten for the subtree as today, so
the rename cost barely changes. This is the next structural step, not part of version 3.

### Alternatives to trigram FTS for names

- **In-memory name buffer, SIMD substring scan**: folded names joined in one buffer take
  25.8 MB per million names (29.6 MB with 4-byte offsets); a full `memchr::memmem` scan of the
  2 M-name buffer takes 1.7–8.7 ms on this machine (selective term to `img` with 442,318
  occurrences). At 5 M names: ~150 MB with offsets, before the `dir_id`, `kind`, `size` and
  `mtime` filters and ranking need (+16 bytes per file, ~80 MB) [extrapolated]. It removes
  every random read of the name tiers, but costs most of AC-9's 300 MB on a memory-starved
  PC and a sequential load of the `files` table at start (153 MiB at 2 M files, about
  2–4 s on a free HDD [estimate]). Not recommended while memory is the constraint; worth
  revisiting if the PC has memory to spare.
- **Suffix array / FM-index** over the same buffer: 4–5 bytes per character for a suffix
  array (~250 MB at 2 M names), 0.5–1 byte per character for an FM-index plus a locate
  sample; both need a rebuild for every change, and the watcher changes names all day.
  Rejected [estimate, not built].
- **Sorted token dictionary** (words of the names, with posting lists): smaller than trigrams
  but cannot find substrings inside a word (`notar` in `notaría2019`), which VFS-1 requires.
  Rejected.

### Cache sizing and pre-read

- Version 3, 16 KB pages, warm pass with the reader page cache at 16 MB instead of 64 MB:
  CPU p50 / p95 13.4 / 49.5 ms vs 10.8 / 48.1 ms (load 28–30), peak RSS of the warm-only
  process 38 MB vs 102 MB. On this machine the OS file cache holds the whole index, so a
  smaller SQLite cache costs almost nothing; on the target PC the OS cache is small and
  shared with the NAS client, and SQLite's own cache may be the only warm cache there is.
  Not changed: measure on the PC.
- **Pre-read at start**: after the migration's `VACUUM`, every table is one contiguous run in
  the file. A sequential read of `files` and `dirs` (205 MiB at 2 M files) into the OS cache
  would turn the ~2,000 random reads of a cold query into cache hits, at about 2–4 s of
  sequential disk time [estimate], and only if the OS keeps those pages. The FTS segment
  indexes (`*_idx`, under 1 MiB) and b-tree interior pages are the only small hot set; the
  rest of the hot set is the rows of whatever was searched. Not implemented: whether the OS
  keeps 200 MB of cache on that PC is the open question.

### Query side

- **Folder rows only when needed** (implemented): the name tiers read `files` alone when no
  folder condition applies (every root searched, none being removed, no folder filter); the
  folder row is read only to check a word missing from the name and to fill `dir` for the
  page returned. `dirs` reads over the set: 67,273 → 20,085 pages. It also stops the head
  from alternating between the two tables on every candidate.
- **Early exit** (implemented): once the list is full and marked truncated by the cap, no
  further statement runs. The folder scan after a full file scan read every directory for
  nothing (short class p95: 14,955 → 926 pages).
- **Fewer round trips per tier**: each tier is already one statement per table; the cost is
  the rows, not the round trips.
- **Covering indexes for filter-only queries**: a covering index would have to hold every
  column the hit needs, a second copy of `files`. Rejected.
- **No-hit queries**: already exit early after one probe (VFS-1 S8); `kwyjibo` still reads
  ~4,600 pages because a typo of it could exist (its split parts match), and the typo pass
  then reads its candidates.

## Recommendation, ranked by benefit on a cold, contended HDD

Implemented in version 3 (all result-identical, verified on every page of the query set):

1. **16 KB pages** (new indexes, and once after migrating): −51% pages per query on its own.
2. **Folder rows only when needed + early exit** (code only): −27% pages over the set, −70%
   of `dirs` reads; p95 −52% on its own.
3. **`files_fts` without positions** (start marker, exact re-check): −114 MiB.
4. **`columnsize=0` on both per-file indexes**, deleted by value: −49 MiB.
5. **One-time `VACUUM` after the migration**, so the freed fifth of the file is returned and
   every table is contiguous (prerequisite for any pre-read).

Together: 889 → 722 MiB, pages per query p50 1,759 → 793 and p95 12,524 → 3,132, first cold
single-word query 5,130 → 1,970 pages, warm CPU p95 97 → 43 ms, peak RSS 123 → 138 MB.

Not implemented, in order of expected benefit:

6. **Directory tree in memory** (parent pointer + name): removes the remaining 18% of reads
   that are `dirs` rows and ~65 MiB. A new spec.
7. **Sequential pre-read of `files` + `dirs` at start**: potentially most of the remaining
   cold reads, if the OS keeps the cache. Needs the real PC.
8. **Plain `files(dir_id)` index instead of `UNIQUE(dir_id, name)`**: −63 MiB. Owner decision.
9. **`dirs_fts` without positions**: −56 MiB; needs exact re-checks before its capped reads.
10. **Smaller SQLite cache** (16 MB): −64 MB RSS here; needs the real PC.

## Owner decisions (change which results are returned)

- **Dropping `UNIQUE(dir_id, name)`**: the single-word path tier reads a matching folder's
  files in index order and stops at the 2,000-hit cap. With the unique index that order is
  by name; with a plain `dir_id` index it is by file id. For `whatsapp` (a 60,000-file
  folder) the 2,000 path hits returned differ: same tier, same rank rule, a different subset
  of the 60,000 equally good matches. Nothing a user would call wrong, but different. Gain:
  −63 MiB (−9% of version 3) and denser folder-driven reads.

## What only the real PC can tell

- Seek cost per page on that disk under the NAS client's load, and whether OS readahead
  merges any of the near-contiguous reads; hence the real first-query time (the page counts
  above are machine-independent; the milliseconds are not).
- How much OS file cache Windows gives the index on that PC, and for how long: this decides
  whether 16 KB pages (fewer seeks, more bytes) beat 8 KB, whether a pre-read pays, and how
  small the SQLite cache can be.
- The migration and `VACUUM` time on the HDD at the real archive size (70 s at 2 M files on
  this SSD; minutes there [estimate]) and whether antivirus slows the temporary copy.
- The real archive's name and folder distribution (the synthetic one has deep paths and
  60,000-file folders by design), which moves every page count.
