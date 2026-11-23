# Document content search: design

Status: design; a measured CLI-only prototype exists (VFS-2: `specs/vfs-2-content-search/`,
crate `crates/vfs-content`, results in `docs/content-search-measurements.md`). Where the
prototype settled a choice below, the text says so. It is milestone
[Fase 3 · Contenido de documentos](https://github.com/AndresHG/VFS/milestone/4), after
name and path search (Fase 1) and validation on the real PC (Fase 2). Issues: extraction
in an isolated process (#23),
Spanish full-text search (#24), content results with a highlighted fragment (#25).

## Goal and scope

Let the user find a **document** by words inside it ("contrato alquiler Peñalara") as well
as by its name and folder. Documents means the types `file_kind` already calls `document`
(`crates/vfs-core/src/kind.rs`): PDF, Word, Excel, PowerPoint, OpenDocument, RTF, plain
text, Markdown, CSV, Apple iWork and EPUB.

Out of scope: photos, audio and video content; OCR of scanned PDFs and embeddings in the
first step (both are later, optional steps); any LLM.

Content search is optional. It must never slow down name search, and the app must stay
useful if extraction is switched off or fails on every file.

## How figures are marked

Every number in this document carries one of these labels:

- **[measured]**: measured in this repository, with where and how.
- **[published]**: published by a third party (paper, vendor, documentation), not
  reproduced by us. Source named.
- **[estimate]**: our arithmetic or judgement from other figures; the assumptions are given.

Nothing in this document has been measured on the reference machine.

## The reference machine

One Windows 10+ PC, slow and short of memory, with the whole archive (4–5 TB, designed for
5 million files) on **one 8 TB spinning hard disk** that a Synology sync client also uses.
For budgeting we assume an older 4-core x86 CPU and 8 GB of RAM **[estimate]**; the real
CPU generation (and whether it has AVX2) is unknown. The number of documents in the archive
is unknown; this document budgets for **60,000 documents, 40 % of them PDF, about 3,000
words each** **[estimate]**, and scales linearly beyond that.

Constraints that follow:

- **One I/O stream on the HDD.** Reading file contents means seeking; two readers on one
  spinning disk make both slower. Extraction shares the disk with the name indexer and with
  the sync client.
- **Memory.** The app's budget is under 300 MB with the index open (AC-9). The name-search
  page caches may grow to 160 MiB (32 writer + 64 reader + 64 search), 192 MiB while a
  folder is added or removed (a short-lived second writer of 32 MiB), set in
  `crates/vfs-core/src/db.rs`, as in [architecture](architecture.md). Measured: the 2 M-file
  benchmark process peaks at about 138 MB RSS (schema version 3; 123 MB at version 2).
  Content search cannot add another large cache inside the app
  process; extraction must run in a separate process with its own cap.
- **Hostile input.** A decades-old archive holds corrupt, truncated, password-protected and
  enormous files. An extractor crash or hang must never take the app or the name indexer
  down.

## Pipeline

```
files.content_state = queued
        │   (documents only, after the name scan of the root is done)
        ▼
extraction worker process ── time limit, memory limit, size limits
        │   ExtractedText { title, body }  or  a failure code
        ▼
indexer thread: one transaction per batch
        ├─ content FTS table (Spanish analyzer)   rowid = files.id
        ├─ snippet text (compressed, capped)       for the highlighted fragment
        └─ files.content_state = indexed | failed | skipped
        ▼
search: name/path pass as today, plus a content pass; results merged
```

### 1. Queueing

- A file whose kind is `document` gets `content_state = queued` when the name indexer
  inserts it, or when its size or mtime changes (`ingest::update_file`).
- Extraction for a root starts only when its name scan is finished (`status = ready`), so
  the user can search by name first, and it yields to the name indexer whenever directories
  are pending.
- A file that failed is not retried until its size or mtime changes.
- Duplicate documents are common in personal archives. Before extracting, look for an
  already-indexed file with the same size and a matching content hash, and reuse its text.
  Hashing reads the file, so it only pays when the same size appears more than once.

### 2. Extraction in an isolated worker

- A separate executable, `vfs-extract`, shipped as a Tauri sidecar (VFS-2 D16, VFS-27 D1);
  the CLI also runs itself as the worker with its hidden `content-worker` subcommand. One
  file at a time, the whole process at background CPU and I/O priority.
- The app sends a path, receives `ExtractedText` or a failure. The worker is killed and
  restarted after a timeout or a crash; the file is marked `failed`, and the scan goes on.
- Limits of the prototype (starting values, to be tuned on the real PC) **[estimate]**; the
  largest worker peak measured on the test corpus was 123 MB **[measured]**:

  | Limit | Value | Why |
  |---|---|---|
  | wall clock per file | 30 s; a timeout is retried once on a later run | a starved background worker is not a broken file |
  | worker memory | 512 MB heap cap (counting allocator) | a file that needs more is `failed` |
  | input file size | `content_max_mb` in `index.db` `meta`, else 100 MB; skipped unread, and queued again when the limit is raised | larger "documents" are scans or exports, not text |
  | extracted text per file | 1 MB | caps index growth from one giant spreadsheet |
  | ZIP expansion (OOXML/ODF) | Xberg's own security limits | zip bombs (the 1 GB bomb of the evaluation corpus and the 300 MB bomb of the AC-4 test fail cleanly) |

- Password-protected files are `skipped`, not `failed`.
- Cloud-only placeholders are never opened (VFS-2 R12): the index flag, then the file's
  attributes just before the open (Windows recall-on-open, recall-on-data-access or offline;
  macOS `SF_DATALESS`). They are `skipped` as `cloud` and read once they are local.
- Background pace (VFS-27 R6–R8): the app calls the run with a `Gate` consulted between
  documents only. `vfs_content::pace` decides from probe readings, in order: low space on the
  app-data drive pauses (below 1 GB, resumed above 2 GB or by the user), a name walk or large
  name pass waits, a busy disk waits under VFS-5's back-off (Windows; never more than
  `max_backoff` without `min_work` of reading), and input within 60 s spaces documents 15 s
  apart. The probes are `vfs_core::probes`. Paused folders (VFS-32) are passed over and keep
  their state; time asleep does not count against a document's limit.
- The app's background indexer (VFS-27 27b): `vfs_content::service` keeps the switch and the
  pause in `content.json`, runs the engine on one thread while content search is on, waits
  for the name indexer before starting a run, shows documents done of all, the rate over
  working time and the time left, writes a run-open marker and checks the store with
  `quick_check` after an unclean stop (deleting it only on the user's choice), turns
  run-level stops into a «Reintentar» state, and writes diagnostic records without names.
- Worker lifetime (VFS-2 R17, R18): the worker says hello with its version and whether the
  OS reports it at background priority; another version or no hello stops the run before
  any state changes. It exits as soon as its stdin closes. On Windows it runs in a Job
  Object with kill-on-close and a committed-memory limit of the heap cap plus 128 MB.
- A run stops, leaving unprocessed files as they were, when the worker cannot start, when
  `content.db` cannot be written, or after 5 crashes in a row; timeouts and memory aborts
  are the document's own failures. Documents of an offline root are passed over and keep
  their state.
- Size: `vfs-extract` built with the `extract` profile (`opt-level = "s"`, fat LTO, one
  codegen unit) is 22.3 MB, 6.6 MB xz-compressed **[measured, macOS arm64]** (VFS-2 D16).
- Library: **Xberg 1.4.2** with its pure-Rust PDF backend, chosen after a hands-on
  comparison with pdf-extract, pdf_oxide, PDFium, calamine and a zip + XML extractor; it is
  the only one covering legacy DOC, XLS and PPT, at the cost of about 34 MB of binary
  (measurements doc). Avoid MuPDF/PyMuPDF (AGPL) and Poppler (GPL). Fidelity on real, partly
  corrupt legacy files from the owner's archive is still unverified.
- Antivirus: extraction opens every document, so Windows Defender's real-time scan runs on
  each one (name indexing never opens files). Its cost on the reference machine is unmeasured.

### 3. Full-text index: BM25, Spanish stemming, accent folding (the first step)

The first shippable step is lexical: no model, no download.

- **Analyzer** (prototype): lowercase → Snowball Spanish stemmer (`facturas` → `factura`) →
  fold accents exactly like `fold_key` (`crates/vfs-core/src/fold.rs`); the stemmer runs
  first because its rules expect accented input. Quoted phrases match on stemmed positions.
  No unstemmed column yet: the stemmer over-merges some families (`arrendamiento` also finds
  `arrendador`), and an exact-word boost is the open fix.
- **Ranking**: BM25. SQLite FTS5 has a built-in `bm25()` function **[published: SQLite
  documentation]**.
- **Query semantics** (VFS-2 R13, R14): words are AND-ed; when no document has them all,
  documents with any of them come back, marked `partial`. A word in more than 5 % of the
  indexed documents (`fts5vocab`) filters but does not rank: the filter expression decides
  the matches and a second MATCH over the rare words scores them. When every word is that
  common, BM25 ranks only the first 1,000 candidates.
- **Engine calls** (VFS-2 R15): `retrieve` (text or structured `{all, any, phrases}`
  queries, paging, `partial`, `total` up to 1,000), `passage_text` (a chunk with its
  neighbours) and `unread` / `unread_counts`. Hits carry title, authors and creation date
  from the extractor (R16); passages carry a BM25 score computed over the chunk. No other
  crate opens `content.db`.
- **Golden set** (VFS-2 AC-17, D17): about 45 Spanish queries over a committed corpus of
  synthetic household documents, BOE PDFs and Wikipedia excerpts (`crates/vfs-content/tests/
  golden`, sources and licences in its NOTICE.md); the share found in the top 5 may only go
  up.
- **Engine**: the prototype measured both options on 20,000 documents and chose FTS5, in a
  separate `content.db` rather than `index.db`, with keys analysed in Rust before insertion
  instead of a custom tokenizer (measurements doc). The options as first considered:
  1. *FTS5 in the same `index.db`* with a custom tokenizer that applies the analyzer. FTS5
     accepts custom tokenizers through its C API; its built-in `porter` stemmer is
     English-only **[published: SQLite documentation]**. One file, one transaction for base
     rows and content rows, no cross-store drift. Unverified: how much glue `rusqlite` needs
     to register a Rust tokenizer.
  2. *Tantivy* in a directory next to the index, keyed by `files.id`, with its Spanish
     stemmer and ASCII folding. SQLite stays the source of truth; a startup check re-queues
     files whose content version disagrees. Unverified here: Tantivy's Spanish pipeline and
     its index size on this corpus.

  Option 1 is preferred if the tokenizer works, because the rest of the index already
  relies on doing everything in one SQLite transaction.
- **Why lexical first**: on long Spanish documents BM25 beat every dense retrieval model in
  one published comparison (NanoMLDR-es: BM25 0.944; best dense model 0.908;
  multilingual-e5-small 0.540) **[published: Bekko embedding paper, arXiv 2607.25180]**. On
  short passages small dense models were somewhat better (NanoBEIR-es: BM25 0.506, small
  models 0.53–0.60) **[published: same source]**. For an archive of contracts, invoices and
  reports, BM25 is the right first step.
- **Snippets**: the highlighted fragment (#25) needs the text. The prototype stores the
  whole extracted text (capped at 1 MB) compressed once per document, plus addressable
  chunks of about 2,000 characters (offsets and page), so search returns passages with a
  citation without re-reading the document from the HDD.
- **Query side**: query terms go through the same analyzer. Content hits are a separate
  `MatchKind` variant (the enum's doc comment already reserves it). Name and path hits keep
  ranking first; content hits follow, ordered by BM25. The content pass must respect the
  same search deadline and interruption as today, and it is skipped when the user has
  content search switched off.

### 4. Small embeddings (optional, later)

Only after BM25 is in use and real queries show where it misses.

- Model: a small multilingual embedding model in ONNX, run with ONNX Runtime on the CPU.
  Candidates: bekko-embedding-v1-a8m (MIT, 384 dimensions, 8,192-token input, released
  2026 by one author) and multilingual-e5-small (MIT, 384 dimensions, 512 tokens, widely
  used) **[published: model cards and the Bekko paper]**.
- Throughput: bekko-a8m 364 passages/s on a Ryzen 9 7950X and 33/s on a Raspberry Pi 5;
  e5-small 226/s and 19/s **[published: Bekko paper, ~130-token passages, OpenVINO]**. On
  the reference machine, roughly 15–20 chunks/s for bekko-a8m and 10/s for e5-small, assuming the
  PC is 1.5–2× a Pi 5 and a 350–400-token chunk costs 3× a benchmark passage
  **[estimate]**.
- Chunks of 350–400 tokens, at most 20–30 per document; about 0.5–0.9 million vectors for
  60,000 documents **[estimate]**. Store int8 vectors plus a 1-bit copy; search the bits
  first and re-score the best few hundred with int8. No approximate-nearest-neighbour graph
  is needed at this size **[estimate]**.
- Merge with BM25 by reciprocal rank fusion. No reranker, no LLM.
- Enable only after a first-run check: the CPU has AVX2 (current ONNX Runtime prebuilt
  binaries may require it — **unverified**) and a short benchmark is fast enough. Otherwise
  stay on BM25.
- Adds about 150 MB of download (runtime plus model) **[estimate]** and 150–250 MB of
  memory while embedding or searching **[estimate]**, which must live outside the app
  process or be loaded only on demand to respect the 300 MB budget.

## What the code already provides

The v1 code reserves the extension point without using it (`crates/vfs-core/src/content.rs`,
test `extension_point_is_present_and_unused`):

| Piece | Where | What it gives content search |
|---|---|---|
| `files.content_state INTEGER NOT NULL DEFAULT 0` | `crates/vfs-core/src/schema.sql` | per-file pipeline state already in every row; v1 writes 0 only |
| `enum ContentState { None = 0, Queued = 1, Indexed = 2, Failed = 3, Skipped = 4 }` | `content.rs` | the values of that column |
| `struct ExtractedText { title: Option<String>, body: String }` | `content.rs` | what an extractor returns |
| `trait ContentExtractor: Send + Sync { fn supports(&self, extension: &str) -> bool; fn extract(&self, path: &Path) -> Result<ExtractedText>; }` | `content.rs` | the boundary the isolated worker implements: an implementation of `extract` can send the path to the worker process and wait with a timeout |
| `SCHEMA_VERSION` and `PRAGMA user_version` | `crates/vfs-core/src/db.rs` | versioned schema; later versions add tables without touching existing ones, and an older build refuses a newer index (`schema_too_new`) instead of damaging it |
| `MatchKind` | `crates/vfs-core/src/types.rs` | the result contract already says content search adds its own variant |
| `files.kind` | `schema.sql`, `kind.rs` | selects which files are documents |
| low thread priority, batched commits, persistent queue in the index | `crates/vfs-core/src/indexer/`, `platform.rs` | the patterns the extraction queue reuses |

What content search still has to add:

- A migration to the next schema version. `db::open_rw` already applies missing migrations in
  order (`MIGRATIONS` in `db.rs`; version 2 added the per-file path index, versions 3 and 4
  made the index smaller, VFS-4 and VFS-7), so this is one more SQL file.
- An index on the queue, e.g. a partial index `WHERE content_state = 1`; v1 has none,
  because nothing reads the column.
- The content FTS table (or Tantivy directory), the snippet store, the worker executable,
  the setting to switch content search on and off, and the UI for content hits.
- Queueing on insert and update in the indexer.

## Storage and CPU budget

For the 60,000-document budget case:

| Item | Size | Label |
|---|---|---|
| Name index today, per file | about 280 bytes (5.4 MB for 20,000 synthetic files and 1,538 folders) | [measured: `vfs gen-db --files 20000` on the development Mac, 2026-11-20] |
| Name and path index at 2 M files, schema version 2 | 889 MB (577 MB before the per-file path index) | [measured: `vfs gen-db --files 2000000` on the development Mac] |
| Extracted text, compressed (snippet store, if full text were kept) | 0.3–0.5 GB | [estimate] |
| Full-text index with positions | 0.8–1.2 GB | [estimate] |
| Vectors, int8 + 1-bit copy (optional step) | 0.2–0.4 GB | [estimate] |

Capping the snippet store (for example the first 4 KB of text per document) makes it
negligible next to the full-text index **[estimate]**. The index lives in the local app
data folder on the system drive (`%LOCALAPPDATA%\app.vfs.desktop\` on Windows), not on the
archive disk; free space there is unknown.

Time on the reference machine, for 60,000 documents:

| Stage | Time | Label |
|---|---|---|
| Text extraction | 0.5–1.5 h (seek-bound on the HDD) | [estimate] |
| Full-text index build | 15–30 min | [estimate] |
| First step total | about 1–2 h of background work | [estimate] |
| Embeddings, bekko-a8m (optional) | about 7–17 h | [estimate] |
| Embeddings, multilingual-e5-small (optional) | about 14–25 h | [estimate] |

At 500,000 documents these are about 8× longer **[estimate]**.

## Risks

- **Real document count and size are unknown.** Every budget above scales with them. The
  first step itself produces the real numbers: count documents from the name index before
  starting, and measure the first thousand extractions.
- **HDD contention.** Extraction, the name indexer's re-walks and the Synology client compete
  for one spinning disk. Mitigation: one stream, background I/O priority, extraction only
  while no directory is pending, pause control in the UI.
- **Defender and third-party antivirus** scan every opened document; unmeasured.
- **Extractor fidelity** on legacy `.doc`, `.xls`, `.ppt` and damaged PDFs; unmeasured.
- **Index growth**: the content index may be several times the name index. Show its size;
  allow switching content search off and deleting its tables.
- **Memory**: the app process already has up to 160 MiB of page caches (192 MiB during a
  folder change); the content pass
  must reuse the search connection's cache, not add a new one.
- **Two stores** (if Tantivy is chosen) can drift after a crash; SQLite stays authoritative.
- **Spanish stemming over-merges** some words; keeping an unstemmed column and boosting exact
  matches limits the damage.
- **Embeddings**: the leading small model is new and from one author; its scores are not
  independently replicated.

## Explicitly unverified

- Anything on the reference machine: CPU generation, AVX2, free space on the system drive, HDD
  throughput, antivirus behaviour.
- Throughput and fidelity of any extraction library on real files.
- Whether a custom Spanish FTS5 tokenizer can be registered from Rust with acceptable effort.
- Tantivy's Spanish analyzer and index size on this corpus.
- ONNX Runtime's minimum instruction set for current Windows binaries.
- Every embedding throughput and quality figure, beyond what its authors published.
- The size estimates for full-text and vector indexes; only the name index size per file
  has been measured, on synthetic data.
