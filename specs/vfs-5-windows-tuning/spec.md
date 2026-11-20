# VFS-5 — Windows tuning for the first install on the reference machine

- **Status**: approved (from the coordinating session's brief, 2026-11-20)
- **Size**: L (platform-specific scheduling, Windows-only probes, connection pragmas)
- **Links**: [`docs/research/windows-recommendations.md`](../../docs/research/windows-recommendations.md)
  R1, R2, R3, R4, R7, R8; [`windows-io.md`](../../docs/research/windows-io.md);
  [`target-pc-measurement-plan.md`](../../docs/research/target-pc-measurement-plan.md);
  issues 19, 20, 45, 46, 48; builds on VFS-1 (PR 61)

## Problem

The reference machine is a slow Windows machine with one 8 TB spinning disk that holds the archive and
the index, a Synology client syncing the same folders all day, and Defender scanning. Today the
indexer thread enters Windows background mode once for its whole life. Background mode issues
very-low-priority I/O, which Windows only sends after about 50 ms of disk silence, so under
Synology and Defender load the first walk could take days instead of hours (estimate, never
measured). The same thread commits and checkpoints in that mode, so it can hold SQLite's write
lock while starved and block the UI's root changes for up to the 30 s busy timeout. Nothing
backs off when the disk is busy or right after boot, nothing records how indexing really
behaves over weeks, and the SQLite settings were chosen on a Mac. This change makes the first
installation safe and observable, with every guessed threshold named, defaulted and tied to the
step of the measurement plan that will replace the guess.

## Requirements (EARS)

- R1. WHILE a root is walked in full (first index or re-walk), or more than
  `incremental_max_dirs` directories are pending, THE indexer thread SHALL run at normal I/O
  priority, lowest CPU priority and low memory priority (the *walk profile*).
- R2. WHILE no root is walked in full and at most `incremental_max_dirs` directories are
  pending, THE indexer SHALL list each directory in background mode (very-low I/O) and SHALL
  leave background mode before touching the index.
- R3. THE indexer SHALL never begin a write transaction, commit, checkpoint or run
  `quick_check` while its thread is in background mode.
- R4. WHILE the walk profile is active and the disk holding the roots is busy, THE indexer SHALL
  pause between directories, without holding the write lock, and SHALL resume when the disk
  calms down; IF it has paused for `max_backoff`, THEN it SHALL work for at least `min_work`
  before pausing again.
- R5. WHEN the indexer starts within `boot_grace` of the system booting, THE start-up catch-up
  re-walks SHALL wait until the system has been up for `boot_grace`; first scans of new roots,
  manual re-walks and watcher work SHALL NOT wait.
- R6. IF the disk-load probe is unavailable, THEN THE indexer SHALL treat the disk as never busy
  and SHALL record that once in the diagnostic log.
- R7. WHILE the indexer runs, THE app SHALL write a diagnostic record to its existing rotating
  log file: every minute of a pass and at its end (mode, directories, files, directories and
  files per minute, back-off seconds, commits and their durations, checkpoints and their
  durations, retries pending, errors), on every watcher overflow, and every 10 minutes (working
  set, WAL size, index size).
- R8. THE diagnostic records SHALL contain no file or folder name, and the log file SHALL
  receive info-level records only from the diagnostic target.
- R9. WHEN the app opens the index, THE app SHALL mark the index folder not content-indexed on
  Windows and SHALL log a warning when the index folder lies inside a Synology sync root.
- R10. THE index writer SHALL grow the database file in `chunk_size` steps on Windows, and THE
  page caches SHALL keep VFS-1 AC-9 (warm p95 under 150 ms per class, RSS under 300 MB).
- R11. WHEN the main window is minimised on Windows, THE app SHALL hide the WebView2 controller
  and ask it to suspend; WHEN it is restored, THE app SHALL make it visible again.

## Configuration (named, with documented defaults)

All values live in `vfs_core::indexer::sched::SchedConfig::default()` (R1–R6) or as named
constants in `db.rs` (R10) and the desktop shell (R7). Only `boot_grace` is public, through
`vfs_core::IndexerOptions`, because tests on a freshly booted CI runner need it at zero. Every default is an estimate until the
measurement named next to it exists.

| Name | Default | Why this value | Replaced by |
|---|---|---|---|
| `incremental_max_dirs` | 64 | At the worst very-low I/O rate (about 1 request/s, `windows-io.md` §2.2) and ~3 I/Os per listing, 64 directories finish in about 3 minutes; larger batches are walk-like and get the walk profile | N4 (step 3) |
| `sample_every` | 1 s | PDH rate counters need two collections; 1 s is the usual minimum interval and costs one cheap call per second | — |
| `busy_queue` | 2.0 | `Avg. Disk Queue Length` of the roots' physical disk. The indexer issues one synchronous request at a time, so it contributes at most 1; 2 or more means someone else is waiting for the disk. `% Idle Time` cannot be used, because the walk itself drives it to zero | N10 (step 6) |
| `window` / `busy_in_window` | 5 / 3 | Busy when 3 of the last 5 samples are busy: one burst (a Firefox cache flush) does not stop the walk, a sustained queue does | N10 |
| `max_backoff` | 8 s | Longest continuous pause | N10, N13 |
| `min_work` | 2 s | Work slice forced after `max_backoff`; with 8 s it guarantees at least 20 % duty, so a permanently busy disk makes the first index at most 5× slower instead of stalling | N10, N13 |
| `boot_grace` | 10 min | Synology and Defender do their own start-up scanning in the first minutes after login (estimate, `windows-recommendations.md` R3) | step 7.4, step 8 log |
| `diag_every` | 60 s | One line per minute of work: about 300 KB per day of continuous indexing | — |
| `diag_memory_every` | 10 min | Working set and WAL size over weeks, ~15 KB per day | — |
| `LOG_KEEP` (desktop shell) | 4 old files of 1 MB | At most 5 MB on disk; about two weeks of mostly idle use, three days of continuous indexing | — |
| `WRITER_CHUNK_BYTES` | 32 MiB, Windows only | `SQLITE_FCNTL_CHUNK_SIZE` "may reduce" fragmentation of a file that grows while Synology writes to the same disk (`windows-recommendations.md` R7). On Windows SQLite extends with `SetEndOfFile`, which allocates without writing; on macOS it writes a byte per block, so it is off there | N12 |
| Reader page caches | unchanged, 64 MiB per reader (Decisions 6) | Bounded RSS on a PC that already swaps; a cut to 32 MiB waits for a measurement that can resolve its cost | N12 |

## Acceptance scenarios

Tests cite these ids (`VFS-5 AC-<m>`) in their doc comments. "Measured on the PC" names what the
first field test checks; the plan's step numbers are in `target-pc-measurement-plan.md`.

- AC-1 (R1) Given a root waiting for its first index (or a full re-walk), When the indexer lists
  its directories, Then the thread is in the walk profile for every listing and never enters
  background mode. *Test*: recorded priority calls during a scan. *Measured on the PC*: step 8,
  Process Monitor's *Priority* column shows normal I/O on the walk; diag log `mode=walk`.
- AC-2 (R2) Given every root ready and at most `incremental_max_dirs` directories marked by the
  watcher, When the indexer lists them, Then each listing runs between one background-begin and
  one background-end; with more pending directories it uses the walk profile. *Test*: recorded
  calls. *Measured*: diag log `mode=incremental` lines after a file change.
- AC-3 (R3) Given an incremental pass, When the indexer commits, Then no commit and no
  checkpoint happens between a background-begin and its background-end. *Test*: the commit hook
  and the recorded priority calls write to one ordered log. `quick_check` runs only on the
  facade's thread at open, which never enters background mode (asserted in the same test by
  construction of the indexer thread). *Measured*: no "database is locked" warning in the log
  during root changes in step 8.
- AC-4 (R1, R2) Given the real platform backend, When the walk profile is applied and background
  mode is entered and left, Then on Windows: `GetThreadPriority` reports
  `THREAD_PRIORITY_LOWEST`, `GetThreadInformation(ThreadMemoryPriority)` reports
  `MEMORY_PRIORITY_LOW`, entering twice fails with `ERROR_THREAD_MODE_ALREADY_BACKGROUND` and
  leaving twice fails with `ERROR_THREAD_MODE_NOT_BACKGROUND` (so the mode really toggled); on
  macOS: the thread's disk I/O policy is `IOPOL_THROTTLE` inside background mode and the default
  outside. *Test*: platform tests, the Windows one on the `windows-latest` runner.
- AC-5 (R4, R6) Given the walk profile and a fake load source, When 3 of the last 5 samples show a
  queue at or above `busy_queue`, Then the next listing waits (the open transaction is committed
  first) and listing resumes once the window is calm; with the probe unavailable it never waits.
  *Test*: back-off policy on a fake clock; scan loop commits before a wait. *Measured*: step 8,
  diag `backoff_s` against `perf.csv` queue lengths.
- AC-6 (R4) Given a disk that is busy in every sample, When the walk runs for 60 s of fake time,
  Then it works at least `min_work` after every `max_backoff` pause, at least 20 % of the time.
  *Test*: fake clock. *Measured*: step 8, diag `dirs_per_min` never 0 for a whole minute.
- AC-7 (R5) Given a system up for 2 minutes when VFS starts, When the start-up catch-up is due,
  Then it starts once uptime reaches `boot_grace`; a root added or a manual re-walk in the
  meantime starts at once. *Test*: fake uptime. *Measured*: diag `catchup_deferred_s` in the
  `start` line after a reboot (step 8, second start).
- AC-8 (R7) Given a pass over a temporary tree, When it ends, Then the diagnostic sink received a
  pass record with every field of R7 and a watcher overflow produces an overflow record. *Test*:
  capturing sink. *Measured*: the log brought back after weeks (step 11).
- AC-9 (R8) Given files and folders with distinctive names, When a pass with errors runs, Then no
  diagnostic record contains any of those names, and the app logger writes an info record from
  the diagnostic target but not an info record from another target. *Test*: capturing sink;
  desktop logger test reading the log file.
- AC-10 (R9) Given an index folder, When the app opens it, Then on Windows the folder carries
  `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED`, and a folder with a `.SynologyWorkingDirectory` ancestor
  is reported as inside a sync root. *Test*: Windows CI attribute check; ancestor detection on
  both systems. *Measured*: step 8, `attrib` on the index folder shows `I`; step 7 answers
  whether the warning should ever fire.
- AC-11 (R10) Given a fresh index on Windows, When the writer adds rows, Then the file size is a
  multiple of `WRITER_CHUNK_BYTES`; and the AC-9 benchmark on a 2-million-file index passes with
  the chosen caches. *Test*: Windows CI; benchmark before and after in the PR body. *Measured*:
  step 9 working set.
- AC-12 (R11) Given the main window, When it is minimised, Then the shell asks for hide +
  suspend once, and When it is restored, Then it asks for visible once; on other systems it asks
  for nothing. *Test*: the event-to-action mapping. The WebView2 calls themselves only
  type-check here and never ran on real hardware. *Measured*: step 9 before and after.

## Non-goals

- Reading the USN journal at start (R5 of the study, issue 32): separate later work, after N1/N2.
- Watcher overflow handling and filters (R6), skipping unchanged leaf directories (R9),
  `FileIdExtdDirectoryInfo` listing (R10), cloud placeholders (R11).
- Pausing on suspend and re-checking volumes on resume (study R3 item 4): listing errors already
  retry; worth it only if the log shows errors after resume.
- A disk-load probe on macOS: macOS is a development system; there the disk is never busy.
- A "Copiar diagnóstico" button: the log path is documented in the measurement plan instead,
  while the UI is being redesigned.
- Showing the sync-root warning in the status bar: the UI is being rewritten in parallel; the
  warning goes to the log, and the status-bar text is a follow-up for the new UI.
- Any schema change, FTS `optimize`/`merge` policy, or page size change (index-footprint work).
- EcoQoS and process-wide background mode (not recommended by the study).

## Pre-mortem

- Slow PC, multi-TB HDD: this is the point of the change. One sequential stream per disk is kept
  (single indexer thread); priority is normal for walks with lowest CPU; resumability is
  unchanged (VFS-1 transactions).
- Paths over 260 characters, case sensitivity: n/a, no new path handling; the sync-root check
  walks ancestors with `Path::ancestors`.
- NFC/NFD: n/a, no names are compared or stored.
- Antivirus: the back-off sees Defender's I/O in the disk queue like any other process.
- Killed mid-write: commits keep their VFS-1 batches; a pause commits first, so a kill during a
  pause loses nothing.
- Million-file query: unaffected; reader caches only change if the benchmark allows it.
- Permission-denied folders, links, placeholders: unchanged listing code.
- Opening or writing user files: none. The only new write is the attribute on VFS's own data
  folder.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Spec id | VFS-5: VFS-3 and VFS-4 were claimed by sibling sessions (robustness, index footprint) | Claude | 2026-11-20 |
| 2 | How to guarantee "no lock in background mode" | Background mode wraps only the directory-listing call; in incremental mode each directory is listed outside any transaction and committed alone. A DB write in background mode is then impossible by construction, and the test still checks it | Claude | 2026-11-20 |
| 3 | Busy signal | Average disk queue length of the roots' physical disk via PDH with English counter names (localised Spanish names otherwise). Not `% Idle Time`, which the walk itself drives to zero | Claude | 2026-11-20 |
| 4 | Back-off in incremental mode | None: background mode already yields to every other I/O | Claude | 2026-11-20 |
| 5 | Checkpoint timing | A WAL hook on the indexer connection runs the same passive checkpoint at the same page count as `wal_autocheckpoint`, timed for the log | Claude | 2026-11-20 |
| 6 | Reader page caches | Kept at 64 MiB. Seven interleaved runs on a 2-million-file index: 32 MiB cut peak RSS from ~120 to ~81 MB, but p95 moved −8 % to +45 % per class while two identical builds differed by up to 2× (the Mac was shared, load 20–30). The cost is not resolvable there; decide with N12 or on a quiet machine. Numbers in the PR body | Claude | 2026-11-20 |

## Open questions

None blocking. Every default above is replaced by the measurement named next to it.

Known risk: uptime comes from `GetTickCount64`. With Fast Startup (on by default in Windows
10), a normal shut down and power on resumes the kernel session, so uptime does not reset and
the boot grace may rarely apply after such a start. Measurement-plan step 8.8 checks both a
reboot and a normal shut down; if the second shows no deferral, the clock should become the
logon session's start.
