# Audit of the phase 1 search screen

Audited from `origin/feat/vfs-1-freshness` (the newest UI on 2026-11-20; `feat/vfs-1-release`
did not exist yet), driven against the development fake backend (`/ui/?fake`) in headless
Chromium, Spanish locale, 1280×800 unless noted. Screenshots are in [`audit/`](audit/).

## Verdict

The foundations are good and should be kept: bundled Atkinson Hyperlegible, sizes in `rem`,
44 px targets, strong focus rings, system light/dark, forced-colours support, a real combobox
with live announcements, plain impersonal Spanish, and honest states for every edge case
(deadline, unavailable disk, fuzzy match). Nothing here is broken for a screen reader.

What it lacks is calm. Every part of the screen asks for attention at the same volume: a bold
label above the box, six outlined chips, three outlined selects, a yellow banner with a left
bar, a bordered card around the list, a bordered button on every row, and a second toolbar at
the bottom. The result reads as a competent form, not as the quiet tool the owner asked for,
and the chrome eats the space the results need: in a 900×600 window only two rows are visible,
and with 150 % text none are.

## State by state

| State | Screenshot | Notes |
|---|---|---|
| First run, no folders | [01](audit/01-first-run-no-folders.png) | Clear message and one primary button. But the filters are shown, active-looking, with nothing to filter. |
| First scan | [02](audit/02-first-scan.png) | Two progress indicators (centre and footer) say the same thing. "quedan 2.691 carpetas" is a number that only goes up and down; the person cannot act on it. |
| Idle, ready | [03](audit/03-idle-ready.png) | Good copy ("No importan las mayúsculas, los acentos…"). The big empty bordered card makes the screen look unfinished. |
| Results | [04](audit/04-results.png) | Name, path, date, size are all there. The column of identical outlined "Mostrar en carpeta" buttons is the loudest thing on screen and repeats on every row. The "Carpeta"/"Documento" text column repeats what the icon shows. |
| Row selected | [05](audit/05-results-row-selected.png) | Selection is unmistakable (teal fill, edge bar, filled button). Too unmistakable: three signals where one or two suffice. The full path is in the footer, far from the row it describes. |
| No results | [06](audit/06-no-results.png) | Good copy; same empty card. |
| Fuzzy | [07](audit/07-fuzzy.png) | The note is correct but grey, and the rows lose their highlight, so nothing shows why "escritra" matched. |
| Search cut short | [08](audit/08-search-cut-short.png) | Wording follows the spec (U1). An info icon in accent colour for a failure is mildly confusing. |
| Search error | [09](audit/09-search-error.png) | Fine. |
| Unavailable folder | [10](audit/10-unavailable-results.png) | The banner is helpful but heavy: two lines of yellow at full width on every screen until dismissed, above the results it is meant to explain. The per-row "No disponible" badge is good. |
| Filters set | [11](audit/11-filters-set.png) | The custom date range opens a new row and pushes the list down. The selected chip (solid teal) is clear; "Quitar filtros (1)" is far away on the right of the summary line. |
| Folder panel | [12](audit/12-folder-panel.png) | Calm, readable, honest about state. "Volver a revisar" as an outlined button on every folder, plus "Quitar" as a link, plus two footer actions, is four kinds of control in one panel. |
| Dark | [13](audit/13-dark-idle.png), [14](audit/14-dark-results.png), [15](audit/15-dark-folder-panel.png) | Contrast is fine. The green-black base and mint accent feel cold and slightly "terminal"; the olive highlight is muddy. |
| 900×600 | [16](audit/16-900x600-results.png) | Filters wrap to two rows; banner takes two lines; **two result rows visible**. |
| 900×600, 150 % text | [17](audit/17-900x600-150pct-results.png) | **No result rows visible**: search, filters (three rows) and banner fill the window and the list scrolls off. This fails the intent of WCAG 1.4.4/1.4.10 for the very reader the app is for. |
| 1280×800, 150 % text | [18](audit/18-1280x800-150pct-results.png) | Under two rows visible; the filters alone take two rows. |

## Findings, ranked

1. **The results get too little of the screen.** Header + filters + banner + summary + footer
   take ~420 px at 1280×800 and all of a 900×600 window at 150 % text. Filters should collapse
   to one line (or a disclosure), the banner should shrink to one line, and the location line
   and status should share one footer row.
2. **A button on every row.** Twelve identical outlined buttons per screen are noise and an
   extra Tab stop per row in spirit, even if the list is a single listbox. Show the action on
   the selected (and hovered) row only; Return and double-click already perform it.
3. **Too many borders.** Box around the search, chips, selects, banner, list card, every row,
   every button, footer rule. Pick one: hairline row separators on a plain surface.
4. **Filters are shown at full volume even when unused.** Six chips + three selects with labels
   above them is the second-largest element on screen. Spec R9 requires filters to be visible,
   not loud: one quiet row of compact controls, a token for each active filter.
5. **The highlight is shouty.** Saturated yellow behind bold text. A softer tint (or an
   underline in the accent colour) is enough and keeps the name readable.
6. **The "Tipo" column duplicates the icon.** Drop it; keep it in the accessible name.
7. **The location of the selected item is in the footer**, ~350 px away from the row. Put the
   full path inside the selected row (second line expands), or right under the list.
8. **Colour identity is cold.** Green-grey neutrals with a teal accent read clinical; the
   owner's reference (claude.ai) is warm paper and ink.
9. **Atkinson's slashed zero in dates and sizes** ("2Ø25") looks like a glitch to some readers.
   It is a deliberate legibility feature of the face; keep the face, but check Atkinson
   Hyperlegible Next, which offers more weights (and is also OFL), before deciding.
10. **Duplicate progress.** First scan shows a progress bar in the centre and in the footer.
11. **"Pausar" is offered during a routine check**, which invites the person to stop something
    they do not need to think about. Offer pause only during the long first scan.
12. **Status punctuation**: "Comprobando cambios… · 30.569 archivos" doubles the ellipsis and
    the middle dot. Use one separator.
13. **Dev-only date input locale**: with a non-Spanish system the date picker shows
    `mm/dd/yyyy`; in the real app the WebView follows Windows, so this is fine for the user,
    but screenshots must use `es-ES`.

## What to keep exactly

- Copy in `strings.js` (with the small fixes above), the combobox/listbox semantics, the live
  announcer, Escape/arrow/Return behaviour, the reveal fallbacks and error mapping.
- Atkinson Hyperlegible, `rem` sizes, 44 px targets, `:focus-visible` rings, forced-colours
  rules, reduced-motion handling, the virtualised list.
- Folders as results, the "No disponible" badge, middle-truncated paths that keep the last
  folder visible.
