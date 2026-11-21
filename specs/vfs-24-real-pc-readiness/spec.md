# VFS-24 — Real-PC readiness: long-path reveal, WebView2 memory, accessibility

- **Status**: approved
- **Size**: M
- **Links**: VFS-1 AC-13, VFS-5 AC-12, VFS-8 (`specs/vfs-8-windows-e2e/spec.md`); issues 21, 48, 22

## Problem

Three Windows behaviors were built but never checked on Windows, or were knowingly left weak:

- Issue 21: "Mostrar en carpeta" on a path over 260 characters opened the nearest folder
  Explorer could address, with nothing selected.
- Issue 48: minimising the window hides and suspends WebView2 (VFS-5 AC-12), but those calls
  had never run, and nothing measured the memory they free.
- Issue 22: screen-reader names, Windows high contrast and reduced motion were never verified.

## Acceptance scenarios

- AC-1 (issue 21) Given a file whose path is over 260 characters, When "Mostrar en carpeta" is
  used on it, Then Explorer shows it selected in its folder, with 8.3 short names enabled or
  disabled on the volume. Only if the Shell cannot address it does the nearest folder open
  (`opened_parent`).
- AC-2 (issue 48) Given the app idle after a search, When the window is minimised for 5 minutes
  and restored, Then the Windows e2e journey records app and WebView2 memory: open, at 15 s,
  60 s and 300 s minimised, and 10 s after restoring. The app log says whether WebView2
  suspended, and the restored window still searches and renders.
- AC-3 (issue 22) Given the app with results, Then UI Automation (what Narrator reads) exposes
  the search box, the results list and a result row with the names the page gives them. When
  Windows high contrast is on, the page matches `forced-colors: active`; when Windows
  animations are off, it matches `prefers-reduced-motion: reduce`. Both settings are restored
  afterwards. Static UI tests check that every control has a name, that motion runs only
  without "reduce motion", and that high contrast keeps the selected row, the folder state
  dots and the progress bars visible.

## Non-goals

- Closing the window to the tray instead of suspending it (issue 37).
- A full WCAG audit, or a test with a real Narrator session.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | How to select a long path (issue 21) | `SHOpenFolderAndSelectItems` with the ID list of the plain path (no `\\?\`). Measured on the Windows runner with a 362-character path (8.3 names on) and a 334-character path (8.3 names off): `ILCreateFromPathW` and `SHParseDisplayName` refuse the `\\?\` form and accept the plain one, and Explorer selects the file. `explorer /select,` stays limited to `MAX_PATH`. Only paths whose plain form names the same item (no component ending in a dot or a space) go this way. Supersedes VFS-8 AC-3's "nearest openable ancestor" for long paths | lead | 2026-11-21 |
| D2 | Where to measure issues 48 and 22 | Steps in the VFS-8 Windows journey (workflow `e2e.yml`), on the installed webdriver build | lead | 2026-11-21 |
