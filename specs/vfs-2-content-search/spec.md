# VFS-2 — Document content search: measured prototype

- **Status**: draft (prototype built against it; owner approval of the open questions pending)
- **Size**: L — opens user files for the first time, adds an on-disk store next to the index,
  adds a child process with platform-specific limits, spans several components.
- **Links**: [VFS-1](../vfs-1-core-search/spec.md) R12 / AC-15 (extension point);
  `docs/content-search-design.md` (design); GitHub milestone "Fase 3 · Contenido de
  documentos", issues #23 (isolated extraction), #24 (Spanish full-text search), #25 (results
  with a highlighted fragment); measurements in `docs/content-search-measurements.md`.

## Problem

The owner can find a file by its name and folder (VFS-1) but not by words inside it: a
contract whose file is called `escaneo0034.docx` is invisible unless the user remembers the
name. Most of the archive's documents are Spanish Word, PDF and spreadsheet files, many of
them old, some damaged. The target PC is slow and short of memory, with the data and the index
on one busy 8 TB HDD that a NAS sync client also uses, and Microsoft Defender scans every file
that is opened. Content search must therefore be optional, must never slow down or endanger
name search, and its cost on that PC must be known before it is offered. This step builds and
measures a prototype, reachable from the command line only.

## What I know

- The extension point is in place and unused: `files.content_state` (0 none, 1 queued,
  2 indexed, 3 failed, 4 skipped), `ContentState`, `ExtractedText` and `ContentExtractor` in
  `crates/vfs-core/src/content.rs`; `files.kind = 1` marks documents
  (`crates/vfs-core/src/kind.rs`). v1 writes `content_state = 0` only and never resets it
  when a file changes.
- Name search keeps its own page caches (up to 224 MiB) and must stay under 300 MB of RAM
  with the index open (VFS-1 AC-9). The name indexer is the only writer of `index.db`.
- Other work runs in parallel on the schema and search internals of `vfs-core`; this step
  must not change `vfs-core`'s schema, its `SCHEMA_VERSION` or its search code.
- Assumptions: an older 4-core x86 CPU and 8 GB of RAM on the target PC; 60,000–500,000
  documents, about 40 % PDF (design doc, **[estimate]**).

## Scope

**Formats extracted**: DOCX, DOC (Word 97–2003), PDF (text layer only), XLSX, XLS, PPTX,
PPT, ODT (plus ODS and ODP where the chosen library covers them), RTF, TXT, and the other
plain-text document kinds (MD, CSV).

**Document kinds not extracted in this step** (kept `skipped`): Apple iWork (`pages`,
`numbers`, `key`) and EPUB, unless the chosen library covers them at no extra cost.

**Excluded for good**: the content of photos, audio and video. **Excluded from this step**:
OCR of scanned PDFs, embeddings, any machine-learning model, any desktop-UI change.

## Requirements (EARS)

- R1. THE content indexer SHALL extract text only from files whose kind is `document` and
  whose extension is in the supported list, and SHALL never write, move or delete a user file.
- R2. THE extraction SHALL run in a separate worker process with a wall-clock limit per file,
  a memory limit and a cap on the extracted text; IF the worker exceeds a limit, crashes or
  hangs, THEN THE content indexer SHALL kill it, mark that file failed, start a new worker and
  continue with the next file.
- R3. WHEN a file has failed or been skipped, THE content indexer SHALL not read it again until
  its size or modification time changes, except that a file that failed by timeout SHALL be
  tried once more on a later run (a background-priority worker on a busy disk can be starved
  past the limit); WHEN an indexed file's size or modification time changes, THE content
  indexer SHALL extract it again; WHEN neither changed, it SHALL not read the file.
- R4. IF the content indexer is interrupted, THEN the next run SHALL continue from the files
  not yet processed, without redoing finished ones.
- R5. WHILE extracting, THE worker SHALL run at the lowest CPU and I/O priority the OS offers
  without administrator rights, and THE content indexer SHALL read one file at a time.
- R6. WHEN the user searches content, THE content search SHALL match words of the documents
  case- and accent-insensitively and with Spanish stemming (`facturas` finds `factura`), SHALL
  support quoted phrases, and SHALL return hits ranked by BM25, each with a snippet of the
  document text and the offsets of the matched words inside that snippet.
- R7. THE content search SHALL combine with the existing type, date, size and folder filters
  with the same meaning as in name search.
- R8. WHILE content search is not switched on, THE app and CLI SHALL behave exactly as before:
  no content store is created, no document is opened, and name search returns the same
  results.
- R9. THE content store SHALL live outside `index.db` and SHALL be deletable as a whole,
  which switches content search off without touching the name index.
- R11. THE content store SHALL keep each document's text in addressable chunks (document id,
  chunk ordinal, byte and character offsets into the stored text, page number where the
  format has pages), and THE content search SHALL return, for each document hit, its
  best-matching passages with those addresses.
- R10. THE prototype SHALL be reachable from the CLI only (`vfs content-index`,
  `vfs content-search`), behind a build switch that is off by default.

## Acceptance scenarios

Tests cite these ids (`VFS-2 AC-<m>`) in their doc comments.

- AC-1 (R1) Given an indexed folder with a DOCX, a PDF, an XLSX, a JPG and an MP3, When the
  content indexer runs, Then only the document files are opened (the photo and audio files
  stay `content_state = 0` and have no content row), and every user file is byte-identical
  afterwards.
- AC-2 (R6) Given Spanish DOCX, DOC, PDF, XLSX, XLS, PPTX, PPT, ODT, RTF and TXT files with
  a known sentence containing accents and `ñ`, When they are content-indexed, Then a search
  for a word of that sentence finds each file, and the snippet contains the word with its
  accents intact.
- AC-3 (R2, R3) Given a file whose extraction exceeds the time limit (a test hook makes the
  worker hang), When the content indexer runs, Then the worker is killed within the limit plus
  one second, the file is marked `failed`, and the next file is still indexed; the next run
  tries that file once more, and the run after that does not open it.
- AC-4 (R2) Given truncated, randomly corrupted, password-protected, zero-byte and zip-bomb
  documents, When the content indexer runs, Then the run completes, each such file ends
  `failed` or `skipped` (password-protected and empty-text files `skipped`), and the valid
  files in the same folder are indexed.
- AC-5 (R2) Given a worker that would exceed the memory limit or whose extracted text exceeds
  the output cap, When it runs, Then the memory case is killed and marked `failed`, and the
  output case stores at most the cap.
- AC-6 (R3) Given a content-indexed folder, When the indexer runs again with nothing changed,
  Then it opens no file; When one file's mtime changes, Then only that file is extracted again
  and a search finds its new text and not its old text; When a failed file is unchanged, Then
  it is not opened again.
- AC-7 (R4) Given a run stopped after N files, When the indexer runs again, Then it processes
  only the remaining files.
- AC-8 (R6) Given indexed Spanish documents, When the user searches `facturas`, `accion`,
  `"contrato de alquiler"` (quoted) and `PEÑALARA`, Then documents containing `factura`,
  `acción`, that exact phrase, and `Peñalara` are found; the phrase query does not match a
  document that has the words apart; hits are ordered by BM25, and the highlight offsets
  point at the matched words in the snippet.
- AC-9 (R7) Given content hits in several folders, kinds, dates and sizes, When the search
  carries a kind, date range, size range or folder filter, Then only hits satisfying every
  filter are returned.
- AC-10 (R8, R9) Given an index on which content search was never switched on, When the user
  runs name searches, Then no content store exists and no document has been opened; When the
  content store is deleted, Then name search still works and content search reports that it is
  off.
- AC-11 (R10) Given the CLI built with the content switch, When the user runs
  `vfs content-index --db … ` on a folder of documents and then `vfs content-search --db …
  "<a phrase from one document>"`, Then that document is listed with a snippet in which the
  phrase is highlighted. Given the default build, Then those commands do not exist.

- AC-12 (R11) Given a long document whose match is near its end, a PDF and a plain-text file,
  When they are content-indexed and searched, Then each hit carries passages with a chunk
  ordinal, character offsets whose text contains the matched word, and a page number for
  the PDF and none for the plain-text file; chunks tile the text without crossing pages.

## Budgets

Measured on the development Mac and estimated for the target PC in
`docs/content-search-measurements.md`. Values are starting limits for the prototype, to be
tuned on the real PC.

| Budget | Value | Label |
|---|---|---|
| Worker wall clock per file | 30 s | [estimate] a hang costs at most half a minute |
| Worker memory | 512 MB heap cap | [estimate] above the measured peak of the chosen library on the corpus's large files |
| Input file size | 100 MB; larger documents are `skipped` unread | [estimate] |
| Extracted text kept per file | 1 MB of UTF-8 | [estimate] caps one giant spreadsheet |
| Content store size | ≤ 30 % of the source bytes of the documents indexed | [estimate] to be confirmed by the measurement |
| Query latency | p95 < 150 ms on the 20,000-document corpus on the Mac | same bar as VFS-1 AC-9 |
| Memory of the content indexer process (parent, excluding the worker) | < 150 MB | [estimate] |
| I/O | one file at a time, read sequentially once, background I/O priority | R5 |

## Non-goals

- OCR, embeddings, rerankers or any model; content of images, audio and video.
- Any change to the desktop UI, the Tauri commands, or `vfs-core`'s schema, search or indexer.
- Automatic queueing on the name indexer's insert/update (`ingest::update_file`); this step
  detects changes itself by comparing size and mtime (see Decisions).
- Duplicate detection by content hash.
- A Windows Job Object memory limit (the prototype uses an allocator cap that works on both
  platforms; a Job Object is the hardening step once native code is involved).

## Pre-mortem

- Slow PC / HDD: one sequential read per file, one file at a time, background priority,
  resumable queue; costs measured on the Mac and estimated for the HDD with Defender in the
  measurements document.
- Paths over 260 characters: the worker opens files through the same long-path handling as
  VFS-1 (`\\?\` prefix on Windows); case sensitivity n/a because files are reached by the path
  stored in the index.
- NFC/NFD: document text is NFC-normalised before indexing, and query and document go through
  the same fold as `fold_key`.
- Antivirus: Defender scans each opened document; this is the dominant unknown on the target
  PC and is budgeted as an estimate with stated assumptions. The content store is a separate
  SQLite file in the app data folder, opened once.
- Kill / power cut: the content store is SQLite in WAL mode, one transaction per batch; a file
  whose transaction did not commit is still queued and is extracted again. A corrupt content
  store is deleted and rebuilt; it holds nothing that cannot be re-extracted.
- Huge result sets: content search returns at most `limit` hits (default 20, max 500) and
  ranks inside FTS5; snippets are built only for the returned page.
- Permission-denied, locked files and cloud placeholders: an open error marks the file
  `failed` with the error kind; a placeholder is opened like any file (the OS may download it
  — open question Q4).
- User files: documents are opened read-only and never modified (AC-1).

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Where does the content index live? | A separate `content.db` next to `index.db`, attached read-only to `index.db` for filters. Keeps the name indexer the only writer of `index.db` (no lock contention), is deletable as a whole (R9), and leaves `vfs-core`'s schema untouched. | agent (prototype) | 2026-11-20 |
| D2 | How is `content_state` driven without changing `vfs-core`? | The content indexer sets `content_state` itself: `queued` when it enqueues, `indexed`/`failed`/`skipped` when it finishes. `content.db` keeps the size and mtime each file had when it was processed; a mismatch re-queues the file. | agent | 2026-11-20 |
| D3 | Extraction library | Chosen by the hands-on evaluation in the measurements document. | agent | 2026-11-20 |
| D4 | Full-text engine | Chosen by the measured comparison (SQLite FTS5 vs Tantivy) in the measurements document. | agent | 2026-11-20 |
| D5 | Build switch | Cargo feature `content` on `vfs-cli`, off by default. | agent | 2026-11-20 |
| D6 | Memory limit mechanism | A counting global allocator in the worker that aborts above the cap (portable, catches every Rust allocation). | agent | 2026-11-20 |
| D8 | Addressable chunks (constraint from the owner, relayed by the planning session, for the later optional AI phase, milestone "Fase 5", issues 38–41) | Chunks of about 2,000 characters (1,200–2,000, cut at a paragraph, sentence or space), tiling the text, never crossing a page; stored as offsets into the single compressed text of the document, not as separate text. BM25 stays document-level; passages are ranked inside each hit. | planning session for the owner | 2026-11-20 |
| D7 | Timeouts at background priority | One retry on a later run, then final. Found while building: with the Mac busy, the background-band worker took 6.3 s of wall clock for a 24-byte file using 0.02 s of CPU. | agent | 2026-11-20 |

## Open questions (for the owner; the prototype works for either answer)

- Q1. Do content results mix with name results in one list, or sit in a separate section
  ("En el contenido de los documentos")? Or is document content a third source in the VFS-6
  source selector, next to files and bookmarks? The query API returns content hits separately
  from its own store, so it fits any of the three; `vfs content-search --layout mixed|sections`
  shows the first two (mixed: name hits first, then content hits not already listed).
- Q2. Is content search on by default, or does the user switch it on? Today it is off until
  `content-index` is run; switching on costs the background time and disk measured in the
  measurements document.
- Q3. How much disk on the system drive may the content store take, and should the stored
  text (needed for snippets) be capped further to save space?
- Q4. May extraction open cloud placeholders (which can trigger a download), or should
  offline-only files be skipped?
- Q5. Should OCR of scanned PDFs be a later step at all, given its cost on the target PC?
- Q6. Chunking and the later optional AI phase (Fase 5): chunks are ~2,000 characters
  (~300–400 words) without overlap, so one embedding per chunk fits small embedding models
  (512–8,192 tokens) and a cited passage is short enough to quote. Smaller chunks give more
  precise citations and more vectors (≈ 25 per average document instead of ≈ 4); larger
  chunks give fewer, blurrier vectors and longer passages sent to a remote model. Overlap
  can be added at retrieval time by joining neighbouring ordinals, without re-chunking.
  Changing the chunk size later only rewrites the `chunks` table from the stored text (no
  re-extraction), but invalidates any embeddings already computed. Page numbers come from
  Xberg's per-page output (PDF, and its page split of Word documents); spreadsheets and
  plain text have none, so their citations are character offsets only. Decide the size
  before embeddings are computed.
