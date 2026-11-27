# VFS-44 — Bookmarks from Chrome and Brave

- **Status**: approved (owner's plan of 2026-11-26: Chrome and Brave, no Edge; each browser
  off at first; an address in two browsers shows once)
- **Size**: L. It reads files outside the chosen folders (other browsers' profiles), on
  paths that differ between Windows and macOS, and it changes the bookmark contract the
  screen reads.
- **Links**:
  - [VFS-6](../vfs-6-bookmarks/spec.md): Firefox bookmarks, `bookmarks.db`, `search_sources`,
    «Origen». This spec extends it; it does not supersede it.
  - Issue #197 (bookmarks from any browser, Chrome and Brave first).

## Problem

VFS reads bookmarks only from Firefox; users of Chrome or Brave get nothing (#197). Both
browsers keep each profile's bookmarks in one small JSON file, so one reader covers both
and costs almost nothing.

## What we know

- `crates/vfs-bookmarks` reads Firefox into `bookmarks.db`; its `profiles` table already has
  a `browser` column (`Browser::Firefox` = 0), so a new browser needs a new code, not a new
  table or schema version.
- Chromium keeps a profile's bookmarks in `<profile>/Bookmarks`, JSON: `roots` with
  `bookmark_bar`, `other` and `synced`; each node has `type` (`url` or `folder`), `name`,
  `url`, `guid`, `date_added` (microseconds since 1601-01-01 UTC, as a string) and
  `children`. Bookmarks kept in the Google account without sync go to `AccountBookmarks`,
  same format. The browser replaces these files by writing a temporary file and renaming it
  (`ImportantFileWriter`), and keeps the previous version as `Bookmarks.bak`. Checked on the
  dev Mac (Chrome and Brave, 2026-11-27): `Bookmarks` is plain JSON with those keys; the
  `EncryptedBookmarks2` file beside it is an encrypted copy VFS does not need.
- Profiles are listed in `<data folder>/Local State`, `profile.info_cache`: the key is the
  profile folder (`Default`, `Profile 1`…), `name` is what the browser shows.
- Data folders:

  | Browser | Windows (`%LOCALAPPDATA%`) | macOS (`~/Library/Application Support`) |
  |---|---|---|
  | Chrome | `Google\Chrome\User Data` | `Google/Chrome` |
  | Brave | `BraveSoftware\Brave-Browser\User Data` | `BraveSoftware/Brave-Browser` |

- Rust opens a file on Windows with read, write and delete sharing, so a read never stops
  the browser from replacing the file (as VFS-6 R3 relies on for Firefox).

## Requirements (EARS)

- R1. THE folder panel SHALL list Firefox and each of Chrome and Brave whose data folder
  exists, each with its own switch («Incluir marcadores de Chrome»), off by default like
  Firefox (VFS-6 R1).
- R2. WHEN a Chromium browser's switch is on, THE app SHALL read `Bookmarks` and
  `AccountBookmarks` of every profile that `Local State` lists (or, without a readable
  `Local State`, of `Default` and `Profile *`), read-only: it SHALL never write, lock or
  rename a browser file, and SHALL read nothing else from the profile.
- R3. IF `Bookmarks` cannot be parsed or changes while it is read, THEN THE app SHALL retry
  up to 3 times 500 ms apart, then read `Bookmarks.bak` and say so («copia de seguridad»),
  then keep what it read before and report the failure, as VFS-6 does for Firefox. An
  unreadable `AccountBookmarks` (no backup exists) keeps what was read before and reports
  the failure.
- R4. THE app SHALL re-read a Chromium profile only when the size or modification time of
  its `Bookmarks` or `AccountBookmarks` changed, on the same occasions as Firefox (start,
  switch turned on, «Buscar cambios ahora», every 30 minutes).
- R5. Chromium bookmarks and folders SHALL be searched, filtered and sorted exactly like
  Firefox bookmarks (VFS-6): same tiers, typo rule, folder filter and «por origen»; the
  date is `date_added`. The roots map to «Barra de marcadores» (`bookmark_bar`), «Otros
  marcadores» (`other`) and «Marcadores del móvil» (`synced`).
- R6. A bookmark's action SHALL open the address in the browser it came from («Abrir en
  Chrome»), falling back to the default browser; only `http` and `https` (VFS-6 R10).
- R7. WHEN bookmarks of more than one browser are stored, a bookmark row and a «Carpeta»
  choice SHALL name the browser; a profile name SHALL show only when that browser has more
  than one profile with bookmarks.
- R8. THE same address saved in two browsers SHALL show as one result, the better ranked,
  naming the other browsers too; two profiles of one browser keep one row each (VFS-6 D8).
- R9. WHEN the user turns a browser's switch off, THE app SHALL delete that browser's rows
  and vacuum `bookmarks.db` (VFS-6 R8), leaving the other browsers' rows.
- R10. With every Chromium switch off, this spec SHALL open no browser file and start no
  thread. A profile read parses one file of about 1 MB per 1,000 bookmarks once, on the
  existing low-priority bookmark thread.

## Acceptance scenarios

Tests cite these ids (`VFS-44 AC-<m>`) in their doc comments.

- AC-1 (R1) Given a Chrome data folder and no Brave one, When the status is read, Then it
  lists Firefox and Chrome, both off, and no Brave.
- AC-2 (R2, R5) Given a `Local State` with two Chrome profiles, one with `AccountBookmarks`,
  When the Chrome switch is turned on and read, Then a word from a bookmark in each file
  finds it under its root, a folder is a result and a folder filter, and the fixture files'
  bytes and modification times are unchanged.
- AC-3 (R3) Given a truncated `Bookmarks` and a valid `Bookmarks.bak`, When read, Then the
  backup's bookmarks are found and the browser's state is «from backup»; Given neither,
  Then the earlier bookmarks are kept and the state is «failed».
- AC-4 (R4) Given a read profile, When it is refreshed with the files unchanged, Then it is
  not read again; When `Bookmarks` changes, Then it is.
- AC-5 (R6) Given a Brave bookmark, When it is opened, Then the address goes to Brave;
  a `chrome://` address is refused.
- AC-6 (R7) Given bookmarks from Firefox and Chrome, Then each hit and «Carpeta» choice
  names its browser, and with one profile per browser no profile name shows.
- AC-7 (R8) Given one address in Chrome and Brave, When it is searched, Then one result
  shows, naming both.
- AC-8 (R9) Given Firefox and Chrome bookmarks stored, When the Chrome switch is turned
  off, Then no Chrome row remains and the Firefox rows are untouched.
- AC-9 (R10) Given every Chromium switch off, When a refresh runs, Then no Chromium file is
  read.

## Non-goals

- Edge (the owner does not need it), Safari (binary plist in a privacy-protected folder),
  Vivaldi, Opera and Arc: later, on request.
- A common item model for future sources (chat exports, YouTube): each of those specs adds
  what it needs.
- Writing to any browser, merging duplicates inside one browser, history or reading lists.

## Pre-mortem

- Slow PC and HDD: one JSON file per profile, read only when its stamp changes, on the
  low-priority bookmark thread; parsing 1,000 bookmarks takes milliseconds.
- A huge or hostile file: files over 64 MB are not read; nesting deeper than 128 is cut.
- The browser rewrites the file mid-read: a parse failure or a changed stamp retries (R3).
- Chrome stops writing plain `Bookmarks` (only the encrypted copy): the state says no
  bookmarks were found; nothing breaks.
- Duplicates hiding results: only an identical address in another browser folds into one
  row, which names every browser.
- Opening, writing, moving or deleting user files: no; it reads `Local State`, `Bookmarks`,
  `AccountBookmarks` and `Bookmarks.bak` only.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Which browsers? | Chrome and Brave; no Edge | owner | 2026-11-26 |
| D2 | One switch per browser or one for all? | One per browser, off at first, so a user can leave out a browser | owner (plan) | 2026-11-26 |
| D3 | The same address in two browsers? | One result naming both browsers | owner (plan) | 2026-11-26 |
| D4 | Copy the file first, as for Firefox? | No: the file is replaced by rename, never written in place, so reading it directly is consistent; a failed parse retries | agent | 2026-11-27 |
| D5 | When are browser and profile named? | Browser only when more than one browser has bookmarks; profile only when its browser has several | agent | 2026-11-27 |
