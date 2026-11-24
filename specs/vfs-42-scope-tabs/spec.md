# VFS-42 — «Nombres» and «Documentos» tabs

- **Status**: approved. The owner decided D1–D3 before 0.3.0; D4–D7 are the agent's.
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
  - VFS-38 R6's `vfs.scope` values, R8's scope chip and its share of «Quitar filtros (N)»,
    and R13's names-only empty view (R5, R6 here).
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

- R1. WHILE the results view is shown, THE app SHALL show two tabs at the top of the results:
  «Nombres» and «Documentos» (EN «Names», «Documents»). They are a `role="tablist"` labelled
  «Buscar en» for assistive technology; each tab is a `role="tab"` with `aria-selected` and
  `aria-controls` the results, which are its `role="tabpanel"`. One tab stop: the arrow keys,
  Home and End move between the tabs and choose the tab they land on. The filter line holds
  «Tipo», «Fecha» and «Más filtros» only.
- R2. WHILE «Nombres» is chosen (the default), THE app SHALL search file and folder names
  and folder paths, as VFS-1 does (core `scope: all`), with bookmarks following the source
  choice (VFS-6), and SHALL run no content query.
- R3. WHILE «Documentos» is chosen and content search is on, THE app SHALL run only the
  content search (VFS-27 R12–R14) and no name search. Its query starts when the tab is
  chosen or the text or filters change on it. It shows the hits with their highlighted
  fragment, «Ver más», and the reading and partial notes.
  - A query under 3 characters shows «Escribe al menos 3 letras para buscar dentro de los
    documentos».
  - When no document matches, the view says so and offers «Buscar en los nombres», which
    chooses «Nombres».
  - The source choice («Origen») and its chip, and «Orden», are not shown: documents are not
    bookmarks and are ordered by relevance.
- R4. WHILE content search is off, THE «Documentos» tab SHALL still be shown. Choosing it
  runs nothing and shows what document search does and the button that turns it on, through
  the same path as VFS-27's offer (its count, rough time and disk use where the offer shows
  them). Once content search is on, the tab searches as R3.
- R5. THE app SHALL remember the last tab in `localStorage` under `vfs.scope`, read through a
  `ScopeTab` enum (`names`, `documents`) like VFS-38 R6: a missing, unknown or old value
  (`all`) gives «Nombres», and so does `documents` while content search is off at start. A
  tab is not a filter: it never counts in «Más filtros (N)» or «Quitar filtros (N)», has no
  chip, and «Quitar filtros» leaves it as it is.
- R6. WHEN a «Nombres» search finds nothing, THE empty view and the VFS-13 suggestion SHALL
  behave as before VFS-38 (no scope wording; `dropFilters` drops only the filters).
- R7. Each tab SHALL show a count when one is known without extra work: «Nombres» the total
  of its last name search for the current text and filters, «Documentos» the total of its
  last content answer for them («más de N» when capped). A tab whose search has not run for
  the current text and filters shows no count. No query is run to count.
- R8. At 1366×768 and 1280×720 at 125 %, at 1920×1080, and with large text, in Spanish and
  English, THE tab row SHALL fit on one row with «Orden» (when shown) and no sideways scroll,
  and every VFS-35 rule holds (nothing wider than the window, the bottom bar sticky).

## Acceptance scenarios

Tests cite these ids (`VFS-42 AC-<m>`) in their doc comments.

- AC-1 (R1) The tab row is a tablist named «Buscar en» with two tabs, «Nombres» selected;
  Tab reaches one tab; the arrow keys, Home and End move and choose; the results panel is
  labelled by the chosen tab. The filter line has no scope buttons.
- AC-2 (R2) With content search on and bookmarks on, typing `foto` under «Nombres» lists name
  and path hits (a file found only by its folder path included) and sends no content query
  (counted in the fake backend).
- AC-3 (R3) Choosing «Documentos» with `foto` sends one content query and no name search;
  the content hits show with highlights and «Ver más»; «Origen» and «Orden» are hidden; `fo`
  shows the 3-letter message; a text with no match shows «Buscar en los nombres», which
  chooses «Nombres».
- AC-4 (R4) With content search off, «Documentos» is shown; choosing it sends no query and
  shows the invitation; its button turns content search on, after which a search runs.
- AC-5 (R5) A remembered «Documentos» opens on «Documentos» after a restart with content on,
  and on «Nombres» with content off; `all`, an unknown value and a throwing storage give
  «Nombres». With a type and a folder set, «Quitar filtros (2)» counts no tab and keeps it.
- AC-6 (R7) After a «Nombres» search the «Nombres» tab shows its count and «Documentos»
  none; after choosing «Documentos» it shows its count; changing the text clears the count
  of the tab not shown.
- AC-7 (R8) At the VFS-35 viewports above, normal and large text, both languages, with
  bookmarks on, the tab row is one row, inside the window, and nothing scrolls sideways.

## Non-goals

- No core or CLI change: `scope: names` stays for `vfs search --names-only`.
- No mixed view of names and documents, and no names-only mode in the app.
- No new remembered setting: the tab reuses `vfs.scope`.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | How does the user choose what to search? | Two tabs at the top of the results, «Nombres» and «Documentos», replacing VFS-38's three-option group; no «Solo …» wording. | owner | 2026-11-24 |
| D2 | What does «Nombres» search? | Name and folder path, like the classic search, and never content, so the default search does no content work. | owner | 2026-11-24 |
| D3 | What leaves the screen? | The mixed «Todo» view and the names-only mode. The core keeps `scope: names` for the CLI. | owner | 2026-11-24 |
| D4 | With content search off, hide «Documentos» or show it? | Show it as an invitation (R4). A hidden tab is never discovered, and a tab row that changes with a setting moves the layout; choosing it explains and offers the switch, and nothing runs until the user turns it on. | agent | 2026-11-24 |
| D5 | Remember the tab? | Yes: VFS-38 R6 already remembers the scope, so the tab reuses `vfs.scope`; «Documentos» is not restored while content search is off. | agent | 2026-11-24 |
| D6 | Counts on the tabs? | Only counts already known for the current text and filters (R7); a count is never worth a query. | agent | 2026-11-24 |
| D7 | Is the tab a filter? | No: it chooses what is searched, so it has no chip and «Quitar filtros» keeps it. | agent | 2026-11-24 |

## Open questions

None blocking.
