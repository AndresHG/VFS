# VFS-42 — «Nombres» and «Documentos» tabs

- **Status**: approved. The owner decided D1–D3 before 0.3.0; D4–D7 are the agent's. One
  independent critic pass asked for changes; all of them are made here.
- **Size**: M. One subsystem (the results view in `apps/desktop/ui`), one PR. No core,
  schema, store or platform change: the core keeps `scope: names` for the CLI (VFS-38 R14).
- **Links**:
  - [VFS-38](../vfs-38-search-modes/spec.md): the scope group this replaces, the filter line,
    remembered choices (R6), «Más filtros» and chips (R7, R8), widths (R10).
  - [VFS-27](../vfs-27-content-in-app/spec.md): the content switch (R5), the content
    section (R12–R14), the content offer.
  - [VFS-35](../vfs-35-small-screens/spec.md): small screens and large text.
  - [VFS-13](../vfs-13-no-hits-suggestion/spec.md): the suggestion when nothing is found.
- **Supersedes** (implemented specs are not edited; this list is the record):
  - VFS-38 R1–R5: the «Buscar en» group «Todo / Solo nombres / Solo documentos» in the filter
    line becomes two tabs above the results (R1–R4 here). «Todo» (names with content below)
    and «Solo nombres» (names without paths) leave the screen.
  - VFS-38 R6 for the scope (the tab is not remembered; source and sort still are), R7's
    item 1 (the scope group on the filter line), R8's scope chip and its share of «Quitar
    filtros (N)», R9 (the tab row is now the results header), R10's three-button and «Buscar
    en» label rules, R11's «Solo nombres» sentence, R13's names-only empty view, and R14's
    `ScopeChoice` (the core `Scope` stays) (R1, R5, R6, R7 here).
  - VFS-38 AC-4, AC-5, AC-11 (UI part), and the scope parts of AC-6, AC-7 and AC-8: their
    tests in `apps/desktop/ui-dev` (`filters.test.mjs`, `prefs.test.mjs`, `layout.test.mjs`,
    the `browser.mjs` `scope()` helper) are rewritten for the tabs.
  - VFS-27 R12's "under «Todo», content hits below the name hits": content hits now show only
    on the «Documentos» tab (R3 here). R12's other rules (20 hits, «Ver más», filters,
    300 ms debounce, 3 characters, newest answer wins) and R13–R14 still hold there.

## Problem

VFS-38 put three scope buttons on the filter line. On the owner's Mac the group crowded the
line, the words «Solo …» read as restrictions, and «Todo» ran a content query on every
keystroke under the name hits, which costs the most on the slow PCs VFS is tuned for. The
owner wants the classic name search by default, lighter, and documents one deliberate step
away.

## Requirements (EARS)

- R1. WHILE the results view is shown (not the start view), THE app SHALL show two tabs at
  the start of the results header: «Nombres» and «Documentos» (EN «Names», «Documents»). They
  are a `role="tablist"` labelled «Buscar en» for assistive technology; each tab is a
  `role="tab"` with `aria-selected` and `aria-controls` the results, which are its
  `role="tabpanel"`. One tab stop: the arrow keys, Home and End move between the tabs and
  choose the tab they land on. The filter line holds «Tipo», «Fecha» and «Más filtros» only.
- R2. WHILE «Nombres» is chosen (the default), THE app SHALL search file and folder names
  and folder paths, as VFS-1 does (core `scope: all`), with bookmarks following the source
  choice (VFS-6), and SHALL run no content search (`contentSearch`). The VFS-27 R14 unread
  labels on document name hits stay.
- R3. WHILE «Documentos» is chosen and content search is on, THE app SHALL run only the
  content search (VFS-27 R12–R14) and no name search. Its query starts when the tab is
  chosen or the text or filters change on it. It shows the hits with their highlighted
  fragment, «Ver más», the count «· N documentos» («más de N» when capped), and the reading
  and partial notes.
  - A query under 3 characters shows «Escribe al menos 3 letras para buscar dentro de los
    documentos».
  - When no document matches, the view says so and offers «Buscar en los nombres», which
    chooses «Nombres», and «Quitar filtros» while a filter is set. While documents are still
    being read the VFS-27 R13 note stays above it; a failed query shows the VFS-27 failure,
    never the no-match view.
  - «Carpeta» does not offer bookmark folders. A bookmark folder chosen earlier stays chosen;
    the view then says «Las carpetas de marcadores no tienen documentos», offers «Quitar la
    carpeta», and runs no query.
  - The source choice («Origen») and its chip, and «Orden», are not shown: documents are not
    bookmarks and are ordered by relevance. A non-default source counts in neither «Más
    filtros (N)» nor «Quitar filtros (N)» there, and applies again on «Nombres».
- R4. WHILE content search is off, THE «Documentos» tab SHALL still be shown. Choosing it
  runs no search and shows what document search does and a button that turns it on, as the
  VFS-27 offer does. The offer's numbers (document count, rough time, disk use) are asked for
  at most once per session and kept; when they are not known (0 documents, first scans
  running, an error), the text and button show without numbers, and choosing the tab never
  waits for them. WHEN content search is turned off while «Documentos» is chosen, the tab
  stays chosen and shows this invitation. Once content search is on, it searches as R3.
- R5. THE app SHALL open on «Nombres» every time. The tab is not remembered; `vfs.scope` is
  no longer read. A tab is not a filter: it never counts in «Más filtros (N)» or «Quitar
  filtros (N)», has no chip, and «Quitar filtros» leaves it as it is.
- R6. WHEN a «Nombres» search finds nothing, THE empty view and the VFS-13 suggestion SHALL
  behave as VFS-38 R8 and R13, minus the scope: no scope wording, and `dropFilters` applies
  «Quitar filtros» (filters and source) but keeps the bookmark folder.
- R7. THE tab row SHALL be the results header: the tabs first, then the result count and,
  on «Nombres», «Orden» at its end. The tabs carry no counts. IF the row does not fit (large
  text, a narrow window), THEN the count and «Orden» wrap to a second line; the tabs never
  shrink, wrap or scroll.
- R8. At 1366×768 and 1280×720 at 125 %, at 1920×1080, the minimum window and with large
  text, in Spanish and English, with bookmarks on, THE tabs SHALL be on one row inside the
  window with nothing scrolling sideways, and every VFS-35 rule holds.

## Acceptance scenarios

Tests cite these ids (`VFS-42 AC-<m>`) in their doc comments.

- AC-1 (R1) The tab row is a tablist named «Buscar en» with two tabs, «Nombres» selected;
  Tab reaches one tab; the arrow keys, Home and End move and choose; the results panel is
  labelled by the chosen tab. The filter line has no scope buttons, and the start view shows
  no tabs.
- AC-2 (R2) With content search and bookmarks on, typing `foto` under «Nombres» lists name
  and path hits (a file found only by its folder path included) and sends no `contentSearch`
  call (counted in the fake backend); a document hit with an unread reason still carries its
  label.
- AC-3 (R3) Choosing «Documentos» with `foto` sends one `contentSearch` and no name search;
  the hits show with highlights, the count and «Ver más»; «Origen» and «Orden» are hidden;
  `fo` shows the 3-letter message; a text with no match shows «Buscar en los nombres»
  (choosing «Nombres») and, with a type set, «Quitar filtros»; while reading, the reading
  note stays above the no-match view; a failed query shows the failure. With a bookmark
  folder chosen, «Documentos» shows the bookmark-folder message and «Quitar la carpeta» and
  sends no query, and «Carpeta» offers no bookmark folders there.
- AC-4 (R4) With content search off, «Documentos» is shown; choosing it twice sends no search
  and calls `contentOffer` at most once; the invitation's button turns content search on,
  after which a search runs. Turning content search off while on «Documentos» keeps the tab
  and shows the invitation.
- AC-5 (R5) A stored `vfs.scope` of `documents`, `all`, `names` or junk opens on «Nombres».
  With a type and a folder set on «Documentos», «Quitar filtros (2)» counts no tab and
  keeps «Documentos».
- AC-6 (R6) A «Nombres» search with a type that finds nothing offers `dropFilters` with no
  scope wording; pressing it clears the type and keeps a chosen bookmark folder.
- AC-7 (R7, R8) At the viewports of R8, normal and large text, both languages, bookmarks on,
  the tabs are one row inside the window, the count and «Orden» are visible (wrapping below
  only where they do not fit), and nothing scrolls sideways.

## Non-goals

- No core or CLI change: `scope: names` stays for `vfs search --names-only`.
- No mixed view of names and documents, and no names-only mode in the app.
- Moving through content hits with the arrow keys from the search box (follow-up).

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | How does the user choose what to search? | Two tabs at the top of the results, «Nombres» and «Documentos», replacing VFS-38's three-option group; no «Solo …» wording. | owner | 2026-11-24 |
| D2 | What does «Nombres» search? | Name and folder path, like the classic search, and never content, so the default search does no content work. | owner | 2026-11-24 |
| D3 | What leaves the screen? | The mixed «Todo» view and the names-only mode. The core keeps `scope: names` for the CLI. | owner | 2026-11-24 |
| D4 | With content search off, hide «Documentos» or show it? | Show it as an invitation (R4). A hidden tab is never discovered, and a tab row that changes with a setting moves the layout; choosing it explains and offers the switch, and nothing runs until the user turns it on. Its numbers are asked for once, since counting documents reads the whole file list. | agent | 2026-11-24 |
| D5 | Remember the tab? | No: the app opens on «Nombres» every time (R5). A remembered «Documentos» would run content queries on every keystroke after a restart with no deliberate step, the cost the owner removed, and could hide name hits from someone looking for a file. | agent | 2026-11-24 |
| D6 | Counts on the tabs? | No. Counts that come and go while typing change the tabs' width, and the result count already sits in the same row (R7). | agent | 2026-11-24 |
| D7 | Is the tab a filter? | No: it chooses what is searched, so it has no chip and «Quitar filtros» keeps it. | agent | 2026-11-24 |

## Open questions

None blocking.
