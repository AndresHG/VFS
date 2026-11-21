# VFS-8 — The real app, installed and driven on Windows and macOS, and first slow-disk numbers

- **Status**: approved
- **Size**: L (new platform-specific path: installer verification and window automation on two
  operating systems; multi-component: workflow, scripts, a build feature)
- **Links**: VFS-1 (`specs/vfs-1-core-search/spec.md`, AC-9 benchmark, AC-13 reveal, AC-14
  installer), `docs/research/target-pc-measurement-plan.md`, `docs/research/tools/vfs-probe.ps1`,
  issues 17, 18, 19, 21, 32

## Problem

The Windows installer has been built but never installed, and nobody has seen the real window on
either operating system: the dev Mac cannot run Windows binaries and its agents have no screen
capture. Every claim about the installed product (per-user install, Start menu entry, the
WebView2 bootstrapper, "Mostrar en carpeta" opening Explorer on the right file, catch-up at
start) is unverified, and nothing has been measured on a disk that is busy the way the target's
single 8 TB HDD is. The probe script for the reference-machine field test has never run on Windows either.

## Requirements (EARS)

- R1. WHEN the e2e workflow runs (manual dispatch or nightly), THE `windows-journey` job SHALL
  install the NSIS installer silently as the runner user and verify what was installed.
- R2. WHEN the app is installed, THE journey SHALL drive an installed build through the user's
  journey with W3C WebDriver and capture a screenshot of the window and of the whole desktop at
  every step; THE installed release build SHALL get a launch-and-screenshot smoke test.
- R3. WHEN the e2e workflow runs, THE `macos-journey` job SHALL drive a built `.app` through a
  short version of the journey, with screenshots.
- R4. WHEN the slow-disk job runs, THE job SHALL measure first scan, query latency, start-up
  catch-up and memory on a separate virtual disk, once quiet and once under a competing
  workload, and the 2,000,000-file synthetic benchmark under both conditions.
- R5. WHEN the probe job runs, THE job SHALL run `vfs-probe.ps1` as a standard (non-admin)
  user and keep its output.
- R6. IF a step fails, THEN THE job SHALL still upload the screenshots, logs and timings gathered
  so far.
- R7. THE automation hook (the embedded WebDriver server) SHALL be compiled only with the
  opt-in `webdriver` cargo feature; release builds and the shipped installer SHALL NOT contain
  it.

## Acceptance scenarios

Tests cite these ids (`VFS-8 AC-<m>`) in their doc comments.

- AC-1 (R1) Given the installer from this commit, When it runs with `/S`, Then the app is under
  `%LOCALAPPDATA%\VFS`, a Start menu shortcut and an uninstaller exist, the uninstall entry is
  under HKCU (not HKLM), the installer process did not ask for elevation, and a WebView2
  runtime is present afterwards; after the journey, the uninstaller removes the app.
- AC-2 (R2) Given a generated tree of about 100,000 files with Spanish names, accents, deep
  paths and some paths over 260 characters, When the journey adds it, Then the first scan shows
  progress, a search during the scan returns hits, and searches by name, by a middle path
  segment, without accents, with one typo, with each filter, and for a folder each return the
  planted target.
- AC-3 (R2) Given a planted file, When "Mostrar en carpeta" is used on its hit, Then an
  Explorer window opens on the file's folder with the file selected (or, for a path over 260
  characters, on the nearest openable ancestor).
- AC-4 (R2) Given the app open, When files are created, renamed and deleted in the tree, Then
  searches follow; and Given the app closed and the tree changed, When it reopens, Then the
  catch-up at start makes the changes searchable.
- AC-5 (R3) Given the macOS `.app`, When the short journey runs (first run, add folder, scan,
  search, filter, reveal), Then each step has a screenshot.
- AC-6 (R4) Given the slow-disk job, Then it writes one JSON report per condition with first-scan
  seconds and files/s, in-app query latencies, catch-up seconds, app and WebView2 memory, and
  `vfs bench --json` for the 2 M index.
- AC-7 (R5) Given a standard user created in the job, When the probe runs as that user, Then its
  JSON output (including which change-journal calls succeed) is uploaded.
- AC-8 (R2) The tree generator is deterministic and produces the shape AC-2 relies on (count,
  accents, NFD names, paths over 260 characters, planted targets); unit tests prove it.
- AC-9 (R7) Given a default build, Then the WebDriver plugin is not in the dependency graph.
- AC-10 (R2, issue 48) Given the app idle after a search, When the window is minimised for
  5 minutes and restored, Then the journey records app and WebView2 memory open, at 15 s,
  60 s and 300 s minimised, and 10 s after restoring; the app log says whether WebView2
  suspended (VFS-5 AC-12); and the restored window still searches and renders.
- AC-11 (R2, issue 22) Given the app with results, Then UI Automation (what Narrator reads)
  exposes the search box, the results list and each result row with their Spanish names;
  When Windows high contrast is turned on, Then the page matches `forced-colors: active` and a
  screenshot is kept; When Windows animations are turned off, Then the page matches
  `prefers-reduced-motion: reduce`. Both settings are restored afterwards.

## Non-goals

- No change to product behaviour. The only product-code change is the feature-gated plugin.
- Not an every-push check: the workflow is manual and nightly, because Windows minutes cost 2×
  and macOS 10× in a private repository.
- No emulation of HDD seek latency: no credible tool exists on a hosted Windows runner without a
  custom driver (see Decisions). Contention is measured, latency is not.
- Signing, auto-update, a real reference machine (Fase 2 field test).

## Pre-mortem

- Wrong platform behaviour: the journey runs on the real OS for each platform; nothing is
  inferred from the other one.
- Native dialogs: WebDriver cannot drive the native folder picker. On Windows the job drives it
  with UI Automation; if that fails the journey falls back to the same `add_root` command the
  dialog leads to, and records which path it took.
- Release build not drivable (it happened, decision 1): the journey runs against an installed
  `webdriver`-feature build and the installed release build gets a launch-and-screenshot smoke
  test.
- Index location: the slow-disk job relocates `%LOCALAPPDATA%\app.vfs.desktop` with a directory
  junction to the virtual disk; the app is unchanged.
- Cost: jobs are capped with `timeout-minutes`; macOS is one short job.
- Flaky timing on shared runners: every number is reported with the runner's hardware, as
  measured on that run, never as a guarantee.
- User data: the jobs only touch generated trees on the runner.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | How to drive the window on Windows | Tried first: `tauri-driver` 2.1.0 + Edge WebDriver matching the runner's WebView2 (157.0.4341.46), against the installed release build. The app started but the session failed with "DevToolsActivePort file doesn't exist". So the journey drives an installed NSIS build of the same app with the `webdriver` feature (decision 2), and the release installer gets install/verify, a launch-and-screenshot smoke test and uninstall; the smoke test also records whether the release build honours `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, the switch Edge WebDriver relies on | session | 2026-11-20 |
| 2 | How to drive it on macOS (no WKWebView driver exists) and, after 1, on Windows | `tauri-plugin-wdio-webdriver` (embedded W3C server, the free provider the Tauri docs point to) behind the opt-in `webdriver` cargo feature; the journey talks raw W3C WebDriver from Python, no npm test stack | session | 2026-11-20 |
| 3 | Disk throttling | None credible on a hosted runner: Hyper-V QoS needs a VM (nested virtualization unsupported), Storage QoS needs a cluster, job-object I/O rate control is documented as unsupported since 1607. Use a VHDX for isolation and cold cache (dismount/remount) plus a competing workload (DiskSpd at a capped rate and a sync/antivirus emulator touching the tree) | session | 2026-11-20 |
| 4 | Trigger | `workflow_dispatch` + nightly `schedule`; not on push | brief | 2026-11-20 |

## Open questions

None blocking.
