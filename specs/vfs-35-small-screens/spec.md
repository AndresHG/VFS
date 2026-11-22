# VFS-35 — Layout for small and low-resolution screens

- **Status**: approved (scope set by issue 169 and the lead session's brief)
- **Size**: M (the desktop UI only: `styles.css`, the window size check; no IPC, no schema)
- **Links**: issue 169; issue 106 (`fit_window` shrinks the window to the work area);
  VFS-1 R9 (filters stay on screen); docs/design/system.md (Papel)

## Problem

On a Windows laptop with a 1366×768 screen at 125 % scaling the window's content area is about
1093×534 CSS px. The main view does not fit: the bottom bar ("Carpetas (N)", "Ajustes") ends
below the window edge because the whole page scrolls, the filter pills wrap onto two rows and
the search header takes height before any result. In the folders panel, "Añadir carpeta…"
sits at the end of a column that can be taller than the window. A larger system text size
makes all of it worse.

## Requirements (EARS)

- R1. THE bottom bar SHALL always be visible inside the window and never clipped, in the
  start view and the results view, whatever the content above it.
- R2. WHILE the window is short (content height under 40 rem), THE header SHALL be compact
  (less padding, a smaller search box, the start-view prompt kept for screen readers only)
  and THE filter pills SHALL stay on one row that scrolls sideways instead of wrapping (a
  custom date range the user opens still gets its own line). Taller windows keep today's
  layout.
- R3. In the results view, THE results list SHALL take the height left between the filters
  and the bottom bar, and keep at least 14 rem (about three rows): notices above it give way
  first and scroll inside their own area.
- R4. In the folders panel (and every side panel), THE action row with "Añadir carpeta…"
  SHALL stay visible at the bottom; the rest of the panel scrolls above it.
- R5. THE page SHALL never scroll sideways: nothing overflows the window width.
- R6. THE window's default (1100×720) and minimum (720×480) sizes stay; `fit_window`
  (issue 106) already shrinks the window and its minimum to the monitor's work area, so a
  1280×720 screen at 125 % (work area ≈ 1024×544, content ≈ 1024×512) opens at a size that
  fits.

## Acceptance scenarios

Viewports are the content area of a maximised window under Windows 11 (taskbar 48 px, title
bar 32 px, in CSS px), plus the minimum window: 1280×720, 1366×768 and 1920×1080 screens at
100 % and 125 % scaling, and 720×480. Text sizes: normal and large (root text at 125 %).

- AC-1. Given the start view with folders, at each viewport and text size, then "Carpetas (N)"
  and "Ajustes" are fully inside the window and not covered.
- AC-2. Given the results view after a search, at each viewport and text size, then the bottom
  bar buttons are fully visible and at least three result rows are in sight (two at the
  minimum window with large text, where nothing more fits beside a two-line bottom bar).
- AC-3. Given the folders panel open, at each viewport and text size, then "Añadir carpeta…"
  is fully visible and not covered.
- AC-4. Given no folders yet (start view), at each viewport and text size, then the
  "Añadir carpeta…" button of the empty state and the bottom bar are fully visible.
- AC-5. At each viewport and text size, in every state above, the page is no wider than the
  window (no sideways scroll).
- AC-6. Given the start and results views, then the first filter pill is fully visible, and
  on a short window (under 40 rem tall) every filter pill sits on one row.

The checks run in a headless Chrome against the development fake backend
(`apps/desktop/ui-dev/layout.test.mjs`), in the CI "UI tests" step on Windows and macOS.
"Fully visible" means inside the window and on top at the middle of its top edge, its centre
and the middle of its bottom edge (nothing, such as the sticky bottom bar, covers it).

## Non-goals

- A "Filtros" button that folds the filters away (superseded by VFS-38 D1: «Más filtros»
  holds size, folder and source).
- A screenshot in the Windows e2e journey at a small window size (issue 169 sketch): the
  headless viewport checks cover the layout; the e2e workflow is left as it is.
- An in-app text size setting: "large text" here is the system text size, which the UI
  follows through `rem`.
- Changing the window size logic in the shell.

## Decisions

- D1. One scrollable row of filters on short windows (R2), not a "Filtros" button; the issue
  allows either, and the row keeps every filter visible and needs no new copy. VFS-38 D1
  (owner) later moved size, folder and source behind «Más filtros». The sideways scroll
  remains the fallback below 1280×720 and with large text (VFS-38 R10).
- D2. The bottom bar is sticky at the bottom of the page instead of the page losing its
  scroll: on a window too short for everything, the header scrolls away under a visible
  bottom bar rather than being clipped. Only the minimum window at large text gets there.
- D4. On a short and narrow window (under 50 rem wide) a status text that does not fit beside
  the bottom bar's buttons takes its own line (two lines at most) instead of pushing one
  button onto a line of its own.
- D3. Height thresholds use a size container on the app (`@container app (height < 40rem)`),
  so they follow the text size; media queries would measure rem against the browser default.
