# VFS-29 — Hybrid search: embeddings by API or a downloaded local model (outline)

- **Status**: draft (outline). It is to be completed from the template, and approved, before
  any code. It follows VFS-28, and its gate is the content golden set (VFS-2 AC-17).
- **Size**: L. It adds a vector store to `content.db`, sends document text to an outside
  service (with consent), and adds an optional model download with CPU-specific runtimes.
- **Links**:
  - [VFS-2](../vfs-2-content-search/spec.md): `retrieve`, chunks (D14) and the golden set
    (AC-17).
  - [VFS-28](../vfs-28-ask-archive/spec.md): the AI settings, the key storage, the "Qué se ha
    enviado" view and the `Embedder` slot.
  - Research: `docs/research/content-vector-rag.md` §3–§5.
  - Issue #41.

## Problem

BM25 finds the words the user types, not the idea: «gastos médicos» misses a document that
says «factura del dentista». Vectors close that gap. Computing them is the expensive part:
about 45–90 h of CPU per 60,000 documents on the target PC with the smallest common local
model **[estimate]**, or about $5–10 per million chunks through an API **[published
prices]**. Storing and searching them is cheap: a scan of 1 M × 384-dimension 1-bit vectors
takes 7 ms **[measured]**. The owner chose BM25 by default, with hybrid search opt-in, through
an embedding API key or a local model downloaded from Ajustes (VFS-2 D15).

## Shape (to be turned into EARS requirements)

- **Switch.** In Ajustes, a third level: names → + content → + «búsqueda por significado».
  Two sources:
  - an embedding API key: OpenAI or Voyage, separate from the answer key, kept in the OS
    store through VFS-28's key storage;
  - «Descargar modelo local», which shows the size, an estimated time for this archive and
    the RAM needed before downloading.
- **Consent.** Embedding by API sends the whole text of every document once. It needs its own
  explicit consent, per folder, with the estimated cost shown first. The "Qué se ha enviado"
  view (VFS-28) records it per folder and date.
- **Storage.** Vector sets in their own file, `vectors.db`, beside `content.db`, so that
  deleting or rebuilding the content store (VFS-27 R5, R11) never silently wipes embeddings
  that cost hours or money. They are keyed by (provider, model, dimension), with chunk
  addresses that stay valid while the chunking (VFS-2 D14) is unchanged: one 1-bit
  row per chunk for the scan and int8 for the rescore. There is no vector database and no ANN
  index below ~3–5 M chunks (research §3). Changing the model builds a new set beside the old
  one, and the old set is dropped once the new one is complete. A new set is built only
  after a free-space check like VFS-27 R6.
- **Search.** `retrieve` runs BM25 and the vector scan, then fuses them with RRF (k = 60).
  The query is embedded once per search, by API or locally. With an API, every content
  query therefore leaves the PC. The full spec decides how «Qué se ha enviado» records them,
  for example one line per day with the count, rather than one entry per query. Offline, without a key, or
  before the vectors are complete, `retrieve` falls back to BM25 and the section says so.
- **Background.** The same pace rules as VFS-27 R7. API embedding runs in batches, with
  retries and cost metering. Local embedding runs in a child process with its own memory cap.
- **Local model spike (its own PR, before the local option ships).**
  - Candidates: bekko-a8m, granite-97m-multilingual-r2, multilingual-e5-small, and a
    model2vec static model.
  - Runtimes: ONNX Runtime (`ort`, still at release candidates; some builds need AVX2) vs
    pure Rust (candle, or static embeddings with no runtime).
  - Measure on the real PC: time per chunk, RSS, and the AVX2 check.

## Gate before approval

The margin is fixed here, before any run:
- **Meaning-only subset.** The content golden set (VFS-2 AC-17) gains at least 15 labelled
  meaning-only queries, where the document shares no keyword with the query («gastos
  médicos» → «factura del dentista»). Hybrid search must put the expected document in the
  top 5 for at least 3 more of them than BM25 does.
- **Keyword subset.** On the existing keyword queries, hybrid search must not lose any query
  that BM25 has in its top 5.
- **Which models.** The gate is run first with at least one API model, which decides 29a.
  The chosen local model must pass the same gate before 29c ships.
- **Count.** The real-PC measurement (VFS-27 27m) gives the document count, which turns the
  local model's hours into a real number.

If the API model fails the gate, hybrid search is not built.

## Non-goals (already decided)

- A bundled model. The model is downloaded on request, never in the installer.
- A local cross-encoder reranker or a local LLM.
- A vector database (LanceDB, Qdrant, sqlite-vec) below the scale above. sqlite-vector is
  excluded by its licence (Elastic License 2.0).

## Expected PRs

| PR | Scope | Effort |
|---|---|---|
| 29a | Vector sets in `vectors.db`, the API `Embedder` (OpenAI, Voyage), per-folder consent and cost, RRF in `retrieve`, offline fallback, golden-set comparison | 5–7 days **[estimate]** |
| 29b | Local model spike: measure the candidates on the Mac and the real PC; pick model and runtime | 3–4 days **[estimate]** |
| 29c | Local model download, check and child-process `Embedder` | 4–6 days **[estimate]** |
