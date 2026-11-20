# VFS architecture

This document describes the code as it is on the VFS-1 integration branch: how a folder on
disk becomes rows in the index, and how a query becomes a ranked list of results. File
references are relative to the repository root; `core/` means `crates/vfs-core/src/`.
Requirement and decision ids (`AC-n`, `S2`, `I4`, …) are those of
[`specs/vfs-1-core-search/spec.md`](../specs/vfs-1-core-search/spec.md).

## Components

```
┌──────────────── Tauri 2 app (one process) ─────────────────┐
│ apps/desktop/ui        static Spanish screen (JS, no build) │
│        │  invoke(command) / listen(event)                   │
│ apps/desktop/src-tauri commands.rs, reveal.rs, lib.rs       │
│        │                                                    │
│ crates/vfs-core        Vfs facade (core/lib.rs)             │
│   ├─ roots.rs          add / remove / list folders          │
│   ├─ search.rs         candidates, verification, paging     │
│   │    ├─ fuzzy.rs     one-typo candidates and check        │
│   │    └─ rank.rs      ordering                             │
│   ├─ indexer/          thread "vfs-indexer"                 │
│   │    ├─ mod.rs       scheduler, scan loop, resume         │
│   │    ├─ walk.rs      list one directory                   │
│   │    ├─ apply.rs     diff a listing against the index     │
│   │    └─ watch.rs     FSEvents (notify) / own Windows loop │
│   ├─ platform.rs       volume key, drive kind, priority     │
│   ├─ db.rs             connections, schema, ingest, session │
│   └─ fold.rs, kind.rs  search keys, extension → type        │
└──────────────────────────────┬─────────────────────────────┘
                               │ SQLite, WAL mode
                    <app data dir>/index.db
```

`crates/vfs-cli` uses the same `Vfs` facade, with its own index file.

The app is one process. The core opens these SQLite connections to the same file
(`core/lib.rs`, `core/db.rs`):

| Connection | Opened by | Page cache | Use |
|---|---|---|---|
| writer | indexer thread (`IndexerHandle::spawn`) | 32 MiB | the scan loop and watcher hints |
| short-lived writer | `add_root`, `remove_root`, `mark_dirty` | 32 MiB | root changes from commands |
| reader | `Vfs::open_with` | 64 MiB | roots, stats, paths for "mostrar en carpeta" |
| search reader | `Vfs::open_with` | 128 MiB | queries only; interrupted by each new query |

All connections set `journal_mode=WAL`, `synchronous=NORMAL` and `mmap_size=0`, so resident
memory is bounded by the page caches (AC-9 budget: under 300 MB). Readers set
`query_only=1`. A writer waits up to 30 s for the other writer; readers wait up to 5 s.

## Schema

`core/schema.sql`, schema version 1 (`PRAGMA user_version`). `db::open_rw` creates it on a
new file; an index from a newer build fails with `schema_too_new` and is never discarded.

| Table | Holds |
|---|---|
| `meta` | key/value pairs (unused in v1) |
| `volumes` | one row per volume key (Windows: volume GUID path; macOS: mount point) |
| `roots` | the folders the user chose: `path` (absolute, native separators), `path_key`, `status` (0 pending, 1 scanning, 2 ready, 3 unavailable, 4 removing), `last_full_scan_at`, counts |
| `dirs` | every directory once, empty ones included: `rel_path` (original names, `/`-separated, `''` for the root), `path_key`, `name_key`, `mtime`, `scan_state` |
| `files` | one row per file: `dir_id`, original `name`, `name_key`, `kind` (0 other, 1 document, 2 photo, 3 video, 4 audio), `size`, `mtime`, `flags` (bit 0: name was not valid Unicode), `content_state` (phase 2; always 0) |
| `files_fts` | FTS5 trigram index over `files.name_key`, `rowid = files.id` |
| `dirs_fts` | FTS5 trigram index over `dirs.path_key` and `dirs.name_key`, `rowid = dirs.id` |

Design points:

- **A file never repeats its folder path.** Paths are rebuilt from `roots.path` +
  `dirs.rel_path` + `files.name` (`db::file_abs_path`, `db::dir_abs_path`). Indexing the full
  path of every file would repeat deep paths thousands of times in the trigram index.
- **Both FTS tables are contentless** (`content=''`, `contentless_delete=1`) with
  `tokenize='trigram case_sensitive 1'`. Keys are folded before they are stored, so the
  tokenizer does no case or accent work. There are no triggers: every FTS row is written by
  `db::ingest` in the same transaction as its base row.
- **No foreign keys.** The indexer deletes subtrees in its own order
  (`ingest::delete_dir_subtree`, `ingest::delete_root`).
- Indexes: `dirs_pending` (partial, `scan_state <> 0`) is the indexer's work queue;
  `dirs_parent` serves child listing; `UNIQUE(root_id, rel_path)` and `UNIQUE(dir_id, name)`.

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
from `lstat`. It skips:

- symlinks, junctions and other name-surrogate reparse points (`FileType::is_symlink`), so a
  walk never loops;
- excluded names: `$RECYCLE.BIN`, `System Volume Information`, `.Trashes`, `.Trash`,
  `.Spotlight-V100`, `.fseventsd`, `.DocumentRevisions-V100`, `.TemporaryItems`,
  `.DS_Store`, `Thumbs.db`, `desktop.ini` (ASCII case-insensitive), and names starting with
  `._` or `~$`;
- the app's own data directory;
- entries that vanish or fail while being listed.

A name that is not valid Unicode is stored in its lossy form with `flags` bit 0 set.

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

A rename or move is a delete plus an insert; a moved folder is walked again at its new
place. If a directory cannot be listed because of a permission or I/O error, what is
indexed stays, the error is counted, and the directory is set clean. If it is gone, its
subtree is deleted — unless the root itself is unreachable, in which case the root becomes
`unavailable` and nothing is deleted.

When no directory of a root is pending, `finalize_root` recounts files and folders, marks
the root `ready`, and after a full scan stamps `last_full_scan_at` and asks both FTS tables
for a segment merge.

### Resume after interruption

Writes happen in transactions that commit between directories every 20,000 changed rows or
5 s (I5). Because the queue lives in the same transaction as the rows, a run killed at any
point redoes only the directories of its last uncommitted transaction. On start
(`Worker::init`) the indexer finishes roots left in `removing`, recounts roots that are not
ready, queues a re-walk of every ready root whose last full scan is older than 24 h, starts
the watcher, and then simply continues with whatever is pending (AC-6).

### The indexer thread

`IndexerHandle::spawn` starts the thread `vfs-indexer`, which lowers its own priority
(`platform::enter_background_priority`: macOS background QoS plus throttled disk I/O
policy; Windows `THREAD_MODE_BACKGROUND_BEGIN`). The thread:

- takes commands from a channel: roots changed, rescan, pause, watcher events;
- runs the scan loop one transaction batch at a time, yielding to any new command;
- publishes progress (`IndexProgress`) at most every 500 ms and `index-changed` at most every
  2 s after a commit;
- when idle, waits up to 60 s for a command;
- every hour, queues a re-walk of roots whose last full scan is older than 7 days (I4).

There is one I/O stream: roots are scanned one after another, one directory at a time.

### Watcher and re-walk

`core/indexer/watch.rs` turns filesystem events into hints; the scan loop does the actual
work. A `Dirty(path)` event is coalesced for 2 s, then `mark_dirty` sets the path's parent
directory (and the path itself, if it is an indexed directory) pending shallow. If the
parent is not indexed yet, its nearest indexed ancestor is marked. A clean directory is the
only one that changes state, so a hint never downgrades a directory pending deep (I1).
Events under the app data directory are dropped (I3).

| System | Backend | When events may be lost |
|---|---|---|
| macOS | `notify` (FSEvents), recursive per root, canonical paths | `need_rescan` flag or a watcher error → re-walk of the affected root (or every root when the path is unknown) |
| Windows | own `ReadDirectoryChangesW` loop, one thread per root (`vfs-watch-<id>`), 64 KB buffer, overlapped I/O | 0 bytes or `ERROR_NOTIFY_ENUM_DIR` (overflow) → re-walk, re-arm on the same handle; any other failure → re-walk, close the handle, reopen every 2 s, re-walk again once reopened (I2) |

A re-walk (`queue_rewalk`) sets the root directory pending deep and the root `scanning`;
the deep listing then cascades through the whole tree. Re-walks also happen when the user
asks (`rescan`), when an unavailable root becomes reachable, on start after 24 h and while
running after 7 days. If the watcher cannot start, indexing still runs and these re-walks
still catch changes. The Windows loop has never run (see "Not yet run").

### Session marker and recovery

`db::open_session` starts a session by creating and locking
`<index>.session/<pid>-<n>.lock`. A clean shutdown (`Vfs::shutdown`, or dropping the last
`Vfs` clone) unlocks and removes it. On the next open, a marker that no live process holds
locked means the previous session ended uncleanly; only then does the core run
`PRAGMA quick_check(1)`. An index that cannot be opened or fails the check is renamed to
`index.db.bad` (with its `-wal` and `-shm`), a fresh index is created, and every root path
that can still be read from the bad file is re-added pending deep (C5). Several processes
(the app and the CLI) can share one index; each holds its own marker.

## From a query to results

`search::search` (`core/search.rs`) runs on the dedicated search connection. The facade
calls `InterruptHandle::interrupt` on it before every query, so a newer query stops an older
one (S6).

### Parsing

The text has `\` replaced by `/`, is folded, and is split on whitespace into at most 8
distinct terms. With no terms and no filter, the response is empty.

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

### Candidate generation (exact pass)

Roots with the same effective terms form a group. For each group (`exact_group`):

1. **Choose the generator term (S2).** For each term of 3 or more characters, count its
   matches in `files_fts` and in `dirs_fts`, exactly up to 50,000 each. The term with the
   fewest matches (ties: the longer term) drives the query.
2. **G1:** files whose name contains the generator term, read by rowid from the
   `files_fts` match and joined to `files` and `dirs`.
3. **G2:** folders whose path contains it (the FTS match drives a `CROSS JOIN` so SQLite
   reads each directory by rowid), then the files of those folders. At most 20,000 matching
   folders are read; more sets `truncatedBy = "cap"`.
4. **Verify** every candidate in Rust against all terms (`verify_file`, `verify_dir`).

Terms shorter than 3 characters cannot use the trigram index (S5). They are checked during
verification; when a group has only short terms, or the query has filters and no terms, the
group is a filtered scan with `instr` in SQL (`scan_group`).

At most 2,000 hits are accepted; one more sets `truncatedBy = "cap"`.

### Deadline

Each query has two deadlines, checked every 1,000 SQLite VM steps and between rows:

- **soft**, `SearchConfig::deadline` (default 1,000 ms): applies to scans, the fuzzy pass, and
  generators whose term matched more than 5,000 rows;
- **hard**, the larger of the soft deadline and 2 s: applies to everything else, so a
  selective generator runs to completion (S3).

An expired deadline or an interruption returns what was found with `truncatedBy =
"deadline"`. Such a list is not cached (U1).

### Typo tolerance (fuzzy pass)

Runs only when the exact pass found fewer than 20 hits and was not stopped
(`fuzzy_pass`). For each term of 6 or more characters, `fuzzy::candidate_queries` builds FTS5
expressions such that every string within one insertion, deletion, substitution or
transposition contains at least one of them: for each edit position the parts before and
after it, joined with `AND`, parts shorter than 3 characters dropped (S4). The pass reads at
most 5,000 rows in all and verifies each candidate with `fuzzy::contains_within_one`
(approximate substring search, optimal string alignment distance 1). Hits found this way
are `matched = "fuzzy"`.

### Ranking

`rank::rank` (`core/rank.rs`) sorts by tier, then shorter name, newer modification time,
lower id, and files before folders:

| Tier | Rule |
|---|---|
| 0 | the name, or the name without its extension, equals the whole query |
| 1 | every term is in the name and the name starts with the first term |
| 2 | every term is in the name |
| 3 | some term matched only in the path |
| 4 | fuzzy match |

A query with filters and no terms lists newest first (S6).

### Paging and cache

The whole ranked list (at most 2,000) is computed once; `offset` and `limit` (default 100,
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
| `list_roots` | — | `Root[]` (each with `drive`: `local`, `network`, `removable`, `unknown`) |
| `pick_folder` | — | `string \| null` (native folder dialog) |
| `add_root` | `path` | `Root` |
| `remove_root` | `id` | `null` |
| `search` | `query: SearchQuery` | `SearchResponse` |
| `reveal` | `id`, `isDir` | `"revealed"` or `"opened_parent"` |
| `get_progress` | — | `IndexProgress` |
| `rescan` | `id \| null` | `null` (`unavailable` if the indexer is not running) |
| `set_paused` | `paused` | `null` |

Events: `index-progress` (payload `IndexProgress`) and `index-changed` (payload `null`), sent
by `EventSink` in `apps/desktop/src-tauri/src/lib.rs`.

Key shapes:

- `SearchQuery { text, filters: { kinds, modifiedFrom, modifiedTo, sizeMin, sizeMax, folder },
  limit, offset }`; dates are unix seconds (`modifiedTo` exclusive), sizes bytes.
- `SearchHit { id, name, dir, kind, size, modified, matched, isDir }`. `id` is a `files.id`,
  or a `dirs.id` when `isDir`; the two id spaces overlap, so `reveal` needs both.
- `SearchResponse { hits, total, truncated, truncatedBy, elapsedMs }`.

`reveal` builds the path from the index. If it no longer exists on disk, it marks the folder
for a re-list and rejects with `not_found`. Otherwise macOS runs `open -R <path>`; Windows
runs `explorer.exe /select,"<path>"`, or opens the nearest ancestor Explorer can address when
the path is over 259 UTF-16 units (`apps/desktop/src-tauri/src/reveal.rs`). No file is
opened with its application.

The app opens the index at `<app data dir>/index.db` in `setup`, starts the indexer, and on
exit calls `Vfs::shutdown`. A startup failure shows a Spanish error dialog and quits. A
second launch focuses the running window (single-instance plugin). The webview may only use
core IPC (`capabilities/default.json`) under a strict content security policy
(`tauri.conf.json`).

## Not yet run

Nothing under `cfg(windows)` has executed, and there is no record of it compiling (the CI
workflow has not run on GitHub yet):
the `ReadDirectoryChangesW` loop, volume GUID lookup, drive type, background thread mode,
long-path handling in the watcher, and the Explorer reveal. The same holds for the NSIS
installer. Performance at 2 million files is checked only by `vfs bench` on a synthetic
database on the development Mac, with warm OS file caches; nothing has been measured on the
target PC's HDD.
