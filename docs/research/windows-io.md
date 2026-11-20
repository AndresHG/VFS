# Windows underneath VFS: scheduling, disk I/O, NTFS, USN, SQLite, WebView2, Defender, Synology

What Windows does below VFS, as far as it matters on the real target: one Windows 10/11 PC,
an older Intel i7, short of memory, a single internal 8 TB spinning disk holding 4–5 TB of data
**and** the index, Firefox with many tabs, Defender on, and a Synology client permanently
syncing the same folders. VFS indexes about 5 M files in 400 k directories, without
administrator rights, only while its window is open.

The ranked changes that follow from this study are in
[`windows-recommendations.md`](windows-recommendations.md). The field-test script that measures
what is still guessed is [`target-pc-measurement-plan.md`](target-pc-measurement-plan.md).

**Evidence labels.** Every figure and claim carries one:
- **[doc]**: stated by the primary source cited (Microsoft Learn, Microsoft specs and blogs,
  sqlite.org, Windows Internals, vendor docs).
- **[3p]**: published by a third party: measured or observed by someone else (Chromium tests,
  Bruce Dawson, an open-source tool's test report).
- **[est]**: our estimate or inference, not measured by anyone.
- **[code]**: what the VFS code does today.

Nothing in this document was measured on Windows by us: all development has been on an Apple
Silicon Mac with an SSD.

**Code compared.** The brief names `origin/feat/vfs-1-release`, which does not exist on origin.
The newest code for the indexer, watcher and platform layer is `origin/feat/vfs-1-freshness`
at `494603a` (the integration branch plus the freshness fixes). All **[code]** references are
to that commit: `crates/vfs-core/src/{platform.rs, db.rs, indexer/}` and
`apps/desktop/src-tauri/`.

---

## 1. Scheduling, priority classes and background mode

### 1.1 What background mode changes

- `SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN)` and
  `SetPriorityClass(GetCurrentProcess(), PROCESS_MODE_BACKGROUND_BEGIN)` "lower the resource
  scheduling priorities" of the thread or process. Microsoft says a CPU priority class alone is
  not enough, because even an idle-priority process hurts responsiveness through disk and
  memory use. **[doc]** [SetThreadPriority][stp], [SetPriorityClass][spc]
- Background mode lowers CPU, memory (page) and I/O priority, and every I/O request is marked
  *very low* (idle). **[doc]** [I/O Prioritization in Windows Vista][iopri] (Microsoft white
  paper, linked from the `FILE_IO_PRIORITY_HINT_INFO` page)
- On Windows 8 and later, the thread form sets memory priority 1 (VERY_LOW) and very-low I/O,
  but `GetThreadPriority` stays NORMAL. **The CPU priority is not lowered.** Chromium treats
  this as a kernel bug and also calls `THREAD_PRIORITY_LOWEST`. **[3p]**
  [Chromium platform_thread_win_unittest.cc][chrtest], [platform_thread_win.cc][chrwin]
- "Background mode = CPU priority 4" was the value seen on Windows 7. No source confirms it for
  10/11. **[3p]** [Chromium win7 constant][chr78]
- Per thread: "each thread can enter background processing mode independently". The thread
  form works only on the calling thread. A background thread "may not be scheduled promptly,
  but it will never be starved". **[doc]** [SetPriorityClass][spc]
- Entering background mode is itself slow on a busy machine: over 2 s after a reboot in one
  report. **[doc]** [Raymond Chen][chen-bg]

**[code]** The indexer worker thread calls `THREAD_MODE_BACKGROUND_BEGIN` once at start
(`indexer/mod.rs:732` → `platform.rs:212`) and never leaves it. That same thread lists
directories **and** writes to SQLite: commits, checkpoints and the `quick_check` after an
unclean shutdown. The CPU priority is not lowered (see above). No process-wide mode is used,
which is right (§1.2).

### 1.2 Pitfalls

**Working-set cap (process form).** `PROCESS_MODE_BACKGROUND_BEGIN` caps the process working
set at about 32 MiB and keeps trimming on every fault. In Bruce Dawson's measurements, memory
scans ran ~250× slower, and the pages only move to the standby list, so nothing is saved.
Chrome removed the call; Mozilla stopped using it. **[3p]** [Dawson][dawson],
[Mozilla 1856287][moz]. No Microsoft Learn page documents this; the "Learn note" the brief
expected does not exist. Whether the *thread* form avoids the cap is unverified. Chromium
still ships the thread form, which is weak evidence that it does. **[3p]**

**Priority inversion.** Microsoft: a background-mode thread "should minimize sharing resources
such as critical sections, heaps, and handles" with other threads, "otherwise priority
inversions can occur". **[doc]** [SetPriorityClass][spc]. The kernel boosts I/O priority only
for threads holding `ERESOURCE` locks inside file-system drivers. **[doc]** [Windows Internals
6th ed., ch. 8][wi6]. So **user-mode locks get no boost [est]**. That covers the SQLite write
lock (`LockFileEx` byte ranges), its in-process mutexes and Rust `Mutex`.

Consequence for VFS **[est]**: while the indexer thread holds the SQLite write lock or runs a
checkpoint, its I/O waits in the very-low queue (§2.2). Anything else that needs the write
lock waits behind it, up to the 30 s `WRITER_BUSY_TIMEOUT` (`db.rs`): the facade's root
add/remove. In WAL mode, readers (searches) are not blocked by the writer, except by the
checkpoint itself competing for the same disk. A search on the UI path also shares the page
cache memory and the disk queue.

### 1.3 Priority classes, EcoQoS, memory priority

- Base priorities: IDLE class + NORMAL thread = 4; BELOW_NORMAL = 6; NORMAL = 8;
  `THREAD_PRIORITY_IDLE` = 1. Priority classes affect **CPU only**. **[doc]**
  [Scheduling priorities][sched]
- **EcoQoS** (`SetThreadInformation(ThreadPowerThrottling)` with `EXECUTION_SPEED`): "Eco"
  always picks the most efficient frequency and cores, but only on **Windows 11**. On Windows
  10 the throttling levels act only on battery. **[doc]** [Quality of service][qos]. At launch
  it was tuned for mobile CPUs, not desktops. **[doc]** [Introducing EcoQoS][ecoqos]. On an
  older desktop i7 without E-cores, expect little or no effect **[est]**. It is harmless to set.
- A minimised or fully hidden window gets Low QoS automatically. **[doc]** [qos]
- **Memory priority** (`SetThreadInformation(ThreadMemoryPriority)`, 1–5, default 5): pages a
  low-priority thread brings in go to low standby lists, which are reused first. Microsoft's own
  example is a file indexer lowering its memory priority. **[doc]**
  [MEMORY_PRIORITY_INFORMATION][mempri], [SetProcessInformation][spi]; standby-list priorities
  0–7 **[doc]** [Russinovich, Inside Vista kernel][russ1]. Thread background mode already sets
  memory priority 1 on Win8+ **[3p]**, so nothing extra is needed while the thread is in
  background mode.
- `EmptyWorkingSet` / `SetProcessWorkingSetSizeEx(-1,-1)` only move pages to standby. On a
  low-RAM machine that causes re-faults. Lowering memory priority is the better tool. **[doc]**
  [EmptyWorkingSet][ews], [Working set][ws]

---

## 2. The I/O stack on a single spinning disk

### 2.1 Priority hints

- `SetFileInformationByHandle(FileIoPriorityHintInfo)` sets a per-handle priority: VeryLow (0),
  Low (1) or Normal (2). Drivers treat it as a hint. **[doc]** [FILE_IO_PRIORITY_HINT_INFO][fiop]
- The I/O manager uses the handle's hint if set, otherwise the thread's. Requests issued from
  kernel mode are raised to Normal even if the handle hint is lower. **[doc]** [iopri], [wi6]
- With cached I/O, the cache manager can **lose** the priority. Cached reads keep it when the
  *thread* is marked low, but not when only the *handle* is. **[doc]** [iopri] So the thread
  form (what VFS uses) is the one that reaches cached and metadata paths.
- Apps cannot set anything above Normal. Defender scans, defrag, Superfetch and the search
  indexer use Very Low. **[doc]** [Raymond Chen][chen-prio], [wi6]

### 2.2 How very-low I/O is scheduled, and the starvation risk

- The idle strategy lives in the storage class driver (`classpnp.sys`), so it applies to every
  disk. An idle request is issued only after **50 ms with no normal I/O**. If normal I/O never
  stops, a timer still lets through **at least one idle I/O per half second** (Windows Internals
  6th ed., 2012), or one per second in the 2006–07 sources. **[doc]** [wi6], [iopri],
  [Russinovich 2007][russ2]
- In the book's experiment, a very-low-priority writer on a busy disk got about **2 KB/s**.
  **[doc]** [wi6]
- Very-low reads also disable cache-manager read-ahead. **[doc]** [russ2]
- Whether `NtQueryDirectoryFile` and the NTFS index and MFT reads it causes carry the thread's
  very-low priority is **not documented**. Since they are synchronous cache misses in the
  caller's context, they very likely do **[est]**. Process Monitor's *Priority* column shows it
  on the real PC.

**What this means on the target [est].** The Synology client and Defender keep the disk busy,
so it may rarely be quiet for 50 ms. Then a background-mode walk gets **1–2 disk reads per
second**. A cold walk of 400 k directories needs at least one random read per directory, and
more for large directories (index allocation blocks):

| Condition | Reads/s for the walk | 400 k dirs × ~2 reads |
|---|---|---|
| Quiet disk, normal priority (~12 ms random read) | ~80 | ~2.8 h |
| Busy disk, normal priority (shares with Synology) | ~20–40 | 6–11 h |
| Busy disk, very-low priority, trickle at 1–2 I/O/s | 1–2 | **4.5–9 days** |

All three rows are estimates. The first matches the architecture review's "~2 h quiet disk"
(issue 33). The third row is why background mode for the **first** index, and for any full
re-walk, may be a wrong default rather than a polite one. The probe's cold walk alternates the
two priorities every 2,000 directories to measure exactly this
([measurement plan](target-pc-measurement-plan.md), step 3).

### 2.3 Cache manager, buffered, unbuffered and mapped I/O

- File-system metadata "is always cached". `FILE_FLAG_NO_BUFFERING` does not apply to it or to
  mapped views. **[doc]** [File caching][fcache]. Directory listings therefore always go through
  the cache and fill the standby list. Thread memory priority 1 (§1.3) puts those pages on the
  lists reused first, so a walk evicts less of Firefox's cache **[est]**.
- `FILE_FLAG_SEQUENTIAL_SCAN` doubles prefetch and evicts pages behind the read position.
  `FILE_FLAG_RANDOM_ACCESS` disables read-ahead and keeps mapped views in the cache working set,
  which can take a lot of RAM on a small machine. **[doc]** [Chen on cache hints][chen-seq],
  [KB 2549369][kb2549369]
- The lazy writer runs once per second and writes one eighth of the dirty pages each time.
  **[doc]** [fcache]
- `FlushFileBuffers` writes every dirty page of the file and sends a cache-flush command to the
  drive. **[doc]** [FlushFileBuffers][ffb], [Chen on FLUSH_CACHE][chen-flush]. Its latency on a
  consumer HDD has no primary figure. Tens of milliseconds, more when the drive is busy, is our
  estimate **[est]**; the probe's `flush` step measures it.
- SysMain (Superfetch) prefetches with very-low I/O. Its current behaviour on HDD systems is
  undocumented. **[doc]** [russ1]

**[code]** VFS opens no user file. SQLite's Windows VFS uses ordinary buffered I/O. `mmap_size=0`
keeps the 2 GB index out of mapped views (`db.rs`), which is right on this PC (§6.4).

### 2.4 Detecting a busy disk, a busy CPU and an absent user (no admin)

| Mechanism | Standard user | Notes |
|---|---|---|
| PDH `\PhysicalDisk(n)\% Idle Time`, `Current Disk Queue Length`, `Avg. Disk sec/Read` | Yes, in an interactive session | **[doc]** [Limited user access][pdhuser]. % Idle Time is the share of time with zero outstanding requests; current queue length is instantaneous. **[doc]** [Disk performance internals][ntdbg]. Keep the query open: opening and closing it per sample disturbs the counters. Counter **names are localised**: on Spanish Windows use `PdhAddEnglishCounterW` **[doc]** [PdhAddEnglishCounter][pdheng] or the WMI classes `Win32_PerfFormattedData_PerfDisk_PhysicalDisk`. |
| `IOCTL_DISK_PERFORMANCE` on `\\.\PhysicalDriveN` with zero access | Unclear | The `CreateFileW` page contradicts itself on non-admin access. **[doc]** [ioctl][ioctldp]. Test it, or use PDH. |
| `GetSystemTimes` | Yes | System-wide CPU idle/kernel/user. **[doc]** [GetSystemTimes][gst] |
| `GetLastInputInfo` | Yes | Last input in the calling session; the tick count can wrap. **[doc]** [GetLastInputInfo][glii] |
| `PowerRegisterSuspendResumeNotification` | Yes | Suspend and resume callbacks, Win8+. **[doc]** [prsrn] |
| `GetProcessIoCounters` | Yes | All I/O of a process, not disk-only. **[doc]** [gpic] |
| Task Scheduler idle conditions | n/a | For reference: Windows considers the user absent after 4 min without input, and the machine idle when CPU and disk are below 80 % busy. **[doc]** [Task idle conditions][taskidle] |

**[code]** VFS has no busy, idle or suspend detection. Pausing exists only as a user action.

---

## 3. NTFS for a file-name indexer

### 3.1 Directory enumeration

- `FindFirstFileExW` with `FindExInfoBasic` "does not query the short file name".
  `FIND_FIRST_EX_LARGE_FETCH` "uses a larger buffer" (size undocumented). Both work since
  Windows 7. **[doc]** [FINDEX_INFO_LEVELS][findex], [FindFirstFileExW][ffex]
- `GetFileInformationByHandleEx` with `FileIdBothDirectoryInfo` (Vista+) or
  `FileIdExtdDirectoryInfo` (Win8+) fills a caller-sized buffer. Extd returns, per entry, four
  timestamps, size, allocation, attributes, **reparse tag** and a **128-bit file id**. **[doc]**
  [FILE_INFO_BY_HANDLE_CLASS][fibh], [FILE_ID_EXTD_DIR_INFO][fidextd]
- `NtQueryDirectoryFileEx` (1709+) adds `SL_RETURN_ON_DISK_ENTRIES_ONLY`. **[doc]** [ntqdfex]
- All of these end in the same directory-control request. The differences are buffer size,
  the short-name lookup and the fields returned. No benchmark exists **[doc]**; the probe
  compares them warm.
- Listing a directory reads that directory's index (names plus a duplicated copy of each
  child's metadata), **not** each child's MFT record. Per-file seeks happen only if files are
  opened. **[est]**, from the index structure in [ntfs-tn] and MS-FSA's duplicated-information
  algorithm [fsa-dup].

**[code]** `walk.rs` calls `std::fs::read_dir`. On Windows, Rust's standard library uses
`FindFirstFileExW(FindExInfoBasic)` **without** `LARGE_FETCH` and takes `metadata()` and
`file_type()` from `WIN32_FIND_DATAW` without opening the file
(`library/std/src/sys/fs/windows.rs`, `readdir` and `DirEntry::metadata`, rust-lang/rust
master of 2026-11-13). So VFS already lists without opening files. It gets no file id.

### 3.2 How fresh the listed metadata is

- The directory entry keeps a **copy** of each file's size and times. Since Vista the copy is
  refreshed when the **last handle closes**. A file another process is still writing shows an
  old size and time in the listing. **[doc]** [Raymond Chen, stale sizes][chen-size],
  [MS-FSA duplicated information][fsa-dup]
- NTFS defers `LastChangeTime` until handle close. **[doc]** [MS-FSA appendix A][fsa-notes]
- Writes through memory-mapped views may not update last-write time at all. **[doc]** [Chen,
  RDCW][chen-rdcw] (Chen hedges this)

For a name index this is harmless. For the size and date filters, a file Synology is
downloading shows its old size until Synology closes it. The watcher's last-write event fires
after the cache flush (§5), possibly before the close, so the re-list it triggers can still
read the stale copy **[est]**. The probe's `semantics` step measures it.

### 3.3 Directory modification time

| Change inside directory D | D's mtime changes | Source |
|---|---|---|
| File or subdirectory created in D | **Yes** | **[doc]** [MS-FSA create][fsa-create] |
| Rename within D; move into or out of D | **Yes** | **[doc]** [MS-FSA rename][fsa-rename] |
| File deleted from D | Probably; MS-FSA is silent, an old XP-era test says yes | **unverified** [ucla] |
| A file in D edited (size or content) | **No** | **[doc]** [MS-FSA modify][fsa-mod], [ucla] |
| A change in D\E (grandchild) | **No** for D; E's new time shows in D's listing | **[doc]** [Chen][chen-grand] |

So a directory's mtime reliably flags **name changes among its direct children**. It cannot
prune a whole subtree, and it never flags edited files. Any program can also set timestamps
explicitly. Issue 34's "skip unchanged directories" therefore saves one listing per unchanged
directory. Every directory still has to be *reached* (its mtime comes from its parent's
listing, which is free), and file sizes and dates in unchanged directories go stale until a
full listing. **[est]**

**[code]** `apply.rs` stores each directory's mtime (`set_dir_mtime`) but never uses it to
skip.

### 3.4 Last access, 8.3 names, reparse points, cloud placeholders, long paths

- **Last access** updates are off by default since Vista. Since 1803 they are "system managed":
  on only if the boot volume is small. **[doc]** [fsa-notes], [fsutil behavior][fsutil-beh]. On
  an 8 TB disk the walk causes no last-access writes **[est]**. The probe's `info` step checks.
- **8.3 names**: per-volume setting (`NtfsDisable8dot3NameCreation` = 2 by default). With 8.3
  on, every long name has a second index entry and creation is slower. **[doc]**
  [fsutil 8dot3name][fsutil-83]. `FindExInfoBasic` skips fetching them. The watcher may report
  a **short name** if a program used one **[doc]** [ReadDirectoryChangesW][rdcw],
  [FILE_NOTIFY_EXTENDED_INFORMATION][fnei]. VFS would then mark a parent dirty from a path that
  does not match the index **[est]**. Harmless, since the parent is re-listed.
- **Reparse points**: junctions and symlinks are name surrogates. `FindFirstFileEx` on a link
  describes the link. **[doc]** [ffex]. **[code]** `walk.rs` skips anything
  `FileType::is_symlink()` reports, which on Windows includes junctions and mount points.
- **Cloud placeholders** (Synology On-demand sync, OneDrive):
  - `RECALL_ON_DATA_ACCESS` (0x400000): reading data fetches it. Every placeholder must set
    it. **[doc]** [File attribute constants][fattr], [Placeholder guidance][phguide]
  - `RECALL_ON_OPEN` (0x40000): **opening** fetches it. **[doc]** [fattr]
  - Enumeration and attribute-only opens do not hydrate file data. Reads do, and show the user
    a toast. **[doc]** [phguide], [Cloud sync engine][cfapi]
  - Under `CF_POPULATION_POLICY_FULL`, listing a directory that is not fully populated waits
    for the sync provider, and so for the network. **[doc]** [cf-pop]
  - Placeholders may or may not look like reparse points to an ordinary app; Microsoft's pages
    disagree. **[doc]** [cfapi] vs [rtl-placeholder]
  - Synology's On-demand sync needs Win10 1809+ and NTFS, and keeps names and sizes locally.
    **[doc]** [Synology On-demand][syn-od], [FAQ][syn-od-faq]. That it uses the Cloud Files API
    is implied, not stated.

  **[code]** VFS never reads file data, so it never hydrates. It does not record the
  placeholder attributes, so it cannot show "solo en la nube". It does not open directories
  beyond listing them.
- **Long paths**: the `\\?\` prefix lifts the 260-character limit with no registry setting.
  The process-wide opt-in needs both `LongPathsEnabled=1` (admin to set) and a
  `longPathAware` manifest. "The shell and the file system have different requirements".
  **[doc]** [Maximum path length][maxpath]. **[code]** Rust's std adds `\\?\` itself
  (`maybe_verbatim`), and `platform::long_path` exists for direct calls. Explorer's own limit
  for "mostrar en carpeta" is issue 21.

### 3.5 MFT layout and order

- Listing order is NTFS's B-tree order (ordinal sort); do not rely on it. **[doc]**
  [Chen, order][chen-order]
- The 64-bit file reference number is a 48-bit MFT index plus a 16-bit reuse counter. **[doc]**
  [fsa-notes]
- The MFT zone (200–800 MB) is reserved against MFT fragmentation and shrinks as the volume
  fills. **[doc]** [fsutil-beh]
- On a long-lived, 50–60 % full 8 TB volume, large directories' index blocks are probably
  scattered. Whether the walk order matters is unmeasured **[est]**. A depth-first walk
  (VFS's queue order) at least keeps siblings together **[est]**.

---

## 4. The USN change journal

### 4.1 The calls

| Call | Handle | Standard user (Win10/11) | Source |
|---|---|---|---|
| `FSCTL_QUERY_USN_JOURNAL` | volume, or a directory on the volume | **Yes** on a directory handle or `\\.\X:` opened with `FILE_TRAVERSE`. Volume opened with `GENERIC_READ`: open fails, error 5 | **[doc]** [query][fsctl-q]; **[3p]** BuildXL, AssetCooker, PeekBuddy |
| `FSCTL_READ_USN_JOURNAL` | volume | **No** (error 5) | **[doc]** [read][fsctl-r]; **[3p]** PeekBuddy |
| `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` (0x903AB) | directory on the volume, or volume with `FILE_TRAVERSE` | **Yes**, on current builds | **undocumented** (no Learn page); **[3p]** below |
| `FSCTL_ENUM_USN_DATA` (whole-MFT dump) | volume | **No** (error 5) | **[doc]** [enum][fsctl-e]; **[3p]** PeekBuddy |
| Create, resize or delete the journal | volume | **No**, administrators only | **[doc]** [Creating a change journal][usn-create] |

Third-party evidence for the unprivileged read:
- **Microsoft BuildXL** uses it when not elevated. It opens the volume GUID path with a trailing
  backslash as a directory, because "unprivileged and privileged read journal operations
  require different types of handles". It dates the call to Windows 10 RS2 (1703). **[3p]**
  [InProcChangeJournalAccessor.cs][bxl1], [JournalAccessorGetter.cs][bxl2],
  [NativeIOConstants.cs][bxl3]
- **AssetCooker** opens `\\.\C:` with only `FILE_TRAVERSE` and resolves paths with
  `OpenFileById`. **[3p]** [AssetCooker FileSystem.cpp][assetcooker]
- **PeekBuddy** tested it as a real standard user on Windows 11 builds 26100 and 26200: query
  and unprivileged read work, the privileged read and `ENUM_USN_DATA` fail with error 5, and a
  volume without a journal returns error 1179. **Every record had an empty file name**, even
  for an administrator, and records for files the caller cannot access still arrived. Windows
  10 22H2 was not tested. **[3p]** [PeekBuddy PR 137][peek]

### 4.2 What a record gives without admin, and how to get a path

- With names empty, a record carries the file reference number (FRN), the **parent** FRN, a
  reason mask, attributes and a timestamp. **[3p]** [peek]; record layout **[doc]**
  [USN_RECORD_V2][usnrec]
- `OpenFileById(hint, frn)` + `GetFinalPathNameByHandleW` resolves an FRN to a path, if the
  file still exists and the user may open it. Otherwise it fails: error 87 for deleted files,
  error 5 for denied ones. **[doc]** [OpenFileById][ofbi]; **[3p]** [peek]. On an HDD each
  resolution is a random read of the MFT record **[est]**.
- Opening by id even with access 0 is still an *open*. On a `RECALL_ON_OPEN` placeholder that
  triggers a fetch. **[doc]** [fattr]
- So USN catch-up needs a **persistent FRN → (parent FRN, name) map** filled during the walk.
  Deletions and renames-away can only be named from it. **[est]**, matching PeekBuddy's
  conclusion. VFS stores no file ids today **[code]**. Adding them is a schema change (§9).
- A simpler use needs no map at all **[est]**: map each record's **parent FRN** to an indexed
  directory and re-list that directory. VFS already has "mark directory dirty, re-list it"
  (`WatchEvent::Dirty`). That needs only an FRN per *directory* (400 k rows, not 5 M), and
  every change type (create, delete, rename, size) becomes "re-list the parent".

### 4.3 Journal size, wrap and identity

- `MaximumSize` is a target. NTFS trims old records from the start when the journal exceeds
  `MaximumSize + AllocationDelta`. **[doc]** [usn-create], [fsutil usn][fsutil-usn]
- The default size has no Microsoft figure. 32 MB maximum with a 4–8 MB delta is reported on
  the Everything forum. **[3p]** [voidtools forum][vt-forum]. At ~80–100 bytes per record that
  holds roughly 300–400 k changes **[est]**.
- "The USN journal is active by default on Windows Vista and later". Whether that covers every
  data volume is not stated. **[doc]** [fsa-notes]. A standard user cannot create it. The app
  could ask once for elevation to run `fsutil usn createjournal`, which contradicts the no-admin
  rule.
- Loss detection: a changed `UsnJournalID` means the journal was recreated. A saved USN below
  `FirstUsn` gives `ERROR_JOURNAL_ENTRY_DELETED`. **[doc]** [USN_JOURNAL_DATA_V0][usnjd],
  [READ_USN_JOURNAL_DATA_V0][rujd]
- Records coalesce per open/close, and `USN_REASON_CLOSE` summarises. **[doc]** [Change journal
  records][usn-records]
- Synology churn: the client rewrites its own metadata and touches every file it syncs. How
  many hours of history 32 MB covers on this PC is the central unknown (probe step 10).

### 4.4 What other tools do

- **Everything**: NTFS indexing "requires the Everything service or running Everything as
  administrator". The service is installed by default. **[doc]** [voidtools FAQ][vt-faq],
  [Everything Service][vt-svc]
- **WizTree**: fast MFT scanning "only available when running as administrator". **[doc]**
  [WizTree guide][wiztree]
- **ntfs-reader** (Rust): only the privileged read, on a read/write volume handle. **[doc]**
  [docs.rs ntfs-reader][ntfsreader]
- About a dozen open-source projects call the unprivileged FSCTL (GitHub code search). **[3p]**

### 4.5 Verdict

**A standard user can read the journal on current Windows 10/11, but only as an accelerator
for catching up at start, never as the only mechanism.** Mechanically it works: Microsoft's own
BuildXL ships it, and independent tests on current builds confirm it. But four conditions must
all hold at every start: (1) the data volume has a journal, (2) its id is unchanged, (3) it
still reaches back to the last USN VFS saw, and (4) the undocumented call still behaves. When
any of them fails, VFS needs today's re-walk as the fallback. Synology churn makes (3)
uncertain on this PC. The cheapest correct design is: save `(volume GUID, journal id, next
USN)` at shutdown; at start, replay records into "re-list parent directory" marks; fall back to
the re-walk on any failure. That design needs directory FRNs only.

Whether it pays off depends on two numbers only the real PC can give: N1 (does the call work
on *its* Windows build) and N2 (how many hours the journal holds). Probe steps 2 and 10 measure both.

### 4.6 Minimal test program (to confirm on the real PC)

[`tools/vfs-probe.ps1`](tools/vfs-probe.ps1) step `usn`, as a standard user:
1. Opens `\\.\D:` with `GENERIC_READ` (expected error 5), with access 0, with `FILE_TRAVERSE`,
   the volume GUID path as a directory, and `D:\` with backup semantics. It runs
   `FSCTL_QUERY_USN_JOURNAL` on each handle that opens.
2. On `D:\`: reads the two oldest 64 KB buffers with the unprivileged FSCTL and reports the
   record count, records with names, the oldest timestamp and five sample names. It then
   resolves 200 parent FRNs with `OpenFileById` + `GetFinalPathNameByHandleW` and reports the
   success rate and average time.
3. Reads up to 4,000 buffers to measure catch-up speed.
4. Tries the privileged read and `FSCTL_ENUM_USN_DATA` on the directory handle (expected error 5).
5. Repeats the query at the end of the field test to derive journal bytes per hour.

---

## 5. ReadDirectoryChangesW and its alternatives

- The system allocates an internal buffer the size of the first call's buffer, for the handle's
  lifetime. Changes between calls queue there. **On overflow the call returns TRUE with 0
  bytes** and the queued changes are lost. `ERROR_NOTIFY_ENUM_DIR` is a separate failure. Both
  mean "rescan". **[doc]** [ReadDirectoryChangesW][rdcw]
- Buffers over 64 KB fail on network paths. The buffer must be DWORD-aligned. **[doc]** [rdcw]
- `FILE_NOTIFY_CHANGE_SIZE` and `LAST_WRITE` fire only when the data reaches the disk (after a
  cache flush). **[doc]** [rdcw]
- `ReadDirectoryChangesExW` with `ReadDirectoryNotifyExtendedInformation` (Win10 1709+, **NTFS
  only**) adds file id, parent file id, size, four timestamps and attributes to each event.
  **[doc]** [rdcwex], [fnei]
- Completion by event, IOCP or APC. **[doc]** [rdcw]
- Mature watchers: notify-rs uses 64 KB and loads the Ex form dynamically; libuv uses 64 KB for
  recursive watches; Watchman starts at 16 KB and retries at 64 KB. All three treat 0 bytes and
  `ERROR_NOTIFY_ENUM_DIR` as a rescan. Watchman batches for 30 ms. **[3p]**
  [notify-rs windows.rs][notify], [libuv fs-event.c][libuv], [Watchman win32.cpp][watchman]
- Whether the watch handle blocks renaming or deleting the watched folder's **parent** is not
  documented and reports conflict. **[doc/3p]** [Chen][chen-sharedel]
- Per-flag cost is undocumented. Every size or last-write event from a syncing client fills
  the same 64 KB buffer as the name events VFS needs most **[est]**.

**[code]** `watch.rs`: one thread per root, overlapped I/O with an event, a 64 KB buffer,
recursive. Filter = `FILE_NAME | DIR_NAME | SIZE | LAST_WRITE | CREATION`. Overflow or
`ERROR_NOTIFY_ENUM_DIR` re-arms and sends `Rescan(root)`, i.e. a **full re-walk** of the root,
deferred by the one-hour cooldown (`OWED_COOLDOWN_SECS`). Under heavy Synology churn,
overflows turn into re-walks, which on this disk cost hours (§2.2). Narrowing the filter to
names during a walk, or answering an overflow with a USN replay instead of a re-walk, are the
two levers (recommendations R6, R5).

---

## 6. SQLite on Windows with an HDD

### 6.1 WAL and checkpoints

- Auto-checkpoint runs when a commit leaves the WAL at 1,000 pages or more (`wal_autocheckpoint`),
  and is always PASSIVE. PASSIVE never waits. FULL waits for writers; RESTART also waits for
  readers; TRUNCATE also empties the WAL. **[doc]** [WAL][wal], [pragma][pragma]
- Readers that always overlap starve checkpoints, and the WAL "will grow without bound". A
  checkpoint does not shrink the WAL unless `journal_size_limit` is set. **[doc]** [wal]
- **[code]** `wal_autocheckpoint=4000` (~16 MB at 4 KiB pages), `journal_size_limit=64 MiB`.
  The writer commits every 20 k rows or 5 s. Reasonable: few large checkpoints mean few flushes
  **[est]**.

### 6.2 `synchronous`

- In WAL mode, FULL syncs the WAL on every commit. NORMAL syncs only around checkpoints: always
  consistent, but the last commits "might roll back" after a power loss. OFF can corrupt the
  database on an OS crash. **[doc]** [pragma synchronous][pragma-sync]
- On Windows a sync is `FlushFileBuffers` (`os_win.c` `winSync`). **[doc]** [howtocorrupt],
  [os_win.c][oswin]. In WAL mode a failed sync can corrupt the database only during a
  checkpoint. **[doc]** [howtocorrupt]
- **[code]** `synchronous=NORMAL`. Right for a rebuildable index: losing the last 5 s of
  indexing after a power cut is harmless, and VFS re-lists those directories anyway. OFF would
  risk a multi-hour rebuild on this disk. **[est]**

### 6.3 Page and cache size

- `page_size` defaults to 4096 and is "recommended for most applications". No sqlite.org page
  advises larger pages for HDDs. In WAL mode VACUUM cannot change the page size. **[doc]**
  [pragma page_size][pragma-ps]
- `cache_size` is per connection and allocated on demand up to the limit. **[doc]** [pragma]
- **[code]** Writer 32 MiB, reader 64 MiB, search reader 128 MiB: **up to 224 MiB** of page
  cache once warm, against an AC-9 budget of < 300 MB for the whole app. On a PC already
  swapping, much of it will be paged out and faulted back from the same HDD **[est]**. Issue 19
  measures it. A smaller search cache plus the OS file cache may serve better here, because the
  OS cache shrinks under pressure and a process heap does not **[est]**.
- `SQLITE_FCNTL_CHUNK_SIZE` grows the file in chunks and "may reduce" fragmentation. **[doc]**
  [file controls][fcntl]. The index grows to ~2 GB while Synology writes to the same disk, so
  growing in 16–64 MiB chunks is cheap insurance against fragmentation **[est]**.

### 6.4 Memory-mapped I/O

- Off by default. An I/O error on a mapped file crashes the process. Windows cannot truncate a
  mapped file, so VACUUM shrinks fail silently. **[doc]** [mmap][mmap]
- **[code]** `mmap_size=0`. Keep it.

### 6.5 Antivirus and the index files

- SQLite's Windows VFS retries reads, writes and deletes **up to 10 times**, starting at 25 ms
  and growing linearly (worst case ~1.4 s). It retries on `ERROR_ACCESS_DENIED`,
  `ERROR_SHARING_VIOLATION` and `ERROR_LOCK_VIOLATION`; the source blames antivirus.
  `SQLITE_FCNTL_WIN32_AV_RETRY` tunes it. **[doc]** [fcntl], [os_win.c][oswin]
- Defender scans on open, and Defender's team describes scans on open and close. **[doc]**
  [Defender scan practices][mdav-scan], [exclusions overview][mdav-excl]; **[3p]**
  [Eric Lawrence][lawrence]. Whether it rescans an open database on every write is not
  documented. Keeping the database open for the whole session (VFS does) means one open-scan,
  not one per query **[est]**.
- Exclusions, performance mode and Dev Drive all need admin. Dev Drive also needs Windows 11,
  ReFS and a dedicated volume. **[doc]** [exclusions][mdav-exclcfg], [Dev Drive][devdrive],
  [performance mode][perfmode]. There is no documented non-admin mitigation. The levers left
  are few files, long-lived handles and no temp-file churn.
- Temp files go to `%TEMP%` on the same disk. VACUUM needs up to twice the database size.
  **[doc]** [temp files][tempfiles]. **[code]** `temp_store=MEMORY`. Keep it.
- **Never** let a sync client copy the database without its `-wal`: that corrupts it. **[doc]**
  [howtocorrupt]

### 6.6 Where the index lives

- **[code]** `app_data_dir()` (`apps/desktop/src-tauri/src/lib.rs:101`) resolves to
  `dirs::data_dir()`, which on Windows is **`%APPDATA%` (Roaming)**: `%APPDATA%\app.vfs.desktop\`
  (tauri `crates/tauri/src/path/desktop.rs`). The WebView2 data is already in
  `app_local_data_dir()` (`%LOCALAPPDATA%`).
- Roaming data follows the user profile. Large, machine-specific data belongs in Local.
  **[doc]** [KNOWNFOLDERID][kfid], [Microsoft roaming guidance][roaming]. Synology Drive can sync
  any folder the user picks, but refuses a home folder as an On-demand root. **[doc]**
  [syn-od-faq]
- `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED` keeps Windows Search out of a folder. A user can set it
  on their own folder. **[doc]** [fattr]
- Conclusion: move the index to `%LOCALAPPDATA%\app.vfs.desktop\`, mark the folder not content
  indexed, and check at start that the folder is not inside a Synology sync root (issue 36).

### 6.7 FTS5 maintenance on an HDD

- Every transaction writes a new FTS5 segment. automerge (default 4) merges incrementally
  during later writes. crisismerge (default 16) merges all at once and stalls that write.
  `optimize` "can take a long time"; `'merge', N` does bounded work. **[doc]** [FTS5][fts5]
- `detail=column` or `none` made the docs' example index 2–5× smaller. **[doc]** [fts5]
  (index-footprint work belongs to the parallel footprint session).

---

## 7. WebView2: what a Tauri window costs

- One browser process per user-data folder, plus renderer, GPU and utility processes.
  WebView2 shares no processes with Edge, but shares binaries in memory when the versions
  match. **[doc]** [Process model][wv2-proc], [Performance][wv2-perf]
- Microsoft publishes no typical figure. Tauri's own benchmark issue has no minimal-app number
  on Windows. **unverified** [tauri-5889]. Measure on the real PC (plan step 9).
- `ICoreWebView2_19::MemoryUsageTargetLevel = LOW` (runtime 114+): scripts keep running,
  memory may be swapped out, and the app restores NORMAL itself. **[doc]** [ICoreWebView2_19][wv2-19]
- `ICoreWebView2_3::TrySuspend` requires `IsVisible = false`. It works like a sleeping tab and
  resumes when shown. Microsoft's sample calls it on minimise. **[doc]** [ICoreWebView2_3][wv2-3]
- Do not disable the GPU except for troubleshooting. Production apps should not use browser
  flags. **[doc]** [wv2-perf], [flags][wv2-flags]
- Tauri 2: `additionalBrowserArgs` **replaces** the default
  `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`, so re-add it.
  `backgroundThrottling` is unsupported on Windows. wry 0.57 exposes `set_memory_usage_level`.
  tauri 2.12 `with_webview` gives the `ICoreWebView2Controller` for `IsVisible(false)` +
  `TrySuspend`. Whether Tauri does any of this on minimise by itself is unverified: assume
  not. **[doc]** [Tauri config][tauri-cfg], [wry WebViewExtWindows][wry-ext], [tauri PlatformWebview][tauri-pw]
- **[code]** `tauri.conf.json` sets no browser arguments, and nothing reacts to minimise.
- The largest saving is **destroying the WebView window when hidden** and recreating it on
  demand. That removes the renderer and GPU processes. The cost is a slower reopen on an HDD
  (not measured) **[est]**. Issue 37 already weighs this.

---

## 8. Defender and the Synology client

### 8.1 Defender

- Real-time protection scans files "before they are accessed or executed", synchronously at
  open. **[doc]** [mdav-scan], [perfmode]. Scans also happen on close. **[3p]** [lawrence]
- Whether listing a directory or an attribute-only open triggers a content scan is **not
  documented**. Microsoft tells filter drivers not to turn attribute-only opens into data reads.
  **[doc]** [phguide]. That suggests listing is cheap, but it is no proof for Defender.
- Scheduled scans: `ScanAvgCPULoadFactor` defaults to 50 %, as guidance only. Scans while
  idle are not throttled. A full scan "can last from several hours to several days". **[doc]**
  [mdav-scan]. On 8 TB a full scan competes with the indexer for the disk; the back-off in R3
  covers it.
- **[code]** VFS never opens user files. That is the best non-admin defence there is. The
  index files stay open for the session.

### 8.2 Synology clients

- **Drive Client** (sync): how it detects changes is undocumented. A new or relinked task
  compares all files, "Advanced consistency check" hashes them, and filters cover folders, size,
  names and extensions. **[doc]** [Synology Drive Client help][syn-help]. Whether it re-scans
  the whole tree after each reboot is unknown. Plan step 7 asks.
- **On-demand sync**: online-only files keep only names and sizes locally. Opening, copying or
  pasting downloads them; moving and renaming do not. **[doc]** [syn-od-faq]. VFS's "mostrar en
  carpeta" only selects the file in Explorer, which does not download it **[est]**.
- **Active Backup for Business agent**: Changed Block Tracking on top of VSS snapshots.
  **[doc]** [ABB help][abb]. VSS copy-on-write adds I/O on the same disk while the index
  changes. A frequently rewritten 2 GB index inflates backups if the agent backs up `AppData`
  **[est]**.
- **[code]** `walk.rs` and the watcher filter excludes Synology's metadata names
  (`.SynologyWorkingDirectory`, `@eaDir`, `#recycle`, `#snapshot`) and temporary files (issue 30).

---

## 9. The code against this study

| Area | Today ([code]) | Verdict | Recommendation |
|---|---|---|---|
| Indexer priority | Thread background mode for the whole life of the worker, including the first index and full re-walks | **Does it, partly wrong**: under constant Synology I/O, very-low I/O may starve a cold walk into days (§2.2); CPU priority not lowered on Win8+ | R1 |
| SQLite work on the background thread | Commits, checkpoints and `quick_check` run at very-low I/O while holding locks | **Risk** of priority inversion with root changes (§1.2) | R1 |
| Process-wide background mode | Not used | **Right** (§1.2) | — |
| Listing call | `std::fs::read_dir` = `FindFirstFileExW(FindExInfoBasic)`, no large fetch, metadata from the listing, no file opens | **Right** in what matters; large fetch or `FileIdExtdDirectoryInfo` may save syscalls, and Extd gives file ids | R10 |
| Placeholders | Listed by name and size; attributes not stored | **Right** (no hydration); no "solo en la nube" | R11 |
| Directory mtime | Stored, not used to skip | **Does not do it** | R9 (leaf directories only), R5; issue 34 |
| Catch-up at start | Full re-walk of every root | **Costly** on this disk; USN replay possible as an accelerator | R5, issue 32 |
| Watcher | RDCW, 64 KB, names + size + last-write + creation; overflow → full re-walk after a 1 h cooldown | **Works**; the filter invites overflows under churn, and overflow → re-walk is expensive | R6 |
| Busy-disk back-off | None | **Does not do it** | R3, issue 20 |
| `synchronous`, WAL, mmap, temp_store | NORMAL, WAL, mmap 0, MEMORY | **Right** | — |
| Page caches | 32 + 64 + 128 MiB | **Probably too large** for this PC | R7, issue 19 |
| File growth | Default | **Does not do it**: no chunked growth | R7 |
| Index location | `%APPDATA%` (Roaming) | **Wrong place** | R2, issue 36 |
| Windows Search on the index folder | Not marked | **Does not do it** | R2 |
| WebView2 when minimised | Nothing | **Does not do it** | R8, issues 19, 37 |
| Long paths | `\\?\` via std and `long_path` | **Right**; Explorer side is issue 21 | — |

---

## Sources

Microsoft Learn and Microsoft publications (fetched 2026-11-20):

- `stp`: <https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadpriority>
- `spc`: <https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setpriorityclass>
- `iopri`: <https://download.microsoft.com/download/a/f/7/af7777e5-7dcd-4800-8a0a-b18336565f5b/Priorityio.doc>
- `chen-bg`: <https://devblogs.microsoft.com/oldnewthing/20161028-00/?p=94595>
- `wi6`: <https://www.microsoftpressstore.com/articles/article.aspx?p=2201309&seqNum=3>
- `sched`: <https://learn.microsoft.com/en-us/windows/win32/procthread/scheduling-priorities>
- `qos`: <https://learn.microsoft.com/en-us/windows/win32/procthread/quality-of-service>
- `ecoqos`: <https://devblogs.microsoft.com/performance-diagnostics/introducing-ecoqos/>
- `mempri`: <https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/ns-processthreadsapi-memory_priority_information>
- `spi`: <https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation>
- `russ1`: <https://learn.microsoft.com/en-us/previous-versions/technet-magazine/cc162480(v=msdn.10)>
- `russ2`: <https://learn.microsoft.com/en-us/previous-versions/technet-magazine/cc162494(v=msdn.10)>
- `ews`: <https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-emptyworkingset>
- `ws`: <https://learn.microsoft.com/en-us/windows/win32/memory/working-set>
- `fiop`: <https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_io_priority_hint_info>
- `chen-prio`: <https://devblogs.microsoft.com/oldnewthing/20201127-00/?p=104492>
- `fcache`: <https://learn.microsoft.com/en-us/windows/win32/fileio/file-caching>
- `chen-seq`: <https://devblogs.microsoft.com/oldnewthing/20120120-00/?p=8493>
- `kb2549369`: <https://learn.microsoft.com/en-us/troubleshoot/windows-server/application-management/operating-system-performance-degrades>
- `ffb`: <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers>
- `chen-flush`: <https://devblogs.microsoft.com/oldnewthing/20170510-00/?p=95505>
- `pdhuser`: <https://learn.microsoft.com/en-us/windows/win32/perfctrs/limited-user-access-support>
- `ntdbg`: <https://learn.microsoft.com/en-us/archive/blogs/ntdebugging/disk-performance-internals>
- `pdheng`: <https://learn.microsoft.com/en-us/windows/win32/api/pdh/nf-pdh-pdhaddenglishcounterw>
- `ioctldp`: <https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-ioctl_disk_performance>
- `gst`: <https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getsystemtimes>
- `glii`: <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getlastinputinfo>
- `prsrn`: <https://learn.microsoft.com/en-us/windows/win32/api/powerbase/nf-powerbase-powerregistersuspendresumenotification>
- `gpic`: <https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getprocessiocounters>
- `taskidle`: <https://learn.microsoft.com/en-us/windows/win32/taskschd/task-idle-conditions>
- `findex`: <https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ne-minwinbase-findex_info_levels>
- `ffex`: <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-findfirstfileexw>
- `fibh`: <https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ne-minwinbase-file_info_by_handle_class>
- `fidextd`: <https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_extd_dir_info>
- `ntqdfex`: <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntquerydirectoryfileex>
- `ntfs-tn`: <https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-2000-server/cc938949(v=technet.10)>
- `fsa-dup`: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/7e375703-5641-48f3-b844-c2ef1ee70461>
- `fsa-notes`: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/4e3695bd-7574-4f24-a223-b4679c065b63>
- `fsa-create`: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/498a5eeb-afc9-445f-99ed-a7e4b73c648b>
- `fsa-rename`: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/87f86c9b-6c2a-4803-84b7-131a74a434fa>
- `fsa-mod`: <https://learn.microsoft.com/en-us/openspecs/windows_protocols/MS-FSA/75cdaba1-4401-4c53-b09c-69ba6cd50ce6>
- `chen-size`: <https://devblogs.microsoft.com/oldnewthing/20111226-00/?p=8813>
- `chen-rdcw`: <https://devblogs.microsoft.com/oldnewthing/20110812-00/?p=9913>
- `chen-grand`: <https://devblogs.microsoft.com/oldnewthing/20180712-00/?p=99225>
- `chen-order`: <https://devblogs.microsoft.com/oldnewthing/20140304-00/?p=1603>
- `chen-sharedel`: <https://devblogs.microsoft.com/oldnewthing/20211022-00/?p=105822>
- `fsutil-beh`: <https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/fsutil-behavior>
- `fsutil-83`: <https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/fsutil-8dot3name>
- `fsutil-usn`: <https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/fsutil-usn>
- `fattr`: <https://learn.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants>
- `phguide`: <https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/placeholders_guidance>
- `cfapi`: <https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine>
- `cf-pop`: <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_population_policy_primary>
- `rtl-placeholder`: <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-rtlsetprocessplaceholdercompatibilitymode>
- `maxpath`: <https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation>
- `fsctl-q`: <https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_query_usn_journal>
- `fsctl-r`: <https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_read_usn_journal>
- `fsctl-e`: <https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_enum_usn_data>
- `usn-create`: <https://learn.microsoft.com/en-us/windows/win32/fileio/creating-modifying-and-deleting-a-change-journal>
- `usn-records`: <https://learn.microsoft.com/en-us/windows/win32/fileio/change-journal-records>
- `usnrec`: <https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v2>
- `usnjd`: <https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_journal_data_v0>
- `rujd`: <https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-read_usn_journal_data_v0>
- `ofbi`: <https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-openfilebyid>
- `rdcw`: <https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesw>
- `rdcwex`: <https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesexw>
- `fnei`: <https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-file_notify_extended_information>
- `kfid`: <https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid>
- `roaming`: <https://technet.microsoft.com/library/cc766489.aspx>
- `mdav-scan`: <https://learn.microsoft.com/en-us/defender-endpoint/mdav-scan-best-practices>
- `mdav-excl`: <https://learn.microsoft.com/en-us/defender-endpoint/microsoft-defender-antivirus-exclusions-overview>
- `mdav-exclcfg`: <https://learn.microsoft.com/en-us/defender-endpoint/microsoft-defender-antivirus-exclusions-configure>
- `devdrive`: <https://learn.microsoft.com/en-us/windows/dev-drive/>
- `perfmode`: <https://learn.microsoft.com/en-us/defender-endpoint/microsoft-defender-endpoint-antivirus-performance-mode>
- `wv2-proc`: <https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model>
- `wv2-perf`: <https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance>
- `wv2-19`: <https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_19>
- `wv2-3`: <https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_3>
- `wv2-flags`: <https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/webview-features-flags>

SQLite (sqlite.org and the official source mirror):

- `wal`: <https://www.sqlite.org/wal.html>
- `pragma`: <https://www.sqlite.org/pragma.html>
- `pragma-sync`: <https://www.sqlite.org/pragma.html#pragma_synchronous>
- `pragma-ps`: <https://www.sqlite.org/pragma.html#pragma_page_size>
- `howtocorrupt`: <https://www.sqlite.org/howtocorrupt.html>
- `oswin`: <https://github.com/sqlite/sqlite/blob/master/src/os_win.c>
- `fcntl`: <https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html>
- `mmap`: <https://www.sqlite.org/mmap.html>
- `tempfiles`: <https://www.sqlite.org/tempfiles.html>
- `fts5`: <https://www.sqlite.org/fts5.html>

Tauri, wry:

- `tauri-cfg`: <https://v2.tauri.app/reference/config/>
- `wry-ext`: <https://docs.rs/wry/latest/x86_64-pc-windows-msvc/wry/trait.WebViewExtWindows.html>
- `tauri-pw`: <https://docs.rs/tauri/latest/x86_64-pc-windows-msvc/tauri/webview/struct.PlatformWebview.html>
- `tauri-5889`: <https://github.com/tauri-apps/tauri/issues/5889>

Vendors:

- `syn-help`: <https://kb.synology.com/en-my/DSM/help/SynologyDriveClient/synologydriveclient?version=7>
- `syn-od`: <https://kb.synology.com/en-my/DSM/tutorial/What_is_On-demand_Sync>
- `syn-od-faq`: <https://kb.synology.com/en-my/DSM/tutorial/Frequently_Asked_Questions_On_Demand_Sync>
- `abb`: <https://kb.synology.com/en-id/DSM/help/ActiveBackup/activebackup_business_pc_device?version=7>
- `vt-faq`: <https://www.voidtools.com/faq/>
- `vt-svc`: <https://www.voidtools.com/support/everything/everything_service/>
- `vt-forum`: <https://voidtools.com/forum/viewtopic.php?p=38762>
- `wiztree`: <https://diskanalyzer.com/guide>

Third-party code and measurements:

- `chrtest`: <https://chromium.googlesource.com/chromium/src/base/+/HEAD/threading/platform_thread_win_unittest.cc>
- `chrwin`: <https://chromium.googlesource.com/chromium/src/base/+/HEAD/threading/platform_thread_win.cc>
- `chr78`: <https://chromium.googlesource.com/chromium/src/+/78.0.3904.87/base/threading/platform_thread_win.h>
- `dawson`: <https://randomascii.wordpress.com/2023/10/01/32-mib-working-sets-on-a-64-gib-machine/>
- `moz`: <https://bugzilla.mozilla.org/show_bug.cgi?id=1856287>
- `bxl1`: <https://github.com/microsoft/BuildXL/blob/main/Public/Src/Utilities/Storage/ChangeJournalService/InProcChangeJournalAccessor.cs>
- `bxl2`: <https://github.com/microsoft/BuildXL/blob/main/Public/Src/Utilities/Storage/JournalAccessorGetter.cs>
- `bxl3`: <https://github.com/microsoft/BuildXL/blob/main/Public/Src/Utilities/Native/IO/NativeIOConstants.cs>
- `assetcooker`: <https://github.com/jlaumon/AssetCooker/blob/main/src/FileSystem.cpp>
- `peek`: <https://github.com/nobuddyorg/PeekBuddy/pull/137>
- `ntfsreader`: <https://docs.rs/ntfs-reader/0.4.5/src/ntfs_reader/journal.rs.html>
- `notify`: <https://github.com/notify-rs/notify/blob/main/notify/src/windows.rs>
- `libuv`: <https://github.com/libuv/libuv/blob/v1.x/src/win/fs-event.c>
- `watchman`: <https://github.com/facebook/watchman/blob/main/watchman/watcher/win32.cpp>
- `lawrence`: <https://textslashplain.com/2026/04/10/understanding-defender-av-scans/>
- `ucla`: <https://bookstack.kb.ucla.edu/books/macos-windows-and-linux/page/effect-of-file-operations-on-its-own-and-its-parent-directorys-modification-times>

<!-- link definitions -->
[stp]: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadpriority
[spc]: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setpriorityclass
[iopri]: https://download.microsoft.com/download/a/f/7/af7777e5-7dcd-4800-8a0a-b18336565f5b/Priorityio.doc
[chen-bg]: https://devblogs.microsoft.com/oldnewthing/20161028-00/?p=94595
[wi6]: https://www.microsoftpressstore.com/articles/article.aspx?p=2201309&seqNum=3
[sched]: https://learn.microsoft.com/en-us/windows/win32/procthread/scheduling-priorities
[qos]: https://learn.microsoft.com/en-us/windows/win32/procthread/quality-of-service
[ecoqos]: https://devblogs.microsoft.com/performance-diagnostics/introducing-ecoqos/
[mempri]: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/ns-processthreadsapi-memory_priority_information
[spi]: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation
[russ1]: https://learn.microsoft.com/en-us/previous-versions/technet-magazine/cc162480(v=msdn.10)
[russ2]: https://learn.microsoft.com/en-us/previous-versions/technet-magazine/cc162494(v=msdn.10)
[ews]: https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-emptyworkingset
[ws]: https://learn.microsoft.com/en-us/windows/win32/memory/working-set
[fiop]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_io_priority_hint_info
[chen-prio]: https://devblogs.microsoft.com/oldnewthing/20201127-00/?p=104492
[fcache]: https://learn.microsoft.com/en-us/windows/win32/fileio/file-caching
[chen-seq]: https://devblogs.microsoft.com/oldnewthing/20120120-00/?p=8493
[kb2549369]: https://learn.microsoft.com/en-us/troubleshoot/windows-server/application-management/operating-system-performance-degrades
[ffb]: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers
[chen-flush]: https://devblogs.microsoft.com/oldnewthing/20170510-00/?p=95505
[pdhuser]: https://learn.microsoft.com/en-us/windows/win32/perfctrs/limited-user-access-support
[ntdbg]: https://learn.microsoft.com/en-us/archive/blogs/ntdebugging/disk-performance-internals
[pdheng]: https://learn.microsoft.com/en-us/windows/win32/api/pdh/nf-pdh-pdhaddenglishcounterw
[ioctldp]: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-ioctl_disk_performance
[gst]: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getsystemtimes
[glii]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getlastinputinfo
[prsrn]: https://learn.microsoft.com/en-us/windows/win32/api/powerbase/nf-powerbase-powerregistersuspendresumenotification
[gpic]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getprocessiocounters
[taskidle]: https://learn.microsoft.com/en-us/windows/win32/taskschd/task-idle-conditions
[findex]: https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ne-minwinbase-findex_info_levels
[ffex]: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-findfirstfileexw
[fibh]: https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ne-minwinbase-file_info_by_handle_class
[fidextd]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_extd_dir_info
[ntqdfex]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntquerydirectoryfileex
[ntfs-tn]: https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-2000-server/cc938949(v=technet.10)
[fsa-dup]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/7e375703-5641-48f3-b844-c2ef1ee70461
[fsa-notes]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/4e3695bd-7574-4f24-a223-b4679c065b63
[fsa-create]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/498a5eeb-afc9-445f-99ed-a7e4b73c648b
[fsa-rename]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/87f86c9b-6c2a-4803-84b7-131a74a434fa
[fsa-mod]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/MS-FSA/75cdaba1-4401-4c53-b09c-69ba6cd50ce6
[chen-size]: https://devblogs.microsoft.com/oldnewthing/20111226-00/?p=8813
[chen-rdcw]: https://devblogs.microsoft.com/oldnewthing/20110812-00/?p=9913
[chen-grand]: https://devblogs.microsoft.com/oldnewthing/20180712-00/?p=99225
[chen-order]: https://devblogs.microsoft.com/oldnewthing/20140304-00/?p=1603
[chen-sharedel]: https://devblogs.microsoft.com/oldnewthing/20211022-00/?p=105822
[fsutil-beh]: https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/fsutil-behavior
[fsutil-83]: https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/fsutil-8dot3name
[fsutil-usn]: https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/fsutil-usn
[fattr]: https://learn.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants
[phguide]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/placeholders_guidance
[cfapi]: https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine
[cf-pop]: https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_population_policy_primary
[rtl-placeholder]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-rtlsetprocessplaceholdercompatibilitymode
[maxpath]: https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation
[fsctl-q]: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_query_usn_journal
[fsctl-r]: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_read_usn_journal
[fsctl-e]: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_enum_usn_data
[usn-create]: https://learn.microsoft.com/en-us/windows/win32/fileio/creating-modifying-and-deleting-a-change-journal
[usn-records]: https://learn.microsoft.com/en-us/windows/win32/fileio/change-journal-records
[usnrec]: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v2
[usnjd]: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_journal_data_v0
[rujd]: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-read_usn_journal_data_v0
[ofbi]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-openfilebyid
[rdcw]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesw
[rdcwex]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesexw
[fnei]: https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-file_notify_extended_information
[kfid]: https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid
[roaming]: https://technet.microsoft.com/library/cc766489.aspx
[mdav-scan]: https://learn.microsoft.com/en-us/defender-endpoint/mdav-scan-best-practices
[mdav-excl]: https://learn.microsoft.com/en-us/defender-endpoint/microsoft-defender-antivirus-exclusions-overview
[mdav-exclcfg]: https://learn.microsoft.com/en-us/defender-endpoint/microsoft-defender-antivirus-exclusions-configure
[devdrive]: https://learn.microsoft.com/en-us/windows/dev-drive/
[perfmode]: https://learn.microsoft.com/en-us/defender-endpoint/microsoft-defender-endpoint-antivirus-performance-mode
[wv2-proc]: https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model
[wv2-perf]: https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance
[wv2-19]: https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_19
[wv2-3]: https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_3
[wv2-flags]: https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/webview-features-flags
[wal]: https://www.sqlite.org/wal.html
[pragma]: https://www.sqlite.org/pragma.html
[pragma-sync]: https://www.sqlite.org/pragma.html#pragma_synchronous
[pragma-ps]: https://www.sqlite.org/pragma.html#pragma_page_size
[howtocorrupt]: https://www.sqlite.org/howtocorrupt.html
[oswin]: https://github.com/sqlite/sqlite/blob/master/src/os_win.c
[fcntl]: https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html
[mmap]: https://www.sqlite.org/mmap.html
[tempfiles]: https://www.sqlite.org/tempfiles.html
[fts5]: https://www.sqlite.org/fts5.html
[tauri-cfg]: https://v2.tauri.app/reference/config/
[wry-ext]: https://docs.rs/wry/latest/x86_64-pc-windows-msvc/wry/trait.WebViewExtWindows.html
[tauri-pw]: https://docs.rs/tauri/latest/x86_64-pc-windows-msvc/tauri/webview/struct.PlatformWebview.html
[tauri-5889]: https://github.com/tauri-apps/tauri/issues/5889
[syn-help]: https://kb.synology.com/en-my/DSM/help/SynologyDriveClient/synologydriveclient?version=7
[syn-od]: https://kb.synology.com/en-my/DSM/tutorial/What_is_On-demand_Sync
[syn-od-faq]: https://kb.synology.com/en-my/DSM/tutorial/Frequently_Asked_Questions_On_Demand_Sync
[abb]: https://kb.synology.com/en-id/DSM/help/ActiveBackup/activebackup_business_pc_device?version=7
[vt-faq]: https://www.voidtools.com/faq/
[vt-svc]: https://www.voidtools.com/support/everything/everything_service/
[vt-forum]: https://voidtools.com/forum/viewtopic.php?p=38762
[wiztree]: https://diskanalyzer.com/guide
[chrtest]: https://chromium.googlesource.com/chromium/src/base/+/HEAD/threading/platform_thread_win_unittest.cc
[chrwin]: https://chromium.googlesource.com/chromium/src/base/+/HEAD/threading/platform_thread_win.cc
[chr78]: https://chromium.googlesource.com/chromium/src/+/78.0.3904.87/base/threading/platform_thread_win.h
[dawson]: https://randomascii.wordpress.com/2023/10/01/32-mib-working-sets-on-a-64-gib-machine/
[moz]: https://bugzilla.mozilla.org/show_bug.cgi?id=1856287
[bxl1]: https://github.com/microsoft/BuildXL/blob/main/Public/Src/Utilities/Storage/ChangeJournalService/InProcChangeJournalAccessor.cs
[bxl2]: https://github.com/microsoft/BuildXL/blob/main/Public/Src/Utilities/Storage/JournalAccessorGetter.cs
[bxl3]: https://github.com/microsoft/BuildXL/blob/main/Public/Src/Utilities/Native/IO/NativeIOConstants.cs
[assetcooker]: https://github.com/jlaumon/AssetCooker/blob/main/src/FileSystem.cpp
[peek]: https://github.com/nobuddyorg/PeekBuddy/pull/137
[ntfsreader]: https://docs.rs/ntfs-reader/0.4.5/src/ntfs_reader/journal.rs.html
[notify]: https://github.com/notify-rs/notify/blob/main/notify/src/windows.rs
[libuv]: https://github.com/libuv/libuv/blob/v1.x/src/win/fs-event.c
[watchman]: https://github.com/facebook/watchman/blob/main/watchman/watcher/win32.cpp
[lawrence]: https://textslashplain.com/2026/04/10/understanding-defender-av-scans/
[ucla]: https://bookstack.kb.ucla.edu/books/macos-windows-and-linux/page/effect-of-file-operations-on-its-own-and-its-parent-directorys-modification-times
