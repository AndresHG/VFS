# Decision log

One paragraph per decision: what was decided, why, and the alternative that was rejected.
The numbered product decisions are in the table of
[`specs/vfs-1-core-search/spec.md`](../specs/vfs-1-core-search/spec.md); this log adds the
reasoning. Third-party figures are published by others and not reproduced by us;
measured figures come from our own runs on the development Mac (Apple Silicon, SSD), never
from Windows or an HDD.

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
a file. Searching inside documents is a later, optional step, milestone Fase 3
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

Each search has a soft deadline (`SearchConfig::deadline`, default 1 s) for filtered scans
and the split-term typo pass, and a hard bound of 2 s for the FTS-driven tiers, so an
indexed query always runs to completion even on a cold cache (spec C4, S3). A cut-short
search returns what it found with `truncatedBy = "deadline"`; the
UI says the list may be incomplete, or, with no hits, that the search did not finish, never
"Sin resultados". The cut list is not cached, so trying again searches again (U1). A new keystroke interrupts the running query.
On a slow HDD some queries cannot finish in time, and a partial answer that says so is better
than a frozen box or a false "nothing found". Rejected: one hard deadline for all work,
which would cut cheap selective queries on a cold disk and report them as partial; and no
deadline, which lets a broad query block the next keystroke.

## Retrieval in ranking order, with a per-file path index

A search reads candidates tier by tier in the order the ranking will sort them: names
starting with the first word, names holding every word, then files whose folder path and
name together hold every word, then typos (spec S2). Each tier is one FTS5 intersection with
the filters as SQL predicates, and the name tiers read up to 4,000 candidates and rank them
before the cut to 2,000, so a full list never pushes out a better match. The path tier needs
each file's folded path in one trigram index, `file_paths_fts` (schema version 2), stored
without positions (`detail=none`). That costs 311 MB on the 2 M-file synthetic index (577 MB
→ 889 MB, measured); the same table with positions was put at about 1 GB during the design
(not re-measured), and candidates are verified anyway, so adjacency is not needed. Rejected:
choosing one "generator" word and reading every file and folder that contains it, the
previous design, whose hits arrived in rowid order, so a full list could be filled before a
better-ranked match was read.

## Schema migrations instead of a rebuild

`PRAGMA user_version` numbers the schema, and `db::open_rw` applies each missing migration
in order in one transaction; version 2 fills the path index from the rows already in the
index. A user with a version-1 index keeps searching after an update without rescanning a
multi-TB archive from an HDD. Rejected: discarding an older index and scanning again, which
is simple but costs hours on the target PC. An index from a newer build is refused, never
overwritten.

## A smaller index that reads fewer pages, same results (VFS-4)

On the target PC a cold query pays about one disk seek per page read, and 83% of those pages
were `files` and `dirs` rows, not FTS. Schema version 3 and its search code cut the 2 M-file
index from 889 to 722 MiB and the pages a query reads by 55% at the median and 75% at p95,
with every page of the AC-9 query set identical (`docs/research/index-footprint.md`): 16 KB
pages, folder rows read only when a check or the returned page needs them, `files_fts`
without positions (a U+0001 start marker and an exact re-check replace phrases and `^`), no
per-row size tables (rows deleted by value), and a one-time `VACUUM` after migrating.
Rejected or left open: dropping `UNIQUE(dir_id, name)`, because it changes which of more
than 2,000 equally ranked path hits are returned (an owner decision); an in-memory name
buffer, about 150 MB at 5 M names on a memory-starved PC; a single per-file index with
columns, 818 MiB against 479.

## A no-hit query proves it cheaply

Before the typo tiers, a query that has found nothing probes the path index once for a row
holding, for every word, the word or one of its one-edit alternatives; if there is none, no
tier can produce a hit and the search ends (spec S8). Without it, a query with no answer
was the slowest of all, because every tier ran to exhaustion: 146 ms warm p95 for the
`no_hit` class in one run before the change, against 29–59 ms in three runs with the probe
(measured). Rejected: a lower read cap on the typo pass, which would also drop real typo
matches.

## Typos only in words of 6 or more letters

One typo (insertion, deletion, substitution or adjacent transposition) is tolerated only in
words of at least 6 characters (`FUZZY_MIN_LEN`, spec S4). The trigram index can only find
candidates through a part of the word of 3 or more characters that the edit left intact;
from 6 characters one side of every edit position is that long, so every one-edit spelling
has an indexed query (`fuzzy::candidate_queries`). In shorter words some edits leave no
such part, and one edit is a large share of the word: "boda" is one edit from "bota",
"toda", "moda" and "bodas", which would bury the exact results. Rejected: tolerance for
every word of 3 or more characters.

## A catch-up re-walk at every start

Every start owes each folder a full re-walk, resumable, at low priority, searchable
meanwhile (spec I4). While the app is closed nothing watches the disk, and the Synology
client keeps syncing; without a change journal (which needs administrator rights) only a
walk finds what changed. Rejected: re-walking only when the last walk is older than a day,
the previous rule, which missed changes made the same day while the app was closed.

## One full re-walk at a time, with a cooldown for lost events

Full re-walks run one at a time and a walk in progress is never restarted; a request for a
folder being walked owes one more pass after it. Requests caused by lost events (watcher
overflow, a watch that failed) wait until the folder's last full walk is an hour old (spec
I10). A sync client can produce event storms that overflow the watcher buffer repeatedly;
restarting or chaining full walks would keep the HDD busy all day and never finish one.
Rejected: restarting the walk on every overflow, and walking several folders in parallel on
one disk.

## Failed listings are retried, not marked clean

A directory that cannot be listed keeps its rows, stays pending and is retried after 1
minute, doubling, until 6 failures; then it waits for the next full walk (spec I11). A
folder briefly locked by the sync client or the antivirus would otherwise be marked current
with stale contents until the next weekly walk. Rejected: marking it clean with the error
counted, the previous behaviour.

## One exclusion list for walker and watcher

`walk::is_excluded` (system folders, Synology metadata such as `@eaDir` and `#recycle`,
Office and LibreOffice lock files, temporary names) is the only list, used by the walker
and by the watcher, which drops events under an excluded component before marking
anything (spec I8). Two lists drift apart, and a watcher that reacts to files the walker
ignores re-lists folders for nothing during sync churn. Rejected: separate lists per
component.

## Directory rename as a path update

When the Windows watcher reports a folder's old and new name as a pair, the index rewrites
the paths of the folder's subtree in one transaction (folder rows, folder search entries and
the files' path index rows) and keeps every file row (spec I9). A renamed top-level folder
of a large archive would otherwise delete and re-insert all its files and walk them again.
Anything not provably a rename inside one root falls back to re-listing both parents.
Rejected: always treating a rename as delete plus insert. Caveat: the Windows watcher has
never run.

## The index stays on this computer

The app keeps the index in the local app data folder (`%LOCALAPPDATA%` on Windows, Tauri
`app_local_data_dir`), not the roaming one, and moves an index an earlier build left in the
roaming folder once at start (spec C6). The target PC syncs folders to a NAS; a roaming
profile folder can be synced or backed up, and a multi-GB file that changes constantly must
not travel with it. On macOS both folders are the same. Rejected: the roaming folder, the
first build's choice.

## Bookmarks are read from a copy of Firefox's database

Firefox bookmarks (VFS-6) are read from a copy of `places.sqlite` and its WAL made in the
app's data folder, checked for consistency and deleted after reading; the daily
`bookmarkbackups` file is the fallback. On macOS Firefox holds the database with an
exclusive lock, so a normal read-only open fails; measured with Firefox 160, an
`immutable=1` open saw 2 of 11 bookmarks because the rest were still in the WAL, while a
copy of both files saw all 11. On Windows a read-only open would join Firefox's locking.
The copy takes no lock and writes nothing in the profile. Rejected: opening in place
(locks, missed or torn data) and backups only (up to a day old). Bookmarks live in their
own `bookmarks.db`, searched in memory: 3,000 rows need no full-text index, and the file
index schema stays untouched.

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
Business plan; and no tracker at all, which was the starting point but left no place for
work that is not yet specified, such as the Fase 2 measurements. Spec decision 10 records
the choice.
