# VFS-2 — Document content search: the engine

- **Status**: approved. The prototype in PR 98 was built against R1–R11. The owner answered
  the open questions on 2026-11-21 (D9–D15). R12–R15 and the phased PRs were added on
  2026-11-22 to close the engine before the app step. The spec passed an independent critic
  review.
- **Size**: L. The engine opens user files for the first time, adds an on-disk store next to
  the index, and adds a child process with platform-specific limits. It spans several
  components.
- **Links**:
  - [VFS-1](../vfs-1-core-search/spec.md) R12 / AC-15 (the extension point).
  - [VFS-25](../vfs-25-cloud-only-files/spec.md) R5 (the cloud-only flag).
  - Consumers: [VFS-27](../vfs-27-content-in-app/spec.md) (the app),
    [VFS-28](../vfs-28-ask-archive/spec.md) (ask the archive),
    [VFS-29](../vfs-29-hybrid-search/spec.md) (hybrid search).
  - Design: `docs/content-search-design.md`. Research: `docs/research/content-vector-rag.md`.
  - Measurements: `docs/content-search-measurements.md`, on PR 98's branch.
  - Milestone "Fase 3 · Contenido de documentos": issues #23 (isolated extraction), #24
    (Spanish full-text search), #25 (results with a highlighted fragment).

## Problem

The owner can find a file by its name and folder (VFS-1) but not by words inside it: a
contract whose file is called `escaneo0034.docx` is invisible unless the user remembers the
name. Most of the archive's documents are Spanish Word, PDF and spreadsheet files, many of
them old, some damaged. The target PC is slow and short of memory, with the data and the index
on one busy 8 TB HDD that a NAS sync client also uses, and Microsoft Defender scans every file
that is opened. Content search must therefore be optional, must never slow down or endanger
name search, and its cost on that PC must be known before it is offered. This step builds the
engine and measures it: extraction, the content store, and the retrieval interface that the
app (VFS-27), the questions (VFS-28) and hybrid search (VFS-29) all call. It is reachable from
the command line only.

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
- R10. THE engine SHALL be reachable from the CLI only (`vfs content-index`,
  `vfs content-search`), behind a build switch that is off by default.
- R12. IF a file is cloud-only, THEN THE content indexer SHALL not open it, SHALL mark it
  skipped with the reason "in the cloud", and SHALL extract it on a later run once it is local
  (D12). A file is cloud-only when:
  - on Windows, `ingest::FLAG_CLOUD_ONLY` is set, or its attributes, checked just before the
    open, carry recall-on-open, recall-on-data-access or offline;
  - on macOS, `stat` reports `SF_DATALESS`.
- R13. WHEN a content query has two or more words and no operator, THE content search SHALL
  return only documents that contain every word (AND). IF that returns no hit, THEN it SHALL
  return documents that contain any of the words, ranked by BM25, and mark the result
  `partial`.
- R14. WHEN a query word occurs in more than 5 % of the indexed documents (counted with
  `fts5vocab`), THE content search SHALL use that word to filter but not to rank. IF every word
  is that common, THEN it SHALL rank by BM25 over at most the first 1,000 candidates.
- R15. THE `vfs-content` crate SHALL expose one retrieval call, `retrieve(query, filters, k)`.
  It returns ranked documents, each with its best passages: file id, chunk ordinal, character
  range, page, BM25 score, and a snippet with highlight offsets. The CLI, the app (VFS-27),
  the questions (VFS-28) and hybrid search (VFS-29) SHALL all use this call. None of them
  SHALL query `content.db` directly.
- R16. WHEN a document's format carries metadata, THE content store SHALL keep its title, its
  author(s), and its creation date as reported by the extractor, next to the document row.
  It SHALL return them with each `retrieve` hit, for VFS-28's bibliography. Formats without
  metadata store none; nothing is guessed from the text.

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

- AC-13 (R12) Given a document with `FLAG_CLOUD_ONLY` set, When the content indexer runs,
  Then it is never opened, it ends `skipped` with the reason "in the cloud", and a search by
  its name still finds it. When the flag clears and its mtime changes, Then the next run
  extracts it. On a Windows runner, a file given the offline attribute after the walk is
  also not opened. The macOS `SF_DATALESS` branch is covered by a unit test on the flag check.
- AC-14 (R13) Given documents A (contains `alquiler` and `fianza`) and B (contains only
  `alquiler`), When the user searches `alquiler fianza`, Then only A is returned and the
  result is not partial. When the user searches `alquiler zapatilla`, Then B is returned and
  the result is marked partial.
- AC-15 (R14) Given 2,000 documents where `documento` occurs in every one and `Peñalara` in
  three, When the user searches `documento Peñalara`, Then the three are returned, ranked by
  the `Peñalara` score. When the user searches `documento` alone, Then at most `limit` hits
  come back, and the p95 latency stays within the query budget.
- AC-16 (R15) Given an indexed folder, When `retrieve("contrato de alquiler", filters, 10)`
  is called, Then it returns the same documents in the same order as `vfs content-search`.
  Each returned passage's character range, read from the stored text, contains a matched
  word. A test asserts that no crate other than `vfs-content` opens `content.db`.
- AC-17 (R6, R13, R14) Given the content golden set, ~40 labelled Spanish queries over the
  public-domain test corpus in `bench/` (see D17), When the golden test runs in CI, Then the
  expected document is in the top 5 for at least the share recorded in the test file. That
  share is fixed by the first run, and it can only go up.
- AC-18 (R16) Given a PDF and a DOCX whose metadata holds a title, an author and a creation
  date, and a TXT, When they are content-indexed and retrieved, Then the PDF and DOCX hits
  carry those three values and the TXT hit carries none.

## Phased PRs

Each PR leaves `main` releasable, and the `content` feature stays off by default throughout.

| PR | Scope | Proves | Effort |
|---|---|---|---|
| 2a | Resume PR 98. Rebase it on `main`, get the Windows and macOS CI green, implement D12 (R12), and pin the toolchain for the extractor (D16). | AC-1–AC-13 | 2–3 days **[estimate]** |
| 2b | Query semantics and the retrieval API: AND by default with an OR fallback (R13), the frequency guard (R14), `retrieve` (R15), document metadata (R16), and the content golden set (AC-17, D17). | AC-14–AC-18 | 2–3 days **[estimate]** |

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
  Those belong to VFS-27.
- Tantivy. FTS5 stays the engine until the real archive has more than ~100,000 documents. At
  that point a derived Tantivy index, rebuildable from `content.db`, sits behind the same
  `retrieve` call (D4, research §2).
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
  `failed` with the error kind; a cloud-only placeholder is never opened (D12).
- User files: documents are opened read-only and never modified (AC-1).

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Where does the content index live? | A separate `content.db` next to `index.db`, attached read-only to `index.db` for filters. Keeps the name indexer the only writer of `index.db` (no lock contention), is deletable as a whole (R9), and leaves `vfs-core`'s schema untouched. | agent (prototype) | 2026-11-20 |
| D2 | How is `content_state` driven without changing `vfs-core`? | The content indexer sets `content_state` itself: `queued` when it enqueues, `indexed`/`failed`/`skipped` when it finishes. `content.db` keeps the size and mtime each file had when it was processed; a mismatch re-queues the file. | agent | 2026-11-20 |
| D3 | Extraction library | Xberg 1.4.2 (MIT). It is the only candidate covering every format in scope, DOC, PPT and RTF included. It costs +32–34 MB of binary, kept out of the app in the `vfs-extract` executable (measurements document, "Choice: Xberg"). | agent | 2026-11-20 |
| D4 | Full-text engine | SQLite FTS5, contentless, BM25 (p95 18 ms on 20,000 documents, no added binary). Above ~100,000 real documents, Tantivy becomes a derived index behind `retrieve` (research §2). | agent | 2026-11-20, threshold 2026-11-22 |
| D5 | Build switch | Cargo feature `content` on `vfs-cli`, off by default. | agent | 2026-11-20 |
| D6 | Memory limit mechanism | A counting global allocator in the worker that aborts above the cap (portable, catches every Rust allocation). | agent | 2026-11-20 |
| D8 | Addressable chunks (constraint from the owner, relayed by the planning session, for the later optional AI phase, milestone "Fase 5", issues 38–41) | Chunks of about 2,000 characters (1,200–2,000, cut at a paragraph, sentence or space), tiling the text, never crossing a page; stored as offsets into the single compressed text of the document, not as separate text. BM25 stays document-level; passages are ranked inside each hit. | planning session for the owner | 2026-11-20 |
| D7 | Timeouts at background priority | One retry on a later run, then final. Found while building: with the Mac busy, the background-band worker took 6.3 s of wall clock for a 24-byte file using 0.02 s of CPU. | agent | 2026-11-20 |
| D9 | Q1: where content hits appear | A separate section below name and path hits ("En el contenido de los documentos"), with the highlighted fragment. | owner | 2026-11-21 |
| D10 | Q2: on by default? | Offered once the name scan is done: VFS shows the document count, estimated time and disk, and the user accepts or declines. Background pace: lowest priority, much slower while the user is active or the disk is busy, full pace when idle; pause and progress with time left. | owner | 2026-11-21 |
| D11 | Q3: disk on the system drive | Keep the full extracted text (snippets anywhere, citations later); show the store size in Settings; warn when free space is low. No fixed cap. | owner | 2026-11-21 |
| D12 | Q4: cloud-only placeholders | Never opened (no download). Detected from the file attributes (Windows recall-on-access/offline, macOS dataless); left unread, searchable by name, marked "not read (in the cloud)". | owner | 2026-11-21 |
| D13 | Q5: OCR | A later, optional step: off by default, per folder, after normal extraction. Scanned PDFs are marked "no text (scanned)". | owner | 2026-11-21 |
| D14 | Q6: chunk size | Keep chunks of ~2,000 characters (≈ 450–550 Spanish tokens), tiling, never crossing a page, without stored overlap; neighbours are joined at retrieval time. Fixed now, before any vector exists. | agent (owner delegated) | 2026-11-21 |
| D15 | Embeddings and AI (for the later phases) | BM25 by default and offline. Hybrid search is opt-in, using an embedding API key (OpenAI or Voyage, separate from the answer key) or a local model downloaded from Settings. Without a connection, search falls back to BM25. Per question, only the passages used leave the PC, visibly. | owner | 2026-11-21 |
| D17 | Content golden set | About 40 Spanish queries over committed public documents: BOE PDFs (free reuse with attribution), Spanish Wikipedia rendered to PDF/DOCX (CC BY-SA), and the synthetic personal documents of `bench/content-corpus/make_corpus.py`. The format and the per-case contract follow VFS-11. The corpus is shared with VFS-28's RAG golden set (D9 there). | agent | 2026-11-22 |

## Owner answers (former open questions Q1–Q6)

The research, options and figures behind each answer are in
`docs/research/content-vector-rag.md`. The answers are recorded as D9–D15 above.

- Q1–Q3 shape the app step (VFS-27), not this engine.
- D12 is R12 here, implemented in PR 2a.
- D14 is already true of the prototype.

## Open questions

None blocking. The real-PC measurement (document count, Defender's cost per opened file) does
not change this spec. It sets VFS-27's estimates, and the Tantivy threshold in D4.
