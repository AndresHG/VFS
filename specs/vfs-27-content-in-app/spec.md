# VFS-27 — Content search in the app

- **Status**: approved (owner decisions D9–D13 of VFS-2 and D1–D3 here; independent critic
  review)
- **Size**: L. It ships a second executable in both installers, opens user files from the app
  for the first time, and adds platform-specific idle detection. It is multi-PR.
- **Links**:
  - [VFS-2](../vfs-2-content-search/spec.md), the engine: `retrieve`, `content.db` and the
    `vfs-extract` worker.
  - [VFS-5](../vfs-5-windows-tuning/spec.md): priority policy, the busy-disk back-off and the
    diagnostic log.
  - [VFS-25](../vfs-25-cloud-only-files/spec.md): cloud-only files.
  - [VFS-21](../vfs-21-updates/spec.md): the updater.
  - [VFS-22](../vfs-22-i18n/spec.md): both string catalogs.
  - [VFS-32](../vfs-32-pause-root/spec.md): the per-folder pause pattern.
  - Research: `docs/research/content-vector-rag.md` §9–§11.
  - Issues #23, #25.

## Problem

VFS-2 builds the content engine, but only the CLI can reach it. The owner's archive holds tens
or hundreds of thousands of documents that are only findable by a word inside them. On the
target PC (slow CPU, one busy HDD, Defender, Synology syncing), reading them all takes hours
to days **[estimate, VFS-2 measurements: 0.3 s per document, 5 h per 60,000]**, and the store
grows by ~1 GB per 60,000 documents on the system drive. So the user must choose it knowingly.
It must never get in the way of name search or of the person using the PC. And its progress
must be visible and pausable. The owner's decisions (VFS-2 D9–D13) say how: offered after
the name scan with estimates, a separate result section, throttling while the PC is in use,
cloud-only files never opened, scanned PDFs marked.

## Requirements (EARS)

Packaging and lifecycle

- R1. THE Windows installer and the macOS app SHALL include the `vfs-extract` executable
  next to the app's own executable (D1). THE updater SHALL replace it together with the app.
  The app SHALL refuse to start a worker whose version differs from its own.
- R2. WHILE content search is off, THE app SHALL behave exactly as before: it creates no
  `content.db`, opens no document and starts no worker (VFS-2 R8).

Offer and settings

- R3. WHEN the first name scan of every folder has finished and content search has never been
  offered, THE app SHALL offer it once. The offer shows the number of documents it would read
  (excluding cloud-only files), the estimated time as a range, and the estimated disk use on
  the drive that holds the app data, with the choices «Activar» and «Ahora no». «Ahora no»
  SHALL not ask again; the switch in Ajustes stays available.
- R4. THE Ajustes panel SHALL have a switch «Buscar también dentro de los documentos», the
  current size of the content store, and a button «Borrar lo leído de los documentos». The
  button asks for confirmation, then deletes `content.db` and switches content search off.
  Name search is not touched (VFS-2 R9).
- R5. IF free space on the app-data drive is below the larger of 2 GB and twice the estimated
  store size when the offer is shown, THEN the offer SHALL warn about it. WHILE content
  indexing runs, IF free space drops below 1 GB, THEN THE app SHALL pause content indexing
  and say why. Indexing resumes when space returns or the user resumes it.

Background pace (owner decision T1)

- R6. WHILE content indexing runs, THE app SHALL extract one document at a time through the
  worker at the lowest priority (VFS-2 R5), and SHALL apply this pace:
  - Name indexing comes first. WHILE a name walk or a pass of more than
    `incremental_max_dirs` folders runs, content indexing waits.
  - WHILE VFS-5's busy-disk back-off is active, content indexing waits.
  - WHILE the user has used the keyboard or mouse in the last 60 s, there is a pause of 15 s
    between documents.
  - After 2 min without input, content indexing runs at full pace (no pause between
    documents).
- R7. THE app SHALL show content progress in the status area: documents read out of the
  total, documents per hour, and the time left. The rate is computed from the last 200
  documents, or from the VFS-2 estimate until 200 have been read. The status area also offers
  «Pausar» / «Reanudar». A pause holds across restarts until the user resumes.
- R8. WHEN the app closes or the machine sleeps mid-run, THE next start SHALL continue from
  the unread documents (VFS-2 R4), and SHALL not show the offer again.

Search

- R9. WHEN the user searches with content search on, THE app SHALL show the name and path hits
  first, without waiting for content. It SHALL then show, in a separate section «En el
  contenido de los documentos» below them, up to 20 content hits from `retrieve`. Each hit
  shows the file name, the folder, the highlighted fragment, the page when known, and «Mostrar
  en carpeta». A «Ver más» link loads the next 20. The section respects the active type,
  date, size and folder filters (VFS-2 R7).
- R10. THE content query SHALL start 300 ms after typing stops, only for queries of at least 3
  characters. A newer query SHALL cancel a running content query, and a stale result SHALL
  never replace a newer one.
- R11. WHILE content indexing has not finished, THE section SHALL say «Todavía leyendo
  documentos: N de M», so that missing hits are not mistaken for absence. IF `retrieve`
  marks the result partial (VFS-2 R13), THEN THE section SHALL say «Ningún documento tiene
  todas las palabras; estos tienen alguna».

Documents that were not read

- R12. THE app SHALL count, and show in Ajustes next to the store size, the documents left
  unread by reason: «en la nube» (cloud-only, never opened, VFS-2 R12), «sin texto
  (escaneado)» (a PDF with no text layer), «protegido con contraseña», «demasiado grande» and
  «no se pudo leer». WHEN a name hit is a document left unread for one of these reasons, THE
  hit SHALL carry that reason as a small label.

Privacy and diagnostics

- R13. THE diagnostic log SHALL record each content run's start, stop, pause reason, documents
  per hour, failures by reason, and store size, with no file or folder name (VFS-5 R8).

## Acceptance scenarios

Tests cite these ids (`VFS-27 AC-<m>`) in their doc comments.

- AC-1 (R1) Given a release build on each CI platform, When the bundle is produced, Then
  `vfs-extract` is inside it next to the app's executable, and the app finds and starts it
  from there. Given a worker reporting a different version, Then the app does not use it and
  reports the mismatch in the log.
- AC-2 (R2) Given content search was never switched on, When folders are indexed and
  searched, Then no `content.db` exists, no worker process was started, and no document was
  opened. This is the existing VFS-2 AC-10 check, run through the app's commands.
- AC-3 (R3) Given a fresh install with 3 folders, When the last first scan finishes, Then the
  offer appears once with the document count (cloud-only files excluded), a time range and a
  disk estimate. When the user picks «Ahora no» and restarts, Then the offer does not appear
  again, and the switch in Ajustes is off.
- AC-4 (R4) Given content search on with a store, When the user confirms «Borrar lo leído de
  los documentos», Then `content.db` is gone, the switch is off, the content section no
  longer appears, and name search returns the same hits as before.
- AC-5 (R5) Given a simulated free space of 0.9 GB on the app-data drive, When content
  indexing runs, Then it pauses with the low-space reason. When space is simulated back to
  5 GB, Then it resumes.
- AC-6 (R6) Given a fake clock and fake input and disk signals:
  - When a name walk is running, Then no document is extracted.
  - When the disk is busy, Then no document is extracted.
  - When input happened 10 s ago, Then documents are extracted at least 15 s apart.
  - When the last input was 3 min ago, Then documents are extracted back to back.
- AC-7 (R7, R8) Given a content run of 1,000 documents, When the user pauses after 300 and
  restarts the app, Then it stays paused with 300 of 1,000 shown. When resumed, it reads only
  the 700 left, and the time left is computed from the measured rate.
- AC-8 (R9, R10) Given an indexed folder where `factura.pdf` contains «Peñalara» in its text
  but not in its name, When the user types `peñalara`, Then the name hits render before the
  content query starts. The content section then lists `factura.pdf` with the word
  highlighted and its page. When the user types a new query while a content query is
  running, Then the first query is cancelled and its result never appears.
- AC-9 (R9) Given content hits in two folders and kinds, When a folder or type filter is
  active, Then the content section shows only hits that pass it.
- AC-10 (R11) Given content indexing at 40 of 100 documents, When the user searches, Then the
  section shows «Todavía leyendo documentos: 40 de 100».
- AC-11 (R12) Given a cloud-only DOCX, a scanned PDF, a password-protected PDF and a corrupt
  DOC, When content indexing finishes, Then Ajustes shows one document under each reason.
  The four files' name hits carry their labels, and the cloud-only file was never opened.
- AC-12 (R13) Given a content run, When the diagnostic log is read, Then it has the start,
  pause and stop records with rates and counts, and no file or folder name appears in it (the
  VFS-5 redaction test, extended).
- AC-13 (all) Given the Windows and macOS e2e harnesses (VFS-8), When a folder of the VFS-2
  fixtures is added, content search is accepted and the run finishes, Then a content query
  finds each fixture through the app's command.

## Non-goals

- OCR (issue #28, later and optional per folder, VFS-2 D13).
- Hybrid search and embeddings (VFS-29), and questions to a model (VFS-28).
- Content search over bookmarks (VFS-6) or other sources.
- Per-folder content switches. Content search is on or off for every folder. A folder
  excluded from name search is excluded from content.
- Indexing while the app is closed. There is no background service, so content indexing
  runs only while VFS runs.
- Tantivy. It is added only if the real archive exceeds ~100,000 documents (VFS-2 D4), as a
  separate PR behind `retrieve`.

## Pre-mortem

- **Slow PC / HDD.**
  - One document at a time, lowest priority, waiting for name walks and a busy disk (R6).
  - The run is resumable (R8).
  - Estimates: 0.3 s per document, 5 h per 60,000 **[estimate]**, replaced by the real-PC
    measurement (D3).
- **Paths over 260 characters.** The worker uses VFS-1's long-path handling (VFS-2
  pre-mortem). Covered by AC-13 on the Windows runner with a long-path fixture.
- **NFC/NFD.** Handled by VFS-2's fold. The UI shows the stored name.
- **Antivirus.** Defender scans each opened document; its cost is the main unknown and is
  measured on the real PC (D3). `content.db` lives in the app data folder, next to
  `index.db`.
- **Kill / power cut.** `content.db` is SQLite in WAL mode; an uncommitted document is read
  again (VFS-2). A corrupt store is deleted and rebuilt after telling the user.
- **Huge result sets.** 20 content hits per page, built by `retrieve` (VFS-2 R14 caps the
  ranking work); «Ver más» pages on.
- **Permission-denied, locked files, cloud placeholders.** Cloud-only files are never opened
  (R12, VFS-2 R12). Locked or denied files are marked «no se pudo leer» and retried only when
  their size or mtime changes (VFS-2 R3).
- **User files.** Read-only opens by the worker. No write, move or delete (VFS-2 AC-1).

## Phased PRs

| PR | Scope | Proves | Effort |
|---|---|---|---|
| 27a | Package `vfs-extract` as a Tauri sidecar in both bundles. The CI release job builds it, and the updater ships it. Add version handshake and installer size check. | AC-1 | 2–3 days **[estimate]** |
| 27b | Background content indexer in the app: scheduler with the pace rules (R6), idle detection on Windows and macOS, pause and resume, progress and time left, low-space pause, Tauri commands, diagnostic records. | AC-2, AC-5–AC-7, AC-12 | 4–6 days **[estimate]** |
| 27c | User interface: the offer, the Ajustes switch, store size and delete, unread counts and labels, and the content section with its debounce and cancellation. Strings go in both catalogs. Add the e2e scenario. | AC-3, AC-4, AC-8–AC-11, AC-13 | 4–5 days **[estimate]** |
| 27m | Measurement on the real PC (docs and probe script): document count and bytes under the real folders, open-and-read time for 1,000 documents (Defender on), free space on C:. It can run any time before 27c ships. | D3 | 1 day + the field visit |

Total ≈ 11–15 working days, plus the field measurement **[estimate]**.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | How does `vfs-extract` reach the user? | Inside the installer as a Tauri sidecar, so content search works with no connection and updates in lockstep. | owner | 2026-11-22 |
| D2 | Pace numbers | 60 s since the last input means "in use", with 15 s between documents while in use (Copernic's published setting). After 2 min idle, full pace. Name walks and the busy disk always win. Tuned later from the diagnostic log. | agent (owner set the policy, T1) | 2026-11-22 |
| D3 | Estimates in the offer | Time: 0.17–0.55 s per document. Disk: 13–16 MB per 1,000 documents (VFS-2 measurements, target-PC estimate). The numbers are replaced by the real-PC measurement (27m), and the running rate replaces them after 200 documents. | agent | 2026-11-22 |
| D4 | Switch off vs delete | The switch pauses indexing and hides the section, but keeps the store. Deleting the store is a separate confirmed button. | agent | 2026-11-22 |

## Open questions

None blocking. 27m's numbers change only the estimates' constants, never an interface or the
store.
