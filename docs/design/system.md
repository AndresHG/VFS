# VFS visual system

Small on purpose (the mockups' `tokens.css` implements only what they show): one typeface, one accent, six text sizes, an 8-step spacing scale, a dozen
colour tokens per theme. Everything else is derived. Grounded in [references.md](references.md)
and the problems listed in [audit.md](audit.md).

## Principles

1. **The results are the interface.** Chrome takes the least space that keeps it usable.
2. **One volume.** Ink for content, muted ink for context, accent for the one thing to do now.
3. **Visible, not loud.** Filters and status are always on screen (spec R9) but quiet until used.
4. **Big by default, bigger on request.** Sizes in `rem`; the layout survives 200 % text.
5. **Nothing moves unless the person moved it.**

## Type

Atkinson Hyperlegible (OFL, bundled as TrueType, regular 400 and bold 700). No second family.
The bundled copy has a plain zero: the face's slashed zero ("2Ø19") reads as a letter to some
older readers, so `scripts/plain_zero.py` replaces its slashed counter with one oval (issue 111).
Base size is the system size (`1rem`); the body runs one step above it.

| Token | Size | Line height | Weight | Use |
|---|---|---|---|---|
| `--t-search` | 1.5rem | 1.3 | 400 | Text typed in the search box |
| `--t-title` | 1.375rem | 1.3 | 700 | Empty-state headline, dialog title |
| `--t-name` | 1.125rem | 1.35 | 700 | Result name |
| `--t-body` | 1.125rem | 1.5 | 400 | Body copy, buttons, notices |
| `--t-meta` | 1rem | 1.4 | 400 | Path, date, size, filter controls, status |
| `--t-small` | 0.875rem | 1.3 | 700 | Badges only ("No disponible") |

Rules: never below `--t-small`; no light weights; no all-caps; `text-wrap: balance` on
headlines and `pretty` on paragraphs; tabular numerals for dates and sizes.

## Spacing

A 4 px base in `rem`: `--s1` 0.25 · `--s2` 0.5 · `--s3` 0.75 · `--s4` 1 · `--s5` 1.5 ·
`--s6` 2 · `--s7` 3 · `--s8` 4 (rem). Page gutter `clamp(--s4, 2.5vw, --s6)`. Content
measure: the search box and list share one left edge and cap at 60rem.

Targets: every control is at least 2.75rem (44 px at 100 %) tall; rows are at least 3.5rem (3rem in the one-line table of direction C).

## Colour

Warm paper and ink with one terracotta accent. Values are checked for contrast against their
background (ratios in brackets; AA text needs 4.5, controls 3).

| Token | Light | Dark | Use |
|---|---|---|---|
| `--paper` | `#faf8f3` | `#1c1b19` | Window background |
| `--surface` | `#ffffff` | `#242320` | Search box, dialogs |
| `--sunk` | `#f1ede4` | `#2c2a27` | Hover, quiet buttons, active filter |
| `--ink` | `#1f1e1b` (15.7) | `#edeae3` (14.3) | Primary text |
| `--ink-2` | `#5c584f` (6.7) | `#aaa59a` (7.0) | Paths, meta, labels |
| `--line` | `#e6e1d6` | `#34322e` | Decorative hairlines between rows |
| `--line-strong` | `#8a8579` (3.5) | `#77726a` (3.6) | Borders that define a control |
| `--accent` | `#a3471f` (5.7) | `#e39a73` (7.5) | Primary action, selection edge, links |
| `--on-accent` | `#ffffff` (6.0) | `#2a1408` (7.6) | Text on accent |
| `--accent-soft` | `#f6e6dc` | `#3b2b22` | Selected row background (ink 13.7 / 11.2) |
| `--mark` | `#f5dfb3` | `#5a4520` | Match highlight background (ink 12.8 / 7.6) |
| `--warn-bg` / `--warn-ink` | `#fbf0d9` / `#7a4f00` (6.3) | `#2e2615` / `#f0c987` (9.6) | Unavailable disk |
| `--danger` | `#a8321e` (6.3) | `#ff9b85` (8.4) | Destructive confirm only |
| `--focus` | `#1f5fd6` (5.4) | `#8ab4ff` (8.2) | Focus ring, distinct from the accent |

File-type icons use `--ink-2`; the folder icon alone keeps a warm tint so folders are found by
shape and colour. Colour never carries meaning alone: unavailable rows also have a badge, the
active filter also has a check and bold label, the selected row also has an edge bar.

Implementation: raw values as custom properties, switched under
`@media (prefers-color-scheme: dark)` on `:root` (older WKWebView lacks `light-dark()`), with
`color-scheme: light dark` and `<meta name="color-scheme">`. Forced colours map selection to
`Highlight`, focus to `CanvasText`, marks to `Mark`.

## Components and states

| Component | Rest | Hover | Focus (keyboard) | Active / selected | Disabled |
|---|---|---|---|---|---|
| Search box | surface, 1.5px `--line-strong`, radius 0.875rem | — | 3px `--focus` ring, 2px offset | — | — |
| Primary button | accent fill, on-accent text | 8 % darker | ring | 2 % darker | 55 % opacity, no pointer |
| Quiet button | no border, ink-2 text | `--sunk` bg, ink text | ring | `--sunk` bg | 55 % opacity |
| Filter control | `--sunk` bg, ink text, chevron | 4 % darker | ring | accent-soft bg, bold, chosen value as the label; "Quitar filtros (n)" clears all | — |
| Result row | paper, hairline below | `--sunk` | ring inset on the list | accent-soft bg + 4px accent edge, action button shown, full path shown | unavailable: ink-2 name, badge |
| Badge | warn-bg, warn-ink, `--t-small` (only "No disponible" today) | — | — | — | — |
| Notice | one line, warn-bg, icon + sentence + quiet button | — | — | — | — |

The reveal action appears on the selected row and on hover; Return and double-click perform it
on any row. Hidden-until-hover is never the only path: keyboard selection shows it too.

## Motion

- Colour/background transitions only, 120 ms ease-out.
- Panel open: 160 ms slide + fade, only under `prefers-reduced-motion: no-preference`.
- Busy indicator: a small 3-dot pulse or slow indeterminate bar beside the status text; slows
  to 12 s under reduced motion. Never a spinner over the results.
- No scroll-linked effects, no blur, no shadows larger than `0 1px 2px`.

## Wording (Spanish)

- Impersonal *usted*, plain verbs: *buscar, revisar, mostrar, quitar, añadir*. Never
  *indexar, índice, reindexar, escanear, base de datos, sincronizar*.
- Say what happened and what to do, in one sentence each: "No se encuentra el disco
  «E:\Copias antiguas». Conéctelo y se volverá a revisar."
- Numbers with Spanish grouping (`30.569`), dates like `8 nov 2025`, times `9:14`.
- One separator per status line (` · `), never after an ellipsis.
- Buttons are verbs: *Mostrar en carpeta, Añadir carpeta…, Buscar cambios ahora, Quitar*.
  The ellipsis means "asks something more" (a picker or a confirm).
- No exclamation marks, no "¡Ups!", no blame ("ha introducido mal…").
- English copy (VFS-22) keeps the same rules in plain words ("check", "show", "remove", never
  "index" or "scan"); numbers group with commas (`30,569`) and dates and times follow the
  system's English locale (`Nov 8, 2025`, `9:14 AM` in en-US).

## Sources (VFS-6)

Search covers files and, when "Incluir marcadores de Firefox" is on in the "Carpetas" panel,
Firefox bookmarks ([spec](../../specs/vfs-6-bookmarks/spec.md)). The layout keeps its shape:
- **Where to search** ("Archivos y marcadores", the default; "Archivos"; "Marcadores") is the
  first pill in the filter line and shows only while bookmarks are included. **"Orden"** is
  the last pill and always shows, with five orders (most alike, newest, oldest, name A–Z,
  largest) plus «Archivos primero, luego marcadores» while bookmarks are included (VFS-12).
  Both are native-select pills like date and size rather than the type checklist: each is
  one choice, and a select also works where the Popover API is missing.
- **Bookmark rows** reuse the result row: a bookmark icon, the title as the name, and the
  bookmark folder path ("Menú de marcadores › Recetas › Postres") in the path line with the
  same middle truncation, because the folder path matters as much as for files. The date slot
  carries the date the bookmark was saved and the size slot its site (`cocina.es`). The details line shows the whole
  path and the address.
- The bookmark row's one action is **"Abrir en Firefox"**: it opens the bookmarked page in
  Firefox, like "Mostrar en carpeta" for files, with the same Intro/double-click behaviour.
  Firefox cannot be told from outside to select a bookmark in its bookmark manager, so the
  folder path on the row is what tells where the bookmark is saved (owner's decision).
- **Bookmark folders** are results with their own icon, a folder with a bookmark ribbon in
  the accent colour (VFS-36), so they never pass for a disk folder; their action is **"Ver su
  contenido"**, which makes the folder the "Carpeta" filter ("Dentro de: Recetas") and lists
  what it holds in Firefox's order. The "Carpeta" select also offers the bookmark roots under
  "Marcadores de Firefox".

## Directions for the main screen

Three static mockups use the same tokens and sample data. Open them by serving
`apps/desktop/` (`python3 -m http.server 8765 --directory apps/desktop`) and visiting
`/ui-dev/mockups/<name>.html`, with `?state=idle` for the empty screen. Screenshots are in
[`mockups/`](mockups/).

| | A · Papel | B · Panel | C · Lista |
|---|---|---|---|
| Idea | One calm column. Idle is a centred question and the box, like a blank page; typing moves it up. | Every option as a one-click list in a left panel, with the folders and their state. | Today's layout, cleaned: one bar for search and filters, a one-line-per-item table. |
| Idle | [a-papel-idle](mockups/a-papel-idle.png) | [b-panel-idle](mockups/b-panel-idle.png) | [c-lista-idle](mockups/c-lista-idle.png) |
| Results | [a-papel-results](mockups/a-papel-results.png) | [b-panel-results](mockups/b-panel-results.png) | [c-lista-results](mockups/c-lista-results.png) |
| Dark | [a-papel-results-dark](mockups/a-papel-results-dark.png) | [b-panel-results-dark](mockups/b-panel-results-dark.png) | [c-lista-results-dark](mockups/c-lista-results-dark.png) |
| 900×600 | [5 rows](mockups/a-papel-900x600.png) | [5 rows; status scrolls off](mockups/b-panel-900x600.png) | [6 rows](mockups/c-lista-900x600.png) |
| 900×600, 150 % | [1–2 rows; footer wraps](mockups/a-papel-900x600-150pct.png) | [2–3 rows](mockups/b-panel-900x600-150pct.png) | [2 rows; footer scrolls off](mockups/c-lista-900x600-150pct.png) |
| Filters | Four quiet buttons; type opens a short checklist | Always-open lists; no menus at all | Four native selects |
| Feels like | claude.ai, Spotlight | Finder sidebar, Mail | Everything, a tidy spreadsheet |
| Cost to build | Medium: one popover for the type checklist | Medium: new layout, folder panel merges into it | Small: mostly CSS on today's markup |
| Risk | Type filter is one click deeper than today | Busiest screen; the panel eats width on a small window | Least calm; dense rows are harder for an older reader to track |

(The phase 1 UI today shows 2 rows at 900×600 and none at 150 %.)

**Chosen: A · Papel** (the owner's pick; the built screen in every audited state, named as in [`audit/`](audit/) for before/after comparison, is in [`papel/`](papel/); the same screen in the real macOS Tauri window, dark mode, is in [`papel/tauri/`](papel/tauri/)). It is the direction that answers the owner's brief — simple,
clean, calm, in the spirit of claude.ai — and it fixes the audit's top findings at once: the
results get the screen, one action instead of a column of buttons, no boxes in boxes, warm
colour, a quiet idle page that says what to do. It keeps every filter visible on one line (R9)
and every keyboard and screen-reader behaviour of today. Two things to borrow when building it:
from B, the type checklist uses large one-click rows with ticks; from C, the result count moves
into the status line so short windows keep one more row. Pick B instead if the person using it
struggles with anything that opens; pick C if the goal is the smallest change before phase 2.
