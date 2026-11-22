# VFS-27 — Content search in the app

- **Status**: approved. The owner decided D9–D13 of VFS-2 and D1 here. An independent critic
  review asked for revisions, and all of them are made here. PR 27a is implemented. 27b
  ships in two PRs: 27b-1 (the engine's gate and pace policy, the probes, paused folders,
  sleep) and 27b-2 (the app's background indexer); 27b-1 is implemented.
- **Size**: L. It ships a second executable in both installers, opens user files from the app
  for the first time, and adds platform-specific idle and disk probes. It is multi-PR.
- **Links**:
  - [VFS-2](../vfs-2-content-search/spec.md), the engine: `retrieve`, `unread`, `content.db`
    and the `vfs-extract` worker (R15, R17, R18).
  - [VFS-5](../vfs-5-windows-tuning/spec.md): priority policy, the `DiskLoad` probe and the
    diagnostic log.
  - [VFS-25](../vfs-25-cloud-only-files/spec.md): cloud-only files.
  - [VFS-21](../vfs-21-updates/spec.md): the updater.
  - [VFS-22](../vfs-22-i18n/spec.md): both string catalogs.
  - [VFS-32](../vfs-32-pause-root/spec.md): pausing a folder.
  - Research: `docs/research/content-vector-rag.md` §9–§11.
  - Issues #23, #25.

## Problem

VFS-2 builds the content engine, but only the CLI can reach it. The owner's archive holds tens
or hundreds of thousands of documents that are only findable by a word inside them. On the
target PC (slow CPU, one busy HDD, Defender, Synology syncing), reading them all takes hours
to days **[estimate, VFS-2 measurements: 0.3 s per document, 5 h per 60,000]**, and the store
grows by ~1 GB per 60,000 documents on the system drive. So the user must choose it knowingly.
It must never get in the way of name search or of the person using the PC, and its progress
must be visible and pausable. The owner's decisions (VFS-2 D9–D13) say how:
- it is offered after the name scan, with estimates;
- its hits appear in a separate section;
- it throttles while the PC is in use;
- cloud-only files are never opened;
- scanned PDFs are marked.

## Requirements (EARS)

Packaging and lifecycle

- R1. THE Windows installer and the macOS app SHALL include `vfs-extract` as a Tauri sidecar,
  next to the app's own executable (D1). THE updater SHALL replace it together with the app.
  The app SHALL refuse a worker whose version differs from its own: per VFS-2 R17, the run
  stops with the reason «el lector de documentos no es de esta versión».
- R2. WHILE content search is off, THE app SHALL behave exactly as before: it creates no
  `content.db`, opens no document and starts no worker (VFS-2 R8).
- R3. WHEN the user installs an update, THE app SHALL stop the content run and wait for the
  worker to exit (at most 5 s, then kill it) before the installer starts. This extends
  VFS-21 R8 from the indexer to the worker.

Offer and settings

- R4. WHEN the app starts or a first name scan finishes, IF every folder has finished its
  first name scan and content search has never been offered, THEN THE app SHALL offer it
  once. This includes installs upgraded from 0.2.x. The offer shows:
  - the number of documents it would read, excluding cloud-only files;
  - the estimated time, as a range;
  - the estimated disk use on the drive that holds the app data;
  - the choices «Activar» and «Ahora no».
  «Ahora no» SHALL not ask again; the switch in Ajustes stays available.
- R5. THE Ajustes panel SHALL have:
  - a switch «Buscar también dentro de los documentos»;
  - the current size of the content store;
  - the unread counts (R13);
  - a button «Borrar lo leído de los documentos», which asks for confirmation, then deletes
    `content.db` and switches content search off. Name search is not touched (VFS-2 R9).
  Turning the switch off stops the run and hides the content section, but keeps the store.
  Turning it on again clears any pause and resumes (D4).
- R6. IF free space on the app-data drive is below the larger of 2 GB and twice the estimated
  store size when the offer is shown, THEN the offer SHALL warn about it. WHILE content
  indexing runs, IF free space drops below 1 GB, THEN THE app SHALL pause content indexing
  with the reason «poco espacio en disco». It SHALL resume by itself once free space is above
  2 GB, or when the user resumes it. On macOS, free space is the volume's available capacity
  for important usage (it counts purgeable space), not `statvfs`.

Background pace (owner decision T1)

- R7. WHILE content indexing runs, THE app SHALL extract one document at a time through the
  worker at the lowest priority (VFS-2 R5), and SHALL apply these rules in order:
  1. WHILE a name walk runs, or a name pass of more than `incremental_max_dirs` folders,
     content indexing waits.
  2. On Windows, the content scheduler samples the `DiskLoad` probe (VFS-5) for the drive
     being read, between documents only (so the worker's own read is not counted). It uses
     VFS-5's busy rule (`window` / `busy_in_window`, 3 of the last 5 samples) and its
     guarantee: after waiting `max_backoff`, it reads for at least `min_work` before it may
     wait again. So a disk that a sync client keeps busy slows content down but never stops
     it. macOS has no disk probe, so this rule does not apply there.
  3. IF the last keyboard or mouse input was 60 s ago or less, THEN there is a pause of 15 s
     between documents. Over 60 s, there is no pause.
  4. IF the idle probe fails, THEN the user counts as active.
  The idle probe is `GetLastInputInfo` against `GetTickCount64` on Windows, and
  `CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateHIDSystemState,
  kCGAnyInputEventType)` on macOS. Neither needs administrator rights.
- R8. THE app SHALL show content progress in the status area:
  - documents read out of the total;
  - documents per hour, computed over working time only, not pauses or waits;
  - the time left, from the last 200 documents, or from the D3 estimate until 200 have been
    read;
  - «Pausar» / «Reanudar».
  A pause holds across restarts until the user resumes it. WHEN a folder is paused
  (VFS-32), THE content run SHALL skip that folder's documents until it is resumed.
- R9. WHEN the app closes or the machine sleeps mid-run, THE next start or wake SHALL continue
  from the unread documents (VFS-2 R4), and SHALL not show the offer again. WHILE a folder is
  offline, its documents are skipped and the others are read (VFS-2 R17). WHEN it comes
  back, its documents are read without any action from the user. A document whose extraction spanned a sleep SHALL be retried,
  not marked failed by timeout.
- R10. WHEN the name indexer adds or changes documents after a content run has finished,
  THE app SHALL queue them and read them under the same pace rules. The content run is
  "finished" when its queue is empty.
- R11. THE app SHALL write a "content run open" marker in the app data folder when a content
  run starts, and remove it on a clean stop. WHEN the app starts and finds the marker, THE
  app SHALL run SQLite's `quick_check` on the content store in the background (it can take
  tens of seconds on an HDD), and SHALL not start content indexing until it ends. Content
  search is available meanwhile. IF the check fails, or the store's version differs (VFS-2
  R19), THEN THE app SHALL tell the user «El índice de contenido está dañado», and offer
  «Volver a leer los documentos» (delete the store and start again) or «Desactivar». Nothing
  is deleted without that choice. Name search keeps working meanwhile.

Search

- R12. WHEN the user searches with content search on and the scope «Todo» (VFS-38 R2), THE
  app SHALL show the name and path hits first (under «Solo documentos», VFS-38 R4, only the
  section below is shown, with the same rules), without waiting for content. It SHALL then show, in a separate section «En el
  contenido de los documentos» below them, up to 20 content hits from `retrieve`.
  - Each hit shows the file name, the folder, the highlighted fragment, the page when known,
    and «Mostrar en carpeta».
  - A «Ver más» link loads the next 20 (`retrieve` with `offset`).
  - The section respects the active type, date, size and folder filters (VFS-2 R7).
  - The content query starts 300 ms after typing stops, and only for queries of at least 3
    characters.
  - Each query carries a sequence number. THE app SHALL show only the result of the newest
    query and discard older results when they arrive; `retrieve` itself is not interrupted.
- R13. WHILE the content queue is not empty, THE section SHALL say «Todavía leyendo
  documentos: N de M», so that missing hits are not mistaken for absence. IF `retrieve` marks
  the result partial (VFS-2 R13), THEN THE section SHALL say «Ningún documento tiene todas
  las palabras; estos tienen alguna».
- R14. THE app SHALL show the unread counts by reason from `unread_counts()` (VFS-2 R15):
  «en la nube», «sin texto (escaneado)», «protegido con contraseña», «demasiado grande»,
  «formato no compatible» and «no se pudo leer», mapped as in VFS-2 R15's table. Documents not
  yet read are counted apart, as «pendientes». WHEN a name hit is a document with an unread reason, THE hit SHALL carry that
  reason as a small label, looked up with `unread(file_ids)` for the visible page only.

Run-level failures and diagnostics

- R15. WHEN the run stops for a run-level reason (VFS-2 R17: worker missing or blocked,
  version mismatch, store not writable, worker unstable), THE status area
  SHALL show the reason in plain words with «Reintentar». Per-file states are not touched.
- R16. THE diagnostic log SHALL record, with no file or folder name (VFS-5 R8):
  - each content run's start and stop;
  - each pause and its reason;
  - documents per hour;
  - failures by reason;
  - the store size.

## Acceptance scenarios

Tests cite these ids (`VFS-27 AC-<m>`) in their doc comments.

- AC-1 (R1) Given a release build on each CI platform, When the bundle is produced, Then
  `vfs-extract` is inside it next to the app's executable, the NSIS installer is at most
  16 MB, and the app finds and starts the worker from there. Given a worker reporting a
  different version, Then the run stops with the version reason.
- AC-2 (R2) Given content search was never switched on, When folders are indexed and
  searched, Then no `content.db` exists, no worker was started and no document was opened.
- AC-3 (R3) Given a content run in progress, When an update is installed, Then the worker
  has exited before the installer starts.
- AC-4 (R4) Given a fresh install with 3 folders, When the last first scan finishes, Then the
  offer appears once with the document count (cloud-only files excluded), a time range and a
  disk estimate. Given an upgraded install whose scans finished under 0.2.1, When it first
  starts, Then the offer appears. When the user picks «Ahora no» and restarts, Then the
  offer does not appear again, and the switch in Ajustes is off.
- AC-5 (R5) Given content search on with a store, When the user confirms «Borrar lo leído de
  los documentos», Then `content.db` is gone, the switch is off, the section no longer
  appears, and name search returns the same hits as before. Given a paused run, When the
  switch is turned off and on, Then the run resumes.
- AC-6 (R6) Given a simulated free space of 0.9 GB, When content indexing runs, Then it
  pauses with the low-space reason. At a simulated 1.5 GB it stays paused. At 2.5 GB it
  resumes.
- AC-7 (R7) Given a fake clock and fake idle and `DiskLoad` probes:
  - When a name walk is running, Then no document is extracted.
  - On Windows, when 3 of the last 5 probe samples are busy, Then no document is extracted.
    When the probe stays busy for longer than `max_backoff`, Then a document is still
    extracted within `max_backoff` + `min_work`.
  - When input happened 10 s or 60 s ago, Then documents are extracted at least 15 s apart.
  - When input happened 61 s ago, Then they are extracted back to back.
  - When the idle probe errors, Then the 15 s pause applies.
- AC-8 (R8, R9) Given a content run of 1,000 documents, When the user pauses after 300 and
  restarts the app, Then it stays paused with 300 of 1,000 shown. When resumed, Then it reads
  only the 700 left, and the rate shown excludes the pause. Given a folder paused through
  VFS-32, Then none of its documents is read. Given a document in flight when the machine
  sleeps for 2 min, Then after waking it is retried, not failed. Given one of two folders
  offline, Then the other's documents are read. When the offline folder returns, Then its
  documents are read with no user action.
- AC-9 (R10) Given a finished content run, When a new document is added to an indexed folder,
  Then after the name pass it is read, and a content search finds it.
- AC-10 (R11) Given a `content.db` corrupted on purpose and the run-open marker present, When
  the app starts, Then the window is usable while the check runs, and the damage message
  appears when it ends. Name search works, and the store is
  deleted only after «Volver a leer los documentos».
- AC-11 (R12) Given an indexed folder where `factura.pdf` contains «Peñalara» in its text but
  not in its name, When the user types `peñalara`, Then the name hits render before the
  content query starts. The content section then lists `factura.pdf` with the word
  highlighted and its page. When a newer query is typed while the first is running, Then the
  first result is never shown. Given hits in two folders and kinds, When a folder or type
  filter is active, Then only hits that pass it are shown.
- AC-12 (R13) Given content indexing at 40 of 100 documents, When the user searches, Then the
  section shows «Todavía leyendo documentos: 40 de 100».
- AC-13 (R14) Given a cloud-only DOCX, a scanned PDF, a password-protected PDF, a `.pages`
  file and a corrupt DOC, When content indexing finishes, Then Ajustes shows one document
  under each reason, the five name hits carry their labels, and the cloud-only file was never opened.
- AC-14 (R15) Given the `vfs-extract` binary removed from the bundle, When content indexing
  starts, Then the status area shows the worker reason with «Reintentar», and no document
  changes state.
- AC-15 (R16) Given a content run, When the diagnostic log is read, Then it has the start,
  pause and stop records with rates and counts, and no file or folder name (the VFS-5
  redaction test, extended).
- AC-16 (all) Given the Windows and macOS e2e harnesses (VFS-8), When a folder of the VFS-2
  fixtures is added, content search is accepted and the run finishes, Then a content query
  finds each fixture through the app's command.

## Non-goals

- OCR (issue #28, later and optional per folder, VFS-2 D13).
- Hybrid search and embeddings (VFS-29), and questions to a model (VFS-28).
- Content search over bookmarks (VFS-6) or other sources.
- Per-folder content switches: content search is on or off for all folders. VFS-32's pause
  is the per-folder control.
- Indexing while the app is closed. There is no background service, so content indexing
  runs only while VFS runs.
- A disk-busy rule on macOS, which has no probe (VFS-5 R6).
- Tantivy. It is added only if the real archive exceeds ~100,000 documents (VFS-2 D4), as a
  separate PR behind `retrieve`.

## Pre-mortem

- **Slow PC / HDD.**
  - One document at a time, at the lowest priority.
  - Content waits for name walks and, on Windows, for a busy disk (R7).
  - The run is resumable (R9).
  - Estimates come from D3 and are replaced by the real-PC measurement (27m).
- **Paths over 260 characters.** The worker uses VFS-1's long-path handling. AC-16 covers a
  long-path fixture on the Windows runner.
- **NFC/NFD.** VFS-2's fold. The UI shows the stored name.
- **Antivirus.**
  - Defender scans each opened document. That cost is measured on the real PC (27m).
  - A quarantined or blocked `vfs-extract` stops the run with a reason. It does not mark
    documents failed (R15, VFS-2 R17).
  - `content.db` lives in the app data folder.
- **Kill / power cut.**
  - SQLite in WAL mode; an uncommitted document is read again.
  - Damage is detected and repaired only with the user's yes (R11).
  - The worker dies with its parent (VFS-2 R18).
- **Huge result sets.** 20 content hits per page; VFS-2 R14 bounds the ranking work.
- **Permission-denied, locked files, cloud placeholders, offline folders.**
  - Cloud-only files are never opened.
  - Locked or denied files are «no se pudo leer», retried when they change.
  - An offline folder's documents are skipped, not failed, and are read when it returns
    (R9, VFS-2 R17).
- **User files.** The worker opens them read-only, and nothing is written, moved or deleted
  (VFS-2 AC-1).

## Phased PRs

| PR | Scope | Proves | Effort |
|---|---|---|---|
| 27a | `vfs-extract` as a Tauri sidecar in both bundles, built with VFS-2 D16's size profile by the CI release job before `tauri build`. Also the version handshake, the worker stop before an update, and the installer size check. | AC-1, AC-3 | 2–3 days **[estimate]** |
| 27b | Background content indexer in the app: the pace rules with the idle and `DiskLoad` probes (exposed from `vfs-core`), pause and resume, VFS-32 interplay, progress and time left, the low-space pause, the queue for new documents, damage detection, run-level reasons, Tauri commands, diagnostics. | AC-2, AC-6–AC-10, AC-14, AC-15 | 5–7 days **[estimate]** |
| 27c | User interface: the offer, the Ajustes switch, store size, delete and unread counts, labels on name hits, and the content section with debounce and sequence numbers. It builds on VFS-38 38b's filter line and adds its «Solo documentos» button (VFS-38 R4, R5, AC-4). Strings go in both catalogs. Also the e2e scenario. | AC-4, AC-5, AC-11–AC-13, AC-16; VFS-38 AC-4 | 4–5 days **[estimate]** |
| 27m | Real-PC measurement (docs and probe script): document count and bytes under the real folders, open-and-read time for 1,000 documents with Defender on, and free space on C:. Before 27c ships. | D3 | 1 day + the field visit |

Total ≈ 12–16 working days, plus the field measurement **[estimate]**.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | How does `vfs-extract` reach the user? | Inside the installer as a Tauri sidecar. Content search then works with no connection and updates in lockstep. The cost is ≈ 6.6 MB compressed, so the NSIS installer goes from 6 MB to about 12.5 MB **[measured size, estimated installer]**. Downloading on demand would save that but break offline use. It would also leave an unsigned exe in a user-writable folder. | owner | 2026-11-22 |
| D2 | Pace numbers | Input within the last 60 s means "in use": 15 s between documents (Copernic's published setting). Over 60 s, full pace. Name walks and, on Windows, the busy disk always win. Tuned later from the diagnostic log. | agent (owner set the policy, T1) | 2026-11-22 |
| D3 | Estimates in the offer | Time: 0.17–0.55 s per document. Disk: 13–16 MB per 1,000 documents (VFS-2 measurements, target-PC estimate). Replaced by 27m's numbers, and by the measured rate after 200 documents. | agent | 2026-11-22 |
| D4 | Switch off vs delete | The switch stops the run and hides the section but keeps the store. Turning it on again resumes. Deleting is a separate confirmed button. | agent | 2026-11-22 |
| D5 | Cancelling a stale content query | Discard by sequence number. `retrieve` runs to completion (VFS-2 R14 bounds it), so the engine interface needs no cancel token. | agent | 2026-11-22 |

## Open questions

None blocking. 27m's numbers change only constants in D3, never an interface or the store.
