# VFS architecture

This document describes the code as merged for VFS-1: how a folder on
disk becomes rows in the index, and how a query becomes a ranked list of results. File
references are relative to the repository root; `core/` means `crates/vfs-core/src/`.
Requirement and decision ids (`AC-n`, `S2`, `I4`, …) are those of
[`specs/vfs-1-core-search/spec.md`](../specs/vfs-1-core-search/spec.md).

## Components

```
┌──────────────── Tauri 2 app (one process) ─────────────────┐
│ apps/desktop/ui        static es/en screen (JS, no build)   │
│        │  invoke(command) / listen(event)                   │
│ apps/desktop/src-tauri commands.rs, reveal.rs, lib.rs,      │
│        │               bookmarks.rs (VFS-6), splash.rs      │
│ crates/vfs-bookmarks   Firefox reader, bookmarks.db, merge  │
│        │                                                    │
│ crates/vfs-core        Vfs facade (core/lib.rs)             │
│   ├─ roots.rs          add / remove / list folders          │
│   ├─ search.rs         candidates, verification, paging     │
│   ├─ matching.rs       query parsing, typo rule, tiers      │
│   │    ├─ fuzzy.rs     one-typo candidates and check        │
│   │    └─ rank.rs      ordering                             │
│   ├─ indexer/          thread "vfs-indexer"                 │
│   │    ├─ mod.rs       scheduler, scan loop, resume         │
│   │    ├─ walk.rs      list one directory                   │
│   │    ├─ apply.rs     diff a listing against the index     │
│   │    ├─ cursor.rs    change cursor: USN / FSEvents replay │
│   │    └─ watch.rs     FSEvents (notify) / own Windows loop │
│   ├─ platform.rs       volume key, drive kind, priority     │
│   ├─ db.rs             connections, schema, ingest, session │
│   ├─ link.rs           .lnk / .url / symlink target reader  │
│   └─ fold.rs, kind.rs  search keys, extension → type        │
└──────────────────────────────┬─────────────────────────────┘
                               │ SQLite, WAL mode
              <local app data dir>/index.db
```

`crates/vfs-cli` uses the same `Vfs` facade, with its own index file. Firefox bookmarks
(VFS-6) live in `bookmarks.db` next to `index.db`; see "Bookmarks" below.

The app is one process. The core opens these SQLite connections to the same file
(`core/lib.rs`, `core/db.rs`):

| Connection | Opened by | Page cache | Use |
|---|---|---|---|
| writer | indexer thread (`IndexerHandle::spawn`) | 32 MiB | the scan loop and watcher hints |
| short-lived writer | `add_root`, `remove_root`, `mark_dirty` | 32 MiB | root changes from commands |
| reader | `Vfs::open_with` | 64 MiB | roots, stats, paths for "mostrar en carpeta" |
| search reader | `Vfs::open_with` | 64 MiB | queries only; interrupted by each new query |

All connections set `journal_mode=WAL`, `synchronous=NORMAL` and `mmap_size=0`, so resident
memory is bounded by the page caches (AC-9 budget: under 300 MB; the 2 M-file benchmark
peaks at about 138 MB at schema version 3, 123 MB at version 2, measured). Readers set
`query_only=1`. A writer waits up to 30 s for the other writer; readers wait up to 5 s.

## Schema

`core/schema.sql` is schema version 1, `core/schema_v2.sql` version 2,
`core/schema_v3.sql` version 3, `core/schema_v4.sql` version 4 and `core/schema_v5.sql`
version 5 (`PRAGMA user_version`). `db::open_rw` runs the missing
migrations (`MIGRATIONS` in `core/db.rs`) in order, in one transaction: a new file gets all
of them, and an older index is migrated from the rows already there, so no rescan is
needed. After migrating an existing index it compacts the file once (`VACUUM`, at
`PAGE_SIZE`, 16 KB; a new index is created at that size); if another connection holds the
index, it stays valid and uncompacted. An index from a newer build fails with
`schema_too_new` and is never discarded.

| Table | Holds |
|---|---|
| `meta` | key/value pairs: each root's change cursor, `cursor/<root id>` (VFS-9) |
| `volumes` | one row per volume key (Windows: volume GUID path; macOS: mount point) |
| `roots` | the folders the user chose: `path` (absolute, native separators), `path_key`, `status` (0 pending, 1 scanning, 2 ready, 3 unavailable, 4 removing), `last_full_scan_at`, counts |
| `dirs` | every directory once, empty ones included: `rel_path` (original names, `/`-separated, `''` for the root), `path_key`, `name_key`, `mtime`, `scan_state` |
| `files` | one row per file: `dir_id`, original `name`, `name_key`, `kind` (0 other, 1 document, 2 photo, 3 video, 4 audio), `size`, `mtime`, `flags` (bit 0: name was not valid Unicode; bit 1: a symbolic link or junction, VFS-18), `content_state` (document content search, milestone Fase 3; always 0) |
| `files_fts` | version 3: FTS5 trigram index over U+0001 + `files.name_key`, `detail=none` (rowids only), `rowid = files.id`; the marker's trigrams find names starting with a term |
| `dirs_fts` | FTS5 trigram index over `dirs.path_key` and `dirs.name_key`, `rowid = dirs.id` |
| `file_paths_fts` | version 2: FTS5 trigram index over each file's folded relative path and name (`<dir path_key>/<name_key>`, or `name_key` alone in the root directory), `detail=none` (rowids only, no positions), `rowid = files.id` |
| `links` | version 5 (VFS-18): one row per shortcut (`.lnk`), Internet Shortcut (`.url`) or symbolic link / junction: `file_id`, `kind` (0 shortcut, 1 internet, 2 symlink), `state` (0 pending, 1 read, 2 unreadable), `target` (path or address as recorded in the link), `target_key` (folded target). No FTS: searches scan it (`search::link_pass`) |

Design points:

- **The tables store a folder path once.** Paths are rebuilt from `roots.path` +
  `dirs.rel_path` + `files.name` (`db::file_abs_path`, `db::dir_abs_path`).
- **The per-file path index repeats it, and pays for it.** `file_paths_fts` holds every
  file's folded path and name, so a query whose words are split between the folders and
  the name ("gestoria 2019 escritura") is one FTS intersection instead of a walk over every
  matching folder. Without positions it adds 311 MB to the 2 M-file synthetic index (577 MB
  → 889 MB, measured); position-less trigrams cannot check adjacency, so its candidates are
  verified like all others. A directory rename (I9) rewrites the rows of the files below it.
- **All FTS tables are contentless** (`content=''`) with `tokenize='trigram case_sensitive
  1'`. Keys are folded before they are stored, so the tokenizer does no case or accent work
  (its `remove_diacritics` does no NFKD and leaves the combining marks of NFD names). There
  are no triggers: every FTS row is written by `db::ingest` in the same transaction as its
  base row. `dirs_fts` keeps positions and `contentless_delete=1`; `files_fts` and
  `file_paths_fts` keep neither positions nor per-row sizes (`columnsize=0`, schema version
  3), so their rows are deleted with FTS5's `'delete'` command and the indexed value
  (`ingest::delete_files_fts`), before the base rows change.
- **File hits are read without their folder row when nothing needs it** (VFS-4): the name
  tiers read `files` alone when every root is searched and none is being removed and there
  is no folder filter; the folder row is read to check a word missing from the name, and to
  fill `dir` only for the page returned (`search::resolve_dirs`). On a cold disk each folder
  row is about one seek.
- **No foreign keys.** The indexer deletes subtrees in its own order
  (`ingest::delete_dir_subtree`, `ingest::delete_root`).
- Indexes: `dirs_pending` (partial, `scan_state <> 0`) is the indexer's work queue;
  `dirs_parent` serves child listing; `UNIQUE(root_id, rel_path)`; `files_dir` (version 4,
  VFS-7) serves a folder's files. Files have no `(dir_id, name)` uniqueness constraint: the
  indexer inserts only names its listing of the folder lacks.

`file_kind` (`core/kind.rs`) maps the extension to the type filter. Folders have no stored
kind; they appear as `FileKind::Folder` (code 5) only in results.

## Folding

`fold_key` (`core/fold.rs`) builds every search key: names, directory paths, root paths and
query terms all go through it.

1. Unicode lowercase.
2. NFKD decomposition.
3. Drop combining marks.
4. Lowercase again (a compatibility form such as `℡` → `TEL` can yield uppercase).

ASCII input takes a byte-lowercasing shortcut. So `Notaría`, `NOTARIA` and the NFD form
`Notari\u{301}a` all become `notaria`; `año` becomes `ano`; full-width `ＡＢＣ` becomes `abc`.
Root and query paths have `\` replaced by `/` before folding, so separators match on both
systems. Original names are stored untouched and are the only thing used to build a path.

## From a folder on disk to rows

### Adding and removing a root

`roots::add_root` (`core/roots.rs`) canonicalises the path (`dunce`), rejects a missing path
(`not_found`), a file (`not_a_directory`) and a path that is the same as, inside, or contains
an existing root (`overlapping_root`, message `"<same|inside|contains> <root path>"`).
Comparison uses folded keys. It then inserts the root with its volume key
(`platform::volume_key`) and a root directory row pending deep, in one transaction, and the
facade wakes the indexer.

`roots::remove_root` only sets `status = removing`; search and the watcher ignore the root
from that moment. The indexer deletes its rows in batches (`finish_removing` in
`core/indexer/mod.rs`), also on the next start if the app closed half-way (I6). Adding the
same folder while it is still being removed deletes the old rows at once and adds it anew.

### Listing a directory

`walk::list_dir` (`core/indexer/walk.rs`) reads one directory with `std::fs::read_dir` and
never opens a file. On Windows type and metadata come from the enumeration record; on Unix
from `lstat`. Symlinks, junctions and other name-surrogate reparse points
(`FileType::is_symlink`) are listed as file rows with `flags` bit 1 and a pending `links` row,
never entered, so a walk never loops (VFS-18 R1). Before a directory is listed again, the
root and every folder on its path are checked with `symlink_metadata` (`walk::behind_link`):
one that has been replaced by a link since it was indexed is treated as gone, never
followed, and a root that has become a link is unavailable. It skips:

- excluded names (`walk::is_excluded`, ASCII case-insensitive): `$RECYCLE.BIN`, `System
  Volume Information`, `.Trashes`, `.Trash`, `.Spotlight-V100`, `.fseventsd`,
  `.DocumentRevisions-V100`, `.TemporaryItems`, `.DS_Store`, `Thumbs.db`, `desktop.ini`, the
  Synology names `.SynologyWorkingDirectory`, `@eaDir`, `#recycle` and `#snapshot`, and
  temporary names (`._*`, `~$*`, `.~lock.*#`, `~*.tmp`). The watcher uses the same function
  (I8);
- the app's own data directory;
- entries that vanish, or fail with an error other than permission denied, while being
  listed. An entry whose metadata is denied fails the whole listing, so the folder is
  retried like one that cannot be listed (VFS-3, #56).

A name that is not valid Unicode is stored in its lossy form with `flags` bit 0 set.

### Shortcuts and links (VFS-18)

A file named `*.lnk` or `*.url`, or a listed symlink or junction, gets a pending `links` row
when inserted; a changed one goes back to pending. Once the walks are done and before idle
is reported, the indexer's deferred pass (`link::read_pending`, 200 per transaction, yields
to stop, pause and wake) reads each pending link: at most 64 KiB of the link file itself
(the bounded `.lnk` / `.url` parser in `core/link.rs`), or the link's own reparse data
(`read_link`; a relative target is resolved as text against the link's folder). The target
is never opened. A link that cannot be read is marked unreadable and queued again when its
root's next full walk ends. A search matches links by their own folder, name and target
together, attaches `link` (kind, target, `openable`) to every link hit, and drops a link
whose target is an indexed item the same search returns (`search::attach_links`).

### Scan states: the persistent work queue

`dirs.scan_state` is the indexer's queue, stored in the index itself:

| Value | Meaning | Set by |
|---|---|---|
| 0 clean | listed and up to date | `apply_listing` |
| 1 pending deep | list it, and mark every child directory pending deep too | new directories; a re-walk (on the root directory) |
| 2 pending shallow | list it, children stay as they are | a watcher hint |

`scan_root` (`core/indexer/mod.rs`) repeatedly takes the pending directory with the highest
id in one root (so the walk goes depth-first along the newest directories), lists it and
calls `apply::apply_listing` (`core/indexer/apply.rs`), which:

- matches children by exact name against `ingest::list_children`;
- inserts new files and directories (new directories are pending deep);
- updates a file whose size or mtime changed;
- deletes vanished files and vanished directory subtrees;
- on a deep listing, marks existing child directories pending deep;
- sets the listed directory clean.

Seen by a listing, a rename or move is a delete plus an insert, and a moved folder is
walked again at its new place; the Windows watcher's rename pairs avoid that (see below).
If a directory cannot be listed because of a permission or I/O error, what is indexed
stays, the error is counted, and the directory stays pending: it is retried after 1
minute, then after doubling delays, and after 6 failures it waits for the next full walk
(I11). If it is gone, its subtree is deleted — unless the root itself is unreachable, in
which case the root becomes `unavailable` and nothing is deleted.

When no directory of a root is pending, `finalize_root` recounts files and folders, marks
the root `ready`, and after a full scan stamps `last_full_scan_at` and asks the FTS tables
for a segment merge.

### Resume after interruption

Writes happen in transactions that commit between directories every 20,000 changed rows or
5 s (I5). Because the queue lives in the same transaction as the rows, a run killed at any
point redoes only the directories of its last uncommitted transaction. On start
(`Worker::init`) the indexer finishes roots left in `removing`, recounts the roots, starts
the watcher, then replays each root's change cursor (`indexer/cursor.rs`, VFS-9): the
folders that changed since it become pending shallow, stored with the new cursor in one
transaction. A root with no cursor, or whose history is lost, owes a catch-up re-walk
instead (I4). Then it continues with whatever is pending (AC-6). The catch-up is resumable,
runs at low priority, and the user can search meanwhile; the status bar says "Comprobando
cambios…" while it runs (U6). A full walk stores the cursor taken just before it starts;
while running, a watched root's cursor advances every 5 min to the position taken at the
previous check, unless the watcher lost events for it meanwhile.

### The indexer thread

`IndexerHandle::spawn` starts the thread `vfs-indexer` in the *walk profile*
(`platform::OsThreadPriority`, VFS-5): normal disk I/O, lowest CPU priority and low memory
priority (Windows `THREAD_PRIORITY_LOWEST` + `MEMORY_PRIORITY_LOW`; macOS utility QoS).
Walks and large batches run in that profile and pause while the roots' disk is busy
(`indexer/sched.rs`: PDH average queue length on Windows, a forced 20 % duty floor). Small
incremental work lists each directory in background mode (Windows
`THREAD_MODE_BACKGROUND_BEGIN`, macOS throttled disk I/O) and commits it alone after leaving
it, so the write lock is never held at very-low I/O priority. The start-up catch-up waits
until the system has been up 10 minutes. The thread:

- takes commands from a channel: roots changed, rescan, pause, watcher events;
- runs the scan loop one transaction batch at a time, yielding to any new command;
- publishes progress (`IndexProgress`) at most every 500 ms and `index-changed` at most every
  2 s after a commit;
- when idle, waits up to 60 s for a command;
- every hour, requests a re-walk of roots whose last full scan is older than 7 days or
  stored in the future (a clock set back) (I4);
- every minute, retries watching roots that could not be watched (I12).

There is one I/O stream: roots are scanned one after another, one directory at a time.
Full re-walks run one at a time (`request_rewalk`, `start_owed`), and a walk in progress is
never restarted: a request for the root being walked owes one more pass after it. Requests
caused by lost events (an overflow, a watch that failed), and those owed by a root that was
being walked, wait until that root's last full scan is 1 hour old; manual requests do not
wait, and the start-up catch-up never waits, even for a root whose walk was interrupted: a
power cut can roll the index back to an older walk (VFS-3 AC-5). An owed walk that is due
starts before the indexer reports idle, so the status never reads up to date while one is
owed (I10). So a burst of sync-client churn cannot chain full walks back to back.

### Watcher and re-walk

`core/indexer/watch.rs` turns filesystem events into hints; the scan loop does the actual
work. A `Dirty(path)` event is coalesced for 2 s, then `mark_dirty` sets the path's parent
directory (and the path itself, if it is an indexed directory) pending shallow. If the
parent is not indexed yet, its nearest indexed ancestor is marked. A clean directory is the
only one that changes state, so a hint never downgrades a directory pending deep (I1).
Events under the app data directory are dropped (I3), and so are events for any path with
an excluded component below the root (`walk::is_excluded`, the walker's list, I8), before
anything is marked.

On Windows an old-name record directly followed by its new-name record becomes a
`Renamed` event. When the old directory is indexed and gone from disk, the new one exists in
the same root, is not indexed yet, and its parent is indexed, `indexer::rename_dir` applies
it as a path update in one transaction: the subtree's `dirs` rows get their new `rel_path`
and keys, and their `dirs_fts` and `file_paths_fts` rows are rewritten; file rows and
directory ids are kept. Anything else falls back to re-listing both parents (I9).

| System | Backend | When events may be lost |
|---|---|---|
| macOS | `notify` (FSEvents), recursive per root, canonical paths | `need_rescan` flag or a watcher error → re-walk of the affected root (or every root when the path is unknown) |
| Windows | own `ReadDirectoryChangesW` loop, one thread per root (`vfs-watch-<id>`), 64 KB buffer, overlapped I/O | 0 bytes or `ERROR_NOTIFY_ENUM_DIR` (overflow) → re-walk, re-arm on the same handle; any other failure → re-walk, close the handle, reopen every 2 s, re-walk again once reopened (I2) |

A re-walk (`queue_rewalk`) sets the root directory pending deep and the root `scanning`;
the deep listing then cascades through the whole tree. Re-walks also happen when the user
asks (`rescan`, "Buscar cambios ahora"), when an unavailable root becomes reachable, once a
root that could not be watched is watched (I12), at start for a root whose change cursor
is missing or whose history is lost (VFS-9), and while running after 7 days. If the watcher
cannot start, indexing still runs and these re-walks still catch changes. The Windows loop
has run only on a GitHub runner (VFS-8), where it followed create, rename and delete within
about 2 s.

### Session marker and recovery

`db::open_session` starts a session by creating and locking
`<index>.session/<pid>-<n>.lock`. A clean shutdown (`Vfs::shutdown`, or dropping the last
`Vfs` clone) unlocks and removes it. On the next open, a marker that no live process holds
locked means the previous session ended uncleanly; only then does the core run
`PRAGMA quick_check(1)`. An index that SQLite reports as corrupt or not a database, or that
fails the check, is replaced: a fresh index holding every root path still readable from it
(pending deep, C5) is built at `index.db.new`, the damaged one is renamed to `index.db.bad`
(with its `-wal` and `-shm`), and `index.db.new` takes its place. A kill between the last two
steps is finished by the next open, so the root list survives a crash during recovery
(VFS-3 AC-3). Once every root is ready with a finished full scan, the indexer deletes the
`.bad` file and its companions (`discard_recovered_copy`). Any other open error (busy,
locked, I/O) is returned, never recovered. Several processes (the app and the CLI) can share
one index; each holds its own marker, and the migration takes the write lock before reading
the schema version (VFS-3 AC-9).

## From a query to results

`search::search` (`core/search.rs`) runs on the dedicated search connection. The facade
calls `InterruptHandle::interrupt` on it before every query, so a newer query stops an older
one (S6).

### Parsing

At most 1 KB of the text is read (`MAX_QUERY_BYTES`; the search box allows no more); it has
`\` replaced by `/`, is folded, and is split on whitespace into at most 8 distinct terms,
each cut to 128 characters (`MAX_TERM_CHARS`). Terms over 64 characters get no typo work
(`FUZZY_MAX_LEN`), whose cost grows with the cube of the length (S9). With no terms and no
filter, the response is empty.

**Matching rule.** A file matches when every term is a substring of its `name_key` or of its
directory's `path_key`. A folder matches when every term is in its own `path_key` (for a
root directory: its name) and at least one term is in its name (D1). For each root, terms
already contained in the root's own folded path are ignored, unless none would remain (S1):
in a root called `Fotos`, the query "fotos boda" finds what contains "boda" instead of
nothing (directory `path_key`s are relative to the root, so they never contain its name).

### Filters

Filters become SQL predicates (`file_clause`, `dir_clause`):

- type: `f.kind IN (…)`; `folder` selects directory hits; any other type alone excludes them;
- date: `mtime >= from` and `mtime < to` (folders use their own mtime);
- size: inclusive bounds on files; a size filter excludes folders;
- folder: resolved to a root and a directory (`resolve_scopes`, folded comparison) and
  applied as a `rel_path` range: descendants of `p` are exactly `[p + '/', p + '0')`.

### Candidate generation: tiers in ranking order

Roots with the same effective terms form a group. Candidates are read in the order of the
ranking tiers, so a full list never pushes out a better tier (S2, `run_query`):

1. **Name starts with the first term**, all terms in the name: one `files_fts`
   intersection of covering trigrams, the first term's taken after the U+0001 start marker
   every name is stored behind (`files_fts` keeps no positions, schema version 3), narrowed
   in Rust to exactly the names the phrases would match (`NameCond::holds`); on folder names,
   the phrases with the first term anchored (`^"term"`) in `dirs_fts` (`name_tier`).
2. **All terms in the name**: the same without the anchor. Tiers 1 and 2 read
   up to 4,000 candidates, all ranked before the cut to 2,000 hits, so a better name match
   is never lost to rowid order. Date and size filters have no index, so with them these
   tiers stop at 2,000. Under an order other than relevance every tier reads up to 6,000
   candidates instead (VFS-12, below).
3. **Path**: files whose relative path and name together hold every term, as one
   intersection in `file_paths_fts` (`exact_group`). Because that table has no positions,
   the expression is the trigrams that cover each term without overlap, ANDed; candidates
   are verified. A single term instead reads the folders whose path holds it (`dirs_fts`,
   at most 20,000) and their files, which is exact. Folders whose path holds every term and
   whose own name holds one come from `dirs_fts`.
4. **Typos** (below).

Filters are SQL predicates of every stage. With a folder filter on one folder below a root,
the trigrams of that folder's path join the `file_paths_fts` intersection, so only rows of
that subtree are read. Every candidate is verified in Rust against all terms
(`verify_file`, `verify_dir`).

Terms shorter than 3 characters cannot use the trigram index (S5). They are checked during
verification; when a group has only short terms, or the query has filters and no terms, the
group is a filtered scan with `instr` in SQL (`scan_group`).

At most 2,000 hits are kept; more sets `truncatedBy = "cap"`.

### Sort orders (VFS-12)

`SearchQuery.sort` is relevance (the default, the tiers above), newest, oldest, name or size.
Any order but relevance reads up to 6,000 verified candidates (`SORT_READ_CAP`, 3× the
cap: 20,000 broke the AC-9 budget on 2 M files), ranks them, re-sorts them stably by the order
(`rank::sort_by_order`: an unknown date last in both date orders, folders last by size,
ties in relevance order) and only then cuts to 2,000, so the newest file is first even when
2,000 better-ranked files exist. A filter-only query under relevance still reads up to the
2,000-hit cap (newest first among those). When a cap or the deadline cut the read of a sorted
or filter-only query, the response sets `approximateOrder` and the screen says so.
`search_sources` passes the order to the core and merges bookmarks into the sorted file list
(dates saved, titles; no size, so after files).

### Deadline

Each query has two deadlines, checked every 1,000 SQLite VM steps and between rows:

- **soft**, `SearchConfig::deadline` (default 1,000 ms): applies to filtered scans and the
  split-term typo pass;
- **hard**, the larger of the soft deadline and 2 s: applies to the FTS-driven tiers, which
  run to completion (S3).

An expired deadline or an interruption returns what was found with `truncatedBy =
"deadline"`. Such a list is not cached (U1).

### Typo tolerance

Only terms of 6 to 64 characters tolerate a typo (`FUZZY_MIN_LEN`, `FUZZY_MAX_LEN`); others
match exactly. The upper bound keeps a pasted path or URL from generating typo alternatives
that grow faster than quadratically outside the deadline (VFS-3, #53). Two typo stages
follow the exact tiers:

- **Near spellings**, when the exact tiers found fewer than 20 hits: the deletions and
  adjacent transpositions of a term (`fuzzy::near_variants`), ORed, with the other terms
  exact, in the name; first anchored at the start of the name for the first term, then
  anywhere (S4).
- **Split-term candidates**, only when nothing has matched at all: for each edit position
  the parts of the term before and after it, joined with `AND`, parts shorter than 3
  characters dropped (`fuzzy::candidate_queries`), so every string within one insertion,
  deletion, substitution or transposition contains one of them. This pass keeps at most
  5,000 candidates in all; on `files_fts` (no positions) it reads every name holding the
  parts' trigrams and counts only those holding the parts themselves.

Both verify with `fuzzy::contains_within_one` (approximate substring search, optimal string
alignment distance 1); their hits are `matched = "fuzzy"`.

Before the typo stages, a query with no hit at all probes `file_paths_fts` and `dirs_fts`
once for a row holding, for every term, the term or one of its one-edit alternatives
(`may_match`). Every hit of every tier holds those, so when the probe finds nothing the
typo stages are skipped (S8). On the 2 M-file synthetic index this probe takes a few
milliseconds; the `no_hit` class's warm p95 went from 146 ms (one run before the change)
to 29–59 ms (three runs after), measured.

### Ranking

`rank::rank` (`core/rank.rs`) sorts by tier, then shorter name, newer modification time,
lower id, and files before folders:

| Tier | Rule |
|---|---|
| 0 | the name, or the name without its extension, equals the whole query |
| 1 | every term is in the name and the name starts with the first term |
| 2 | every term is in the name |
| 3 | some term matched only in the path |
| 4 | typo, and the name starts within one edit of the first term |
| 5 | other typos |

A query with filters and no terms lists newest first (S6).

### Copies of a file (VFS-19)

After ranking, `search::group_copies` folds every file into the first (best-ranked) file
with the same folded name and the same size: it becomes one result whose `copies` hold the
others (id, folder, modification date) in ranking order. Folders are never grouped, nor
files whose size differs, so `IMG_0001.jpg` from two cameras stays two results. The final
cut to 2,000 counts results; the read caps of the tiers still count files, so a list full of
copies can hold fewer than 2,000 results with `truncatedBy = cap`. A copy's folder is read like a hit's: only for the page
returned. The UI marks such a result "en N sitios" and lists every place, each with its own
"Mostrar en carpeta" (`ui/copies.js`, the `copies` dialog).

### Paging and cache

The whole ranked list (at most 2,000 results) is computed once; `offset` and `limit` (default 100,
at most 500) slice it. The last list is cached on the search connection, keyed by the folded
query, the filters and SQLite's `PRAGMA data_version`, so the next page comes from the same
list unless the index changed (S7).

## Tauri command contract

`apps/desktop/src-tauri/src/commands.rs`. Every command is `async` (off the main thread)
and rejects with `ApiError { code, message }` (`core/api.rs`), where `code` is one of
`not_found`, `not_a_directory`, `overlapping_root`, `unavailable`, `schema_too_new`, `db`,
`io`. Payloads are the camelCase serde types of `core/types.rs`.

| Command | Arguments | Returns |
|---|---|---|
| `system_locale` | — | `string`: the OS locale as BCP 47, `""` when unknown; the UI picks its language from it (VFS-22) |
| `set_language` | `lang` (`"es"` or `"en"`; anything else is ignored) | `null`; window title, macOS menu and folder picker follow it (VFS-22) |
| `list_roots` | — | `Root[]` (each with `drive`: `local`, `network`, `removable`, `unknown`) |
| `pick_folder` | — | `string \| null` (native folder dialog) |
| `add_root` | `path` | `Root` |
| `remove_root` | `id` | `null` |
| `search` | `query: SearchQuery` | `SearchResponse` |
| `reveal` | `id`, `isDir` | `"revealed"` or `"opened_parent"` |
| `get_progress` | — | `IndexProgress` |
| `rescan` | `id \| null` | `null` (`unavailable` if the indexer is not running) |
| `set_paused` | `paused` | `null` |
| `search_sources` | `query: SearchQuery`, `options: { sources: Source[] (empty = all), bookmarkFolder, sort: SortOrder }` (optional) | `SourcesResponse` (VFS-6) |
| `get_bookmarks_status` | — | `BookmarkStatus` |
| `set_bookmarks_enabled` | `enabled` | `BookmarkStatus`; off deletes every stored bookmark |
| `open_bookmark` | `id` | `null`; `not_found` for an unknown id, `unavailable` for a non-web address |
| `reveal_target` | `id` (a link's `files.id`) | `"revealed"` or `"opened_parent"`; `not_found` when the recorded target does not exist (VFS-18 "Mostrar el destino") |
| `open_link_url` | `id` | `null`; `unavailable` unless the Internet Shortcut has an `http`/`https` address (VFS-18 "Abrir en Firefox") |

`rescan` with `id: null` ("Buscar cambios ahora") also wakes the bookmark reader.

Events: `index-progress` (payload `IndexProgress`) and `index-changed` (payload `null`), sent
by `EventSink` in `apps/desktop/src-tauri/src/lib.rs`; `bookmarks-changed` (payload
`BookmarkStatus`) after every bookmark read, from `src-tauri/src/bookmarks.rs`.

Key shapes:

- `SearchQuery { text, filters: { kinds, modifiedFrom, modifiedTo, sizeMin, sizeMax, folder },
  limit, offset }`; dates are unix seconds (`modifiedTo` exclusive), sizes bytes.
- `SearchHit { id, name, dir, kind, size, modified, matched, isDir, copies?, link? }`. `id`
  is a `files.id`, or a `dirs.id` when `isDir`; the two id spaces overlap, so `reveal` needs
  both. `matched` is `name`, `path`, `target` (found with a link's target, VFS-18) or
  `fuzzy`. `copies` (VFS-19) is left out when empty and never set on folders.
- `link { kind: shortcut | internet | symlink, target: string | null, openable }`: set on a
  file hit that is a shortcut, Internet Shortcut or link (VFS-18); `openable` only for an
  `http`/`https` Internet Shortcut.
- `FileCopy { id, dir, modified }`: another place of a file hit; `id` is a `files.id`.
- `SearchResponse { hits, total, truncated, truncatedBy, elapsedMs }`.

`reveal` builds the path from the index. If it no longer exists on disk, it marks the folder
for a re-list and rejects with `not_found`. Otherwise macOS runs `open -R <path>`, and if
that fails `open -R` on the nearest existing ancestor, which Finder shows selected rather
than opening it (an ancestor that is an `.app` bundle is never launched). Windows calls the
Shell's `SHOpenFolderAndSelectItems` on the item's ID list, on a short-lived thread with its
own COM apartment, which selects the item and reuses an Explorer window already on that
folder. When the call has to open a new Explorer window, the folder opens with nothing
selected (issue 107: measured on the Windows CI runner, also with `explorer /select,`), so
once a new window appears the call is repeated into it. Only if the call fails it runs
`explorer.exe /select,"<path>"`. A path over 259 UTF-16 units opens the nearest ancestor
Explorer can address instead; a trailing backslash (a drive root) is doubled so it cannot
escape the closing quote under the Windows command-line rules (`explorer_arg`; the
drive-root case is unverified on Windows). See `apps/desktop/src-tauri/src/reveal.rs`. No
file is opened with its application.

Warnings and errors go to `<local app data dir>/logs/vfs.log` (`install_logger`, the Tauri
log plugin), rotated at 1 MB with four old files kept. Below warning level only the
indexer's diagnostic records (target `vfs::diag`, VFS-5) are written: per-minute walk rates,
back-off, commit and checkpoint durations, overflows and memory, never a file name.

The main window starts hidden and is shown when its page has loaded (or after 120 s). If
its outer frame is larger than its monitor's work area (the screen minus the taskbar), it is
shrunk to fit and centred, with its minimum size capped at the new size (`fit_window`, issue
106). On Windows a native splash window (`splash.rs`, plain Win32 on its own thread) says
"Preparando VFS…" (or "Getting VFS ready…", by the OS language, VFS-22) until then, because WebView2 can take tens of seconds to paint on its
first run (issue 108).

The app opens the index at `<local app data dir>/index.db` in `setup` (Tauri
`app_local_data_dir`: `%LOCALAPPDATA%\app.vfs.desktop` on Windows, never the roaming
folder, which can be synced to a NAS or backed up with the profile; on macOS the same
`~/Library/Application Support/app.vfs.desktop` as before). If an earlier build left an
index in the roaming folder and the local one has none, `move_legacy_index` moves it once,
with its WAL, shared memory, session folder and `.bad` copy; if that rename fails, a new
index is built (C6). It then starts the indexer, and on exit calls `Vfs::shutdown`. A
startup failure shows an error dialog in the OS language (the UI's, once it has set it;
VFS-22) and quits. A
second launch focuses the running window (single-instance plugin). The webview may only use
core IPC (`capabilities/default.json`) under a strict content security policy
(`tauri.conf.json`).

## Bookmarks (VFS-6)

`crates/vfs-bookmarks` adds Firefox bookmarks as a second source, off until the user turns
on "Incluir marcadores de Firefox" in the folder panel. Spec:
[`specs/vfs-6-bookmarks/spec.md`](../specs/vfs-6-bookmarks/spec.md).

- **Finding profiles** (`firefox/mod.rs`): `profiles.ini` in `%APPDATA%\Mozilla\Firefox`, the
  Microsoft Store build's redirected copy of it, or `~/Library/Application Support/Firefox`;
  a profile with a `StoreID` adds the profiles of `Profile Groups/<StoreID>.sqlite`.
- **Reading** (`firefox/snapshot.rs`, `places.rs`, `backup.rs`): Firefox's own files are never
  opened with SQLite. `places.sqlite` and its WAL are copied into
  `<data dir>/bookmarks.db.snapshot/`; the copy is kept only if both files had the same
  size and modification time before and after copying and `quick_check` passes (3 attempts,
  500 ms apart), and it is deleted after reading. The copy is read through a SQLite
  authorizer that allows only `moz_bookmarks`, `moz_keywords` and `moz_places.id`/`url`
  (`ALLOWED_COLUMNS`), so history cannot be read. If the database cannot be read, the newest
  readable `bookmarkbackups/*.jsonlz4` is used. Tags (folders under the tags root) belong to
  the address; `place:` and `javascript:` entries and identical rows are dropped.
- **Storing** (`store.rs`): `bookmarks.db` (WAL, `secure_delete`): `settings`, `profiles` (one
  row per profile folder with the size/time signature of its files) and `items`, the
  bookmarks and folders (the four roots included) with their parent guid and Firefox
  position, unique per profile and Firefox `guid`, so an item keeps its id across reads. A profile whose
  signature is unchanged is not read again. Turning the setting off deletes every profile
  and bookmark and vacuums the file.
- **Refreshing**: thread `vfs-bookmarks`, in the indexer's walk profile (low CPU priority,
  normal I/O, VFS-5), reads at start, when the setting is turned on, on "Buscar cambios
  ahora", and every 30 minutes.
- **Searching** (`search.rs`, `sources.rs`): every stored item is in memory with folded keys
  (title; for a bookmark the address without scheme and `www.`, folder path, tags and
  keyword; for a folder its whole path), its ancestors' ids and its tree key (profile, then
  Firefox's position at every level), and is matched with `vfs_core::matching`, the file
  rules: substrings, and one typo for terms of 6–64 characters when the exact matches are
  fewer than 20. A folder also needs one term in its own name (VFS-1 D1). The title takes
  the file name's place in the tiers. `search_sources` reads the file list from the core in
  500-hit pages and merges both lists by (tier, folded name or title length, newer first),
  files first on a tie; any other order is applied by the core first and the bookmarks are
  merged into it (see "Sort orders (VFS-12)"), and `source` puts files first. A bookmark folder filter (`bookmarkFolder`) keeps the items with that folder among
  their ancestors and no files; with no words they come in tree order, Firefox's own. Type
  filters drop bookmarks but keep bookmark folders under "Carpetas"; size and file folder
  filters drop both; a date filter applies to the date an item was saved.
- **Opening**: `open_bookmark` passes an `http`/`https` address without spaces, quotes or
  control characters to Firefox: `open -a Firefox <url>` on macOS, `ShellExecuteW` of
  `firefox.exe` on Windows (App Paths or the Store build's alias), and the default browser
  if Firefox cannot be started. Nothing else is opened.

## Not yet run

The `cfg(windows)` code has run only on GitHub's Windows Server 2025 runners (CI, and the
VFS-8 end-to-end journey on an SSD; `docs/research/windows-e2e-results.md`): the installer,
the watcher, the catch-up at start, the Explorer reveal (selection checked by
`tests/windows_shell.rs`) and the splash window ran there; never on Windows 10/11, on an
HDD, or on the reference machine. Window fitting has not run on a screen smaller than the window.
Never run on Windows at all: for VFS-6, the copy of a `places.sqlite` that Firefox holds
open, the Microsoft Store profile path and `ShellExecuteW` for "Abrir en Firefox". The move
of an index out of the roaming folder has run only in unit tests, where both folders are
temporary ones. Performance at 2 million
files is checked only by `vfs bench` on a synthetic database on the development Mac (Apple
Silicon, SSD), with warm OS file caches. There the warm p95 is close to the 150 ms limit:
before the no-hit probe, two of three runs passed and the worst failed by under 10 ms on
two query classes; with it, three of three runs passed, with load averages of 4.4–5.1 from
other work and the slowest class at 114–133 ms per run; peak RSS is about 123 MB (all
measured, schema version 2; version 3 peaks at about 138 MB, VFS-4). Each VFS-12 order
(`vfs bench --sort`) passes there too, at 58–91 ms overall warm p95 and at most about 115 ms
in its slowest class; runs that coincided with bursts of other work on the machine failed by
tripling every class at once, cold runs included. On Windows only the
runner figures of VFS-8 exist; nothing has been measured on an HDD.
