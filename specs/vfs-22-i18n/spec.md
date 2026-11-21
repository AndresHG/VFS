# VFS-22 — The app in Spanish and English

- **Status**: approved
- **Size**: L (two PRs; platform-specific menu and installer strings)
- **Links**: VFS-1 R9 (every visible string in one catalog), issues 110 (macOS menu in English)
  and 112 (Windows publisher shown as "vfs"), coordination issue 96

## Problem

The interface is Spanish only. Part of the audience (university researchers) reads English, and
the macOS app menu is already English while the rest is Spanish (issue 110). Strings live in one
catalog (`apps/desktop/ui/strings.js`, VFS-1 R9) but there is no second language, no way to
choose, and dates and numbers are hard-wired to `es-ES`. A few strings live in Rust (startup
error, folder picker title, Windows splash) and in `index.html`.

## Requirements (EARS)

- R1. THE UI SHALL take every visible string from a per-language catalog (`es`, `en`) with
  exactly the same keys; no template, script or HTML file outside the catalogs holds visible
  copy.
- R2. WHEN the app starts and the user has not chosen a language, THE UI SHALL use Spanish if
  the system locale is `es` or `es-*`, and English otherwise.
- R3. WHEN the user chooses Español, English or "Idioma del sistema" in the Ajustes panel, THE
  UI SHALL switch at once, without restarting, and SHALL keep that choice across restarts.
- R4. THE UI SHALL format dates and numbers for the active language (es: `1.234`, `12 mar
  2021`; en: `1,234`, the system's English date order, or `Mar 12, 2021`).
- R5. THE desktop shell SHALL show its own strings (macOS app menu, folder picker title,
  startup error, Windows splash) in the active language; strings shown before the UI loads
  use the system-locale rule of R2.
- R6. THE Windows installer SHALL offer Spanish and English, picked from the system language,
  and SHALL show the publisher as "VFS", not "vfs".
- R7. Search SHALL be unchanged: accent and case folding do not depend on the language.

## Acceptance scenarios

Tests cite these ids (`VFS-22 AC-<m>`) in their doc comments.

- AC-1 (R1) Given the `es` and `en` catalogs, When compared, Then they have the same keys, and
  every function key takes the same number of arguments and returns a non-empty string.
- AC-2 (R1) Given the UI sources, When scanned, Then every `S.<key>`, `data-s` and
  `data-s-label` names a catalog key, `index.html` has no visible text outside the catalog, and
  no UI script outside the catalogs holds a literal with non-ASCII letters.
- AC-3 (R2) Given no saved choice, When the system locale is `es-ES`, `es-MX` or `es`, Then
  the language is Spanish; When it is `en-US`, `fr-FR`, `ca-ES` or empty, Then it is English.
- AC-4 (R3) Given a saved choice `en` and a Spanish system, When the app starts, Then the UI is
  English; Given the choice "system", Then R2 applies.
- AC-5 (R4) Given English, When 1234567 files and the date 2021-03-12 are formatted, Then they
  read `1,234,567` and `Mar 12, 2021` (en-US); in Spanish `1.234.567` and `12 mar 2021`.
- AC-6 (R5) Given Spanish, When the macOS menu is built, Then its items are Spanish (e.g.
  "Edición", "Ventana"); Given English, Then English.
- AC-7 (R6) Given `tauri.conf.json`, Then NSIS languages are Spanish and English and the
  publisher is "VFS".
- AC-8 (R7) Given the query `informacion` in either language, Then it matches `Información`.

## Non-goals

- Languages other than Spanish and English.
- Renaming the product. The name is one catalog key (`appName`) so a later rebrand changes one
  place per language; the owner wants a new name toward a personal knowledge base (follow-up).
- Translating file names, folder names or Firefox's own bookmark folder titles.
- The Windows splash following a saved non-system choice: it draws before the UI runs.

## Pre-mortem

- Slow PC / HDD: n/a because no I/O beyond reading the system locale once.
- Long paths: n/a because no path handling changes.
- NFC/NFD: folding stays in vfs-core and `format.fold`, independent of language (R7, AC-8).
- Antivirus: n/a because no new files are enumerated.
- Kill mid-write: the choice is one key in WebView storage; a lost write falls back to R2.
- Million matches: n/a because result paths are unchanged.
- Permission denied, links: n/a because nothing new is read.
- User files: none opened or written.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Languages and default | es and en; system locale `es*` → es, otherwise en; switchable | owner | 2026-11-21 |
| 2 | Where the selector lives | A new "Ajustes" panel (gear in the status bar); VFS-20/21 settings join it | owner | 2026-11-21 |
| 3 | English product name | Not decided; owner plans a rebrand. Name kept in one key, `VFS — File Finder` meanwhile | owner | 2026-11-21 |
| 4 | Where the choice is stored | WebView `localStorage`; the UI tells the shell the active language at start and on change | agent | 2026-11-21 |
| 5 | System locale source | Rust `sys-locale` (WKWebView's `navigator.language` follows the bundle's localizations, not the system) | agent | 2026-11-21 |

## Open questions

None blocking.
