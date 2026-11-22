# VFS-38 — «Buscar en» and a shorter filter line

- **Status**: draft
- **Size**: L. It is multi-PR, and two of its pieces ride with other specs' PRs (VFS-27 27c,
  VFS-29). It touches the search request in `vfs-core` and `vfs-bookmarks`, the filter line,
  the result rows, and both string catalogs. It has no index schema change, no new store, and
  nothing platform-specific.
- **Links**:
  - Research: [`docs/research/search-modes.md`](../../docs/research/search-modes.md).
  - [VFS-1](../vfs-1-core-search/spec.md): R7 (name and path matching), R9 (visible filters),
    S2 (tiers), S7 (cache).
  - [VFS-6](../vfs-6-bookmarks/spec.md): "Source choice", D7.
  - [VFS-12](../vfs-12-sort/spec.md) AC-5: the «Orden» select.
  - [VFS-13](../vfs-13-no-hits-suggestion/spec.md): the suggestion when nothing is found.
  - [VFS-27](../vfs-27-content-in-app/spec.md) R5, R12: the content switch and the content
    section.
  - [VFS-29](../vfs-29-hybrid-search/spec.md): meaning search.
  - [VFS-35](../vfs-35-small-screens/spec.md) R2, D1: one row of filter pills.

## Problem

- **No way to narrow.** One query matches every file whose name or folder path holds all
  the words, in one list (VFS-1 R7). A person who remembers the file name gets the whole
  contents of a folder that happens to share the word: «peñalara» also lists every photo
  under `Fotos\Peñalara julio`. Nothing lets them say "only names".
- **More match kinds coming.** v0.3 adds matches inside documents (VFS-27), and VFS-29
  later adds matches by meaning. Each new kind is one more thing the user may want to
  include or leave out.
- **The filter line is already full.** It holds the source, type, date, size, folder and
  sort, with «Borrar filtros» after them. On the target 1366×768 laptop it only fits by
  scrolling sideways (VFS-35 R2). Research puts the comfortable limit of a horizontal filter
  row at 6–8 controls, and finds that older users struggle with drop-downs (research doc).
  One more pill would make it worse.
- **Nothing says why a row is there.** A row found through its folder shows nothing to
  explain it.

## Requirements (EARS)

Scope

- R1. THE filter line SHALL start with «Buscar en», followed by a group of buttons that
  allows exactly one choice:
  - «Todo» (the default);
  - «Solo nombres»;
  - «Solo documentos», present only while content search is on in Ajustes (VFS-27 R5).
  The group SHALL be a radio group for assistive technology: one tab stop, and arrow keys
  move the choice. The chosen button SHALL look selected.
- R2. WHILE «Todo» is chosen, THE search SHALL behave exactly as before VFS-38:
  - names and folder paths are matched with the VFS-1 tiers;
  - bookmarks follow the source choice (VFS-6);
  - the content section appears while content search is on (VFS-27 R12);
  - from VFS-29 on, meaning matches are fused into the content section.
- R3. WHILE «Solo nombres» is chosen, a file hit SHALL hold every query term in its file
  name. A folder hit SHALL hold every term in the folder's own name. A bookmark hit SHALL
  hold every term in its title.
  - A term may still match within one edit, under VFS-1 S4, but only against the name or
    title.
  - A link found through its target (VFS-18 R4) stays a hit when every term is in the
    target's name.
  - The path tier, the path probe of VFS-1 S8, and the content section SHALL NOT run.
  - The type, date, size and folder filters apply as before.
- R4. WHILE «Solo documentos» is chosen, THE app SHALL run no name or path search. It SHALL
  show only the content results, under the same rules as VFS-27 R12–R13: debounce, «Ver
  más», filters, the reading progress, and the partial notice. The heading «En el contenido
  de los documentos» stays.
- R5. WHEN content search is turned off while «Solo documentos» is chosen, THE choice SHALL
  become «Todo».
- R6. THE app SHALL remember the scope choice, the source choice (VFS-6) and the sort order
  (VFS-12) across restarts. They are kept in the webview's `localStorage`, like the
  language choice (VFS-22), and fall back to their defaults when missing, unknown or
  unreadable. The type, date, size and folder filters SHALL still
  start empty at each start.

Filter line

- R7. THE filter line SHALL show, in this order:
  1. the scope group (R1);
  2. «Tipo» and «Fecha», as today;
  3. a button «Más filtros», which reads «Más filtros (N)» when N of the filters behind it
     are active.
  «Más filtros» SHALL open a popover anchored to it, not a dialog. It holds «Tamaño»,
  «Carpeta» and, while bookmarks are included, the source choice («Archivos y
  marcadores», «Archivos», «Marcadores»). A filter changed there applies at once, as on the
  line today; there is no apply button.
- R8. WHILE any filter behind «Más filtros» is active, THE app SHALL show a line below the
  filter line. It holds one removable chip per active filter, such as «Carpeta: Fotos ×» or
  «Solo marcadores ×», and «Borrar filtros».
  - Removing a chip clears that filter only.
  - «Borrar filtros» clears type, date, size and folder, as today, and also returns the
    scope to «Todo». The source stays: it is a remembered preference (R6), removed by its
    own chip.
  - While only «Tipo», «Fecha» or the scope are active, «Borrar filtros» stays on the filter
    line as today.
- R9. THE «Orden» select SHALL move to the results header, beside the result count. It is
  shown whenever there are results to order. This amends VFS-12 AC-5.
- R10. At the window sizes of VFS-35 AC-1, from 1280×720 at 125 % (content about
  1024×512) upward at normal text size, THE filter line SHALL fit on one row without scrolling sideways, with
  bookmarks on and content search on. Below that width, or with large text, the VFS-35
  R2 sideways scroll stays as the fallback.

Why a result matched

- R11. Each query term found literally SHALL be highlighted in bold with the highlight
  background, wherever it appears:
  - in the file or folder name and in the folder line of each visible result row;
  - for a bookmark, in its title and its folder path.
  Matching ignores case and accents as the search does (VFS-1 "Folding"), so «penalara»
  highlights «Peñalara». It covers NFC and NFD names. A term matched only within one edit is
  not highlighted. Highlighting is computed for the rows on screen only. No row SHALL carry
  a "matched in" label.
- R12. Content hits keep their fragment with the words highlighted (VFS-27 R12). A content
  hit that VFS-29 found by meaning alone, with none of the query terms in its fragment,
  SHALL carry the small label «relacionado por tema».

No hits

- R13. WHEN a search with «Solo nombres» or «Solo documentos» finds nothing, THE VFS-13
  suggestion SHALL treat the scope as one of the filters. `dropFilters` is offered when
  «Todo» with no type, date, size or folder filter would find something. Pressing it also
  returns the scope to «Todo». The source stays, as in VFS-13. The empty view SHALL also name the scope, for example «Buscando solo en los
  nombres».

Interface

- R14. `SearchQuery` (VFS-1) SHALL gain `scope: "all" | "names"`. It is optional and
  defaults to `"all"`, so existing callers and stored queries are unchanged.
  - `search_sources` (VFS-6) and `suggest` (VFS-13) SHALL accept it as well.
  - The search cache key (VFS-1 S7) SHALL include it.
  - The CLI SHALL accept `--names-only`.
  - «Solo documentos» is a screen choice: it calls `retrieve` only, so the core needs no
    third value.

## Acceptance scenarios

Tests cite these ids (`VFS-38 AC-<m>`) in their doc comments.

- AC-1 (R2, R14) Given the VFS-11 golden set and the VFS-6 source tests, When run with no
  `scope` and with `scope: "all"`, Then results, order and caps are identical to before.
- AC-2 (R3) Given `Excursión Peñalara.docx` in `D:\Fotos\Sierra`, `IMG_0412.jpg` in
  `D:\Fotos\Peñalara julio`, a folder `Peñalara julio`, and a bookmark titled «Ruta a
  Peñalara», When searching `peñalara` with «Solo nombres» and both sources, Then the
  .docx, the folder and the bookmark are hits and the .jpg is not. With «Todo», all four are
  hits. `penalra` with «Solo nombres» finds the .docx through one edit.
- AC-3 (R3, R14) Given `scope: "names"` and a query whose only match is in folder paths,
  When searched, Then the path tier and the S8 probe do not run (counted by a test hook),
  and the response is empty. The CLI with `--names-only` gives the same hits as the core
  call.
- AC-4 (R1, R4, R5) Given content search off, Then the group shows «Todo» and «Solo
  nombres» only. Given content search on, «Solo documentos» chosen and `factura.pdf`
  holding «Peñalara» in its text, When searching `peñalara`, Then no name list is shown and
  the content section lists `factura.pdf`. When content search is then turned off, Then the
  choice is «Todo».
- AC-5 (R1) Given the scope group, When it is reached with Tab and the arrow keys are
  pressed, Then the choice moves and the search re-runs. The accessibility tree shows one
  radio group named «Buscar en» with the chosen radio checked.
- AC-6 (R6) Given «Solo nombres», source «Archivos», sort «Lo más reciente primero», type
  «Fotos» and folder «Fotos», When the app restarts, Then scope, source and sort are kept
  and type and folder are empty. Given `localStorage` that throws or holds an unknown
  value, Then the defaults apply and the app starts.
- AC-7 (R7, R8) Given bookmarks on, When the folder «Fotos» and the source «Marcadores»
  are chosen in «Más filtros», Then the button reads «Más filtros (2)». The chip line shows
  «Carpeta: Fotos ×», «Solo marcadores ×» and «Borrar filtros». Removing the folder chip
  leaves the source. Given «Solo nombres» and a folder chosen, «Borrar filtros» clears the
  folder and sets the scope to «Todo», and the source chip stays.
- AC-8 (R9, R10) Given the VFS-35 viewport harness at 1280×720 and 1366×768 at 125 %, and
  1920×1080 at normal text size, with bookmarks and content search on, When the results view renders, Then the
  filter line is one row with no sideways scroll, and «Orden» sits in the results header.
  At the minimum window (720×480) and with large text, the row may scroll sideways and nothing overflows the page
  (VFS-35 R5).
- AC-9 (R11) Given results for `penalara julio` (accents and case differing, one name in
  NFD), Then «Peñalara» and «julio» are highlighted in the name or the folder line where
  they occur, nothing else is, and no row has a match label. Given a hit found only by
  typo, Then nothing in it is highlighted.
- AC-10 (R12) Given a VFS-29 meaning-only content hit and a keyword content hit, Then only
  the first carries «relacionado por tema». This test is built in VFS-29's PR.
- AC-11 (R13) Given `excursión` with «Solo nombres», where only a folder path holds the
  word, When nothing is found, Then the empty view says the search is names-only. It offers
  `dropFilters` with the «Todo» count, and pressing it shows the folder's files with the
  scope set to «Todo».

## Non-goals

- Query syntax for the scope (`path:`, `nopath:`, `name:`). P8 (VFS-17) is the expert
  layer, if it is ever built.
- A fourth button for meaning search. Meaning is a level in Ajustes (VFS-29) and fuses into
  the content section.
- A "folder path only" scope. The «Carpeta» filter already restricts to a folder.
- A per-search way to leave content out while it is on. The content section sits below the
  name list, and the Ajustes switch (VFS-27 R5) hides it for good.
- Grouping name hits and path hits into separate sections. Path hits already rank below
  name hits (VFS-1 S2).
- Removing duplicates between the name list and the content section (VFS-27 R12 is
  unchanged).
- Highlighting typo matches, or highlighting in rows that are not on screen.

## Pre-mortem

- **Slow PC and HDD.** «Solo nombres» does less work than today: no path tier, no probe.
  Highlighting runs only for the rows on screen, in JS, with no extra IPC. Remembering the
  choices is three `localStorage` keys, read once at start.
- **Paths over 260 characters.** n/a: no new file access. Highlighting works on the strings
  the row already shows.
- **NFC/NFD.** Highlighting folds the displayed string the way the search does. AC-9 has an
  NFD name.
- **Antivirus.** n/a: no file is opened or written; the choices live in the webview's
  `localStorage`.
- **Kill or power cut.** A lost or damaged `localStorage` value falls back to the defaults
  (AC-6).
- **Huge result sets.** No change to caps or paging (S7). «Solo nombres» returns a subset.
- **Permission-denied, locked files, cloud placeholders.** n/a: matching only, over rows
  already indexed.
- **User files.** Nothing is opened, written, moved or deleted.
- **Hidden state (specific to this spec).** A remembered «Solo nombres» could make the
  archive look empty. Three things guard against it:
  - the scope group is always on screen, with the choice selected (R1);
  - the empty view names the scope and offers «Todo» (R13);
  - «Borrar filtros» resets it (R8).

## Phased PRs

| PR | Scope | Rides with | Proves |
|---|---|---|---|
| 38a | Core `scope` in `vfs-core` search, `vfs-bookmarks` matching and `suggest`; cache key; Tauri arguments; CLI `--names-only` | v0.3, independent: any time, before 38b | AC-1–AC-3, AC-11 (core part) |
| 38b | Filter line: scope group («Todo», «Solo nombres»), «Más filtros» popover, chip line, «Orden» in the results header, remembered choices, highlighting, the empty-view wording; both catalogs; viewport and UI tests | v0.3, after 38a and before VFS-27 27c, so that 27c builds its section into the new line | AC-5–AC-9, AC-11 |
| (27c) | «Solo documentos» button, R4–R5, highlight in fragments: built inside VFS-27 27c | VFS-27 27c | AC-4 |
| (29) | «relacionado por tema» label | VFS-29's interface PR | AC-10 |

Effort: 38a 1–2 days, 38b 3–4 days, the 27c part about 1 day inside 27c **[estimate]**.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Which control chooses the match kinds? | Layout A: a three-button scope group («Todo», «Solo nombres», «Solo documentos») with no drop-down; «Tipo» and «Fecha» visible; the rest behind «Más filtros (N)» with removable chips; «Orden» in the results header. This was chosen over one more pill (past the 6–8 control limit, and still scrolling) and over a Drive-style line (scope hidden before the first search). | owner | 2026-11-22 |
| D2 | Remember the choice across restarts? | Yes, for scope, source and sort. This changes VFS-6 D7. Filters still start empty. | owner | 2026-11-22 |
| D3 | How is the reason for a match shown? | Highlighted words in the name and folder line, and the fragment for content hits. There is no per-row badge: no tool or guideline examined uses one. «relacionado por tema» appears only for meaning-only hits. | owner | 2026-11-22 |
| D4 | Single choice or checkboxes? | Single choice. The cases the owner named are "only names" and "never content", the second handled by the Ajustes switch. Checkboxes would allow empty and odd combinations (path without name) that need explaining. | agent | 2026-11-22 |
| D5 | Does «Solo nombres» also apply to bookmarks? | Yes: the title plays the part of the name (VFS-6 "Matching"). | agent | 2026-11-22 |
| D6 | Where does the source choice go? | Inside «Más filtros», so the line stays at five controls. It is shown as a chip when it is not the default. | agent | 2026-11-22 |

## Open questions

None blocking.
