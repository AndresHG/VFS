# Document content, vectors and RAG: research, owner decisions and plan

Status: research and decisions, no code. Builds on
[`docs/content-search-design.md`](../content-search-design.md) (design), the VFS-2 prototype
(draft PR 98: spec `specs/vfs-2-content-search/spec.md` and
`docs/content-search-measurements.md` on branch `feat/vfs-2-content-search`) and
[`docs/vision.md`](../vision.md) (find, ask, connect, grow). Issues: #23–#25 (content search),
#28 (OCR), #38–#41 (AI module, embeddings), #84 (bibliography), #119 (Obsidian as a concept).

Figures carry the labels of the design doc: **[measured]** (here, with how), **[published]**
(third party, source linked, not reproduced) and **[estimate]** (our arithmetic, assumptions
given). Nothing here was measured on the target PC.

## The answer in one page

1. **Content search is lexical first, and stays lexical offline.** The VFS-2 engine (contentless
   SQLite FTS5 with BM25, Spanish stemming and accent folding, text kept compressed in
   addressable chunks in a separate `content.db`) is the right base. It needs no model and no
   network, and the published evidence favours BM25 on long Spanish documents.
2. **Computing embeddings is the expensive part; storing and searching vectors is cheap.**
   Measured on the development Mac: a brute-force scan of **1 million** 384-dimension vectors
   takes **7 ms** with 1-bit vectors (48 MB) and 34 ms with int8 (384 MB), on one core. Embedding the
   chunks with the smallest common multilingual model takes **0.2 s per chunk** on one core and
   close to 1 GB of RAM. On the target PC that is **about 45–90 h of CPU per 60,000 documents**
   **[estimate]**. Through an API it costs **about $5–10 per million chunks** **[published
   prices]** and takes hours, but the text leaves the PC.
3. **The owner's choice:** BM25 by default and offline. Hybrid search (BM25 + vectors, fused with
   RRF) is opt-in. Vectors come from the user's own embedding API key, or from a **local model
   downloaded on request** in Settings, with its size and time shown first. Hybrid search falls
   back to BM25 when the PC is offline or the key is missing.
4. **One store serves search and RAG.** The same chunks (file, ordinal, character offsets, page)
   feed the content results, the passages sent to a model, the citations and, later, the graph
   and an MCP connector.
5. **"Ask your archive" v1 retrieves with BM25, not vectors.** The user's model rewrites the
   question into keywords, synonyms and filters, local BM25 retrieves 10–20 passages, and the
   model answers with citations to file and page. That costs about **$0.001 (Claude Haiku 5.5)
   to $0.03 (Claude Sonnet 5.5) per question** **[estimate from published prices]**. Published
   work shows that an agent working over BM25 results beats both an agent reading files and
   plain BM25 on large corpora. Vectors then improve it; they are not a prerequisite.
6. **Keep the executable light.** Extraction stays in its own executable, and models are
   never bundled. Vectors are BLOBs in `content.db`, with no vector database. No local
   reranker or LLM.

## Owner's requirements (2026-11-21)

Stated by the owner in this research, on top of the vision's principles:

- Lightweight first: the installer and the running app as small as possible; designed for PCs
  with little memory and CPU.
- Works **offline and online**: everything that needs no model works offline; with a
  connection and an API key, RAG questions (and hybrid search) become available.
- Every capability is a switch in Settings: search by names only, + document content (BM25),
  + hybrid (API or local model).
- Optimised and built with good practices and reusable components; benchmarks planned from the
  start, and later tuning against the local diagnostic log.

## 1. How the prototype works (PR 98), and what it measured

- **Extraction**: Xberg 1.4.2 (MIT, pure Rust) in a worker process (the same binary in a hidden
  mode), one file at a time, background priority, 30 s and 512 MB per file, 100 MB input and
  1 MB text caps; a crash, hang or corrupt file costs only that file. All ten formats in scope,
  legacy `.doc/.xls/.ppt` included; token recall 99.8 % **[measured, PR 98]**.
- **Store**: `content.db` beside `index.db` (deletable as a whole), deflate-compressed text,
  chunks of about 2,000 characters that tile the text without crossing pages, contentless FTS5
  over keys analysed in Rust (lowercase, `fold_key`, Snowball Spanish).
- **Query**: BM25 over documents, top passages per hit with offsets and page, snippet with
  highlight offsets, VFS-1 filters (kind, date, size, folder).

| [measured, PR 98, M1 Pro shared with other sessions] | 20,000 docs, 921 MB source |
|---|---|
| Worker CPU per document | 33 ms |
| Peak memory | indexer 19 MB, worker 48 MB |
| Store | 236.5 MB = 11.8 MB per 1,000 documents (text 134.6 MB, FTS5 76 MB, chunks 4.4 MB) |
| Query CPU | p50 24 ms, p95 53 ms |
| FTS5 vs Tantivy 0.27 | p95 18 vs 3 ms; index 76 vs 56 MB; build +74 vs +276 MB RAM; +0 vs +5.5 MB binary |
| CLI size | 3.3 MB without content, 37.4 MB with it (+34 MB, Xberg) |

Target PC **[estimate, PR 98]**: about 0.3 s per document (seeks, Defender, CPU), so ≈ 5 h
for 60,000 documents and ≈ 40 h for 500,000; store ≈ 0.9 GB and ≈ 7 GB.

## 2. Lexical engine: FTS5 now, Tantivy if the archive is large

- FTS5 has no early termination: `ORDER BY rank LIMIT k` still scores every matching row
  ([SQLite FTS5 §5.2](https://www.sqlite.org/fts5.html)) **[published]**, so a common word over
  500,000 documents costs 0.8–2 s on the Mac and 2–5 s on the target PC **[estimate]**.
- Cheap fixes inside FTS5, in order: AND between words by default (Paperless-ngx 3.0 returned
  18 of 24 documents for a three-word query with OR semantics,
  [famstack PR 51](https://github.com/famstack-dev/famstack/pull/51) **[published]**); a
  **frequency guard**: read each word's document count from `fts5vocab` and rank by the rare
  words, using a word in more than ~5 % of documents only as a filter; two-step ranking (cheap
  candidates, full BM25 over ≤1,000 in Rust); `detail=column` where phrases are not needed (743
  → 340 MiB on SQLite's example corpus **[published, same page]**).
- Tantivy 0.27 adds block-max pruning for conjunctive top-k
  ([changelog](https://github.com/quickwit-oss/tantivy/blob/main/CHANGELOG.md)) **[published]**.
  With one writer thread and a 15–30 MB heap its build peak should be 60–120 MB, not the
  276 MB measured with defaults **[estimate]**. On Windows a memory-mapped segment cannot be
  deleted while mapped, so the index folder grows until readers reload **[published:
  tantivy `mmap_directory.rs`]**.
- SeekStorm (Apache-2.0) claims to beat Tantivy but is young; Milli, sonic and the `bm25`
  crate do not fit **[published/unverified, see sources]**.

**Recommendation**: FTS5 with the frequency guard up to ~100,000 documents. Above that, add
Tantivy (one thread, 20–30 MB heap) as a **derived index rebuildable from `content.db`**, which
stays the source of truth. The document count from the Fase 2 measurement on the real PC
decides. The query interface returns the same passages either way.

## 3. Vectors: storage, quantization and search

### Measured on the development Mac

Rust, release, `target-cpu=native`, one thread, random vectors, load average 12 from other
sessions (scratch harness, not committed). The f32 loop was not auto-vectorised (strict float
order), so f32 is pessimistic; int8 and popcount were.

| [measured] | f32 | int8 | 1-bit, top 400 | rescore 400 with int8 |
|---|---|---|---|---|
| 10,000 × 384 | 2.9 ms (15 MB) | 0.12 ms (4 MB) | 0.07 ms (0.5 MB) | 0.01 ms |
| 100,000 × 384 | 26 ms (154 MB) | 1.3 ms (38 MB) | 0.54 ms (4.8 MB) | 0.01 ms |
| 1,000,000 × 384 | 258 ms (1.5 GB) | 34 ms (384 MB) | **6.8 ms (48 MB)** | 0.03 ms |
| 1,000,000 × 768 | 2.5 s (3.1 GB) | 236 ms (768 MB) | 25 ms (96 MB) | 0.06 ms |

On a CPU 3–5× slower per core, 1 million 384-dimension 1-bit vectors take ≈ 20–35 ms
**[estimate]**: **no approximate-nearest-neighbour index is needed below ~3–5 million chunks**.
The limits are RAM and cold reads from disk, not CPU.

### Quantization

- int8 keeps ~97 % of retrieval quality (~99 % with rescoring); 1-bit keeps ~92.5 %, and
  **96.45 % when the top 4× candidates are rescored**, at 32× less space
  ([Hugging Face, embedding quantization](https://huggingface.co/blog/embedding-quantization))
  **[published]**.
- Matryoshka models can be cut to fewer dimensions: 768 → 256 keeps ~99.9 % nDCG@10 in one
  paper ([arXiv 2608.19957](https://arxiv.org/pdf/2608.19957)) **[published]**. API models
  allow it too (OpenAI `dimensions`, Voyage 256/512/1024).
- Product quantization only pays inside IVF or DiskANN, at sizes VFS will not reach.

| Chunks [estimate: bytes × count] | 1-bit, 384 d (in RAM) | int8, 384 d (on disk) | int8, 512 d (API, cut) |
|---|---|---|---|
| 60,000 docs ≈ 540,000 | 26 MB | 207 MB | 276 MB |
| 1,000,000 | 48 MB | 384 MB | 512 MB |
| 500,000 docs ≈ 4.5 M | 216 MB | 1.7 GB | 2.3 GB |

At 4.5 million chunks the 1-bit matrix alone would take most of the app's 300 MB budget. There,
use 256 dimensions (144 MB), or load the matrix only while hybrid search is in use.

### Where the vectors live

| Option | Same file and transaction as the chunks | Verdict |
|---|---|---|
| **BLOB columns in `content.db`** (1-bit + int8 per chunk, model id and dimension per vector set) | yes | **chosen**: no dependency, a scan in ~50 lines of Rust |
| sqlite-vec 0.1.9 (MIT/Apache) | yes | acceptable alternative; ANN only in 0.1.10 alphas ([releases](https://github.com/asg017/sqlite-vec/releases)) |
| sqlite-vector (sqliteai) | yes | **excluded: Elastic License 2.0** ([repo](https://github.com/sqliteai/sqlite-vector)) |
| usearch 2.26, hannoy, Qdrant Edge 0.7 (~11 MB) | no, a second store | only beyond ~3–5 M chunks, as a derived index |
| LanceDB | no | excluded: Arrow + DataFusion, tens of MB ([lib.rs](https://lib.rs/crates/lancedb)) |
| DuckDB VSS, Milvus Lite, Chroma | no | excluded: experimental persistence, Python only, client crate only |

Store a vector set per `(provider, model, dimension)`. Switching models builds a new set beside
the old one, and the old set is deleted only when the new one is complete. This avoids
Smart Connections' "dimension mismatch, delete and re-embed" problem
([issue 594](https://www.github.com/brianpetro/obsidian-smart-connections/issues/594)).

## 4. Embedding models: local vs API

### Measured on the development Mac

multilingual-e5-small (MIT, 384 dimensions) through ONNX Runtime 1.32 from Python, chunks of
489 tokens (Spanish word salad), batch 8, load average 14 from other sessions (scratch harness).

| [measured] | 1 thread | 4 threads | Peak RSS |
|---|---|---|---|
| int8 (118 MB model) | 208 ms/chunk | 108 ms/chunk (9.3/s) | 0.8–1.0 GB |
| fp32 (470 MB model) | 469 ms/chunk | 145 ms/chunk | 1.2–1.7 GB |

The design doc estimated 15–20 chunks/s on the target PC for a model of this class. That was
off by about 4–8×. Corrected **[estimate]**: one core of the target PC is 2.5–3× slower than
the M1 Pro, so ≈ 0.5–0.6 s per chunk, and with two threads (the rest of the PC stays usable):

| Embedding time on the target PC [estimate] | 60,000 docs (540 k chunks) | 500,000 docs (4.5 M chunks) |
|---|---|---|
| e5-small / granite-97m-r2 / bekko-a25m class | ≈ 40–90 h | ≈ 2–4 weeks |
| bekko-a8m (7.7 M active parameters, ~1.6× e5-small's speed [published]) | ≈ 25–55 h | ≈ 1.5–3 weeks |
| Static model (model2vec potion-multilingual) | minutes | < 1–2 h |

Spread over the hours the app is open, that is weeks of background work for a mid-size
archive. A local model is therefore an option for capable PCs, not the default.

### Candidate local models (late 2026)

| Model | Active / total params | Dims | Licence | MMTEB multilingual retrieval | Notes |
|---|---|---|---|---|---|
| bekko-embedding-v1-a8m | 7.7 M / 106 M | 384 (Matryoshka 256/128/64) | MIT | 56.2 | 8k tokens; 124 MiB ONNX; one author, self-reported ([arXiv 2607.25180](https://arxiv.org/pdf/2607.25180)) |
| granite-embedding-97m-multilingual-r2 | 28 M / 97 M | 384 | Apache-2.0 | **60.3** | IBM, Spanish "enhanced", ONNX and OpenVINO ([model card](https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2)) |
| multilingual-e5-small | 22 M / 118 M | 384 | MIT | 50.9 | widely used, 512 tokens; measured above |
| potion-multilingual-128M (static) | lookup table | 256 | MIT | ~38 (other scale) | 100–500× faster, pure Rust (`model2vec-rs`), clearly weaker |
| EmbeddingGemma-300M, harrier-270m, gte-multilingual-base | ~100 M active | 640–768 | mixed | 57–66 | 6–12 days per million chunks: too slow |
| bge-m3, Qwen3-Embedding-0.6B | 300–440 M | 1024 | MIT/Apache | 55–62 | weeks: infeasible |
| jina-embeddings-v3/v5 | — | — | **CC-BY-NC** | — | excluded by licence |

All scores are **[published]** on model cards and in the bekko paper. No independent
Spanish-only leaderboard was found. One published Spanish comparison (in the bekko paper, cited in the design doc) has
BM25 ahead of every dense model on long Spanish documents (NanoMLDR-es 0.944 vs best dense
0.908) and behind on short passages (NanoBEIR-es 0.506 vs 0.53–0.60).

**Runtime risks for a local model**: the `ort` crate is still 2.0 release candidates (ONNX
Runtime 1.28–1.31). The official Windows CPU package is a ~73 MB zip, and some builds require
AVX2 ([docs.rs](https://docs.rs/crate/ort/latest),
[pre-AVX2 report](https://cdn.jsdelivr.net/gh/syncable-dev/memtrace-public@main/docs/pre-avx2-cpus.md))
**[published]**. Pure-Rust `candle`/`tract` and `model2vec-rs` avoid the native DLL, at a speed
cost. The download is the model plus the runtime (≈ 130–200 MB) **[estimate]**. It must run in its
own process, like the extraction worker, so the app stays under its memory budget.

### API embeddings

Anthropic has no embeddings API and points to Voyage AI
([docs](https://platform.claude.com/docs/en/build-with-claude/embeddings)) **[published]**.

| Provider [published price; cost estimate at 450 tokens/chunk] | $/M tokens | 1 M chunks | 4.5 M chunks |
|---|---|---|---|
| OpenAI text-embedding-3-small (`dimensions` cut) | 0.02 | ≈ $9 | ≈ $41 |
| Voyage-4-lite (256–2048 d; first 200 M tokens free) | 0.02 | ≈ $5 | ≈ $37 |
| Voyage-4 | 0.06 | ≈ $27 | ≈ $122 |
| Cohere Embed 5 Fast | 0.08 | ≈ $36 | ≈ $162 |
| OpenAI text-embedding-3-large | 0.13 | ≈ $59 | ≈ $263 |

Prices: [OpenAI](https://developers.openai.com/api/docs/pricing),
[Voyage](https://docs.voyageai.com/docs/pricing),
[Cohere Embed 5](https://www.hpcwire.com/aiwire/2026/10/01/cohere-releases-embed-5-with-pro-and-fast-tiers-for-enterprise-ai/).

- **Each query is embedded too**: ~15 tokens, about $0.0000003, plus 0.1–0.3 s of network per
  search **[estimate]**. Without a connection, the documents' vectors stay on disk but the query
  cannot be embedded, so search falls back to BM25.
- **Upload**: about 0.9 GB of text per million chunks: hours on a home line, with rate limits
  **[estimate]**.
- **Lock-in**: vectors belong to one model. Changing provider, model or key means paying and
  uploading again.
- **Retention**: OpenAI keeps API abuse logs up to 30 days and does not train on API data by
  default ([your data](https://developers.openai.com/api/docs/guides/your-data)). Anthropic
  deletes API inputs within 30 days and does not train on them
  ([commercial terms](https://www.anthropic.com/legal/commercial-terms)) **[published]**.
  Voyage's terms were not checked.

## 5. Hybrid fusion and reranking

- **RRF** (k = 60) needs no tuning and no score calibration. It gave +18 % nDCG@10 over BM25
  on a BEIR subset. A weighted sum tuned on ~40 labelled queries does better on average
  ([Elastic](https://www.elastic.co/search-labs/blog/improving-information-retrieval-elastic-stack-hybrid))
  **[published]**. Across MIRACL languages, BM25 scores 39.3, a dense model 41.5 and their hybrid
  57.8 nDCG@10 ([MIRACL](https://arxiv.org/pdf/2210.09984)) **[published]**.
- **Local cross-encoder rerankers are not viable on the target PC**: 50 candidates of 512
  tokens take 15–90 s on CPU **[estimate]**. jina-reranker-v2 is CC-BY-NC.
- **API reranking** (Voyage rerank-2.5-lite $0.02/M tokens, rerank-2.5 $0.05/M) costs about
  $0.5–1.4 per 1,000 queries of 50 chunks **[estimate from published prices]**. It is a later
  option, not v1.

## 6. Chunks, citations and one store for search and RAG

- Chroma's study: 400-token chunks without overlap reached recall 89.5. The 800/400-overlap
  default it attributes to OpenAI's file search had the worst precision. Overlap helped only weak
  embedding models
  ([Chroma](https://www.trychroma.com/research/evaluating-chunking)) **[published]**.
- **Decision (Q6)**: keep VFS-2's chunks of ~2,000 characters (≈ 450–550 Spanish tokens),
  tiling the text, never crossing a page, without stored overlap. Context for a model comes from
  joining neighbouring ordinals at retrieval time. Fixed before any vector is computed, because
  re-chunking invalidates vectors.
- **Citations**: each passage carries `file_id`, chunk ordinal, character range and page. With
  Claude, passages go as `search_result` blocks whose `source` is a VFS reference (file + page +
  offset), and the model returns citations to them
  ([Anthropic](https://platform.claude.com/docs/en/build-with-claude/search-results))
  **[published]**. With OpenAI, passages are numbered and the answer must carry `[n]` markers
  (structured output). Every citation opens "mostrar en carpeta" or the file. NotebookLM is the
  reference for the interface: numbered markers, hover shows the passage, click goes to it
  ([NotebookLM](https://support.google.com/notebooklm/answer/14276569)).
- **Contextual Retrieval** (a model prepends 50–100 tokens of document context to each chunk)
  cut retrieval failures by 35–67 % in Anthropic's tests, at $1.02 per million document tokens
  then ([Anthropic](https://www.anthropic.com/engineering/contextual-retrieval)) **[published]**.
  That is too costly for a whole archive, but a later option for one chosen folder.

**One retrieval layer** serves every consumer:

```
content.db: documents → chunks (offsets, page) → FTS5 keys [+ vector sets per model]
                         │
            retrieve(query, filters, k) → ranked passages {file, chunk, range, page, score}
                         │
   ┌──────────────┬──────┴─────────┬──────────────────┬───────────────────┐
 content results   RAG answers      bibliography       later: MCP server,
 (snippet)         (passages+cite)  (cited documents)  graph, notes
```

## 7. "Ask your archive" (AI phase), v1

- **Retrieval**: the user's model (a cheap tier such as Claude Haiku 5.5 or gpt-5.6-luna)
  turns the question into a strict JSON object (keywords, phrases, Spanish/English synonyms,
  kind, dates, folder hint). VFS checks the folder against the real tree and runs BM25 locally,
  1–3 rounds. The 10–20 best passages go to the model.
- **Evidence**: past ~10 M corpus tokens, BM25 beats a file-reading agent by up to ~20 points
  while the agent spends 39× more tokens. An agent over BM25 results scored best (69.4 vs 54.8
  for BM25 alone) ([BM25 Wins at Scale, arXiv 2607.26497](https://huggingface.co/papers/2607.26497)).
  Keyword-only agents reach >90 % of vector RAG ([arXiv 2602.23368](https://arxiv.org/abs/2602.23368))
  **[published]**. VFS's corpus is 0.25–2 billion tokens **[estimate]**.
- **Cost per question** (15 passages ≈ 9.3 k tokens in, 600 out) **[estimate from published
  prices]**: Claude Haiku 5.5 ≈ $0.0012, gpt-5.6-luna ≈ $0.0026, Claude Sonnet 5.5 ≈ $0.025,
  Claude Opus 5.5 ≈ $0.049; 3–4× more with several search rounds.
- **Scope** (owner): ask the whole archive with citations; a bibliography of the cited or found
  documents for researchers (#84). The question respects the filters already on screen (folder,
  kind, date), which gives "ask within this folder" without new interface. Asking over
  hand-picked documents is not in v1.
- **Privacy** (owner): per question only the passages used leave the PC, and the user can see
  what was sent. Embedding by API (the whole text, once) needs its own explicit consent, per
  folder.

## 8. How comparable apps do it

| Product | Lexical | Vectors | Lesson for VFS |
|---|---|---|---|
| Recoll | Xapian | none | index ≈ data size; superlinear writes → split and merge ([manual](https://www.recoll.org/usermanual/webhelp/docs/RCL.INDEXING.STORAGE.BIG.html)); users ask for I/O throttling |
| DocFetcher | Lucene | none | out of memory on big folders/files → size caps ([changelog](https://docfetcherpro.com/changelog-docfetcher-pro/)) |
| Everything 1.5 | in-RAM content | none | content sized for "a couple 100MB of raw text" ([forum](https://www.voidtools.com/forum/viewtopic.php?p=52520)) |
| Spotlight (macOS 26) | own | not documented | OCR deferred until idle; missing progress UI is the top complaint ([Eclectic Light](https://eclecticlight.co/2026/02/10/in-the-background-spotlight-indexing/)) |
| Windows Copilot+ search | ESE | on-device, needs 40+ TOPS NPU | cloud-only OneDrive files: name only; "a couple of hours" first index ([Microsoft](https://support.microsoft.com/windows/indexing-in-windows-faq-da061c83-af6b-095c-0f7a-4dfecda4d15a)) |
| Copernic | own | none | best throttle: waits 15 s between items while the user is active, pauses above 40 % CPU/I/O ([docs](https://copernicsearch.freshdesk.com/en/support/solutions/articles/47001113306-tools-menu-indexing-performance)) |
| Obsidian Smart Connections | none | bge-micro-v2 (384 d) local | freezes CPU-only laptops; model switch forces a wipe |
| AnythingLLM | none | MiniLM + LanceDB | 1,000-char chunks; sources with page; embedding cache survives rebuilds ([docs](https://docs.anythingllm.com/setup/vector-database-configuration/local/lancedb)) |
| GPT4All LocalDocs | ? | nomic-embed v1.5 | per-collection progress; use what is ready while the rest indexes ([docs](https://docs.gpt4all.io/gpt4all_desktop/localdocs.html)) |
| Open WebUI | BM25 | yes | hybrid and cross-encoder reranking opt-in ([docs](https://docs.openwebui.com/features/rag)) |
| Paperless-ngx 3.0 | Tantivy | add-on | OR default returns most documents: default to AND |
| Zotero 7/8 | own SQLite | plugins | caps 500 k chars / 100 pages per file, indexes when idle; AI plugins add MCP servers ([Zotero](https://www.zotero.org/support/searching)) |
| Reor, Rewind | — | — | archived (2026) / shut down after acquisition (2025): open, file-based storage matters |

Lessons that hold across them:
- Content indexing is opt-in in third-party tools, with caps.
- First indexing on large archives needs a resumable queue, visible progress with time left,
  pause, and throttling when the user is active.
- Name search is never blocked.
- Cloud placeholders are never opened.
- OCR is opt-in, cached and runs last.

## 9. Owner decisions (2026-11-21)

| # | Question | Decision |
|---|---|---|
| Q1 | Where content hits appear | **Separate section** below name and path hits ("En el contenido de los documentos"), with the highlighted fragment |
| Q2 | On by default? | **Offered after name indexing**: VFS proposes it with the document count, estimated time and disk; the user says yes or no |
| Q3 | Disk on the system drive | **Full text kept** (snippets anywhere, citations later); size shown in Settings; warning when free space is low |
| Q4 | Cloud-only placeholders | **Skipped**: never opened; searchable by name, marked "not read (in the cloud)" |
| Q5 | OCR | **Later, optional**: off by default, per folder, after normal extraction; scanned PDFs marked "no text (scanned)" |
| Q6 | Chunk size | ~2,000 characters, no stored overlap, neighbours joined at retrieval (agent, §6; owner delegated) |
| E1 | Embeddings | **BM25 by default and offline. Hybrid opt-in through an embedding API key, plus a local model downloadable from Settings** (size, time and RAM shown first). Hybrid falls back to BM25 offline |
| E2 | Embedding provider when the user only has an Anthropic key | **A separate optional key** for embeddings (OpenAI or Voyage); otherwise the local model or BM25 |
| P1 | What may leave the PC | **Only what is needed, visibly**: the passages of each question. Embedding by API needs separate consent per folder |
| A1 | "Ask" v1 must | Ask the **whole archive with citations**, and a **bibliography** of the documents (#84); respect current filters; no hand-picked-documents mode in v1 |
| T1 | Background pace | **Throttle when the PC is in use**: lowest priority always, much slower while the user is active or the disk is busy, full pace when idle; pause button and progress with time left |
| M1 | MCP connector for the user's own AI app | **Future line** in `docs/vision.md`; does not change the plan |
| S1 | Search levels | Settings switches: names only → + content (BM25) → + hybrid (API or local) |

## 10. Lightweight and reusable by design

- **Binary size**: Xberg adds ~34 MB **[measured]**. Keep it in a **separate extraction
  executable**, never linked into the app or loaded into its memory. It can ship in the
  installer, or download when the user accepts content search; the VFS-27 spec decides.
  No vector database, no reranker, no bundled model. The local embedding model and any OCR
  engine are downloads on request.
- **Memory**: the app keeps its 300 MB budget. Extraction, embedding and any OCR run in child
  processes with their own caps. The 1-bit matrix stays in RAM only while hybrid search is on
  (26 MB at 60,000 documents **[estimate]**).
- **Reusable components**: one crate per responsibility, with narrow interfaces:
  - extraction worker (`ContentExtractor`, already in `vfs-core`);
  - `content.db` store and queue;
  - a `Retriever` that returns passages (BM25 now, vectors later, fused by RRF);
  - an `Embedder` trait (API providers, local model);
  - a `ChatModel` trait (Anthropic, OpenAI).

  Search, RAG, bibliography and a later MCP server all call `retrieve`.
- **Benchmarks from the start**:
  - keep PR 98's corpus generator and harnesses (`bench/`);
  - add a **golden set of ~40 labelled Spanish content queries** (the name-search golden tests of
    VFS-11 are the model). It measures BM25 quality, decides whether hybrid helps, and tunes the
    fusion weight;
  - track query p95, store size per 1,000 documents, RSS and documents per second for
    regressions;
  - add content items to the target-PC measurement plan: document count, Defender cost per
    opened file, AVX2, free space on C:;
  - record timings in the local diagnostic log (VFS-5) so tuning can follow real use.

## 11. Phased plan

| Step | What | Gate before the next step |
|---|---|---|
| **3a** · VFS-2 | Finish the engine: rebase PR 98, Windows CI, frequency guard and AND default, owner answers into the spec; then merge (CLI only, feature off) | Spec approved; CI green on both platforms |
| **3b** · VFS-27 | Content search in the app: offer after name indexing with estimates; separate result section; adaptive pace, pause and progress; cloud-only and scanned files marked; Settings switch and store size; extraction executable shipped apart | Measured on the real PC: document count, time per document, Defender, store size. **Above ~100,000 documents, add Tantivy as a derived index** |
| 3c · later | OCR, optional per folder (#28) | Owner reopens it |
| **5a** · VFS-28 | AI module: keys in the OS credential store (answers: Anthropic or OpenAI; embeddings: OpenAI or Voyage, optional); "what was sent" view; "ask the archive" with BM25 retrieval and citations, respecting filters; bibliography (#84) | Golden questions answered with correct citations; cost per question shown |
| **5b** · VFS-29 | Hybrid search: API embeddings opt-in per folder, vector sets per model in `content.db`, 1-bit + int8, RRF, offline fallback; then the optional local model (spike: bekko-a8m vs granite-97m-r2 vs a static model; ONNX Runtime vs pure Rust; AVX2 check on the real PC) | Golden set shows hybrid beats BM25 on Spanish content queries; local model's time per chunk measured on the real PC |
| later | Contextual retrieval for a chosen folder; API reranking; MCP connector; graph | Each with its own spec |

## Explicitly unverified

- Everything on the target PC: CPU generation and AVX2, Defender cost, free space, real
  document count and size.
- Embedding throughput of bekko and granite (published by their authors only). Quality of any
  model on this archive's Spanish documents.
- Both Mac harnesses in this document ran on random vectors or synthetic text, on a machine
  shared with other sessions.
- Third-party prices change; the costs here use November 2026 published prices.
- Voyage's and Cohere's data-retention terms.

## Sources (beyond those linked inline)

- Extraction and engines: PR 98 measurements (`docs/content-search-measurements.md` on
  `feat/vfs-2-content-search`); [SeekStorm](https://docs.rs/crate/seekstorm/3.3.5);
  [hannoy](https://meilisearch.com/docs/resources/internals/hannoy);
  [Qdrant Edge](https://qdrant.tech/documentation/edge/);
  [DuckDB VSS](https://duckdb.org/docs/current/core_extensions/vss);
  [usearch](https://docs.rs/crate/usearch/2.25.2/source/rust/README.md).
- Models: [bekko blog](https://huggingface.co/blog/hotchpotch/bekko-embedding);
  [potion-multilingual-128M](https://huggingface.co/minishlab/potion-multilingual-128M);
  [harrier-oss-v1-270m](https://huggingface.co/microsoft/harrier-oss-v1-270m);
  [Vespa on e5 int8](https://blog.vespa.ai/accelerating-transformer-based-embedding-retrieval-with-vespa/);
  [model2vec-rs](https://docs.rs/crate/model2vec-rs); [fastembed](https://docs.rs/crate/fastembed/6.0.0).
- RAG: [Is Grep All You Need?, arXiv 2605.15184](https://arxiv.org/abs/2605.15184);
  [chunk size by answer type, arXiv 2505.21700](https://arxiv.org/html/2505.21700v2);
  [Claude pricing](https://platform.claude.com/docs/en/about-claude/pricing);
  [Voyage rerank pricing](https://docs.voyageai.com/pricing).
- Apps: [Smart Connections](https://github.com/brianpetro/obsidian-smart-connections);
  [AnythingLLM text splitting](https://docs.anythingllm.com/setup/embedder-configuration/text-splitting);
  [Recoll OCR](https://www.recoll.org/usermanual/webhelp/docs/RCL.INDEXING.OCR.html);
  [Windows Search and OneDrive placeholders](https://techcommunity.microsoft.com/discussions/onedriveforbusiness/does-win10-search-index-file-on-demand-cloud-only-files/324671);
  [Reor](https://github.com/reorproject/reor);
  [Rewind/Limitless](https://9to5mac.com/2025/12/05/rewind-limitless-meta-acquisition/).
