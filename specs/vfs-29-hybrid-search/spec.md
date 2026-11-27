# VFS-29 — Search by meaning: a local model by default (API optional), fused with BM25

- **Status**: approved (owner, 2026-11-25). Code starts after 0.4 ships, with the 29b measurement (D12); the local model multilingual-e5-small int8 is the default (D13, D16; results in `docs/research/embed-eval-29b.md`).
- **Size**: L. It sends whole document text to an outside service for the first time (with
  consent, per folder), adds a store beside `content.db`, and is multi-PR (29a, 29b, 29c).
- **Links**:
  - [VFS-2](../vfs-2-content-search/spec.md): `retrieve` (R15), chunks (D14), the content
    golden set (AC-17).
  - [VFS-27](../vfs-27-content-in-app/spec.md): the content switch (R5), free-space check
    (R6), background pace (R7), run marker (R11).
  - [VFS-28](../vfs-28-ask-archive/spec.md): AI settings, key storage (R1–R3), cost and cap
    (R10), «Qué se ha enviado» (R11). Questions retrieve through `retrieve`, so they gain
    meaning hits with no change of their own.
  - [VFS-42](../vfs-42-scope-tabs/spec.md): the «Documentos» tab, where content hits show.
  - [VFS-38](../vfs-38-search-modes/spec.md) R12, AC-10: the «relacionado por tema» label
    this spec builds.
  - Research: `docs/research/content-vector-rag.md` §3–§5 (with the 2026-11-25 cost review),
    §9 (E1, E2, P1).
  - Issue #41.

## Problem

BM25 finds the words the user types, not the idea: «gastos médicos» misses a document that
says «factura del dentista», and a question to the archive (VFS-28) misses it too, because
it retrieves with the same BM25. The owner wants VFS's search in three levels, by name, by
words inside documents and by meaning, with questions on top. Meaning needs vectors.
Computing them is the expensive part: about $5–10 per million chunks through an API, hours
of upload, and the whole text leaves the PC **[published prices]**; or 40–90 h of CPU per
60,000 documents on the target PC with the smallest common local model **[estimate]**.
Storing and searching them is cheap: a scan of 1 M 1-bit vectors of 384 dimensions takes
7 ms **[measured]**. The owner chose BM25 by default and offline, with meaning opt-in
(D1), by API only for folders the user allows (D3).

## Requirements (EARS)

Turning it on

- R1. THE Ajustes panel SHALL offer a third search level, «Búsqueda por significado», below
  the content switch (VFS-27 R5), available only while content search is on. It SHALL be off
  by default.
- R1a. WHEN the user turns meaning search on, THE app SHALL use the local light model chosen
  in 29b (D13): show its download size, the estimated time for this archive and the RAM it
  needs, then download it and embed every folder whose content is read. Nothing SHALL leave
  the PC, and no key SHALL be asked for.
- R2. AS AN optional alternative to the local model, THE app SHALL take the embedding key as a separate key from the answer key (D2):
  OpenAI or Voyage, stored and shown exactly as VFS-28 R1–R2 store answer keys. An OpenAI
  key already saved for answers MAY be reused after the user confirms it.
- R3. WHEN the user chooses an API provider instead of the local model, THE app SHALL list the indexed folders with,
  for each, its number of documents read, the estimated number of chunks, the text volume
  in MB, the estimated cost at the provider's dated price, and the estimated time. Each
  folder SHALL be off until the user ticks it and confirms with «Enviar texto de estas
  carpetas» (D3). Nothing SHALL be sent before that confirmation.
- R4. WHEN content indexing later reads new or changed documents inside an allowed folder,
  THE app SHALL embed them without asking again; a folder added later SHALL start off.

Building the vectors

- R5. THE app SHALL embed the existing chunks of `content.db` (VFS-2 D14), not new ones, so
  a vector always points to a chunk that search, citations and questions already use.
- R6. THE app SHALL keep vectors in `vectors.db` beside `content.db`, as one vector set per
  (provider, model, dimension): a 1-bit row per chunk for the scan and an int8 row for the
  rescore, keyed by the hash of the chunk text so that copies of a document share one vector
  and are sent once (D10). Deleting or rebuilding `content.db` SHALL NOT delete `vectors.db`;
  vectors whose text no longer occurs in any chunk SHALL be dropped, and only new texts sent.
- R7. WHILE embedding runs, THE app SHALL follow VFS-27 R7's pace and pause rules, send
  batches of at most the provider's limit, retry a failed batch with backoff, show progress
  with time left, and resume after a restart where it stopped.
- R7a. THE first pass of a folder SHALL use the provider's batch endpoint when it is cheaper
  and its free allowance does not apply (OpenAI Batch, −50 %); while Voyage's free tokens last,
  it SHALL use standard calls (D11).
- R8. THE app SHALL count embedding cost against VFS-28 R10's monthly total and cap. IF the
  next batch would pass the cap, THEN THE app SHALL pause embedding and ask, never spend
  past the cap silently.
- R9. «Qué se ha enviado» SHALL record embedding per folder and day: number of chunks, MB of
  text, model and cost; and query embedding as one line per day with the number of
  searches and their cost. It SHALL NOT keep the text of each chunk sent.
- R10. WHEN the user switches model or provider, THE app SHALL build the new set beside the
  old one after a free-space check like VFS-27 R6, keep searching with the old set until the
  new one is complete, then drop the old set.

Searching

- R11. WHILE meaning search is on and the vectors of a folder are complete, `retrieve`
  SHALL embed the query once, scan the 1-bit vectors, rescore the top 4× k with int8, and
  fuse the result with BM25 by reciprocal rank fusion (k = 60). Content results, questions
  (VFS-28) and every other caller of `retrieve` SHALL get the fused list.
- R12. A content hit found by meaning alone SHALL carry «relacionado por tema» (VFS-38 R12,
  AC-10) and show its best passage instead of a highlighted snippet.
- R13. IF the PC is offline, the key is missing or rejected, or the query embedding takes
  more than 2 s, THEN `retrieve` SHALL return BM25 results and the «Documentos» tab SHALL say
  «Búsqueda por significado no disponible ahora; se muestran coincidencias por palabras».
- R14. Name search (VFS-1) and the «Nombres» tab SHALL never wait on an embedding call.

Footprint

- R15. With meaning search off, this spec SHALL add no file, thread, network call or
  memory. With it on, the 1-bit matrix SHALL stay in memory only while the «Documentos» tab
  or a question is in use and be released after 5 minutes idle; at 540,000 chunks it is
  about 26 MB **[estimate]**. Search p95 on «Documentos» SHALL grow by at most 100 ms over
  BM25 on the target PC, network excluded.

Local model (29b, 29c)

- R16. A local model SHALL NOT ship until 29b has measured the candidates (bekko-a8m,
  granite-97m-multilingual-r2, multilingual-e5-small, a model2vec static model) on the target
  PC: time per chunk, peak RSS and the AVX2 check. bekko-static-embedding-v1 joins the
  candidates as the static model (cost review, research §4). When it ships, it is downloaded on
  request after showing its size, the estimated time for this archive and the RAM needed,
  runs in a child process with its own memory cap, and sends nothing off the PC.

## Gate before building any interface

Fixed now, before any run:
- **Meaning-only subset.** The content golden set (VFS-2 AC-17) gains at least 20 labelled
  Spanish and 8 English meaning-only queries where the document shares no keyword with the
  query, over Spanish and English documents of the kind a family archive holds (invoices,
  letters, school and medical papers, manuals). Hybrid search must put the expected document
  in the top 5 for at least 4 more Spanish queries and 1 more English query than BM25 does,
  and Spanish results decide between candidates (D14).
- **Keyword subset.** On the existing keyword queries, hybrid search must not lose any query
  that BM25 has in its top 5.
- **Which models.** One run, before any interface: at least one API model (Voyage-4-lite or
  OpenAI 3-small) and the light local candidates of 29b, bekko-static-embedding-v1 first. The
  first to ship is the one that passes; if both pass, the local model ships first, since it
  is free and sends nothing off the PC (D12).

If neither the local model nor an API model passes the gate, meaning search is not built. If
only an API model passes, the owner decides whether to ship it.

## Acceptance scenarios

Tests cite these ids (`VFS-29 AC-<m>`) in their doc comments.

- AC-1 (R1, R15) Given content search on and meaning search off, When the user searches on
  «Documentos», Then no `vectors.db` exists, no embedding call is made and no vector is in
  memory.
- AC-2 (R3) Given meaning search turned on with two folders, When the user confirms only
  folder A, Then the mock provider receives chunks from A only, and nothing before the
  confirmation.
- AC-3 (R3) Given a folder of 1,000 documents, When the list shows it, Then it gives the
  chunk count, MB of text, cost at the dated price and time, each within 10 % of the run.
- AC-4 (R4) Given folder A allowed and folder B not, When a new document is read in each,
  Then only A's document is embedded.
- AC-5 (R6) Given a complete vector set, When `content.db` is deleted and rebuilt with one
  document changed, Then only that document's changed chunks are sent again.
- AC-6 (R7) Given embedding at 40 %, When the app is killed and restarted, Then it resumes
  from the last stored batch and no chunk is sent twice.
- AC-7 (R8) Given a monthly cap of 5 $ with 4.90 $ spent, When the next batch would cost
  0.20 $, Then embedding pauses and asks before sending it.
- AC-8 (R9) Given 200 searches and one folder embedded today, When «Qué se ha enviado»
  opens, Then it shows one folder line and one search line for today, and no chunk text.
- AC-9 (R10) Given set A complete, When the user switches model, Then searches use set A
  until set B is complete, then set B, and set A's rows are gone.
- AC-10 (R11, R12) Given «factura del dentista» in a document and the query «gastos
  médicos», When searched with vectors complete, Then the document is in the top 5 with
  «relacionado por tema», and a question (VFS-28) retrieves its passage.
- AC-11 (R13, R14) Given the mock provider offline, When the user searches, Then «Nombres»
  answers in its usual time, «Documentos» shows BM25 hits with the fallback sentence.
- AC-12 (Gate) Given the extended golden set, When BM25 and hybrid run, Then the
  meaning-only and keyword conditions of the gate hold, and the run is recorded in the PR.
- AC-13 (R15) Given vectors loaded, When neither «Documentos» nor a question is used for 5
  minutes, Then the matrix memory is released.
- AC-14 (R6, D10) Given the same document in two allowed folders, When both are embedded, Then
  each distinct chunk text is sent once and both copies are found by meaning.
- AC-15 (R7a) Given an OpenAI key and a first pass of a folder, When embedding starts, Then the
  chunks go through the Batch API, and the cost shown uses the batch price.

## Non-goals

- A bundled model; a local cross-encoder reranker or local LLM; API reranking (later).
- A vector database or an ANN index below ~3–5 M chunks; sqlite-vector (Elastic License).
- Embedding file names, bookmarks or chats: content chunks only. Other sources join when
  they have text in `content.db`.
- Contextual retrieval (a model writing context for each chunk): later, per folder.
- Storing answers as searchable pages (owner D12 of VFS-44): its own spec.

## Pre-mortem

- Slow PC and HDD: embedding reads chunks from `content.db`, already compressed text, not
  user files; the pace follows VFS-27 R7, and the upload is the bottleneck, not the disk.
- Long paths, NFC/NFD: n/a because vectors key on the hash of the chunk text, not paths.
- A root offline or paused during a content rebuild: its documents are neither stored nor queued, so their vectors are dropped and embedded again when the root returns; the cost is CPU, not lost search, and a rebuild is rare.
- Antivirus: `vectors.db` is one more SQLite file in the app data folder, like `content.db`.
- Crash mid-write: each batch commits in one transaction with its text hashes; resume reads
  what is stored (AC-6).
- A million matches: the scan is bounded by chunk count (7 ms per million on the Mac,
  ~20–35 ms on the target PC **[estimate]**); the fused list is capped at k.
- Permission-denied, links, placeholders: n/a because only chunks already in `content.db`
  are embedded; cloud-only files were never read (VFS-25).
- Opening, writing, moving or deleting user files: no. It sends their text, with consent,
  which is the main risk of the spec (R3, R9).
- Does any of this already ship? `grep -rn "embed\|vector" crates apps/desktop/ui` finds only
  the research harness names; `retrieve` is BM25 only.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Default search | BM25, offline; meaning opt-in, with BM25 fallback | owner | 2026-11-21 (E1) |
| D2 | Embedding key when the user has only an Anthropic key | A separate optional key, OpenAI or Voyage | owner | 2026-11-21 (E2) |
| D3 | May document text go to an embedding API? | Yes, per folder, with permission and the cost and volume shown first | owner | 2026-11-25 |
| D4 | Order | After 0.4 and before new sources (VFS-44) | owner | 2026-11-25 |
| D5 | Local model in this spec | ~~29a API first; the local model after the 29b measurement~~ replaced by D12 | owner | 2026-11-25 |
| D6 | Providers in 29a | OpenAI and Voyage | owner | 2026-11-25 |
| D7 | Fusion | RRF, k = 60, no tuning; a weighted sum only if the golden set shows a gain | agent | 2026-11-25 |
| D8 | Where vectors live | `vectors.db`, so a content rebuild never wipes paid vectors | agent | 2026-11-22 |
| D9 | Dimension | API models cut to 512 dimensions (OpenAI `dimensions`, Voyage 512) | agent | 2026-11-25 |
| D10 | Duplicates | Vectors keyed by chunk-text hash: each distinct text is embedded and paid once | agent | 2026-11-25 (cost review) |
| D11 | First pass | OpenAI Batch API (−50 %); Voyage standard while its 200 M free tokens last | agent | 2026-11-25 (cost review) |
| D12 | Order after the cost review | 29b's measurement and the gate run first, local light model beside the API; whichever passes ships first, local preferred | owner | 2026-11-25 |
| D13 | Default embedder | The local light model; the API is an optional alternative, not paid by default | owner | 2026-11-25 |
| D14 | Languages | A model good in Spanish and English; the gate weighs Spanish most (the owner's and family's documents), English still checked for open-source users | owner | 2026-11-25 |
| D16 | Which local model | multilingual-e5-small int8 (MIT, 129 MB): the only clear pass of the 29b gate (9/26 Spanish, 4/10 English vs BM25 0/0, no keyword loss). bekko-a8m is the fallback; static models failed. First pass ≈ 20–35 h of paced background CPU per 60,000 documents on the target PC [estimate], to confirm there | owner | 2026-11-26 |
| D17 | Fusion details for 29c | Leave BM25's list out of RRF when it has no document with all the query terms (29b: e5 9→13 ES, 4→8 EN, no keyword loss); measure an int8 scan or a wider rescore, since 1-bit cost e5 five hits | agent | 2026-11-26 (29b) |
| D18 | Local runtime (29c) | ONNX Runtime through `ort` 2.0, linked statically into the `vfs-embed` worker, CPU only (its automatic device choice used Core ML on macOS: 2× slower, 2× memory). Measured on the Mac with e5-small int8: 145 ms/chunk on 1 thread, 90 ms on 2, 0.7 GB peak; tract (pure Rust) took 636 ms/chunk on 1 thread. The worker is 22 MB (5.4 MB compressed), so it is not in the installer (14.6 MB of a 16 MB budget, VFS-27 AC-1): it downloads with the model when meaning search is turned on | agent | 2026-11-26 (29c) |
| D19 | 1-bit rows (29c) | Sign bits of each vector minus its set's mean, the mean taken from the set's first batch (64 texts): e5-small's vectors share a large common component, so plain signs carried little. On the 29b corpus, dense top-5 went from 13 to 18 Spanish (float32 search: 18), and hybrid with D17 gave 16/26 Spanish, 6/10 English, 45/45 keyword; a mean from 50 random chunks or from English text only gave the same. The rescore reads the int8 rows of the top 4× candidates from `vectors.db` by key. In memory while in use: the 1-bit rows, their hashes and every chunk's place, about 96 B per chunk (≈ 52 MB at 540,000 chunks **[estimate]**), loaded in the background, never inside a search; the query worker holds the model (≈ 0.57 GB after load, measured on the Mac) for the same time | agent | 2026-11-26 (29c) |
| D20 | A set while new texts wait (29c) | A set that has covered the whole archive once keeps serving searches while new or changed texts are embedded; those documents are found by their words until their vectors exist. A new model's set serves only once complete (R10) | agent | 2026-11-26 (29c) |
| D21 | Where the local model comes from (29c) | On «Búsqueda por significado», the app downloads three files into `models/` in its data folder: the model and its tokenizer from Hugging Face at a pinned revision, each checked against a pinned SHA-256, and the `vfs-embed` worker from this version's GitHub release, checked against its minisign signature made with the updater's key. Nothing counts as installed until every file is complete and checked; a failed download is retried with backoff | agent | 2026-11-27 (29c) |
| D15 | Questions with vectors | Questions (VFS-28) retrieve with vectors as soon as 29c ships, through the fused `retrieve` (R11); no change of their own | owner | 2026-11-25 |

## Expected PRs

| PR | Scope | ACs | Effort |
|---|---|---|---|
| 29b (first) | Local model spike on the Mac and the target PC, and the gate run with an API model beside it; pick the local model | R16, AC-12 | 3–4 days **[estimate]** |
| 29c | `vectors.db`, the local light-model `Embedder` (default, D13), RRF in `retrieve`, fallback, «relacionado por tema» | R1, R1a, R4–R7, R10–R16; AC-1, AC-4–AC-6, AC-9–AC-14 | 5–7 days **[estimate]** |
| 29a (optional, later) | The API `Embedder` (OpenAI, Voyage), its key, per-folder consent and cost, batch first pass | R2, R3, R7a, R8, R9; AC-2, AC-3, AC-7, AC-8, AC-15 | 3–4 days **[estimate]** |

## Open questions

None.
