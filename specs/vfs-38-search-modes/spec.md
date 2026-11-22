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
  sort, with «Quitar filtros» after them. On the target 1366×768 laptop it only fits by
  scrolling sideways (VFS-35 R2). Research puts the comfortable limit of a horizontal filter
  row at 6–8 controls, and finds that older users struggle with drop-downs (research doc).
  One more pill would make it worse.
- **Nothing says why a row is there.** A row found through its folder shows nothing to
  explain it.

## Requirements (EARS)

Scope

- R1. THE filter line SHALL start with a group of buttons allowing exactly one choice:
  - «Todo» (the default);
  - «Solo nombres»;
  - «Solo documentos», present only while content search is on in Ajustes (VFS-27 R5).
  The group is labelled «Buscar en», visibly when R10's width allows it and always for
  assistive technology. It SHALL be a radio group: one tab stop, arrow keys move the choice,
  and the chosen button looks selected. Each change re-runs the search, and the newer query
  interrupts the running one (VFS-1 S6).
- R2. WHILE «Todo» is chosen, THE search SHALL behave exactly as before VFS-38:
  - names and folder paths are matched with the VFS-1 tiers;
  - bookmarks follow the source choice (VFS-6);
  - the content section appears while content search is on (VFS-27 R12);
  - from VFS-29 on, meaning matches are fused into the content section.
- R3. WHILE «Solo nombres» is chosen, the search SHALL match only names:
  - **Files.** A file hit holds every query term in its file name.
  - **Folders.** A folder hit holds every term in the folder's own name.
  - **Bookmarks.** A bookmark hit holds every term in its title. Its address, folders,
    tags and keyword are not used.
  - **Typos.** A term of 6 or more characters may match within one edit, under VFS-1 S4,
    but only against the name or title.
  - **Links.** A link found through its target (VFS-18 R4) stays a hit when every term is
    in the target's name.
  - **Root names.** The VFS-1 S1 rule, which ignores terms already in a root's own path,
    does not apply: every term must be in the name.
  - **What does not run.** The path tier of VFS-1 S2 does not run. Where the search scans
    instead of using the trigram index (terms under 3 characters, S5), and inside a
    «Carpeta» filter, terms are checked against the name only. The S8 probe still runs
    unchanged: it checks path and name, a superset of names, so when it finds nothing, no
    name can match either. The content section does not run.
  - **Filter-only queries.** A query with no terms lists exactly what «Todo» lists, since
    there is nothing to match.
  - **Filters.** The type, date, size and folder filters apply as before.
- R4. WHILE «Solo documentos» is chosen, THE app SHALL run no name or path search. It SHALL
  show only the content results, under VFS-27 R12–R13 (debounce, «Ver más», filters,
  reading progress, the partial notice), under the heading «En el contenido de los
  documentos».
  - Bookmarks are not searched. The source choice is not shown.
  - «Orden» is not shown: content hits are ordered by relevance only (VFS-27).
  - For a query under 3 characters, the view says «Escribe al menos 3 letras para buscar
    dentro de los documentos».
  - When no document matches, the view says so and offers the button «Buscar en todo»,
    which sets the scope to «Todo».
- R5. WHEN the app starts, or content search is turned off, while the scope is «Solo
  documentos» and content search is off, THE scope SHALL become «Todo».
- R6. THE app SHALL remember the scope, the source choice (VFS-6) and the sort order
  (VFS-12) across restarts, in the webview's `localStorage` under `vfs.scope`, `vfs.source`
  and `vfs.sort`, as `vfs.language` is (VFS-22).
  - A value is read through the contract enums. A missing value, an unknown value, a value
    not offered at start (R5, `sortFor`, `sourcesFor`), or an error from storage gives the
    default.
  - The type, date, size and folder filters still start empty at each start.

Filter line

- R7. THE filter line SHALL show, in this order:
  1. the scope group (R1);
  2. «Tipo» and «Fecha», as today;
  3. a button «Más filtros», which reads «Más filtros (N)» when N of the filters behind it
     are active.
  «Más filtros» holds «Tamaño», «Carpeta» and, while bookmarks are included, the source
  choice («Archivos y marcadores», «Archivos», «Marcadores»).
  - It SHALL open a popover anchored to it, not a dialog.
  - WHERE the Popover API is missing (macOS 11–12 WebKit), it SHALL instead unfold an
    inline row below the filter line, as the type checklist does (`TypeChecklist.INLINE`).
  - A filter changed there applies at once, as on the line today; there is no apply button.
- R8. WHILE any filter is active (type, date, size, folder, a source other than the default,
  or a scope other than «Todo»), THE app SHALL show a line below the filter line. It holds:
  - a removable chip for each active filter behind «Más filtros», such as «Carpeta: Fotos ×»
    or «Solo marcadores ×»;
  - «Quitar filtros (N)» at its end.
  «Quitar filtros» therefore leaves the filter line, which keeps a constant width.
  - Removing a chip clears that filter only.
  - «Quitar filtros» clears everything that narrows the results: type, date, size and
    folder as today, plus the source (back to the default) and the scope (back to «Todo»).
    Sort only orders and hides nothing, so it stays.
- R9. THE «Orden» select SHALL move to the results header, beside the result count. It is
  shown whenever there are file or bookmark results to order. This amends VFS-12 AC-5.
- R10. At normal text size, at 1280×720 at 125 % and every larger VFS-35 viewport, THE
  filter line SHALL fit on one row inside its own width (the `--measure` column, 60 rem),
  in Spanish and in English. It must fit in every filter state, with bookmarks on and the
  three scope buttons present.
  - IF it does not fit at that viewport, THEN the visible «Buscar en» label is hidden
    first, kept for assistive technology. No other control may shrink.
  - At smaller windows or with large text, VFS-35's behaviour stays: one row scrolling
    sideways on short windows, wrapping on tall narrow ones, and never a page wider than
    the window (VFS-35 R5).

Why a result matched

- R11. Each query term found literally SHALL be highlighted in bold with the highlight
  background, wherever it appears:
  - in the file or folder name and in the folder line of each visible result row;
  - for a bookmark, in its title and its folder path.
  The rules:
  - The terms are those the core used, after VFS-1 S9's limits and dedup, returned as
    `terms` in the search response, so the screen never re-parses the query.
  - The screen folds the displayed string as `fold_key` does, keeping a map from folded
    offsets back to the displayed characters, so «penalara» highlights «Peñalara» in NFC
    or NFD.
  - Terms under 3 characters are not highlighted.
  - A term matched only within one edit is not highlighted.
  - In a folder line shortened in the middle, only the visible parts are highlighted.
  - Highlighting runs for the rows on screen only. No row SHALL carry a "matched in" label.
- R12. Content hits keep their fragment with the words highlighted (VFS-27 R12). A content
  hit that VFS-29 found by meaning alone, with none of the query terms in its fragment,
  SHALL carry the small label «relacionado por tema».

No hits

- R13. WHEN a search with «Solo nombres» finds nothing, THE empty view SHALL name the
  scope, «Buscando solo en los nombres». The VFS-13 suggestion SHALL count the scope among
  the filters: `dropFilters` is offered when «Todo», with no type, date, size or folder
  filter, would find files. Pressing it applies «Quitar filtros», except that it keeps the
  bookmark folder (VFS-13). The «Solo documentos» case is R4's button.

Interface

- R14. `SearchQuery` (VFS-1) SHALL gain `scope: "all" | "names"`. It is optional and
  defaults to `"all"`, so existing callers are unchanged.
  - It reaches every path that builds or runs a query: `search`, `search::count`,
    `suggest` (the struct literals in `suggest.rs` included), `search_sources` (VFS-6) and
    the search cache key (VFS-1 S7).
  - `SearchResponse` and the `search_sources` response SHALL gain `terms`, the folded terms
    used (R11).
  - `vfs search --names-only` maps to `"names"`.
  - The `Scope` values SHALL be in `contract.js` and checked against the Rust enum by
    `apps/desktop/src-tauri/tests/contract.rs`, as VFS-6 AC-11 does for sources.
  - «Solo documentos» is a screen choice that calls `retrieve` only, so the core needs no
    third value.

## Acceptance scenarios

Tests cite these ids (`VFS-38 AC-<m>`) in their doc comments.

- AC-1 (R2, R14) Given the VFS-11 golden set and the VFS-6 source tests, When run with no
  `scope` and with `scope: "all"`, Then results, order and caps are identical to before.
- AC-2 (R3) Given the following, When searching `peñalara` with «Solo nombres» and both
  sources:
  - `Excursión Peñalara.docx` in `D:\Fotos\Sierra`;
  - `IMG_0412.jpg` in `D:\Fotos\Peñalara julio`;
  - a folder `Peñalara julio`;
  - a bookmark titled «Ruta a Peñalara»;
  - a bookmark tagged «peñalara» whose title lacks the word.
  Then the .docx, the folder and the titled bookmark are hits, and the .jpg and the tagged
  bookmark are not. With «Todo», all five are hits. `penalra` with «Solo nombres» finds the
  .docx through one edit.
- AC-3 (R3, R14) Given `scope: "names"`:
  - a query whose only match is in folder paths returns nothing. The path tier does not run
    and the S8 probe does (both counted by a test hook).
  - The query `cv` finds `cv.pdf` and not `x.pdf` in a folder `cv`.
  - A filter-only query (type «Fotos», empty box) returns the same hits as «Todo».
  - With the «Carpeta» filter on `D:\Fotos`, `julio` does not find `IMG_0412.jpg`.
  - With a root `D:\Peñalara`, `peñalara julio` does not find `D:\Peñalara\julio.jpg`,
    while «Todo» does (S1).
  - `vfs search --names-only` gives the same hits as the core call.
- AC-4 (R1, R4, R5) Built in VFS-27 27c.
  - Given content search off, Then the group shows «Todo» and «Solo nombres» only.
  - Given content search on, «Solo documentos» chosen, and `factura.pdf` holding «Peñalara»
    in its text, When searching `peñalara`, Then no name list, source choice or «Orden» is
    shown, and the content section lists `factura.pdf`.
  - `pe` shows the 3-letter message. `zzzz` shows the no-match message with «Buscar en
    todo».
  - When content search is turned off, Then the scope is «Todo». The same holds after a
    restart with `vfs.scope` = documents and content search off.
- AC-5 (R1) Given the scope group, When it is reached with Tab and the arrow keys are
  pressed, Then the choice moves and the search re-runs. The accessibility tree shows one
  radio group named «Buscar en» with the chosen radio checked.
- AC-6 (R6) Given «Solo nombres», source «Archivos», sort «Lo más reciente primero», type
  «Fotos» and folder «Fotos», When the app restarts, Then scope, source and sort are kept,
  and type and folder are empty.
  - Given `localStorage` that throws, or that holds an unknown value in any key, Then the
    defaults apply and the app starts.
  - Given a stored source «Marcadores» while bookmarks are off, Then files are searched.
- AC-7 (R7, R8) Given bookmarks on:
  - When the folder «Fotos» and the source «Marcadores» are chosen in «Más filtros», Then
    the button reads «Más filtros (2)», and the chip line shows «Carpeta: Fotos ×»,
    «Solo marcadores ×» and «Quitar filtros (2)».
  - Removing the folder chip leaves the source.
  - Given also «Solo nombres» and sort «Por nombre», «Quitar filtros» clears the folder,
    the source and the scope, and keeps the sort.
  - With the Popover API stubbed out, «Más filtros» unfolds the inline row and the same
    filters work.
- AC-8 (R9, R10) Given the VFS-35 viewport harness at 1280×720 and 1366×768 at 125 %, and
  1920×1080, at normal text size, in Spanish and English, with bookmarks on and a third
  scope button injected (27c re-runs this with the real one), When the results view renders
  in each state, Then the filter line is one row with no sideways scroll and «Orden» sits in
  the results header. The states:
  - no filters;
  - type, date and «Solo nombres» set;
  - every filter set.
  At the minimum window (720×480) and with large text, nothing overflows the page
  (VFS-35 R5).
- AC-9 (R11) Given results for `penalara julio`, with accents and case differing and one
  name in NFD, Then «Peñalara» and «julio» are highlighted in the name or folder line where
  they occur, nothing else is, and no row has a match label.
  - Given a hit found only by typo, or a query `a julio`, Then the typo and «a» are not
    highlighted.
  - A shared JSON fixture of fold cases (NFD, «ﬁ», «İ», «℡») gives the same folded string
    in Rust's `fold_key` and in the JS fold.
- AC-10 (R12) Given a VFS-29 meaning-only content hit and a keyword content hit, Then only
  the first carries «relacionado por tema». Built in the VFS-29 PR that shows meaning hits
  in the app.
- AC-11 (R13) Given `excursión` with «Solo nombres», where only a folder path holds the
  word, When nothing is found, Then the empty view says the search is names-only. It
  offers `dropFilters` with the «Todo» count, and pressing it shows the folder's files with
  the scope set to «Todo».

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
- Highlighting typo matches, terms under 3 characters, or rows that are not on screen.
- Bookmark tags, keywords and addresses under «Solo nombres» (D5).

## Pre-mortem

- **Slow PC and HDD.**
  - «Solo nombres» skips the path tier. It keeps the S8 probe, so a names-only search with
    no hits never falls through to the typo scans (VFS-1 AC-9).
  - Highlighting runs only for the rows on screen, in JS, with no extra IPC.
  - Each arrow-key press on the scope group re-runs the search, and the newer query
    interrupts the older (VFS-1 S6).
  - A remembered non-relevance sort makes every search read up to the VFS-12 sort cap. That
    was already the cost of choosing it. Now it lasts across restarts, which the owner
    chose (D2).
- **Small screens.**
  - R10 was measured on paper by the critic: about 918 px of the 960 px column with the
    defaults. Moving «Quitar filtros» to the chip line keeps the width constant.
  - 38b starts with the AC-8 harness run as a prototype. R10's label rule is the planned
    fallback.
  - The popover has an inline fallback for macOS 11–12 (R7).
- **Paths over 260 characters.** n/a: no new file access. Highlighting works on the
  strings the row already shows.
- **NFC/NFD.** Highlighting folds the displayed string the way the search does, checked by a
  shared fixture (AC-9).
- **Antivirus.** n/a: no file is opened or written. The choices live in `localStorage`.
- **Kill or power cut.** A lost or damaged `localStorage` value falls back to the defaults
  (AC-6).
- **Huge result sets.** No change to caps or paging (S7). «Solo nombres» returns a subset.
- **Permission-denied, locked files, cloud placeholders.** n/a: matching only, over rows
  already indexed.
- **User files.** Nothing is opened, written, moved or deleted.
- **Hidden state (specific to this spec).** A remembered «Solo nombres» or «Solo
  marcadores» could make the archive look empty. Three things guard against it:
  - the scope group is always on screen with the choice selected, and a narrowed source
    shows as a chip (R1, R8);
  - the empty view names the scope and offers «Todo» (R4, R13);
  - «Quitar filtros» resets both (R8).

## Phased PRs

| PR | Scope | Rides with | Proves |
|---|---|---|---|
| 38a | Core `scope` across `search`, `count`, `suggest` and `vfs-bookmarks` matching; `terms` in the responses; cache key; Tauri arguments; `contract.js` and `contract.rs`; CLI `vfs search --names-only` | v0.3, independent: any time, before 38b | AC-1–AC-3, AC-11 (core part) |
| 38b | Filter line: scope group («Todo», «Solo nombres»), «Más filtros» popover and its inline fallback, the chip line with «Quitar filtros», «Orden» in the results header, remembered choices, highlighting with the shared fold fixture, the empty-view wording; both catalogs; viewport and UI tests | v0.3, after 38a and before VFS-27 27c, so that 27c builds its section into the new line | AC-5–AC-9, AC-11 |
| (27c) | «Solo documentos» button and R4–R5, AC-8 re-run with three buttons: built inside VFS-27 27c | VFS-27 27c | AC-4 |
| (29) | «relacionado por tema» label | the VFS-29 PR that shows meaning hits in the app (VFS-29 is an outline; its UI PR is named when it is completed) | AC-10 |

Effort: 38a 1–2 days, 38b 3–5 days, the 27c part about 1 day inside 27c **[estimate]**.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Which control chooses the match kinds? | Layout A: a three-button scope group («Todo», «Solo nombres», «Solo documentos») with no drop-down; «Tipo» and «Fecha» visible; the rest behind «Más filtros (N)» with removable chips; «Orden» in the results header. This was chosen over one more pill (past the 6–8 control limit, and still scrolling) and over a Drive-style line (scope hidden before the first search). | owner | 2026-11-22 |
| D2 | Remember the choice across restarts? | Yes, for scope, source and sort. This changes VFS-6 D7. Filters still start empty. | owner | 2026-11-22 |
| D3 | How is the reason for a match shown? | Highlighted words in the name and folder line, and the fragment for content hits. There is no per-row badge: no tool or guideline examined uses one. «relacionado por tema» appears only for meaning-only hits. | owner | 2026-11-22 |
| D4 | Single choice or checkboxes? | Single choice. The cases the owner named are "only names" and "never content", the second handled by the Ajustes switch. Checkboxes would allow empty and odd combinations (path without name) that need explaining. | agent | 2026-11-22 |
| D5 | What does «Solo nombres» mean for bookmarks? | The title only, which plays the part of the name (VFS-6 "Matching"). Tags and keywords are user-written but not names; under «Todo» they still match. | agent | 2026-11-22 |
| D6 | Where does the source choice go? | Inside «Más filtros», so the line stays at five controls. It shows as a chip when it is not the default. | agent | 2026-11-22 |
| D7 | What does «Quitar filtros» reset? | Everything that narrows the results: filters, source and scope. Sort stays, since it hides nothing. The button moves to the chip line so the filter line keeps a constant width (critic finding). The copy stays «Quitar filtros». | agent | 2026-11-22 |
| D8 | Does VFS-1 S1 (ignore terms in the root's path) apply to «Solo nombres»? | No: «Solo nombres» promises every term in the name. | agent | 2026-11-22 |

## Open questions

None blocking.
