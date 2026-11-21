# VFS-24 — Real-PC readiness: long-path reveal, WebView2 memory, accessibility

- **Status**: approved (reviewed by an independent spec critic under the owner's standing
  instruction, D3)
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
  used on it, Then the Explorer window that action opens shows the file's folder with the file
  among its selected items (read through `Shell.Application`). Both are required: with 8.3
  short names enabled (Windows CI test, C:) and disabled (e2e journey, D:). Given a long path
  with a component that ends in a dot or a space or is a reserved device name (`CON`,
  `NUL.txt`, `COM1`…), Then the Shell is not asked and the nearest folder opens
  (`opened_parent`); the same happens when the Shell cannot build or open the item.
- AC-2 (issue 48) Given the app idle after a search, When the window is minimised for 5 minutes
  and restored, Then the app log says `webview suspended: true`, written from WebView2's own
  `TrySuspend` completion result, and the restored window
  searches and renders its hit; the journey fails otherwise. It also records the working set
  and the private bytes of the app and every WebView2 process: open, at 15 s, 60 s and 300 s
  minimised, and 10 s after restoring. The numbers are a measurement, not a pass condition:
  Windows trims any minimised process's working set, so only the log line proves suspension.
  They are kept in the journey artifact (`journey.json`, `memory_minimised`).
- AC-3 (issue 22) Given the app with results, Then UI Automation (what Narrator reads) exposes
  the search box, the results list and a result row with the names the page gives them.
  Starting from each Windows setting off, the page does not match its media query; with
  high contrast on it matches `forced-colors: active`, with animations off
  `prefers-reduced-motion: reduce`, and it stops matching when the setting is turned back
  off, which happens even when the step fails. Static UI tests check that every control has
  a name, that motion runs only without "reduce motion", and that under forced colours the
  selected row, the folder state dots and the progress bars are drawn with `Highlight`,
  `CanvasText` or an outline (that whole list) rather than a background colour alone.

## Non-goals

- Closing the window to the tray instead of suspending it (issue 37).
- A full WCAG audit, or a test with a real Narrator session.

## Known risks

- Measured on the Windows Server 2025 runner with the webdriver build; the target is a
  Windows 10 PC with the release build. The target-PC measurement plan
  (`docs/research/target-pc-measurement-plan.md`, issue 59, milestone Fase 2) re-runs the
  AC-1, AC-2 and AC-3 checks there with the release build.
- UNC and mapped network paths over 260 characters take the same route (`\\?\UNC\` becomes
  `\\server\share\`) but were not measured.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | How to select a long path (issue 21) | `SHOpenFolderAndSelectItems` with the ID list of the plain path (no `\\?\`). Measured on the Windows runner with a 362-character path (8.3 names on) and a 334-character path (8.3 names off, set with `fsutil 8dot3name set D: 1` in a temporary probe workflow): `ILCreateFromPathW` and `SHParseDisplayName` refuse the `\\?\` form and accept the plain one, and Explorer selects the file. `explorer /select,` stays limited to `MAX_PATH`. Only paths whose plain form names the same item (no component ending in a dot or a space) go this way. Supersedes VFS-8 AC-3's "nearest openable ancestor" for long paths | lead | 2026-11-21 |
| D2 | Where to measure issues 48 and 22 | Steps in the VFS-8 Windows journey (workflow `e2e.yml`), on the installed webdriver build | lead | 2026-11-21 |
| D3 | Approval | The owner's standing instruction: a spec is approved once an independent critic, given only the spec, confirms it makes sense | owner | 2026-11-21 |
