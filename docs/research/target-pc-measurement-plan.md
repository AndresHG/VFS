# Target-PC measurement plan

One field test on a reference machine (Windows 10/11, one 8 TB spinning disk, Synology Drive client
always syncing) should produce every number the Windows design currently guesses. This is the
script for that field test. The background study is [`windows-io.md`](windows-io.md) and the ranked
changes are in [`windows-recommendations.md`](windows-recommendations.md); each recommendation
names the step below that confirms or refutes it.

Everything here runs as the PC's normal **standard user**. Steps marked *(admin, optional)*
need an administrator account only to obtain a reference value. The product must not depend
on them, and they are skipped if nobody has the password.

**Status of the tool.** [`tools/vfs-probe.ps1`](tools/vfs-probe.ps1) was rehearsed with
`-Step all` on a Windows Server 2025 GitHub runner, as a standard user created for the purpose
([`windows-e2e-results.md`](windows-e2e-results.md), VFS-8). Every step runs there. Two things
behave differently for a standard user: step 1 cannot list services (`sync_services` holds the
error), and step 6 may see no disk performance counters (see step 6). Every step catches its own
errors and the next one still runs, so one broken step does not waste the field test.

## What to take

- A USB stick with `vfs-probe.ps1`, the VFS installer (`.exe`, NSIS, per-user) and this page
  printed or on a phone.
- About 3 hours. The cold walk (step 3) needs a freshly rebooted PC. The sampling steps (6, 9)
  need the PC in its normal state: Firefox open and Synology syncing.
- 20 GB free on the data disk for the seek test (step 5). The script skips it otherwise.

## Numbers this field test must produce

| # | Unknown today | Source (step) | Decides |
|---|---|---|---|
| N1 | Can a standard user query and read the USN journal of the data volume? | `usn.json` (2) | Whether R5 (USN catch-up) ships or is dropped; issue 32 |
| N2 | Journal size, and how many hours of Synology churn it holds | `usn.json`, `usn-query-1/2.json` (2, 10) | Whether a catch-up after a weekend away is possible |
| N3 | Directories and files under the real roots; paths ≥ 260 characters; placeholders | `walk.json` (3) | Index size and walk time; issue 21 |
| N4 | Cold directory listing rate (dirs/s) at background priority, with Synology active | `walk.json` (3) | Real full re-walk time; issue 33 |
| N5 | Warm listing rate: basic vs large fetch vs `FileIdBothDirectoryInfo` | `walk.json` (3) | R10 (listing call) |
| N6 | Does a directory's mtime change on child create/rename/delete, and not on edits or grandchildren? How fast? | `semantics.json` (4) | R9 (skip unchanged leaf directories); issue 34 |
| N7 | How stale is a listed file size while a writer holds the file open? | `semantics.json` (4) | Whether the watcher must re-list after close |
| N8 | Cost of `FlushFileBuffers` on the index disk | `flush.json` (5) | `synchronous` and checkpoint cadence (R7) |
| N9 | Random 4 KiB read latency, normal vs very-low I/O priority | `seek.json` (5) | Cold-search estimate; issue 18 |
| N10 | Disk idle %, queue length, latency and top I/O processes during normal use | `perf.csv` (6) | Back-off thresholds (R3); issue 20 |
| N11 | Watcher events per minute and buffer overflows: VFS filter vs names-only filter | `watch.json` (6) | R6 (watcher overflows and filter) |
| N12 | RAM: total, free with Firefox open, VFS + WebView2 working set open and minimised | `info.json` (1), step 9 | R7, R8 (page caches, WebView2); issue 19 |
| N13 | First full index: duration, index size, disk use while it runs | step 8 | Issues 17, 33 |
| N14 | Which Synology product and which folders it syncs or backs up; is `%LOCALAPPDATA%` included? | step 7 (by hand) | R2 (index location); issue 36 |
| N15 | Defender state, other antivirus, WebView2 runtime version, long-path setting | `info.json` (1) | R8 (memory API needs runtime 114+); issue 21 |
| N16 | Does VFS's own walk behave as VFS-5 assumes: walk rate, time backed off, commit and checkpoint cost, catch-up delay after boot? | `vfs.log` (8, 9, and weeks later) | VFS-5 thresholds (`specs/vfs-5-windows-tuning/spec.md`, Configuration) |

## Step by step

Open **PowerShell** normally (Start → type `powershell` → Enter, *not* "Run as
administrator"). Copy the script to the desktop. In the commands below, `D:\` stands for the
data disk or the top folder that will become a VFS root. Use the real one.

### 0. Before rebooting (5 min)

1. Note the edition and build: `winver`.
2. Ask who has administrator rights. If an admin password is available, note it for the
   optional steps only.
3. Open Synology Drive Client and **photograph** every sync and backup task: local folder, NAS
   folder, direction, on-demand on/off, and any filters. This is N14 and cannot be scripted:
   the client's configuration is not documented.
4. Reboot the PC. After logging in, wait until the desktop is usable and do not open Firefox
   or Explorer windows on the data disk: step 3 needs a cold file cache. Synology and Defender
   starting up is expected; that is the real condition.

### 1. System inventory (2 min) → N12, N15

```powershell
cd $HOME\Desktop
powershell -NoProfile -ExecutionPolicy Bypass -File .\vfs-probe.ps1 -DataRoot D:\ -Step info
```

`-ExecutionPolicy Bypass` applies to this one process only and needs no admin. The script
prints its output folder. Every run creates its own `%LOCALAPPDATA%\VFS-probe\<timestamp>`
folder; step 11 collects them all.

### 2. USN journal without admin (1–5 min) → N1, N2

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\vfs-probe.ps1 -DataRoot D:\ -Step usn
```

How to read `usn.json`:

- `volume_open_read_error = 5` is expected: a standard user cannot open `\\.\D:` for reading.
- `volume_traverse_query` and `guid_dir_query` try the two other handles that published
  non-admin users of the journal open (AssetCooker: `\\.\D:` with `FILE_TRAVERSE`; BuildXL:
  `\\?\Volume{…}\` as a directory). Note which ones succeed.
- `root_query` holds the result of `FSCTL_QUERY_USN_JOURNAL` on a handle to `D:\`. If it holds
  `journal_id`, `first_usn` and `maximum_size`, the journal exists and is visible. If `error`
  is `1179` (`ERROR_JOURNAL_NOT_ACTIVE`), the volume has no journal and nothing can be done
  without admin.
- `unprivileged_read_oldest.records > 0` means `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` works for
  this user. **This is the verdict for R5.** `records_with_name` is expected to be 0: a
  published test on Windows 11 builds found the unprivileged read returns no names on NTFS.
  R5 is designed not to need them. `first_record_utc` is the age of the oldest change the
  journal still holds: the window within which a catch-up can work.
- `parent_resolve_ok / parent_resolve_tried` shows whether `OpenFileById` turns a parent
  reference into a path without admin. `parent_resolve_ms_avg` is its cost.
- `unprivileged_read_bulk.records / seconds` is the read speed of a catch-up.
- `privileged_read_on_dir` and `enum_usn_data_on_dir` are expected to fail with `5`. They
  confirm which calls need admin.

*(admin, optional)*: in an elevated window, `fsutil usn queryjournal D:` gives the same
journal figures. Compare them with `root_query`.

### 3. Cold directory walk at background priority (≤ 30 min, unattended) → N3, N4, N5

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\vfs-probe.ps1 -DataRoot D:\ -Step walk -WalkMinutes 30
```

The first pass lists directories with `FindFirstFileExW` + `FindExInfoBasic` (the call
Rust's `std::fs::read_dir` makes). Every 2,000 directories it switches the thread between
background mode (which VFS-5 keeps only for small incremental listings) and normal priority, so one cold walk measures
both under the same disk conditions. That is the test of the starvation risk in
`windows-io.md` §2.2. It stops after 30 min or 400,000 directories. The three warm passes re-list the same
directories to compare the three listing calls (N5). Cold-versus-cold comparisons are not
possible without admin rights, because the standby cache cannot be flushed.

Key fields: `cold_basic_background.background_dirs_per_sec` against `normal_dirs_per_sec`
(VFS-5 already walks at normal priority: if background is several times slower while Synology
is active, that choice stands; if it is about as fast, background mode could return for walks),
`dirs_per_sec`, `ms_per_dir_p95`, `finished`, `files`,
`paths_over_259`, `placeholders`, `reparse_dirs_skipped`. If `finished` is false, extrapolate
the full walk as total dirs ÷ `dirs_per_sec`. The total comes from step 8.

### 4. Directory-timestamp semantics (1 min) → N6, N7

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\vfs-probe.ps1 -DataRoot D:\ -Step semantics
```

It works only inside a new folder `D:\vfs-probe-<timestamp>` that it creates and deletes
afterwards. It refuses to run if that folder already exists, and deletes nothing else. Expected (see
`windows-io.md` §3.3): the directory mtime changes for create, rename and delete, and does not
change for an append to a file or for a grandchild. Any other result changes the design of R9.
Each line gives the folder's mtime twice: as its parent's listing shows it (what a re-walk
sees) and as the folder itself reports it. On the runner the listed value lagged for a delete:
unchanged at once, changed after 3 s.
The two "listed size" lines show N7.

### 5. Flush cost and seek latency (5 min) → N8, N9

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\vfs-probe.ps1 -DataRoot D:\ -Step flush
powershell -NoProfile -ExecutionPolicy Bypass -File .\vfs-probe.ps1 -DataRoot D:\ -Step seek
```

`seek` writes a 2 GiB scratch file on the data disk (about 20–40 s on a healthy HDD, *estimate*),
reads 400 random 4 KiB blocks unbuffered at normal and then at very-low I/O priority, and
deletes the file. If both priorities give the same latency, the disk was not contended at the
time. That is still useful as the floor for cold search.

### 6. Normal-use sampling: disk, memory, watcher (15–30 min) → N10, N11

Now open Firefox with its usual tabs and let the owner use the PC normally, or at least let
Synology run.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\vfs-probe.ps1 -DataRoot D:\ -Step watch -Minutes 30
```

Every 5 s it appends per-disk `% Idle Time`, queue length, read/write latency, available MB,
pages/s and the three processes with the most I/O to `perf.csv`. Two `ReadDirectoryChanges`
watchers run on `D:\` meanwhile: one with VFS's current filter (names + size + last-write +
creation), one with names only. `watch.json` reports the events and overflows of each. The
counters are read through WMI classes (`Win32_PerfFormattedData_*`, plus `Win32_PerfRawData_*`
for read and write latency) because PDH counter paths are localised on a Spanish Windows
(`\Disco físico(...)`). VFS itself reads the same counter through PDH by its English name.
The per-process I/O query is itself not free on a slow PC: expect the probe to add some CPU
load, and do not read small differences into it.

On the runner rehearsal (VFS-8) a standard user saw no disk counter instances: the log
says `perf: no PhysicalDisk counter instances visible to this user` and `perf.csv` keeps only
its header. If that happens on the reference machine, run this step again from an administrator window
*(admin, optional)*; the watcher counts in `watch.json` do not depend on it.

What VFS-5 assumes from this step (its back-off: study R3, spec R4): the indexer pauses when the **average
queue length** of the physical disk holding the roots is **2 or more in 3 of 5 one-second
samples**, pauses at most 8 s and then works at least 2 s. From `perf.csv` (column
`AvgDiskQueueLength`), write down for the data disk: the share of samples with queue ≥ 2,
the longest run of such samples, and `% Idle Time` at the same moments. If queue ≥ 2 is the
normal state (more than half the samples), the threshold is too low and VFS would crawl at its
20 % floor all day; if it never happens while Firefox feels slow, it is too high.

### 7. Synology configuration by hand (10 min) → N14

From the photographs in step 0 and the client UI, answer:

1. Product: Synology Drive Client (sync), Drive Client backup task, or Active Backup for
   Business agent? Which version?
2. Is any sync or backup task rooted at `C:\Users\<user>` or `%LOCALAPPDATA%`? If so,
   `%LOCALAPPDATA%\app.vfs.desktop\` (where the index lives) would be uploaded.
3. Is On-demand sync on for any task on the data disk? The `placeholders` count in step 3 is
   the cross-check.
4. Does the client show "Scanning" or "Indexing" after a reboot, and for how long? That tells
   whether it re-walks the same disk at every start, competing with VFS's catch-up.

### 8. Install VFS and measure the first index (unattended, hours) → N13

1. Run the installer (per-user, no admin). Add the real roots.
2. Leave the probe sampling disk and memory meanwhile, in a second PowerShell window:
   `... -Step perf -Minutes 240`.
3. Note the start time. VFS's status bar shows progress. Record when the first scan finishes,
   or how far it got when the field test ends: the scan resumes at the next start.
4. Index size: `Get-ChildItem "$env:LOCALAPPDATA\app.vfs.desktop" | Select Name, Length`
   (the index is in Local since VFS-1; never in `%APPDATA%`).
5. Index folder attributes (VFS-5 R9): `attrib "$env:LOCALAPPDATA\app.vfs.desktop"` must
   show `I` (not content-indexed). The log's `start` line says `data_dir_in_sync_root=false`;
   if it says `true`, step 7 missed a sync task covering the profile.
6. Priority of the walk (VFS-5 R1): open Resource Monitor (`resmon`, no admin) → Disk tab →
   Disk Activity, and look at the **I/O Priority** column for `vfs.exe` while the first index
   runs. Expected: *Normal*. After the index finishes, rename a file inside a root: the short
   re-listing that follows should show *Very Low* (background mode). *(admin, optional)*:
   Process Monitor's *Priority* column shows the same per request.
7. Read the diagnostic log, `$env:LOCALAPPDATA\app.vfs.desktop\logs\vfs.log`. Every minute of
   indexing writes a `pass` line: `mode=walk`, `dirs_per_min`, `files_per_min`, `backoff_s`
   (seconds paused for a busy disk), `commit_ms_max`, `checkpoint_ms_max`, `errors`. Note
   for the first hour: typical `dirs_per_min`, total `backoff_s` against 3,600 s, and the
   largest `commit_ms_max` and `checkpoint_ms_max` (a checkpoint over a few seconds on this
   disk means the WAL size should be revisited). `dirs_per_min` should never be 0 for a whole
   minute (the 20 % floor). Compare the minutes with high `backoff_s` with the queue lengths
   in the `perf` window: they should coincide.
8. Boot grace (VFS-5 R5): after the first index has finished, reboot, log in and open VFS at
   once. Its `start` line shows `uptime_s` and `catchup_deferred_s`; the catch-up `pass` lines
   must begin about 10 minutes after boot, not at once. Note whether Synology still showed
   "Scanning" at that moment (step 7.4): if it did, the grace is too short. Then do it again
   with a normal **Apagar** and power on, the way the owner starts the PC. With Fast Startup
   (default on Windows 10) `uptime_s` may not reset; if `catchup_deferred_s` is 0 after that
   start, the boot grace never applies in daily use (spec VFS-5, Open questions).

### 9. VFS memory, open and minimised (10 min) → N12

With VFS open and idle after a search:

```powershell
Get-Process vfs*, msedgewebview2 | Select ProcessName, Id, @{n='WS_MB';e={[math]::Round($_.WorkingSet64/1MB)}}, @{n='Private_MB';e={[math]::Round($_.PrivateMemorySize64/1MB)}} | Format-Table
```

Run it three times: open and idle; 5 min after minimising; 5 min after restoring and making a
search. `msedgewebview2` processes belonging to other apps (Teams, Widgets) appear too.
Process Explorer or Task Manager's "Command line" column tells them apart: VFS's processes
carry `app.vfs.desktop` in their `--user-data-dir`.

Since VFS-5 a minimised VFS hides its WebView2 and asks it to suspend (study R8, spec R11):
the `msedgewebview2` working sets should drop clearly in the second reading and recover in
the third. If they do not drop, the suspend call is not working on this runtime: note the
WebView2 version from step 1. The log's `memory` lines (every 10 min: `working_set_mb`,
`peak_mb`, `cpu_s` and the other VFS-41 fields) give the same figure for `vfs.exe` over weeks.

### 10. Journal growth rate (end of the field test, 1 min) → N2

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\vfs-probe.ps1 -DataRoot D:\ -Step usn-query
```

It only queries the journal (no reads). Compare `next_usn` and `taken_utc` in its
`usn-query-2.json` with step 2's `usn-query-1.json` (in step 2's folder). (next_usn₂ − next_usn₁)
÷ hours = bytes of journal per hour. `maximum_size` ÷ that rate = hours of history the
journal holds under real churn. That is N2: if it is below the longest time the app stays
closed (a weekend, a holiday), USN catch-up alone is not enough and the re-walk remains the
fallback.

### 11. Bring back

Zip `%LOCALAPPDATA%\VFS-probe`, `%LOCALAPPDATA%\app.vfs.desktop\logs` and the Synology
photographs. The log's info lines (`start`, `pass`, `pass_end`, `memory`, `watch_overflow`)
hold only counts, durations, sizes and root numbers; its **warning** lines can name a folder
that could not be listed, so read them before sharing with anyone else. Weeks later, bring
the log folder back again (it keeps about 22 MB), or use "Exportar diagnóstico" in the app
(`docs/diagnostic-report.md`). The probe output holds no file
contents, only counts, timings, five sample names from the journal (`sample_names` in
`usn.json`), and process names. Check `usn.json` before sharing it with anyone else.

## What stays unknowable even after the field test

- Behaviour after weeks of use: index fragmentation on the HDD, WAL growth patterns, how often
  overflow re-walks happen. VFS's diagnostic log (VFS-5, R4) records the WAL size, overflows
  and walk rates; fragmentation itself stays unmeasured (`defrag /A` needs admin).
- Cold-cache comparisons between enumeration calls (needs admin to flush the standby list).
- Defender's per-file cost on the index: Defender's internal decisions are not observable
  without admin (Performance Analyzer for Defender, `New-MpPerformanceRecording`, needs
  elevation).
