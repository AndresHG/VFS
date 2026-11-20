# Target-PC measurement plan

One visit to the target PC (Windows 10/11, one 8 TB spinning disk, Synology Drive client
always syncing) should produce every number the Windows design currently guesses. This is the
script for that visit. The background study is [`windows-io.md`](windows-io.md) and the ranked
changes are in [`windows-recommendations.md`](windows-recommendations.md); each recommendation
names the step below that confirms or refutes it.

Everything here runs as the PC's normal **standard user**. Steps marked *(admin, optional)*
need an administrator account only to obtain a reference value. The product must not depend
on them, and they are skipped if nobody has the password.

**Status of the tool.** [`tools/vfs-probe.ps1`](tools/vfs-probe.ps1) was written on macOS. Its
PowerShell parses with PowerShell 7.7, and its embedded C# compiles at C# language level 5
(the level of Windows PowerShell 5.1). It has **never been run on Windows**. Before the visit,
run it once on any Windows 10/11 machine (`-Step all -Minutes 1 -WalkMinutes 1`) and fix what
breaks. Every step catches its own errors and the next one still runs, so one broken step does
not waste the visit.

## What to take

- A USB stick with `vfs-probe.ps1`, the VFS installer (`.exe`, NSIS, per-user) and this page
  printed or on a phone.
- About 3 hours. The cold walk (step 3) needs a freshly rebooted PC. The sampling steps (6, 9)
  need the PC in its normal state: Firefox open and Synology syncing.
- 20 GB free on the data disk for the seek test (step 5). The script skips it otherwise.

## Numbers this visit must produce

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
| N14 | Which Synology product and which folders it syncs or backs up; is `%APPDATA%` included? | step 7 (by hand) | R2 (index location); issue 36 |
| N15 | Defender state, other antivirus, WebView2 runtime version, long-path setting | `info.json` (1) | R8 (memory API needs runtime 114+); issue 21 |

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
background mode (as the VFS indexer runs today) and normal priority, so one cold walk measures
both under the same disk conditions. That is the test of the starvation risk in
`windows-io.md` §2.2. It stops after 30 min or 400,000 directories. The three warm passes re-list the same
directories to compare the three listing calls (N5). Cold-versus-cold comparisons are not
possible without admin rights, because the standby cache cannot be flushed.

Key fields: `cold_basic_background.background_dirs_per_sec` against `normal_dirs_per_sec`
(if background is several times slower while Synology is active, R1 applies in full),
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
(`\Disco físico(...)`). The per-process I/O query is itself not free on a slow PC: expect the
probe to add some CPU load, and do not read small differences into it.

### 7. Synology configuration by hand (10 min) → N14

From the photographs in step 0 and the client UI, answer:

1. Product: Synology Drive Client (sync), Drive Client backup task, or Active Backup for
   Business agent? Which version?
2. Is any sync or backup task rooted at `C:\Users\<user>` or `%APPDATA%`? If so,
   `%APPDATA%\app.vfs.desktop\` (where the index lives today) would be uploaded.
3. Is On-demand sync on for any task on the data disk? The `placeholders` count in step 3 is
   the cross-check.
4. Does the client show "Scanning" or "Indexing" after a reboot, and for how long? That tells
   whether it re-walks the same disk at every start, competing with VFS's catch-up.

### 8. Install VFS and measure the first index (unattended, hours) → N13

1. Run the installer (per-user, no admin). Add the real roots.
2. Leave the probe sampling disk and memory meanwhile, in a second PowerShell window:
   `... -Step perf -Minutes 240`.
3. Note the start time. VFS's status bar shows progress. Record when the first scan finishes,
   or how far it got when the visit ends: the scan resumes at the next start.
4. Index size: `Get-ChildItem "$env:APPDATA\app.vfs.desktop" | Select Name, Length`. Use
   `$env:LOCALAPPDATA` instead if R2 has landed.

### 9. VFS memory, open and minimised (10 min) → N12

With VFS open and idle after a search:

```powershell
Get-Process vfs*, msedgewebview2 | Select ProcessName, Id, @{n='WS_MB';e={[math]::Round($_.WorkingSet64/1MB)}}, @{n='Private_MB';e={[math]::Round($_.PrivateMemorySize64/1MB)}} | Format-Table
```

Run it three times: open and idle; 5 min after minimising; 5 min after restoring and making a
search. `msedgewebview2` processes belonging to other apps (Teams, Widgets) appear too.
Process Explorer or Task Manager's "Command line" column tells them apart: VFS's processes
carry `app.vfs.desktop` in their `--user-data-dir`.

### 10. Journal growth rate (end of the visit, 1 min) → N2

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

Zip `%LOCALAPPDATA%\VFS-probe` and the Synology photographs. The probe output holds no file
contents, only counts, timings, five sample names from the journal (`sample_names` in
`usn.json`), and process names. Check `usn.json` before sharing it outside the family.

## What stays unknowable even after the visit

- Behaviour after weeks of use: index fragmentation on the HDD, WAL growth patterns, how often
  overflow re-walks happen. That needs telemetry from VFS itself (issue 19, R4 diagnostic log).
- Cold-cache comparisons between enumeration calls (needs admin to flush the standby list).
- Defender's per-file cost on the index: Defender's internal decisions are not observable
  without admin (Performance Analyzer for Defender, `New-MpPerformanceRecording`, needs
  elevation).
