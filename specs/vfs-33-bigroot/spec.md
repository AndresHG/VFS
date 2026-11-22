# VFS-33 — A huge root settles: protected folders once, quiet background updates

- **Status**: part 1 implemented (PR #162); part 2 approved (owner decision, then an independent critic pass)
- **Size**: M (indexer only, no schema change; part 2 adds a platform-specific list)
- **Links**: issues 159, 156; VFS-1 I11 and U6; VFS-5 R1–R3 (pass modes); VFS-10 R1 (reasons)

## Problem

The owner added their whole home folder (4.87 M files, 597 k folders) on macOS. The first
walk was fine (1,496 s, ~195 k files/min), but afterwards the app never settled. A
`pass_end mode=walk` came every 1–10 s, the status kept saying «Comprobando cambios…» and
`retries_pending=141` stayed put. There are two causes. First, macOS refuses to list ~140
protected folders under `~/Library` and `~/Pictures` (EPERM). VFS-10 R1 keeps such folders
pending and retries them on the I11 schedule. While they are pending they count in the
pass-mode total (141 > `incremental_max_dirs` = 64), so every watcher pass, however small,
runs as a walk, and each attempt logs a warning with the folder's path. Second, every pass
publishes the scanning state, even a one-folder update, so constant small changes keep
«Comprobando cambios» on. The target PC has the same churn in `AppData` and a sync client
that is always on, so it would never look settled either.

## Requirements (EARS)

- R1. WHEN a directory fails to list with "no permission", THE indexer SHALL record its
  reason at once (VFS-10) and retry it once after `RETRY_FIRST` (1 min). This covers denials
  that pass, such as a folder being deleted on Windows or a sync or antivirus filter. IF it
  fails again, THEN the indexer SHALL mark it clean in the same transaction, with no retry
  left. After that it is listed again only when a full walk of its root runs: «Buscar
  cambios ahora», the weekly periodic walk, the root coming back online, or lost change
  history. A restart replays the change history and does not re-check it (VFS-9). This
  amends VFS-10 R1 and AC-1 and VFS-1 I11 for this reason only. Every other reason keeps the
  six I11 attempts.
- R2. WHILE a directory has the reason "no permission", watcher hints and replayed changes
  (VFS-9, VFS-26) SHALL NOT mark it pending. A watcher hint naming the folder itself marks
  its parent only (the folder may be gone). A hint for any path below it, at any depth,
  marks nothing. A replayed change naming the folder marks nothing. A subfolder indexed
  before the denial is not protected itself: a hint there marks it, and its listing fails
  and settles under R1.
- R3. WHEN the indexer picks a pass mode (VFS-5 R1), it SHALL count only pending directories
  that are not waiting for a retry, the same set the pass lists.
- R4. WHILE a batch runs in incremental mode (no root walked in full, at most
  `incremental_max_dirs` folders due when the batch's mode is chosen), THE indexer SHALL NOT
  publish the scanning state. The status and the footer buttons stay as they were
  («Última revisión…», «Buscar cambios ahora»), results still refresh after each commit, and
  a pass that listed anything ends with a fresh idle snapshot, so the counts follow. The mode is chosen per batch, so a burst of changes that pushes the due
  folders over the limit, or a root coming back online, switches to walk mode and shows
  «Comprobando cambios…»; this is intended. This amends VFS-1 U6: «Comprobando cambios…»
  shows during a full walk or a batch of more than `incremental_max_dirs` folders, never for
  a small background update.

## Acceptance scenarios

Tests cite these ids (`VFS-33 AC-<m>`) in their doc comments.

- AC-1 (R1; Unix, mode 000) Given a scanned root with a folder made unreadable, When a full
  walk lists it, Then it has reason "no permission" and counts at once, and is pending with
  one retry. When the retry is due and fails, Then it is clean with no retry entry, and
  running the indexer again lists nothing and counts no error. When another full walk runs
  after its permission is restored, it is listed and its reason is gone.
- AC-2 (R2) Given a folder with reason "no permission", When a watcher hint names a file
  inside it or a deeper path, or a replayed change names it, Then no folder becomes pending.
  When a watcher hint names the folder itself after it was deleted, Then only its parent
  becomes pending, and listing the parent drops the folder and its reason. Given a readable
  folder, a hint marks it pending as before.
- AC-3 (R1) Given a folder failing with any other error (a file where the folder was), Then
  it is still retried on the I11 schedule with growing delays and settled after
  `RETRY_ATTEMPTS` (VFS-1 AC-7, VFS-10 AC-6 unchanged).
- AC-4 (R3) Given more than `incremental_max_dirs` pending folders that are all waiting for a
  retry, and one other pending folder, When the pass mode is chosen, Then it is incremental.
  Without the waiting retries, the same count of due folders chooses walk.
- AC-5 (R4) Given a ready root and an idle indexer, When a small incremental pass lists a
  changed folder after the progress throttle has elapsed, Then the sink receives no scanning
  progress, the index-changed signal still fires, and a fresh idle snapshot arrives. When a
  full walk runs, the sink receives scanning progress as before.

## Non-goals

- Coalescing watcher passes for longer than the existing 2 s, or rate-limiting the
  `pass_end` log lines of incremental passes: filed as a follow-up with the measurements
  below.
- Granting macOS Full Disk Access, or explaining it in the UI.
- Retry state still lives in memory (VFS-10 non-goal unchanged).
- «Pausado» after «Continuar»: the paused state stays until the next idle or scanning
  update, as before; an incremental pass lasts milliseconds.

## Measurements (a developer Mac, home folder as the root, v0.2.0, SSD)

- Index 1.64 GB for 4.87 M files + 597 k folders (~337 B per file, FTS included); WAL held at
  64 MB. Commits: 325, avg 44 ms, max 977 ms. Checkpoints: 65, avg 129 ms, max 666 ms.
  Working set: 88–135 MB during the walk (AC-9 budget < 300 MB).
- Of the 4.87 M files, ~3.05 M are under code checkouts and their worktrees (Python venvs,
  Rust `target`, `node_modules`), and 0.73 M are under `~/Library` (Caches 374 k, Application
  Support 240 k). About 31 k are under Documents.
- FSEvents overflowed every few minutes under build churn. Each time it replayed ~5,000
  folders (VFS-26), which takes about 3 s to mark.

## Pre-mortem

- Slow PC, HDD: fewer listings than before (no retry loop); no new I/O.
- Long paths, case: n/a because no path handling changes.
- NFC/NFD: n/a because nothing is matched or folded.
- Antivirus, sync clients: on Windows `ERROR_ACCESS_DENIED` also covers passing cases (a
  folder pending deletion, a filter driver). The one retry after 1 min covers those. A
  denial that lasts longer than that waits for the next full walk, as the VFS-10 list
  already tells the user («Buscar cambios ahora» checks again). That passing denials always
  clear within a minute is an assumption the CI does not verify.
- Killed mid-write: the reason and the clean mark are written in one transaction.
- Million-file match: n/a because search is unchanged.
- Permission-denied: this is the change. Such a folder is listed once per full walk and
  shown once in the list, and hints inside it do not loop.
- User files: none opened or written.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Retry "no permission" folders? | Once, then settle. A TCC or ACL denial does not heal by waiting (on macOS: 140 folders, 4 tries each in 15 min), and the VFS-10 list already says «Buscar cambios ahora» checks again. The one retry covers passing denials on Windows | Claude (issue 159 data, critic) | 2026-11-22 |
| 2 | When does «Comprobando cambios» show? | During walk-mode passes only (full walks and batches over 64 folders); incremental updates are silent | Claude | 2026-11-22 |
| 3 | Default exclusions (OS and app-data trees, developer folders) | "Sistema + desarrollo" widened: a researched, sourced list in four categories (system, app data, development, other programs), all excluded by default; one hidden switch in Ajustes › Avanzado to include them; a folder added as a root is still indexed | owner (through the coordinating session) | 2026-11-22 |
| 5 | Venvs: by name or by marker? | By the `pyvenv.cfg` marker, and only the venv's own subfolders: names such as `venv` or `env` are personal-folder names, and `python -m venv .` in a project must not hide the project | Claude (research) | 2026-11-22 |
| 6 | Where does the setting live? | `meta` key `include_all_folders` in the index; each root records `exclusions/<id>` = list version and setting of its last full walk | Claude | 2026-11-22 |
| 4 | Approval of part 1 | Approved by the session after an independent critic pass, per the owner's standing instruction | Claude | 2026-11-22 |

## Open questions

None.

## Part 2: default exclusions

Owner decision (Decisions #3): option 2, "Sistema + desarrollo", widened. The list is
researched and sourced, grouped into four categories, and all of it is excluded by default.
One hidden switch in Ajustes › Avanzado includes these folders. A folder added explicitly as
a root is still indexed.

### Requirements (EARS)

- R5. WHILE the setting «Incluir carpetas del sistema y de programas» is off (the default),
  THE walker SHALL skip every folder below a root that a rule of the exclusion list matches,
  and the watcher and replays (VFS-9, VFS-26) SHALL drop changes inside folders matched by
  a name or path rule. Markers need the folder's entries, so changes inside a marked folder
  still re-list it, and the listing comes back empty or filtered. The
  list is one data table (`crates/vfs-core/src/indexer/exclude.rs`, `LIST`), in four
  categories: system, app data, development, other programs. A rule has one of these forms:
  - a folder name anywhere, ignoring ASCII case;
  - a path below the user's home;
  - every child of a home path except named ones (`~/Library` keeps `Mobile Documents` and
    `CloudStorage`);
  - a path below the root of the root's volume;
  - a marker entry inside a folder, matched by name only, because no file is opened (VFS-1).
    `CACHEDIR.TAG` skips the folder's contents. `pyvenv.cfg` skips only the venv's
    `lib`/`Lib`, `bin`, `Scripts`, `include`/`Include` subfolders.
- R6. A path rule SHALL NOT apply to the root itself or to a folder that contains the root.
  Markers SHALL NOT apply in the root folder itself. So a folder the user adds as a root
  (`~/Library`, `AppData`, a `node_modules`, a Cargo `target`) is indexed, except for name
  rules and markers in its subfolders.
- R7. Matching SHALL cost one map lookup per listed folder plus a scan of the name list per
  subfolder, and no file-system call beyond the listing.
- R8. Each root SHALL record, in its `meta` key `exclusions/<id>`, the list version and the
  setting its last full walk started under. WHEN the setting changes, THE indexer SHALL
  request a full walk of every root (VFS-1 I10: deferred for a root being walked). WHEN the
  indexer starts, every root whose record differs from the current version and setting
  SHALL owe a full walk, never deferred. A walk during which the switch changed keeps the
  setting it started under, so it stays stale. The walk removes newly excluded folders or
  adds newly included ones.
- R9. THE settings panel SHALL show, under a collapsed «Avanzado» section, one switch (off
  by default) and a short text that names the categories with examples and says that adding
  such a folder to the list indexes it. Both catalogs carry the copy (VFS-22).

### Acceptance scenarios

- AC-6 (R5, R7) Matcher unit tests on both platforms' lists:
  - name rules, case-insensitive;
  - home paths, including `~/Library`'s kept children;
  - volume paths;
  - both markers;
  - events dropped under excluded folders and kept elsewhere;
  - Windows paths compared ignoring case (Windows only);
  - the table is well formed and uses every category.
- AC-7 (R6) A root at `~/Library`, `~/Library/Caches`, `AppData` or a `node_modules` folder
  is listed in full. Name rules and markers still apply below it.
- AC-8 (R5) Given a tree with `node_modules`, a `CACHEDIR.TAG` folder, a `pyvenv.cfg`
  project with `lib/` and its own `notas.txt`, and a plain folder, When it is scanned, Then
  only the plain files, the marked folders' own names and `notas.txt` are indexed. When the
  setting is turned on and the root is walked again, everything is indexed. When the setting
  is turned off again and the root is walked, the excluded files are gone.
- AC-9 (R8) Given a root without the current `exclusions/<id>` record, When the indexer
  starts, Then it owes a full walk, and the record is stored when that walk ends. Given a
  walk during which the switch changed, When it ends, Then the root is still stale; the
  next walk clears it.
- AC-10 (R9) UI test: the switch exists, is off by default, sits in a collapsed «Avanzado»
  section, and its copy exists in both catalogs.

### The list and its sources

| Category | OS | Rule | Source |
|---|---|---|---|
| System | Win | volume `Windows`, `Windows.old`, `Program Files`, `Program Files (x86)`, `ProgramData`, `Users/Default`, `Recovery`, `PerfLogs`, `MSOCache`, `$WinREAgent`, `$Windows.~BT`, `$Windows.~WS` | Windows Search default scope (learn.microsoft.com/previous-versions/bb233501); Backblaze Windows exclusions (backblaze.com/computer-backup/docs/configure-exclusions-windows) |
| System | Mac | volume `System`, `Library`, `private`, `usr`, `bin`, `sbin`, `cores`, `Network`; name `.MobileBackups` | Time Machine exclusions as described in eclecticlight.co/2025/05/02/what-isnt-backed-up; Datto macOS default exclusions (continuity.datto.com/help/Content/kb/EB2/EB2-default-exclusions-mac.htm) |
| System | both | names `$RECYCLE.BIN`, `System Volume Information`, `.Trashes`, `.Trash`, `.Spotlight-V100`, `.fseventsd`, `.DocumentRevisions-V100`, `.TemporaryItems`, Synology `@eaDir`, `#recycle`, `#snapshot`, `.SynologyWorkingDirectory` | already excluded since VFS-1 (`walk::EXCLUDED`) |
| App data | Mac | home `Library`, except `Mobile Documents` (iCloud Drive) and `CloudStorage` | Datto (Library/Caches, Containers, Group Containers); Apple File System Programming Guide |
| App data | Win | home `AppData` | Windows Search skips AppData (devblogs.microsoft.com/windows-search-platform/configuration-and-settings) |
| App data | both | home `.cache`, `.local/share/Trash` | XDG Base Directory spec (specifications.freedesktop.org/basedir-spec) |
| Development | both | marker `CACHEDIR.TAG` (Cargo `target`, uv, pytest, mypy, ruff caches) | bford.info/cachedir; doc.rust-lang.org/cargo/guide/build-cache.html |
| Development | both | marker `pyvenv.cfg` → `lib`, `Lib`, `bin`, `Scripts`, `include`, `Include` | PEP 405 (peps.python.org/pep-0405) |
| Development | both | names `node_modules`, `bower_components`, `__pycache__`, `.git`, `.hg`, `.svn`, `.tox` | npm folders (docs.npmjs.com/cli/v10/configuring-npm/folders); PEP 3147; fd and ripgrep skip VCS folders (github.com/sharkdp/fd) |
| Development | both | home `.npm`, `.cargo`, `.rustup`, `.gradle`, `.m2`, `.nuget`, `.conda`, `go/pkg/mod` | each tool's cache docs (npm cache, cargo home, rustup, Gradle directory layout, Maven settings, NuGet global packages, conda envs, go.dev/ref/mod#module-cache) |
| Development | Mac | volume `opt/homebrew` | docs.brew.sh/Installation |
| Programs | both | names `steamapps`, `.dropbox.cache` | help.steampowered.com (library folders); help.dropbox.com/sync/dropbox-cache-folder |

Browser profiles, thumbnail caches, OneDrive and Google Drive internal state, and package
caches under `AppData` or `~/Library` are covered by the app-data rules.

Left out on purpose:
- Generic names anywhere (`target`, `build`, `bin`, `cache`, `tmp`, `temp`, `dist`, `out`,
  `lib`, `env`, `venv`): users name personal folders this way. Markers catch the real cases.
- `VirtualBox VMs` and `Parallels`: few files, and users search for their VMs by name.
- Skipping every dot-folder (fd's default): some apps and NAS tools write dot-folders inside
  user content.
- Checking `CACHEDIR.TAG`'s signature: it would mean opening a file.

Known cost: `~/Library/Mail` (attachments) and `~/Library/Group Containers` (messaging media)
hold personal data, and they are skipped. Backblaze and Datto make the same trade. The opt-in
is the switch, or adding the folder as a root.
