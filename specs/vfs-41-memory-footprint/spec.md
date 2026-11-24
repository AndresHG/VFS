# VFS-41 — A known memory ceiling, and a log that shows the footprint

- **Status**: implemented (PR pending)
- **Size**: M (diagnostic records and one process-wide SQLite setting; no schema change)
- **Links**: VFS-5 R7/R8 (diagnostic records, no names); VFS-1 AC-9 (300 MB budget); VFS-40
  (index compaction); VFS-20 (the feedback report keeps `vfs::diag` lines verbatim)

## Problem

On the owner's Mac, the log shows `memory working_set_mb` at 21–28 MB when idle. It rose to
99–155 MB after the 0.3.0 content run (about 25 k documents) and the opening of a second
root. The log samples once every 10 minutes; `vmmap` on the running app showed a **peak
physical footprint of 210 MB**, which no record caught. Nothing in the log tells apart what
holds the memory:

- Index page caches: 32 MB for the writer and 64 MB for each of the two persistent readers
  (lookups and search), up to 160 MB together. Each root-change command also opens a writer
  of its own. `mmap_size=0` keeps all of this on SQLite's heap (VFS-1 AC-9).
- The content store and the in-process content service. They use SQLite's default cache
  sizes, but FTS5 merges allocate on the same heap.
- The WAL (64 MB) is a file, not memory.

Memory goes back down when idle (21 MB after the run), because the OS reclaims the pages the
caches stopped touching, not because SQLite frees them. Nothing caps the caches as a whole.

The log cannot tell the owner what VFS costs on a user's PC: its CPU use when idle, its peak
memory, its disk I/O per pass, or how long the index took to open. The open matters most:
after an unclean exit, the integrity check reads every used page (VFS-39).

## Requirements (EARS)

- R1. WHEN the indexer writes its `memory` record (every 10 min), THE record SHALL also
  carry, for the whole process since it started:
  - `peak_mb`: peak resident memory;
  - `sqlite_mb` and `sqlite_peak_mb`: SQLite's heap now and at its highest;
  - `cpu_s`: CPU seconds;
  - `read_mb` and `written_mb`: bytes read and written.

  Idle CPU is the `cpu_s` difference between two records divided by the time between them.
- R2. WHEN a pass ends, THE `pass_end` record SHALL also carry the process's `read_mb` and
  `written_mb` during the pass.
- R3. WHEN the core opens the index, THE process SHALL set SQLite's soft heap limit to 96
  MiB, which every SQLite database in the process shares. Above it, page caches reuse their
  own pages instead of growing.
- R4. WHEN the index is opened, THE core SHALL write one record: `open ms=<n> checked=<0|1>
  recovered=<0|1> index_mb=<n>`.
- R5. Every new value SHALL be a count, a duration or a size. No record names a file or a
  folder (VFS-5 R8).

## Acceptance scenarios

Tests cite these ids (`VFS-41 AC-<m>`) in their doc comments.

- AC-1 (R1) On Windows and macOS, the process counters are known. The peak catches a 64 MB
  allocation freed before the next reading. A synced 4 MB write raises the written bytes, and CPU time grows.
- AC-2 (R1, R2, R5) A `memory` record carries every R1 field and none is unknown. A
  `pass_end` record carries `read_mb` and `written_mb`.
- AC-3 (R3) Once the index is open, the process's SQLite soft heap limit is 96 MiB.
- AC-4 (R4) The `open` record carries the time, whether the check ran, whether the index was
  rebuilt and its size, and nothing else.

## Non-goals

- The WebView process (WKWebView, WebView2). It is a separate process; `peak_mb` measures the
  core process only.
- A hard limit. It makes allocations fail, which shows up as an error in a search or a
  commit. The soft limit only stops the caches from growing.
- Freeing caches on a timer when idle. The OS already reclaims the untouched pages (21 MB
  idle in the log), and the limit caps the peak.
- Per-pass CPU. `cpu_s` every 10 min covers idle CPU and the cost of a scan; a per-pass value
  would add a third clock to read.
- Fewer `cannot list` warnings. They name folders but stay on the PC: the feedback report
  replaces every line outside `vfs::diag` (VFS-20).

## Decisions

- D1. One soft limit for the process, not smaller caches. A busy connection keeps its full
  cache while the others are idle; only the total is bounded.
- D2. 96 MiB: see the AC-9 bench on a 2 M-file index, before and after, in the PR.
- D3. On macOS, `getrusage` gives CPU, and `proc_pid_rusage` V4 gives the peak physical
  footprint (the figure `vmmap` shows; `ru_maxrss` lagged behind the resident size in tests)
  and the disk bytes read and written. On Windows, `PeakWorkingSetSize` and `GetProcessTimes` give peak
  memory and CPU, and `GetProcessIoCounters` gives bytes read and written. It counts all the
  I/O the process issues (files and devices), not disk alone. No new dependency or
  `windows-sys` feature.
- D4. The log stays small: the `memory` record grows by about 90 bytes, about 13 KB a day.

## Pre-mortem

- Slow PC + HDD: one more pair of system calls every 10 min and at each pass end;
  negligible.
- Killed mid-write, corrupt index, antivirus, long paths, NFC/NFD, placeholders, user files:
  n/a, nothing is written but log lines.
- A million-file match: once its cache stops growing, a broad search can read pages from
  disk again. The bench before and after (D2) measures that.
- Already shipped? The `memory` record (VFS-5 R7) had only the working set, WAL and index
  size, and no code set a heap limit (`grep soft_heap` found nothing).
