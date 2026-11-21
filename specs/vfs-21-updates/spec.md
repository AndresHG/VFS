# VFS-21 — App updates: offer on open, install with one click

- **Status**: implemented (PR on branch feat/vfs-21-updates; independent spec review 2026-11-21 added the install-failure path and step order)
- **Size**: L. It changes the installer and release pipeline (signed updater artifacts,
  `latest.json`), adds the app's only network connection, and behaves differently on Windows
  (the NSIS installer replaces the app and the app exits) and macOS (the bundle is replaced
  in place and the app relaunches).
- **Links**: GitHub issue 26, coordination board issue 96, [VFS-1](../vfs-1-core-search/spec.md)
  (AC-14 release artifacts, wording rules), VFS-22 (text catalog).

## Problem

Users install VFS once from a GitHub release and never think about it again. Without an
update mechanism they stay on the version they installed, with its bugs, unless someone
downloads and runs a new installer by hand; on a family member's PC nobody will. The owner
decided: on open, VFS checks once for a newer release on GitHub and offers "Actualizar"; one
click downloads it, verifies its signature and installs it. The check can be turned off, and
because it is the app's only network connection the UI and the README say so.

## What we know (research, 2026-11-21)

- **Tauri updater plugin** (`tauri-plugin-updater` 2.x, docs at v2.tauri.app/plugin/updater,
  crate docs 2.14). With `bundle.createUpdaterArtifacts: true` a signed build produces, next
  to the normal bundles, `VFS.app.tar.gz` + `.sig` (macOS) and `VFS_<v>_x64-setup.exe.sig`
  (Windows NSIS; the setup.exe itself is the update payload). Signing reads
  `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from the environment.
- **Static manifest**: `latest.json` with `version` (SemVer, optional leading `v`), optional
  `notes` and `pub_date`, and `platforms.<os>-<arch>.{url, signature}` where `signature` is
  the `.sig` file's content. The whole file is validated before the version, so every listed
  platform must be complete. An endpoint of
  `https://github.com/<owner>/<repo>/releases/latest/download/latest.json` serves the asset of
  the newest non-draft, non-prerelease release.
- **Rust API**: `app.updater_builder()` takes `version_comparator`, `timeout`, `on_before_exit`;
  `check()` returns `Option<Update>`; `Update::download` returns the bytes and `install(bytes)`
  verifies the minisign signature before installing, so tampered or unsigned bytes are never
  installed. On Windows `install` launches the installer and exits the process
  (`on_before_exit` runs first); on macOS the app must be relaunched. Calling the plugin only
  from Rust needs no capability permission. Endpoints must be HTTPS unless
  `dangerousInsecureTransportProtocol` is set.
- **Today**: `release.yml` builds an unsigned NSIS per-user installer (`installMode:
  currentUser`, no UAC) and an ad-hoc-signed macOS arm64 `.app` zipped, plus `SHA256SUMS.txt`
  per platform. The CSP allows no network from the web view; every other feature is offline.
- **v0.1.1 has no updater**, so users on it update by hand once; from v0.2.0 on, updates are
  automatic.

## Requirements (EARS)

- R1. WHEN the app has started and the update check is on, THE app SHALL request
  `latest.json` from the GitHub release endpoint once per launch, in the background, starting
  10 s after the window has loaded, without delaying the window or searches.
- R2. WHEN the manifest's version is newer than the running version, THE app SHALL show a
  non-modal notice "Hay una versión nueva de VFS (x.y.z)" with "Actualizar" and "Ahora no".
  A pre-release version is offered only to a pre-release build (a comparator rule; the
  endpoint itself never serves pre-releases, D7).
- R3. WHEN the user clicks "Actualizar", THE app SHALL download the update showing progress
  and verify its signature before stopping or replacing anything, then install it and run
  the new version. Windows: stop the indexer, start the NSIS installer (passive progress
  window), exit; the installer reopens VFS. macOS: replace the bundle, stop the indexer,
  relaunch.
- R4. IF the network is unavailable, blocked, slow (no answer within 20 s) or the manifest is
  invalid, THEN THE app SHALL show nothing, log the cause, and try again on the next launch.
- R5. IF the download fails or is interrupted, THEN THE app SHALL install nothing, keep running
  normally, and show "No se ha podido descargar la actualización" with "Reintentar" in the
  same notice. IF the signature does not verify or the install step fails (installer cannot
  start, bundle not writable), THEN THE app SHALL keep running on the current version and show
  "No se ha podido instalar la actualización" with "Reintentar"; if the indexer had already
  been stopped (the Windows installer failed to start), it SHALL instead relaunch the current
  version, so the index session and the window come back whole.
- R6. WHERE the user turns off "Buscar actualizaciones al abrir", THE app SHALL make no network
  connection at all; the setting persists across launches and defaults to on.
- R7. THE settings panel and the README SHALL say that the update check is the app's only
  connection to the internet.
- R8. WHILE the index is scanning, THE app SHALL still offer the update, and installing SHALL
  stop the indexer first so the index is left consistent; the scan resumes after the
  relaunch.
- R9. THE release workflow SHALL produce, for each platform, the signed updater artifact and
  its `.sig`, and a `latest.json` that points at the release assets, keeping today's
  installer, zip and checksum artifacts. On a tag it SHALL refuse a tag that differs from the
  app version and create only a draft release with every asset already attached, so a
  published release never has a partial manifest.

## Acceptance scenarios

Tests cite these ids (`VFS-21 AC-<m>`) in their doc comments.

- AC-1 (R2) Given running 0.2.0, When the manifest says 0.2.1 (or v0.2.1), Then an update is
  offered; When it says 0.2.0 or 0.1.9, Then none is.
- AC-2 (R2) Given running stable 0.2.0, When the manifest says 0.3.0-beta.1, Then none is
  offered; Given running 0.3.0-beta.1, When it says 0.3.0, Then it is.
- AC-3 (R6) Given no settings file, Then the check is on; When the user turns it off, Then
  the choice is saved and read back as off after a restart; Given a corrupt settings file,
  Then the check is on and the file is rewritten on the next change.
- AC-4 (R6) Given the check is off, When the UI asks for an update, Then the app returns "no
  update" without building an updater or opening a connection.
- AC-5 (R1) Given the check already ran in this launch, When the UI asks again (reload),
  Then the stored result is returned and no second request is made.
- AC-6 (R4) Given the endpoint cannot be reached or answers garbage, When the check runs,
  Then the result is "no update" and the error is only logged.
- AC-7 (R5) Given an update was offered, When the download fails or the signature is wrong,
  Then the install command rejects, nothing is installed, the indexer keeps running, and the
  notice shows the failure text with "Reintentar".
- AC-8 (R3, R8) Given an update was offered, When the user installs, Then the index is shut
  down before the installer runs (Windows) or before the relaunch (macOS).
- AC-9 (R9) Given a `workflow_dispatch` run of `release.yml`, Then its artifacts contain the
  NSIS setup.exe + `.sig`, `VFS.app.tar.gz` + `.sig`, the existing zip and checksum files,
  and a `latest.json` with `windows-x86_64` and `darwin-aarch64` entries whose signatures
  equal the `.sig` files and whose URLs name the release assets; the manifest job fails on a
  tag that is not `v<app version>`.
- AC-10 (R3) Manual: a local build of 0.2.0 pointing at a local manifest is updated to a
  local 0.2.1 signed with a test key, and relaunches as 0.2.1 (macOS, recorded in the PR);
  the Windows check is the documented manual step in `docs/updates.md`.
- AC-11 (R2, R7) The notice and the setting copy come from the text catalog, in Spanish.
- AC-12 (R5) Given the download completed, When the signature check or the installer fails,
  Then the install command rejects and the notice shows "No se ha podido instalar la
  actualización" with "Reintentar" (also when the server sent no download size); the current
  version is relaunched only if the indexer had already been stopped.
- AC-13 (R3) Given a download in progress, When progress events arrive, Then the notice shows
  the percentage and a progress bar, and "Instalando…" once the download completes.

## Non-goals

- Detecting metered connections. The check downloads about 1 KB; the multi-MB download
  happens only when the user clicks "Actualizar", so no traffic is spent without consent.
- Background or silent installs, scheduled re-checks while the app stays open, release-notes
  display, delta updates, rollback, downgrades.
- Code signing of the installer or the macOS app (separate work); the updater signature is
  independent of OS code signing.
- Updating v0.1.1 installs (no updater inside); they install v0.2.0 by hand once.
- A proxy setting: the system's default HTTP client behaviour is used.

## Pre-mortem

- Slow PC / spinning HDD: the check is one small HTTPS request, started 10 s after launch so
  it never competes with the first paint; the download is streamed into memory (payload
  about 5–10 MB) and written once by the installer.
- Long paths: n/a because the updater writes only to the temp dir and the install dir.
- NFC/NFD: n/a because no user file names are involved.
- Antivirus: Defender may scan the downloaded unsigned setup.exe and SmartScreen may not
  apply (launched by the app, not double-clicked); if the installer fails to launch, R5's
  install-failure branch applies and the log has the cause. A macOS app run from a
  quarantined, translocated (read-only) location cannot replace itself: same branch.
- Killed mid-write: the indexer is shut down (WAL checkpoint) before install; a kill during
  download leaves nothing behind but memory. While the shutdown runs the notice already says
  "Instalando…"; an interrupted scan resumes or restarts at the next launch as on any close.
- Million-match queries: n/a because search is untouched.
- Permission-denied / placeholders: n/a; per-user install dirs are user-writable, so no UAC.
- User files: no user file is opened, written, moved or deleted; only the app's own install.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Check on open, one click, can be turned off, say it is the only connection | As stated | owner (via coordinator brief) | 2026-11-21 |
| D2 | Where the setting lives | The "Ajustes" panel (VFS-22), below the language | agent | 2026-11-21 |
| D3 | Settings storage | `updates.json` in the local app-data dir, separate from the index DB (deleting or rebuilding the index keeps the choice) | agent | 2026-11-21 |
| D4 | Who runs the check | Rust, when the UI asks after load; Rust enforces the setting and the once-per-launch rule, so a UI bug cannot open a connection | agent | 2026-11-21 |
| D5 | Signing key | minisign key with a password, both stored as Actions secrets `TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)`; the only other copy is on the owner's machine for backup; public key in `tauri.conf.json`. Losing it means one manual reinstall for every user; backup and rotation in `docs/updates.md` | agent | 2026-11-21 |
| D6 | Windows install mode | `passive` (progress window, no questions; per-user so no UAC) | agent | 2026-11-21 |
| D7 | Pre-releases | The endpoint serves only the latest non-prerelease release; the comparator also refuses a pre-release for a stable build | agent | 2026-11-21 |
| D8 | "Ahora no" | Hides the notice for this launch only; the next launch offers again | agent | 2026-11-21 |

## Open questions

None.
