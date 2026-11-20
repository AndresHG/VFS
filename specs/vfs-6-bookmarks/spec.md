# VFS-6 — Firefox bookmarks as an optional source

- **Status**: approved
- **Size**: L. It reads files outside the chosen folders (the Firefox profile), adds a second
  database file and a new crate, behaves differently on Windows and macOS (Firefox locks
  `places.sqlite` only on macOS), and adds a result action that is not "Mostrar en carpeta".
- **Links**: [VFS-1](../vfs-1-core-search/spec.md) (search rules, contract, wording),
  GitHub issue 63, PR 61 (VFS-1 release; this work is stacked on it).

## Problem

The end user keeps about 3,000 Firefox bookmarks: some in folder trees by topic, many
unsorted. She thinks in topics, not in file names or sources: "lo de las recetas de mi
madre" may be a PDF in her archive or a page she bookmarked years ago. Today she has to
search twice, in two programs that match differently (Firefox's bookmark search has no
accent folding and no typo tolerance). The owner decided: Firefox bookmarks become an
optional, off-by-default source, and the user chooses which sources a search covers
(files only, bookmarks only, or both). Her other wishes (YouTube playlists, chat apps, the
open internet, reorganising bookmarks, bibliographies) are out of scope.

## What we know (research, 2026-11-20)

Sources: the Firefox tree (`github.com/mozilla-firefox/firefox`, `main`), sqlite.org, and
measurements with Firefox 160.0.1 and 109 on the macOS dev machine. `FF` below means
`github.com/mozilla-firefox/firefox/blob/main`.

- **Where profiles are.** Windows `%APPDATA%\Mozilla\Firefox`; Microsoft Store build
  `%LOCALAPPDATA%\Packages\Mozilla.Firefox_n80bbvh6b1yt2\LocalCache\Roaming\Mozilla\Firefox`
  (the package family name is in FF/xpcom/metrics.yaml; the redirected path is MSIX's
  default and is **unverified**); macOS `~/Library/Application Support/Firefox`. Each has
  `profiles.ini`: `[ProfileN]` sections with `Name`, `IsRelative`, `Path`, optionally
  `StoreID` (FF/toolkit/profile/nsToolkitProfileService.cpp). With the new profile selector,
  `profiles.ini` lists only one profile per group; the rest are in
  `<root>/Profile Groups/<StoreID>.sqlite`, table `Profiles(path, name, …)`, paths relative
  to the root (FF/toolkit/profile/ProfilesDatastoreService.sys.mjs).
- **How Firefox holds `places.sqlite`.** WAL mode, 32 KiB pages, automatic checkpoint at
  2,048,000 bytes of WAL, final checkpoint when the last connection closes
  (FF/toolkit/components/places/Database.cpp `DATABASE_MAX_WAL_BYTES`). On macOS mozStorage
  opens it through the `unix-excl` VFS: an exclusive lock held for the whole session, no
  `-shm` file (FF/storage/BaseVFS.cpp; pref `storage.sqlite.exclusiveLock.enabled`, which
  "has no effect on Windows"). Measured with Firefox 160 running: a `mode=ro` open fails with
  `database is locked`; an `immutable=1` open succeeds but saw 2 of 11 bookmark rows, the
  rest being still in the WAL; a copy of `places.sqlite` + `places.sqlite-wal` saw all 11.
  On Windows Firefox uses normal locking; SQLite opens with `FILE_SHARE_READ |
  FILE_SHARE_WRITE`, so another process can copy the files (sqlite `os_win.c`).
- **Schema** (version 89, FF/toolkit/components/places/nsPlacesTables.h). `moz_bookmarks`:
  `id, type` (1 bookmark, 2 folder, 3 separator), `fk` → `moz_places.id`, `parent`,
  `position`, `title`, `dateAdded`, `lastModified` (microseconds since 1970), `guid`. Roots
  by guid: `menu________`, `toolbar_____`, `unfiled_____` ("Otros marcadores", where
  unsorted bookmarks go), `mobile______` (bookmarks synced from a phone), `tags________`.
  A tag is a folder under the tags root holding bookmark rows with the same `fk`, so tags
  belong to a URL. `moz_keywords(keyword, place_id)`. The URL is `moz_places.url`;
  `moz_places` also holds the whole browsing history (`visit_count`, `last_visit_date`, the
  page `title`), and `moz_historyvisits`, `moz_inputhistory`, `moz_origins` hold more.
- **Backups.** `<profile>/bookmarkbackups/bookmarks-YYYY-MM-DD_<count>_<hash>.jsonlz4`:
  magic `mozLz40\0`, uint32 LE decompressed size, one LZ4 block of JSON (`title, uri,
  dateAdded, lastModified, tags` (comma-separated), `keyword, root, children, typeCode`).
  Written atomically (temp file + rename) on user idle, at most once a day, never at
  shutdown; 15 kept (FF/toolkit/components/places/PlacesBackups.sys.mjs,
  BookmarkJSONUtils.sys.mjs, FF/browser/components/places/PlacesBrowserStartup.sys.mjs).
- **Firefox Sync** merges remote bookmarks into the local `places.sqlite` (phone bookmarks
  under `mobile______`); `moz_bookmarks_deleted` holds sync tombstones and is ignored.
- **Chrome and Edge, for later.** One `Bookmarks` JSON file per profile (`User Data/Default`,
  `User Data/Profile N`; names in `Local State`), roots `bookmark_bar`, `other`, `synced`,
  `date_added` as a decimal string of microseconds since 1601, written atomically
  (`ImportantFileWriter`). Chromium is moving to encrypted bookmark files
  (`kEncryptBookmarks`), so a Chrome reader may not be possible for long.
- **Measured** (Firefox 160 running, synthetic profile of 3,000 bookmarks in 12 topic trees
  plus unsorted, tags, a keyword, duplicates, long titles, Cyrillic, Japanese, Greek):
  copying `places.sqlite` (2.9 MB) and its WAL took 4.8 ms, reading 3,303 rows from the
  copy 5.7 ms (warm cache, SSD). Final numbers for the Rust reader are in the PR.

## Design

### Comparing the ways to read bookmarks while Firefox runs

| Way | Freshness | Safety | Cost |
|---|---|---|---|
| Open `places.sqlite` `mode=ro` | live | joins Firefox's locking; on macOS refused (exclusive lock); may create `-shm`/`-wal` in the profile | small |
| Open with `immutable=1` | stale until the next checkpoint; a fresh profile showed 2 of 11 rows | no lock, no write, but a concurrent checkpoint rewrites pages under the reader: wrong rows or `SQLITE_CORRUPT` (sqlite.org/uri.html) | small |
| Copy `places.sqlite` + `-wal`, open the copy | seconds | no lock, no write; the copy can be torn if Firefox checkpoints during it: detected by comparing size and modification time before and after, and `PRAGMA quick_check` on the copy | reads and writes the whole `places.sqlite` (history included), only when it changed |
| Newest `bookmarkbackups/*.jsonlz4` | up to a day old, sometimes more (written on idle only) | atomic files, never locked | tiny |

**Choice: the copy, with the newest backup as the fallback.** It is the only live way that
never takes a lock or writes in the profile, on both systems. The copy is skipped when the
size and modification time of `places.sqlite` and its WAL have not changed since the last
read, it runs on a background-priority thread, one profile at a time, and it is deleted as
soon as it has been read.

### Components

- **New crate `crates/vfs-bookmarks`**, used by the CLI and the app, never by `vfs-core`:
  - `firefox.rs`: find profile roots and profiles (`profiles.ini`, profile groups), copy
    and read `places.sqlite`, read `jsonlz4` backups. Returns plain `Bookmark` values.
  - `store.rs`: the app's own `bookmarks.db` (in the same local data directory as
    `index.db`): settings, the bookmarks and folders read (one `items` table, one id
    space, ids stable per Firefox guid), and per-profile read stamps. Loaded into memory on
    open (3,000 rows ≈ 1 MB).
  - `search.rs`: matching and ranking of bookmarks in memory.
  - `sources.rs`: the `Source` closed set and the merge of file and bookmark results.
- **`vfs-core`** gains one small public module, `matching`, so both sources share one
  definition of the query and typo rules: the query parsing now inline in `search::search`
  moves there unchanged, plus `contains_within_one`, the typo-length rule and the ranking
  tier. Nothing else in the core changes; `index.db`'s schema does not change.
- **Tauri**: new commands `search_sources(query, options: {sources, bookmarkFolder, sort})`,
  `open_bookmark`, `get_bookmarks_status`, `set_bookmarks_enabled`; `rescan` also refreshes
  bookmarks.
- **UI**: a "Buscar en" pill, first in the filter line, bookmark and bookmark-folder rows,
  bookmark roots in the "Carpeta" filter and a sort choice, behind the setting
  (`docs/design/system.md` "Sources").

### The folder tree (owner decision D10)

The user keeps an elaborate tree of nested bookmark folders by topic, so a bookmark's
folder path is treated as a file's folder path:

- Every folder is read and stored with its path, Firefox's position and its parent, the four
  roots included; bookmarks keep their position too. Sorting by the positions along the
  path gives Firefox's exact order and nesting.
- The folder path is searchable by any segment with the same folding and typo rules.
- Bookmark folders are results, matched as file folders are (VFS-1 D1): every term in the
  folder's path and name, at least one in its own name. A folder row's action is "Ver su
  contenido", which sets the folder filter to it.
- The "Carpeta" filter offers the roots that hold something, by their Spanish names
  ("Menú de marcadores", "Barra de marcadores", "Otros marcadores", "Marcadores del móvil",
  led by the profile name when several profiles have bookmarks), and "Dentro de: <name>"
  once a folder result's action chose it. A bookmark folder filter keeps only what is inside
  that folder at any depth and leaves files out, as a file folder filter leaves bookmarks
  out. With no words it lists the folder's contents in Firefox's order, which is how the
  many unsorted bookmarks ("Otros marcadores") are browsed.
- Rows show the folder path with the same middle truncation as files; the whole path is in
  the details line and the tooltip.

### Sorting (owner decision D12)

A sort choice applies to every source: "Relevancia" (the default: the shared tiers, or
Firefox's order inside a bookmark folder with no words), "Más recientes" (a file's
modification date, a bookmark's date saved) and "Por origen" (files first, then bookmarks,
each by relevance).

### Read only, always (owner decision D11)

VFS is consultative. It never writes to, modifies, reorganises or deletes anything in a
source: not Firefox's database or backups, not the bookmarks, not files. Firefox's files are
opened for reading only, never with SQLite, and a read works with the profile folder and its
files made read-only.

### Storage: own tables, no full-text index

Bookmarks live in `bookmarks.db`, not in `index.db`. They have nothing in common with
`files`/`dirs` rows (no folder on disk, no size, no type), a separate file leaves the
index schema (in flux under other work) untouched, and turning the option off deletes one
file's rows. There is no FTS index: 3,000 (even 30,000) bookmarks are scanned in memory in
a few milliseconds with exactly the file rules (substring of folded keys, one typo), which
an FTS trigram index would only approximate. Tables `profiles(id, browser, dir, name,
signature, read_at)` and `items(id, profile_id, guid, is_folder, parent_guid, position, root,
folder, title, url, tags, keyword, added, modified)`; `folder` holds the folder names below
the root joined by U+001F.

### Matching and ranking

- Keys are built with `fold_key` (VFS-1 "Folding"): title, URL without its scheme and a
  leading `www.`, folder path, tags, keyword.
- A bookmark matches when every query term is a substring of one of its keys; terms of 6
  to 64 characters may instead be within one edit (VFS-1 S4 rule, shared code).
- The title plays the part of a file name in the VFS-1 tiers: 0 the title equals the
  query, 1 every term in the title and the title starts with the first, 2 every term in the
  title, 3 some term only in the URL, folders, tags or keyword, 4–5 typos. Ties: shorter
  title, newer date added, lower id.
- **Both sources** are merged by tier, then by name or title length; on a tie the file
  comes first. Each source keeps its own order. The merged list is paged like VFS-1 S7.
  At most 2,000 bookmark hits are kept.
- Rows: the bookmark has its own icon and is labelled "Marcador"; it shows the title, the
  folder path ("Menú de marcadores › Recetas › Postres"), the site (`ejemplo.es`) and
  "guardado el <fecha>". With several Firefox profiles the profile name leads the path.

### Filters

| Filter | Bookmarks |
|---|---|
| Type | bookmarks are never shown under a type; bookmark folders are shown when "Carpetas" is one of the chosen types |
| Date | applies to the date the bookmark was saved (`dateAdded`) |
| Size | none shown |
| Folder (a folder on disk) | none shown |
| Folder (a bookmark folder) | only what is inside it; no files |

While "Marcadores" is the chosen source, the type, size and folder filters are disabled.

### Source choice

- Setting "Incluir marcadores de Firefox", off by default, in the "Carpetas" panel, with
  one line saying what it reads and that nothing leaves the computer.
- While it is on, a "Buscar en" pill with three choices: "Archivos y marcadores" (default),
  "Archivos", "Marcadores". While it is off, the pill is not shown and searches cover files
  only.
- Turning it on reads bookmarks at once; turning it off deletes every stored bookmark.

### Staying current

Bookmarks are read: when the app starts (if the setting is on), when the setting is turned
on, when "Buscar cambios ahora" is pressed, and every 30 minutes, each time only for
profiles whose `places.sqlite` or WAL changed. A read replaces that profile's rows in one
transaction; a failed read keeps the previous rows.

### Privacy

Bookmarks reveal interests. The bookmark list stays in `bookmarks.db` in the app's local
data directory like the file index; nothing leaves the computer; the app never reads
history, form data, passwords, cookies or favicons. The reader queries the copy through a
SQLite authorizer that allows reading only `moz_bookmarks`, `moz_keywords` and the `id` and
`url` columns of `moz_places`; anything else makes the query fail. The copy itself contains
the history bytes for the few milliseconds it exists; it is written inside the app's data
directory, never queried beyond the allowed columns, deleted after the read, and deleted at
start if a crash left it.

## Requirements (EARS)

- R1. WHILE the setting is off, THE app SHALL not read any Firefox file, SHALL not show the
  "Buscar en" control, and SHALL search files only.
- R2. WHEN the setting is turned on, the app starts with it on, "Buscar cambios ahora" is
  pressed, or 30 minutes pass, THE bookmark reader SHALL read the bookmarks of every
  Firefox profile found whose `places.sqlite` or WAL changed since its last read.
- R3. THE bookmark reader SHALL never open a Firefox file for writing, never take a lock on
  one, and never open one with SQLite; it SHALL read a copy made in the app's data
  directory and delete the copy afterwards.
- R4. IF a profile's database cannot be copied consistently or read, THEN THE reader SHALL
  read the newest bookmark backup of that profile; IF that fails too, THEN it SHALL keep
  the bookmarks stored before.
- R5. THE reader SHALL read only the allowed tables and columns (Privacy).
- R6. WHEN a search covers bookmarks, THE app SHALL match title, address, folder path,
  tags and keyword with the file rules for accents, case and one typo, and SHALL rank and
  merge as described.
- R7. WHEN a type, size or folder filter is set, THE app SHALL show no bookmarks; a date
  filter SHALL apply to the date a bookmark was saved.
- R8. WHEN the setting is turned off, THE app SHALL delete every stored bookmark.
- R9. THE CLI SHALL search files, bookmarks or both with `--source files|bookmarks|all`.
- R10. WHEN the user acts on a bookmark result ("Abrir en Firefox", Intro or a double
  click on the selected row), THE app SHALL open its address in Firefox (in the default
  browser if Firefox cannot be started), only for `http` and `https` addresses with no
  space, quote or control character; other bookmarks SHALL have no action. The app SHALL
  never open a file.
- R11. WHEN a search covers bookmarks, THE app SHALL return bookmark folders whose path and
  name hold every term and whose name holds at least one, as it returns file folders.
- R12. WHILE a bookmark folder is the folder filter, THE app SHALL return only bookmarks and
  folders inside it at any depth and no files; with no words, in Firefox's order.
- R13. WHEN a sort order is chosen, THE app SHALL order the merged list by it.
- R14. THE app SHALL never write to, modify, reorganise or delete anything in Firefox's
  profile or in any other source.

## Acceptance scenarios

Tests cite these ids (`VFS-6 AC-<m>`).

- AC-1 (R2, R6) Given a profile with nested folders, unsorted bookmarks, tags, a keyword,
  duplicates, a 2,000-character title and URL, and titles in Cyrillic, Japanese and Greek,
  When it is read, Then every bookmark appears once per folder it is in with its folder
  path, root, tags, keyword and date; separators, `place:` and `javascript:` entries and
  the tags tree itself are not bookmarks; identical (address, folder, title) rows appear
  once.
- AC-2 (R3) Given a second process holding `places.sqlite` open with a write lock and
  uncommitted changes in its WAL, When bookmarks are read, Then the read succeeds with the
  committed bookmarks, the holder can still write and commit, and no file in the profile
  folder was created or modified by the reader.
- AC-3 (R4) Given a profile whose `places.sqlite` is missing or corrupt and which has
  backups, When it is read, Then the bookmarks of the newest valid backup are used; Given
  neither is readable, Then the previously stored bookmarks remain and the status says the
  read failed.
- AC-4 (R2) Given no Firefox installed, or a `profiles.ini` listing a profile that does not
  exist, When bookmarks are read, Then the status says no Firefox bookmarks were found and
  nothing fails. Given several profiles (including one only in a profile group database),
  Then each profile's bookmarks are read and carry its name.
- AC-5 (R5) Given a `places.sqlite` with history rows, When bookmarks are read, Then the
  only tables and columns the reader's statements touch are the allowed ones, and a
  statement touching `moz_historyvisits` or `moz_places.title` fails.
- AC-6 (R6) Given bookmarks and files, When searching "recetas" with both sources, Then both
  appear, ranked by the shared tiers; "receta" without accent or "recteas" (one typo, 6+
  letters) finds "Recetas"; "Archivos" returns only files; "Marcadores" only bookmarks;
  paging the merged list returns each hit once.
- AC-7 (R7) Given a type, size or file folder filter, Then no bookmark is returned, except
  bookmark folders under the type "Carpetas"; Given a date filter, Then only bookmarks
  saved in that range.
- AC-8 (R1, R8) Given the setting off, Then the app reads no Firefox file and `search_sources`
  returns files only; When the setting is turned off after a read, Then `bookmarks.db` holds
  no bookmark.
- AC-9 (R2) Given an unchanged `places.sqlite` and WAL since the last read, When a refresh
  runs, Then no copy is made.
- AC-10 (R9) `vfs search --source bookmarks recetas` prints bookmarks only; `--source all`
  both; `--source files` files only.
- AC-11 (UI) The "Buscar en" choice and the bookmark row values are in `contract.js`, checked
  against the Rust enums; the control is hidden while the setting is off; a bookmark row
  shows title, folder path, site and date.
- AC-12 (R10) Given an `https` bookmark, When `open_bookmark` is called with its id, Then its
  address is handed to the system opener; Given a `file:`, `ftp:` or `data:` bookmark or an
  unknown id, Then nothing is opened and the command rejects (`not_found` for an unknown id,
  `unavailable` otherwise).

- AC-13 (R11, R12, D10) Given a profile with a 12-level tree of over 300 folders, folders
  with the same name in different branches and folders with accents, When it is read, Then
  every folder is stored with its path; a query for a folder name returns each matching
  folder with its own path ("recetas" finds "Recetas" and "Recétas viejas"); the folder
  filter on one of two same-named folders returns only what is inside it, no files; with
  no words it lists the contents in Firefox's order and nesting.
- AC-14 (R13) Given files and bookmarks, When sorted "Más recientes", Then the hits come
  newest first across both sources; "Por origen" lists every file before any bookmark;
  "Relevancia" is the AC-6 order.
- AC-15 (R14) Given a profile folder and files made read-only, When bookmarks are read,
  Then the read succeeds and the folder is unchanged (names, sizes, times).

## Non-goals

- Other browsers (Chrome, Edge, Safari): the `browser` column and reader layout leave room;
  no code now.
- History, open tabs, reading lists, Pocket, synced tabs, favicons.
- Editing, moving, deduplicating or reorganising bookmarks, or anything else written to
  Firefox or to any source (owner: explicitly not wanted; R14 makes it an invariant).
- Searching the content of bookmarked pages; anything that uses the network.
- Firefox forks (LibreWolf, Waterfox) and Firefox ESR/Nightly profile folders other than
  those in the same `profiles.ini`.
- The visual design of the selector and the row: the "A · Papel" redesign restyles them.

## Pre-mortem

- Slow PC, busy HDD: a read copies `places.sqlite` (history included; tens of MB on a heavy
  profile) only when it changed and at most every 30 minutes, on a background-priority
  thread; measured cost in the PR. The file index's I/O stream is not touched.
- Long paths on Windows: profile paths are under `%APPDATA%`, well under 260 characters;
  the copy target is the app data directory. n/a beyond that.
- NFC/NFD: titles, folders and tags go through `fold_key` like file names.
- Antivirus: a scan can delay or fail the copy; the read then falls back (R4) and retries
  at the next trigger. Never blocks Firefox: the copy opens with read and full sharing
  (Rust's default share mode on Windows includes delete).
- Killed mid-write: `bookmarks.db` is WAL with one transaction per profile read; a leftover
  copy is deleted at start; a corrupt `bookmarks.db` is deleted and re-read (it holds
  nothing that cannot be re-read).
- A query matching all 3,000 bookmarks: capped at 2,000 and paged like files.
- Permission denied, locked files, symlinks: a profile that cannot be read is skipped with a
  status; profile paths are read as given.
- Opening, writing, moving or deleting user files: only Firefox files are read (copied);
  nothing is written outside the app data directory. The bookmark action (D9) is the one
  place that hands something to another program.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Add bookmarks? Which sources? | Firefox bookmarks, optional; user picks files, bookmarks or both | owner | 2026-11-20 |
| D2 | How to read a live profile | Copy + WAL, backup fallback (table above) | session | 2026-11-20 |
| D3 | Storage | `bookmarks.db`, own tables, in-memory search, no FTS | session | 2026-11-20 |
| D4 | Filters on bookmarks | Table "Filters" | session | 2026-11-20 |
| D5 | Ranking across sources | Shared tiers; file first on ties | session | 2026-11-20 |
| D6 | Refresh triggers | Start, setting on, button, every 30 min if changed | session | 2026-11-20 |
| D7 | Default source | Both ("Archivos y marcadores"), reset at each start | owner | 2026-11-20 |
| D8 | Several profiles | All profiles Firefox lists, profile groups included; the profile name shows only when more than one profile has bookmarks ("she probably uses one, but maybe all is better") | owner | 2026-11-20 |
| D9 | Action on a bookmark | "Abrir en Firefox" (first answer was the default browser; changed by the owner through the design session); `http`/`https` only | owner | 2026-11-20 |
| D10 | Folder hierarchy | Essential: searchable, shown on every row, a filter, folders as results, Firefox's order kept | owner | 2026-11-20 |
| D11 | Writing to sources | Never: VFS is consultative (R14) | owner | 2026-11-20 |
| D12 | Sort options | Relevance (default), newest, by source | owner ("reasonable") | 2026-11-20 |

## Open questions

None.
