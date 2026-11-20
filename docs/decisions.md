# Decision log

One paragraph per decision: what was decided, why, and the alternative that was rejected.
The numbered product decisions are in the table of
[`specs/vfs-1-core-search/spec.md`](../specs/vfs-1-core-search/spec.md); this log adds the
reasoning. Third-party figures are published by others and not reproduced by us.

## Stack: Rust core, SQLite with FTS5, Tauri 2

The core is a Rust library; the index is one SQLite file in WAL mode with FTS5 trigram
tables over accent-folded names and paths; the app is a Tauri 2 window over a static
Spanish web page (spec decision 6). Rust gives one native core for Windows and macOS with
direct access to the Win32 and macOS APIs the indexer needs (directory change
notifications, volume identity, background I/O priority). SQLite gives a single
transactional file, so the indexer's work queue, the rows and the FTS entries commit
together and a crash loses at most one batch. Tauri's backend is Rust itself, so the app
calls the core directly and the installer stays small. Rejected: Electron, which bundles
Chromium (installers of roughly 100–200 MB in published comparisons) and would need the
core behind a Node native module; and an in-memory name buffer in the style of Everything,
kept as the fallback if FTS5 trigram turns out too slow at several million names.

## Scope: names and paths first

v1 searches file and folder names and every segment of the path, with filters, typo and
accent tolerance (spec decisions 2 and 7). It reads only directory metadata and never opens
a file. Searching inside documents is a later, optional phase
([design](content-search-design.md)); photos, audio and video are found by name only.
Finding a file by name across millions of entries is the user's daily problem, and it can
be solved from directory listings alone, which are cheap and do not trigger antivirus scans.
Rejected: content search in v1, which needs a per-format extraction pipeline, an isolated
worker for hostile files, and hours of disk reading on the target HDD before it is useful.

## Folders are results

A folder whose name matches is its own result row, empty folders included, marked as a
folder, with "mostrar en carpeta" revealing the folder itself; the type filter has a
"carpeta" option (spec decision 13). The user often remembers the folder ("Gestoría 2019")
rather than the file, and an empty or nearly empty folder would otherwise never appear.
A folder matches only when at least one query word is in its own name, so a query does not
list every folder along a matching path. Rejected: files only, with folders reachable just
as path matches of the files inside them.

## No administrator rights

VFS walks directories with ordinary user rights, installs per user, and runs no service.
The NTFS master file table and the USN change journal give much faster scans and
incremental updates, but Microsoft documents that change journal operations require
administrator privileges, they work only on NTFS, and a raw-volume reader in an unsigned
binary is the profile antivirus heuristics flag. One directory-walk design works on every
local filesystem, and the user never sees an elevation prompt. Rejected: an elevated
helper or service reading the MFT and USN journal (Everything's approach), kept as an
option if measured re-walk times on the real PC are unacceptable.

## Own watcher loop on Windows

On macOS the watcher uses the `notify` crate (FSEvents) and honours its rescan flag. On
Windows VFS has its own `ReadDirectoryChangesW` loop, one thread per root, with a 64 KB
buffer (`crates/vfs-core/src/indexer/watch.rs`, spec I2). Owning the loop makes the
failure behaviour explicit: a buffer overflow (0 bytes or `ERROR_NOTIFY_ENUM_DIR`) queues a
full re-walk of exactly that root and re-arms on the same handle; any other failure queues a
re-walk, closes the handle and reopens it every 2 s; the directory is opened with the
long-path prefix. With `notify`, overflow and reopen handling on Windows would be whatever
its backend does, which we had not audited. Rejected: `notify` on both systems. Caveat: this
loop has never run.

## Search deadline

Each search has a soft deadline (`SearchConfig::deadline`, default 1 s) for filtered scans,
the typo pass and generator terms with more than 5,000 matches, and a hard bound of 2 s for
everything else, so a selective query always runs to completion even on a cold cache
(spec C4, S3). A cut-short search returns what it found with `truncatedBy = "deadline"`; the
UI says the list may be incomplete, or, with no hits, that the search did not finish, never
"Sin resultados". The cut list is not cached, so trying again searches again (U1). A new keystroke interrupts the running query.
On a slow HDD some queries cannot finish in time, and a partial answer that says so is better
than a frozen box or a false "nothing found". Rejected: one hard deadline for all work,
which would cut cheap selective queries on a cold disk and report them as partial; and no
deadline, which lets a broad query block the next keystroke.

## Session marker instead of an integrity check at every start

At open, the core creates and locks a marker file in `index.db.session/`; a clean shutdown
removes it. `PRAGMA quick_check` runs only when a previous marker is found unlocked, that is,
after a crash or a killed process. A failed check renames the index to `index.db.bad` and
rebuilds it from the root paths it can still read (spec C5). `quick_check` reads the whole
database; at several million files that is hundreds of megabytes read from the system drive
at every launch of an app that should open instantly. The marker is a file lock, so the app
and the CLI can share one index without mistaking each other for a crash. Rejected: running
`quick_check` at every start; and never checking, which would leave a corrupt index after a
power cut in place.

## Unsigned installer with the WebView2 bootstrapper

The Windows build is an unsigned NSIS installer, per user, in Spanish, with the WebView2
bootstrapper embedded and run silently (spec decisions 8 and 14, `tauri.conf.json`). There
is one known user, so the SmartScreen warning on first run is accepted; the target PC has
internet at install time, so the small bootstrapper can fetch WebView2 if it is missing.
Rejected: a code-signing certificate (published prices of about $219–400 a year for an
organisation-validated certificate, and Microsoft's cheaper signing service is not open to
individual developers outside the US and Canada), tracked for later in Fase 4; and the
offline WebView2 installer (about 150 MB per Microsoft's documentation), which only matters
without internet. Caveat: the installer has never been built or installed.

## GitHub Issues for tracking

Work items are GitHub issues in the private repository, grouped by milestone per phase
(Fase 0–4). Linear was the first choice, but private teams there need the Business plan,
which is not worth paying for a one-person project; GitHub Issues come with the repository
at no cost and link directly to branches and pull requests. Specs in `specs/` remain the
durable record of what a change must do; issues track the work. Rejected: Linear on its
Business plan; and no tracker at all, which was the starting point (spec decision 10) but
left no place for work that is not yet specified, such as the Fase 2 measurements.
