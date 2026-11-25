# VFS-45 — Settings window, version and credits, quieter results (0.3.1)

- **Status**: approved. The owner approved the mockup as a whole (D1–D4) and chose the
  license (D9) on 2026-11-25; D5–D8 and D10 are the agent's.
- **Size**: M. One subsystem (the desktop UI in `apps/desktop/ui`) plus two small shell
  changes: a fixed-address page opener and a failed update check that rejects instead of
  answering "no update". No core, schema, store or platform-specific change.
- **Links**:
  - [VFS-22](../vfs-22-i18n/spec.md): language setting, both catalogs.
  - [VFS-21](../vfs-21-updates/spec.md): the update check and its notice.
  - [VFS-27](../vfs-27-content-in-app/spec.md): the content section (R5, R14) and content
    hits (R12).
  - [VFS-28](../vfs-28-ask-archive/spec.md): the AI section, hidden without the
    `ai` feature.
  - [VFS-33](../vfs-33-bigroot/spec.md), [VFS-34](../vfs-34-file-filter/spec.md):
    «Avanzado».
  - [VFS-35](../vfs-35-small-screens/spec.md): small screens and large text.
  - [VFS-42](../vfs-42-scope-tabs/spec.md): «Nombres» and «Documentos».
- **Supersedes** (implemented specs are not edited; this list is the record):
  - The side-panel form of Ajustes (VFS-22, VFS-21 R2, VFS-27 R5, VFS-28 R1, VFS-33): Ajustes
    is now a centered window with sections (R1–R3 here). Their settings and rules are
    unchanged. VFS-33 AC-10's "folded «Avanzado»" becomes "«Avanzado» is its own section".
  - The type scale in `docs/design/system.md` steps down once (R8 here).

## Problem

The owner finds Ajustes odd: a full-height side panel where every heading, switch and
paragraph is large and stacked, with no clear structure and no version anywhere. In the
results, «Nombres» marks the selected row with an orange fill and an orange button, and
«Documentos» shows «Mostrar en carpeta» on every row at once. The type is a step larger than
it needs to be and rows sit apart. References: Wispr Flow's settings (a window with a section
list on the left, one setting per row) is the model the owner names.

## Requirements (EARS)

- R1. WHEN «Ajustes» is pressed, THE app SHALL open Ajustes as a modal window centered in
  the app window, with a section list on the left and the chosen section on the right. It
  closes on Esc, on its close button and on a click outside it, keeping unsaved input as
  every panel does (`dismiss.js`). At a window narrower than 40rem or with large text, the
  section list sits above the section as a row that wraps, and nothing scrolls sideways.
- R2. THE section list SHALL be a vertical `role="tablist"` labelled «Ajustes» with, in
  order, «General» (language, update check), «Documentos» (content search, VFS-27),
  «Preguntar (IA)» (VFS-28), «Avanzado» (VFS-33 and VFS-34) and «Acerca de». One tab stop:
  Up, Down, Home and End move and choose. «Preguntar (IA)» is shown only while the AI section
  is (VFS-28). Ajustes opens on «General» each time.
- R3. THE settings SHALL read as rows: a switch's label (bold) with its explanation below in
  muted, smaller text, and the switch at the end of the row; groups sit in bordered cards. A
  checkbox setting looks like a switch and stays a native checkbox for assistive technology.
- R4. THE section list SHALL end with the app version («VFS 0.3.1»), read from the shell
  (Tauri `app.getVersion`). WHEN the version cannot be read, the line is empty.
- R5. «Acerca de» SHALL show the app's name and one-line purpose, the version, the update
  state, «Creado por Andrés Herranz», «Licencia: PolyForm Shield 1.0.0» and «Código fuente:
  Ver en GitHub». The update state is one of:
  - «Está al día» after a check found nothing;
  - «Hay una versión nueva: X» with «Actualizar», which starts the VFS-21 install;
  - «No se ha podido comprobar si hay una versión nueva.» after a check failed, with
    «Buscar actualizaciones»; the shell rejects a failed check, still once per launch;
  - «La búsqueda de actualizaciones está desactivada (en General).» while the VFS-21 setting
    is off.
  While the setting is on, «Buscar actualizaciones» runs the check now if it has not run in
  this launch, and shows its result. With the setting off, no button is shown and nothing
  connects (VFS-21's promise); a check asked for while it was off counts as not run.
- R6. WHEN the license name or «Ver en GitHub» is pressed, THE shell SHALL open the license
  (`LICENSE.md` on the main branch) or the project's public page in the default browser. The
  page names the page (`source`, `license`); the addresses are fixed in the shell.
- R7. In «Nombres», THE selected row SHALL have a neutral background with a thin accent edge,
  and its «Mostrar en carpeta» button an accent outline (no fill). A row under the pointer
  shows a neutral outlined button. In «Documentos», «Mostrar en carpeta» SHALL show only on
  the hit under the pointer or holding the keyboard focus; it stays in the tab order, and on
  a device with no hover it always shows. A double click on a document hit reveals it, as on
  a name hit.
- R8. THE type scale SHALL step down once: search 1.3125rem, titles 1.25rem, names and body
  1rem, meta 0.875rem, badges 0.8125rem. Sizes stay in `rem`; controls stay at least 2.75rem
  tall; rows are tighter (column gap `--s3`).
- R9. Every new string SHALL be in both catalogs with the same keys (VFS-22).

## Acceptance scenarios

Tests cite these ids (`VFS-45 AC-<m>`) in their doc comments.

- AC-1 (R1) Ajustes opens centered (its left and right margins differ by at most 2 px); a
  click inside keeps it open, a click outside and Esc close it.
- AC-2 (R2) The tablist has five tabs in order with «General» selected; Down moves to
  «Documentos» and shows its panel; End reaches «Acerca de»; reopening shows «General».
  With `&ai=off` the «Preguntar (IA)» tab is hidden and the visible text mentions no AI.
- AC-3 (R3) In «General», the update switch is a checkbox whose label and the switch share
  one row, the switch at the row's end.
- AC-4 (R4) The section list ends with «VFS » and the fake backend's version.
- AC-5 (R5) Pure: the update state for each case (none found, found, failed, setting off,
  not checked) gives the text and action listed in R5, in both languages; a check made with
  the setting off leaves it not checked; the shell's one check per launch reports a failure.
  In the page, «Acerca de» shows the version, the author, the license and «Ver en GitHub».
- AC-6 (R6) Pressing «Ver en GitHub» and the license name each call `openProjectPage` once,
  with `source` and `license` (counted in the fake backend). The shell accepts only those two
  names, and both addresses are under the project's page.
- AC-7 (R7) In «Documentos», a hit's button has opacity 0 until the hit is hovered or holds
  the focus, and is reachable with Tab; a double click on a hit calls `reveal` for it. In
  «Nombres», the selected row's button has no accent fill.
- AC-8 (R1, R8) At the VFS-35 viewports and with large text, Ajustes, the file filter in
  «Avanzado» and the results fit with nothing scrolling sideways (existing layout tests,
  updated to the window).

## Non-goals

- No new settings, no change to what any setting does.
- No license text in the app: the license is chosen elsewhere (`LICENSE.md`); «Acerca de»
  names it and links to it.
- No manual update check while the setting is off (VFS-21 keeps "never connects").
- No keyboard navigation through document hits from the search box (VFS-42 follow-up).

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Shape of Ajustes | A centered window with a section list on the left, after Wispr Flow. | owner | 2026-11-25 |
| D2 | Where is the version shown? | Always at the end of the section list, and in «Acerca de» with the update state and credits. | owner | 2026-11-25 |
| D3 | Selected row in «Nombres» | Neutral background, thin accent edge, outlined button: no orange fill. | owner | 2026-11-25 |
| D4 | Type size | One step smaller across the app, rows tighter. | owner | 2026-11-25 |
| D5 | Remember the last section? | No: Ajustes opens on «General», the section most people come for; a remembered «Avanzado» surprises. | agent | 2026-11-25 |
| D6 | Manual check with the update setting off? | No button: VFS-21's copy promises VFS never connects when off, and «General» is one click away. | agent | 2026-11-25 |
| D7 | Hide «Mostrar en carpeta» in «Documentos» with `visibility` or `opacity`? | `opacity`: a hidden button leaves the tab order, so keyboard users could never reach it, and the row width stays still. | agent | 2026-11-25 |
| D8 | How does «Acerca de» open a page? | One shell command that takes a page name (`source`, `license`) and holds the addresses, reusing the feedback report's opener; the page never passes an address, so it cannot be made to open others. | agent | 2026-11-25 |
| D9 | Which license does «Acerca de» name? | PolyForm Shield 1.0.0, chosen in the license work (`LICENSE.md`). | owner | 2026-11-25 |
| D10 | How does «Acerca de» tell "no update" from "the check failed"? | The shell's once-per-launch check keeps the failure and rejects with it; the notice still ignores failures (VFS-21 R4). | agent | 2026-11-25 |

## Cost (R11)

CSS and a few DOM nodes: no measurable CPU, memory, disk or index change. The version is read
once when the window opens; nothing new runs in the background.
