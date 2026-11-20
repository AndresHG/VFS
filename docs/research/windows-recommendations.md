# Windows recommendations for VFS, ranked

Ranked engineering changes that follow from [`windows-io.md`](windows-io.md) (sections cited
as §n). Each entry gives the change, the expected gain, the cost, the risk, how to measure it on
the real PC (step and number in
[`target-pc-measurement-plan.md`](target-pc-measurement-plan.md)), and whether it needs admin.
**None of them needs administrator rights.**

Labels as in the study: **[doc]** primary source, **[3p]** third-party measurement,
**[est]** our estimate, **[code]** today's code (`origin/feat/vfs-1-freshness` @ `494603a`).
No gain below is measured; every gain figure is **[est]** unless marked otherwise.

Ranking: expected gain on the target PC divided by cost, with risk as a tie-breaker. R1–R5 are
the five to do first.

| # | Change | Gain | Cost | Risk | Measure (plan) | Admin | Issue |
|---|---|---|---|---|---|---|---|
| R1 | Indexer priority policy: normal I/O for walks, very low only for small incremental work; never hold DB locks at very low | First index and re-walks in hours instead of possibly days | S | Medium: more contention, needs R4 | N4 (step 3) | No | #45 |
| R2 | Index in `%LOCALAPPDATA%`, folder not content-indexed, warn if inside a sync root | No GB-sized uploads to the NAS; no roaming; no Windows Search churn | S | Low | N14 (step 7) | No | #36 |
| R3 | Busy-disk and boot-time back-off | Foreground (Firefox, Explorer) stays responsive while walking | M | Medium: wrong thresholds stall progress | N10 (step 6) | No | #20 |
| R4 | Local diagnostic log (walk rate, overflows, checkpoints, flushes) | Turns weeks of real use into numbers | S | Low | — | No | #46 |
| R5 | USN catch-up at start, per directory, with re-walk fallback | Start-up catch-up in seconds or minutes instead of hours, when the journal covers the downtime | L | Medium-high: undocumented call, journal wrap | N1, N2 (steps 2, 10) | No (to confirm) | #32 |
| R6 | Watcher: overflow answered without a full re-walk; narrower filter during walks; consider `ReadDirectoryChangesExW` | Fewer multi-hour re-walks under Synology churn | M | Medium | N11 (step 6) | No | #47 |
| R7 | SQLite for this PC: smaller page caches, chunked file growth | 60–100 MB less RAM; less index fragmentation | S | Low | N8, N12 (steps 5, 9) | No | #19 |
| R8 | WebView2: `IsVisible(false)` + `TrySuspend` on minimise | Tens of MB, possibly more, while minimised | S–M | Low | N12 (step 9) | No | #48 |
| R9 | Skip listing **leaf** directories whose mtime is unchanged (fallback when USN is unavailable) | Fewer listings per re-walk, but less than issue 34 assumed | M | Medium: stale sizes and dates | N6 (step 4) | No | #34 |
| R10 | Listing via `FileIdExtdDirectoryInfo` (file id, reparse tag, attributes) | Enabler for R5; fewer system calls | M | Low | N5 (step 3) | No | in #32 |
| R11 | Record cloud-placeholder attributes; never open placeholders (also for phase 3) | No surprise downloads; "solo en la nube" label | S | Low | N3 (step 3) | No | #49, #23 |

Issues 45–49 were opened with this study; the others received a comment with its findings.

---

## R1. Indexer priority policy

**Change.**
1. Run the **first index and full re-walks** at **normal I/O priority**, with CPU priority
   `THREAD_PRIORITY_LOWEST` and memory priority LOW (`SetThreadInformation(ThreadMemoryPriority)`).
   That means leaving thread background mode for those passes. Back off when the disk is busy
   (R3) instead of relying on the very-low queue.
2. Keep **background mode** (very-low I/O) for small incremental work: watcher-marked
   directories and retries.
3. Never hold the SQLite write lock or run a checkpoint at very-low I/O. Leave background mode
   (`THREAD_MODE_BACKGROUND_END`) around commit, checkpoint and `quick_check`, or run them on a
   normal-priority path.
4. Add `THREAD_PRIORITY_LOWEST` whenever background mode is on, because on Windows 8+ background
   mode does not lower CPU priority **[3p]** (Chromium tests, §1.1).

**Why.** Very-low I/O is issued only after 50 ms of disk silence, otherwise trickled at about
one request per 0.5–1 s **[doc]** (Windows Internals 6th ed., §2.2). With Synology and Defender
keeping the disk busy, a cold walk of 400 k directories could take **4.5–9 days** at that rate
instead of 6–11 hours at normal priority **[est]** (§2.2 table). The kernel boosts priority only
for file-system-driver locks, not for SQLite's locks **[doc]**, so a background-mode commit can
block a root change for up to the 30 s busy timeout **[est]** (§1.2).

**Code today.** `platform::enter_background_priority()` puts the worker thread in background
mode once, for its whole life (`indexer/mod.rs:732`). The same thread walks, commits and
checkpoints. **[code]**

**Cost.** S: `platform.rs` gains "enter/leave background" and memory-priority calls; the
indexer chooses the mode per pass and wraps commits. Windows CI covers compile and the call
results; behaviour only shows on the real PC.

**Risk.** A normal-priority walk competes harder with Synology and Firefox. That is why R3
belongs with it. If the probe shows background mode is *not* much slower under real load,
keep it and drop this change.

**Measure.** Probe step 3 alternates background and normal priority every 2,000 directories of
one cold walk: compare `background_dirs_per_sec` with `normal_dirs_per_sec` (N4). Process
Monitor's *Priority* column on the VFS process confirms which priority the listing I/O carries.

---

## R2. Index location

**Change.** Open the index under `app_local_data_dir()` (`%LOCALAPPDATA%\app.vfs.desktop\`)
instead of `app_data_dir()` (`%APPDATA%`, Roaming). Set `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED` on
that folder. If an index exists at the old place, move it with its `-wal`/`-shm` files while
closed. At start, warn in the status bar if the folder lies under a Synology sync root.
Synology's configuration format is undocumented, so the practical check is the presence of
`.SynologyWorkingDirectory` in an ancestor folder. Whether that marker exists in every client
version is unverified **[est]**.

**Why.** Roaming is for small settings that follow the user. Large machine-specific data belongs
in Local **[doc]** (§6.6). A sync or backup task that includes `AppData` would upload a 1–2 GB
file that changes every few seconds, and could separate the database from its `-wal`, which
corrupts it **[doc]** (§6.5). Windows Search in Enhanced mode would otherwise re-index the WAL
churn **[est]**.

**Code today.** `apps/desktop/src-tauri/src/lib.rs:101` uses `app_data_dir()`, which is Roaming
on Windows **[code]**. The WebView2 profile is already in Local.

**Cost.** S. No migration is needed on the target PC, which has never had VFS installed. Dev
machines need the move.

**Risk.** Low.

**Measure.** Plan step 7 (N14): whether any Synology task covers `C:\Users\<user>` or `AppData`.

---

## R3. Back off when the disk is busy, and after boot

**Change.**
1. Open one PDH query for the physical disk that holds the roots, with
   `PdhAddEnglishCounterW` (counter names are localised on Spanish Windows **[doc]**):
   `% Idle Time` and `Current Disk Queue Length`. Sample every 1–2 s, keeping the query open.
2. Pause listing while the disk is busy: idle below ~20 % or queue above ~2 for, say, 3 of the
   last 5 samples. Resume when the disk calms down. Always allow a minimum duty cycle (for
   example 1 s of work per 10 s) so the walk can never stall forever. Thresholds are
   placeholders until N10 is known **[est]**.
3. Delay the start-up catch-up until the system has been up for ~10 minutes
   (`GetTickCount64`), when Synology and Defender finish their own boot work **[est]**.
4. Pause on suspend (`PowerRegisterSuspendResumeNotification`) and re-check volumes on resume.

**Why.** With R1 the walk no longer yields automatically through the very-low queue, so it must
yield deliberately. PDH, `GetSystemTimes` and the suspend notification all work for a standard
user in an interactive session **[doc]** (§2.4). Issue 20 already asks for this.

**Code today.** No busy, idle or suspend detection. Pausing is a user action only. **[code]**

**Cost.** M: a small `cfg(windows)` sampler, plus a pause hook in the scan loop next to the
existing `paused` flag.

**Risk.** Synology may keep the disk "busy" permanently. The duty-cycle floor and the probe
data guard against stalling.

**Measure.** Probe step 6 (`perf.csv`, N10): idle %, queue length and top processes during
normal use. They set the thresholds.

---

## R4. Local diagnostic log

**Change.** Write a small rotating text log next to the index: per pass, directories listed,
dirs/s, the time spent paused by R3, watcher overflows and their cause, checkpoint durations,
WAL size, and the process working set every 10 minutes. Add a "Copiar diagnóstico" button, or
document the path, so the owner can bring the log back.

**Why.** The probe gives one afternoon of numbers. How often overflows happen, how long
re-walks really take, and whether the index fragments only show over weeks, and nobody can
watch the PC (§ "What stays unknowable" in the plan). Without the log, every later tuning
decision is a guess again.

**Code today.** `log::` calls exist, but no file sink on Windows release builds **[code]**.
`vfs-cli bench` measures memory on the dev machine only.

**Cost.** S. **Risk.** Low (bounded size, no file names needed).

---

## R5. USN catch-up at start (per directory)

**Change.**
1. During listing, record each **directory's** 64-bit file reference number (FRN). With R10's
   `FileIdExtdDirectoryInfo` (or `FileIdBothDirectoryInfo`) this comes free; with `read_dir`
   it does not. Store it in `dirs`: one column, 400 k rows.
2. At clean shutdown and at each commit during idle, save `(volume GUID, UsnJournalID,
   NextUsn)` per volume.
3. At start, open the volume root as a directory (no admin), run `FSCTL_QUERY_USN_JOURNAL`,
   and if the journal id matches and the saved USN ≥ `FirstUsn`, read with
   `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` from the saved USN. For every record, look up its
   **parent FRN** among indexed directories and mark that directory dirty (shallow), exactly as
   a watcher hint does today. Unknown parents (new directories) resolve through the parent's
   parent once their parent is re-listed, so keep replaying until no unknown remains, or fall
   back.
4. On any failure (no journal, id changed, wrapped, error), fall back to today's full re-walk.
   The weekly periodic re-walk stays as a safety net.

**Why.** A standard user can read the journal on current Windows 11 builds; Microsoft's
BuildXL ships this path **[3p]** (§4.1). On NTFS the unprivileged records carry **no names**
**[3p]**, so a design keyed on names fails. Keyed on the parent directory, it needs no names
and reuses the existing dirty-directory machinery (§4.2). It turns a multi-hour start-up
re-walk into a replay of the downtime's changes, when the journal still covers them.

**Code today.** Re-walk at every start (`indexer/mod.rs`, owed re-walks). No file ids stored.
**[code]**

**Cost.** L: schema column and migration, Windows-only FSCTL code, replay logic, and tests that
can only run on Windows CI with a real NTFS volume. Issue 32 estimated 3–4 days.

**Risk.** The FSCTL is undocumented and Microsoft could change it. Its first Windows 10 build
and its behaviour on 22H2 are unverified **[3p]**. The journal may not exist on a data volume,
and a standard user cannot create it **[doc]**. The default size (~32 MB, **[3p]**) may hold
only hours of Synology churn. Every failure is detected (journal id, `FirstUsn`, error codes
**[doc]**) and falls back, so the risk is wasted effort, not a wrong index.

**Measure.** **Do not start before the visit.** Probe step 2 (N1: does the call work on her
build, which handles open) and steps 2 + 10 (N2: journal size and bytes per hour, so hours of
history). Build it only if N1 is yes and N2 covers at least a typical gap between uses (a
weekend) **[est]**.

---

## R6. Watcher: cheaper overflows, fewer events

**Change.**
1. On overflow (0 bytes or `ERROR_NOTIFY_ENUM_DIR`), replay the USN journal from the USN saved
   when the watch was armed (R5), instead of queueing a full re-walk. Without R5, mark the root
   for a re-walk as today.
2. While a walk of that root is running, the walk itself catches changes. Narrow the filter to
   `FILE_NAME | DIR_NAME`, which cuts the size and last-write events a syncing client generates
   **[est]**.
3. Consider `ReadDirectoryChangesExW` with extended information (Win10 1709+, NTFS only
   **[doc]**). Its events carry size, timestamps and file id, so a size or date change can
   update the row directly without re-listing the directory.

**Why.** Every overflow today becomes a full re-walk of the root after a one-hour cooldown,
hours of disk time on this PC (§5, §2.2). The 64 KB buffer is the network maximum, and a
larger local buffer only delays the overflow **[doc]**.

**Code today.** `watch.rs`: names + size + last-write + creation, 64 KB, overflow →
`Rescan(root)`. **[code]**

**Cost.** M. **Risk.** Dropping size and last-write events means sizes go stale until the next
listing. Only do it during walks, or if N11 shows overflows.

**Measure.** Probe step 6 (N11): events and overflows in 30 minutes for the current filter
against names-only, side by side.

---

## R7. SQLite settings for a small-memory HDD machine

**Change.**
1. Cut the search connection's page cache from 128 MiB to 32–64 MiB and the reader's from 64 to
   32 MiB, **after** N12 and issue 18's cold-search timing confirm it does not hurt searches.
   The OS file cache shrinks under memory pressure; a process heap is paged out to the same busy
   disk **[est]** (§6.3).
2. Set `SQLITE_FCNTL_CHUNK_SIZE` to 32 MiB on the writer, so the 1–2 GB index grows in large
   extents while Synology writes to the same disk **[doc]** (the docs say it "may reduce"
   fragmentation).
3. Keep WAL, `synchronous=NORMAL`, `mmap_size=0`, `temp_store=MEMORY`, `wal_autocheckpoint=4000`
   and `journal_size_limit=64 MiB`: they are right for this PC (§6).
4. FTS5: no `optimize` in the foreground; bounded `merge` during idle. This belongs to the
   footprint session's work; noted here for the HDD angle.

**Code today.** `db.rs`: 32 / 64 / 128 MiB caches; no chunk size. **[code]**

**Cost.** S. **Risk.** Low: a smaller cache may slow repeated searches; measure first.

**Measure.** N12 (step 9: working set open and idle) and N8 (step 5: flush cost, which confirms
checkpoints should stay infrequent).

---

## R8. WebView2 while minimised

**Change.** On minimise (Tauri window event), call `ICoreWebView2Controller::put_IsVisible(false)`
and then `ICoreWebView2_3::TrySuspend` via `Webview::with_webview`. On restore, set it visible:
it resumes on its own **[doc]** (§7). If `additionalBrowserArgs` is ever set, re-add Tauri's
defaults `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` **[doc]**. Do not
disable the GPU **[doc]**.

**Why.** Microsoft's own sample does exactly this on `SIZE_MINIMIZED`. TrySuspend lets the OS
reclaim renderer memory **[doc]**. On a PC that already swaps, a minimised VFS should cost close
to nothing. Destroying the window (issue 37) saves more, at the cost of a slower reopen.

**Code today.** Nothing reacts to minimise; no browser arguments. **[code]**

**Cost.** S–M (needs the `webview2-com` types through tauri's `with_webview`).
**Risk.** Low: TrySuspend fails harmlessly if the WebView is visible.

**Measure.** Plan step 9 (N12): WebView2 working set open, 5 min after minimising, and after
restoring. Run it once before and once after the change.

---

## R9. Skip listing unchanged leaf directories (fallback without USN)

**Change.** During a re-walk, when a directory's mtime (read from its parent's listing) equals
the stored one **and** the index says it has no subdirectories, skip listing it.

**Why it is narrower than issue 34 says.** A directory's mtime changes only for name changes
among its **direct** children, never for edits or grandchildren **[doc]** (§3.3). To decide
about a subdirectory, VFS needs that subdirectory's mtime, which lives in its parent's listing.
So a directory that *has* subdirectories must still be listed to reach them, and only
**leaf** directories can be skipped. In photo and document archives most directories are
probably leaves **[est]**, so the saving may still be large. The probe's walk counts dirs and
files; leaf counts come from the index after the first scan. The cost is that file sizes and
dates in skipped directories stay stale until a full listing.

**Code today.** `apply.rs` stores the directory mtime (`set_dir_mtime`) but never uses it to
skip. **[code]**

**Cost.** M. **Risk.** Medium: delete-in-directory updating the mtime is unverified on Win10/11
(probe step 4 checks it); any program can set timestamps explicitly.

**Measure.** N6 (step 4) confirms the semantics. After the first index, `SELECT` the share of
leaf directories to size the gain.

---

## R10. List with `FileIdExtdDirectoryInfo`

**Change.** On Windows, replace `std::fs::read_dir` in `walk.rs` with
`GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)` on a directory handle and a 64 KB
buffer. Each entry gives name, size, times, attributes, **reparse tag** and **file id** **[doc]**
(§3.1).

**Why.** It supplies the FRN R5 needs and the attributes R11 needs, and probably fewer system
calls than `FindFirstFileExW` without large fetch (Rust's std passes 0 flags **[code]**). Alone,
the gain is unknown and probably small: the same directory index is read either way **[est]**.

**Cost.** M (Windows-only parser, reparse tag replaces `is_symlink()`). **Risk.** Low.
Opening a directory handle is an open, but directories are not scanned for content **[est]**.

**Measure.** Probe step 3's warm passes (N5) compare basic, large fetch and by-handle listing.

---

## R11. Cloud placeholders

**Change.** Store `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN` and `OFFLINE` per
file. They are available from the listing today through `MetadataExt::file_attributes()`, or
via R10. Show "solo en la nube" in results. **Phase 3 content extraction (issue 23) must skip
these files:** reading one downloads it and shows the user a toast **[doc]** (§3.4).

**Code today.** VFS never reads data, so it never hydrates. It does not record the attributes.
**[code]**

**Cost.** S. **Risk.** Low.

**Measure.** Probe step 3 counts placeholders (N3). If it is 0 and Synology On-demand is off
(step 7), this stays low priority.

---

## Not recommended

- **Process-wide background mode** (`PROCESS_MODE_BACKGROUND_BEGIN`): undocumented 32 MiB
  working-set cap, ~250× slower memory scans **[3p]** (§1.2).
- **EcoQoS**: no measurable effect expected on a desktop i7 without E-cores, and Eco only exists
  on Windows 11 **[doc]** (§1.3). Harmless; not worth code.
- **`mmap_size` > 0**: crash on I/O error and no truncate on Windows **[doc]** (§6.4).
- **`synchronous=OFF`**: corruption on an OS crash would force a multi-hour rebuild **[doc]** (§6.2).
- **Asking for Defender exclusions or a Dev Drive**: both need admin **[doc]** (§6.5).
- **Larger SQLite pages for the HDD**: no source supports it, and in WAL mode it cannot be
  changed later **[doc]** (§6.3). Revisit only with a benchmark on the real disk.
- **`EmptyWorkingSet` on minimise**: it only moves pages to standby and causes re-faults
  **[doc]** (§1.3).

## What stays unknowable until someone measures on the real machine

1. Whether very-low I/O really starves the walk under Synology load: N4, decides R1.
2. Whether the unprivileged USN read works on her Windows build, and how many hours the
   journal holds under Synology churn: N1, N2, decide R5.
3. The real cold-walk rate and file count, hence the first-index time: N3, N4, N13.
4. Disk idle and queue figures in normal use, which set R3's thresholds: N10.
5. Watcher overflow frequency with the current filter: N11, decides R6.
6. The WebView2 + VFS working set, open and minimised: N12, decides R7, R8.
7. The Synology configuration, and whether it covers `AppData`: N14, R2.
8. Long-term behaviour (overflows per week, index fragmentation, WAL growth): only R4's log,
   over weeks.
