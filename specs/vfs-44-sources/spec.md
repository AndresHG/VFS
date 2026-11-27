# VFS-44 — Sources: one item model, and bookmarks from Chrome and Brave

- **Status**: draft, not scheduled: the owner put semantic search (VFS-29) first after 0.4
  (D3). The owner's decisions of 2026-11-25 are D1–D3, D7, D9 and D11–D13.
- **Size**: L. It reads files outside the chosen folders (other browsers' profiles), on
  paths that differ between Windows and macOS, and it changes the hit shape that every
  source and the screen share.
- **Links**:
  - [VFS-6](../vfs-6-bookmarks/spec.md): Firefox bookmarks, `bookmarks.db`, `search_sources`,
    «Origen». This spec extends it; it does not supersede it.
  - [VFS-12](../vfs-12-sort/spec.md): «por origen».
  - Research: `docs/research/future-lines-roadmap.md` ("A common source abstraction"),
    `docs/vision.md` ("More sources").
  - Issue #197 (bookmarks from any browser, Chrome and Brave first).

## Problem

VFS reads bookmarks only from Firefox; users of Chrome or Brave get nothing, and the
owner, testing 0.3.0, asked for Chrome and Brave first (#197). The owner's direction for VFS
is an encyclopedia built from what it ingests, across every source: names, then content,
then meaning, then questions. More sources is the first step, and each new source today
would need its own hit type, its own merge rule and its own row on screen, because
`SourceHit` is `Files | Bookmarks`. Chat exports (WhatsApp, Telegram) and YouTube come next,
each in its own spec (D7). This spec adds one item model every source fills, and uses it
for the cheapest new source: Chrome's and Brave's bookmarks.

## What we know

- `crates/vfs-bookmarks` reads Firefox into `bookmarks.db`; its `profiles` table already has
  a `browser` column (`Browser::Firefox` = 0), so a second browser needs a new code, not a
  new table (`store.rs`).
- Search over sources is `vfs_bookmarks::sources::search_sources`: files from the core in
  500-hit pages, bookmarks matched in memory with `vfs_core::matching`, merged by tier or by
  the VFS-12 order. `Source` is `Files | Bookmarks`; `SourceHit` is
  `Files(SearchHit) | Bookmarks(BookmarkHit)` (`types.rs`).
- The screen names Firefox in its copy: «Incluir marcadores de Firefox», «Abrir en Firefox»,
  «Marcadores de Firefox» (`apps/desktop/ui/strings.es.js`).
- Chromium browsers keep each profile's bookmarks in one JSON file, `Bookmarks`, with roots
  `bookmark_bar`, `other` and `synced`; each node has `type` (`url` or `folder`), `name`,
  `url`, `guid`, `date_added` (microseconds since 1601-01-01 UTC) and `children`. The
  profiles are listed in the browser's `Local State` (`profile.info_cache`). Chromium
  replaces the file by writing a temporary file and renaming it **[published: Chromium
  `ImportantFileWriter`; to re-check before approval]**.
- Profile folders **[to verify on both systems before approval]**:

  | Browser | Windows (`%LOCALAPPDATA%`) | macOS (`~/Library/Application Support`) |
  |---|---|---|
  | Chrome | `Google\Chrome\User Data` | `Google/Chrome` |
  | Brave | `BraveSoftware\Brave-Browser\User Data` | `BraveSoftware/Brave-Browser` |

## Requirements (EARS)

Common item model

- R1. Every hit from every source SHALL carry `source` (`files`, `bookmarks`), `browser`
  for a bookmark (`firefox`, `chrome`, `brave`), `kind`, a `locator` (an absolute
  path or an address) and a `title`, in one shape the screen renders without a branch per
  source. Later sources (chat exports, YouTube) add a `source` value and fill the same
  fields.

Chromium bookmarks

- R2. THE folder panel SHALL list each supported browser found on the computer with its own
  switch («Incluir marcadores de Chrome», «… de Brave»), off by default like
  Firefox (VFS-6 R1), and SHALL not list a browser that is not installed.
- R3. WHEN a browser's switch is on, THE app SHALL read the `Bookmarks` file of every profile
  that `Local State` lists, read-only: it SHALL never write, lock or rename a browser file,
  and SHALL read nothing else from the profile (no history, passwords or cookies).
- R4. IF a `Bookmarks` file changes while it is read, or cannot be parsed, THEN THE app SHALL
  retry up to 3 times 500 ms apart, then try `Bookmarks.bak`, then keep the last good copy
  and report it as VFS-6 does for Firefox.
- R5. THE app SHALL re-read a profile only when its `Bookmarks` file's size or modification
  time changed, at start, when a switch is turned on, on «Buscar cambios ahora» and every
  30 minutes (VFS-6 "Refreshing").
- R6. Chromium bookmarks and folders SHALL be searched, filtered and sorted exactly like
  Firefox bookmarks (VFS-6 "Searching"): same tiers, typo rule, folder filter and «por
  origen»; the date is `date_added`.
- R7. A bookmark row SHALL say which browser it came from, and its action SHALL open the
  address in that browser («Abrir en Chrome»), falling back to the default browser if that
  one cannot be started. Only `http` and `https` addresses SHALL be opened (VFS-6 "Opening").
- R8. WHEN the user turns a browser's switch off, THE app SHALL delete that browser's rows
  from `bookmarks.db` and vacuum it, as VFS-6 does for Firefox.
- R9. THE same address saved in two browsers SHALL show as two rows, one per browser.

Footprint

- R10. With every browser switch off, this spec SHALL add no thread, file read or memory.
  With 10,000 bookmarks across browsers, bookmarks SHALL add at most 15 MB resident and
  search p95 SHALL stay within VFS-1's budget.

## Acceptance scenarios

Tests cite these ids (`VFS-44 AC-<m>`) in their doc comments.

- AC-1 (R1) Given a file, a Firefox bookmark and a Chrome bookmark matching one query, When it
  is searched, Then every hit carries `source`, `kind`, `locator`, `title`, and the bookmarks
  carry their `browser`.
- AC-2 (R2) Given a computer with Chrome profiles and no Brave, When the folder panel opens,
  Then it shows a Chrome switch, off, and no Brave switch.
- AC-3 (R3, R6) Given a fixture `Local State` with two Chrome profiles, When the Chrome switch
  is turned on, Then a word from a bookmark in each profile finds it, and the fixture files'
  bytes and modification times are unchanged.
- AC-4 (R4) Given a `Bookmarks` file that is truncated JSON and a valid `Bookmarks.bak`, When
  the profile is read, Then the backup's bookmarks are searchable and the status says the
  backup was used.
- AC-5 (R5) Given a read profile, When the 30-minute refresh runs with the file unchanged,
  Then the file is not opened.
- AC-6 (R7) Given a Brave bookmark with a `javascript:` address, When the user opens it, Then
  nothing is opened; and Given an `https` Brave bookmark, Then the open command names Brave.
- AC-7 (R8) Given Chrome bookmarks in the store, When the Chrome switch is turned off, Then no
  Chrome row remains and Firefox rows are untouched.
- AC-8 (R10) Given every browser switch off, When the app runs a search session, Then no
  bookmark thread runs for Chromium browsers and no profile file is opened.

## Non-goals

- Edge (the owner does not need it, D3), Safari (its bookmarks are a binary plist under a privacy-protected folder on macOS),
  Vivaldi, Opera and Arc: later, on request.
- Chat and YouTube importers (D7), each in its own spec on this item model.
- Writing to any browser, merging duplicates across browsers, or reorganising bookmarks.
- Browser history, reading-list or tab groups.
- The user's own notes and collections: not a goal for now (D9).

## Pre-mortem

- Slow PC and HDD: one small JSON file per profile, read only when its signature changes, in
  the low-priority bookmarks thread.
- Long paths: profile folders are under `%LOCALAPPDATA%`, short; reads go through the same
  path helpers as VFS-6.
- NFC/NFD: titles are folded with `fold_key` like Firefox titles.
- Antivirus and locks: Chromium does not lock `Bookmarks` while running; a rename mid-read is
  handled by R4's retry.
- Crash mid-write: VFS writes only `bookmarks.db` (WAL, one transaction per profile), as today.
- A million matches: bookmarks are at most tens of thousands; the merge is paged as in VFS-6.
- Permission-denied, links, placeholders: an unreadable profile is skipped and reported; a
  profile folder that is a symlink is followed once.
- Opening, writing, moving or deleting user files: no. It reads browser files outside the
  chosen folders, like VFS-6, and only `Local State` and `Bookmarks`(`.bak`).
- Does any of this already ship? `grep -rn "chrome\|brave" crates apps/desktop/ui`
  finds no Chromium reader; `Browser` has only `Firefox`.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | What may leave the PC to build knowledge? | Only what the user touches: the passages of a question or a compiled page, as in VFS-28; no background processing by an LLM API | owner | 2026-11-25 |
| D2 | Where do compiled pages (fichas) live, when they come? | Markdown in a folder VFS owns, one per entity or topic | owner | 2026-11-25 |
| D3 | What goes first after 0.4? | Semantic search (VFS-29); this spec comes later, Chrome and Brave only, no Edge | owner | 2026-11-25 |
| D7 | Do export importers come now? | No, one spec each later | owner | 2026-11-25 |
| D9 | Hand-written notes and collections? | Not a goal for now; the encyclopedia is built from what VFS ingests | owner | 2026-11-25 |
| D10 | One switch per browser or one for all? | One per browser, so a user can leave out a work profile's browser | agent | 2026-11-25 |
| D11 | May semantic search send folder text to an embedding API? | Yes, per folder, with the user's permission and the cost and volume shown first, or a local model (keeps research E1) | owner | 2026-11-25 |
| D12 | Keep the knowledge questions produce? | Yes: each answer with its citations becomes a searchable page, marked out of date when a cited source changes | owner | 2026-11-25 |
| D13 | Which chat export comes first? | Telegram (Desktop export, `result.json`), then WhatsApp | owner | 2026-11-25 |

## Open questions

- The profile paths and Chromium's atomic write are to be verified on both systems before
  approval (a test machine with each browser).
- Roadmap questions asked of the owner (semantic search, storing answers, next source) do
  not change this spec's interface or schema.
