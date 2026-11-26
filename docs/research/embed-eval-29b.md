# VFS-29b: which local embedding model VFS ships by default

Spec: [VFS-29](../../specs/vfs-29-hybrid-search/spec.md) R16, the gate, D12–D14. Harness:
`bench/embed-eval/` (how to run it is in its README). Background: [content-vector-rag.md](content-vector-rag.md)
§3–§4 and its 2026-11-25 cost review. Figures are labelled **[measured]** (this run),
**[published]** (model cards) or **[estimate]**.

## The answer

- **Two local models pass the gate: multilingual-e5-small (int8) and bekko-embedding-v1-a8m.**
  e5-small passes by a wide margin (+9 Spanish, +4 English meaning-only queries over BM25),
  bekko-a8m by the smallest margin the gate allows (+4, +1). granite-97m-r2 misses by one
  keyword query, and none of the static models comes close **[measured]**.
- **Recommendation: ship multilingual-e5-small int8 as the default**, since Spanish decides (D14)
  and its Spanish margin is more than twice bekko-a8m's. bekko-a8m is the fallback if the target
  PC makes e5-small too slow: it is about 1.5× faster per chunk **[measured]**.
- **"Minutes on the target PC" is off the table for now.** The static models (bekko-static,
  potion) embed a chunk in under 2 ms, but find 2–3 of 26 Spanish meaning-only targets alone,
  and fused with BM25 they add at most 1. A small transformer is the price of meaning search:
  roughly **20–35 h of background CPU per 60,000 documents** on the target PC **[estimate]**,
  to be confirmed there with `time_models.py`.
- **API column: pending.** No OpenAI or Voyage key was available in this environment, so the
  gate's API model was not run.

## What was run

- **Corpus** [measured]: 3,192 documents, 3,640 chunks. The content golden corpus (31 files),
  36 synthetic targets and 28 synthetic hard negatives (invoices, letters, school, medical,
  insurance, utilities, manuals; `bench/embed-eval/synthetic.py`), and 3,097 Wikipedia lead
  excerpts (2,103 Spanish, 994 English; three quarters of them topical, from searches on
  family-archive themes; CC BY-SA, sources in `bench/embed-eval/corpus/NOTICE.md`).
- **Chunks and BM25 from the shipped engine.** `vfs index` + `vfs content-index` built the
  store; the chunks are the stored ones (VFS-2 R11, ≤ 2,000 characters, mean 1,000 here because
  Wikipedia leads are short). BM25 is `vfs content-search --json`, which is `retrieve` itself.
- **Queries.** Keyword subset: the 45 cases of `crates/vfs-content/tests/golden.rs`, read from
  that file. Meaning-only subset: 26 Spanish and 10 English queries, each with a target that
  shares no content keyword with it, checked by `check.py` with the analyzer's rules
  (Snowball Spanish stem, then accent fold; English queries also with the English stem).
- **Hybrid, as R11 says**: each model embeds every chunk; the query's 1-bit vector scans all
  chunks by Hamming distance, the top 4×k chunks are rescored with int8 vectors (per-dimension
  scale), the best chunk stands for its document, and the top k = 20 documents are fused with
  BM25's top 20 by reciprocal rank fusion (k = 60). Hit = expected document in the top 5.
- **Models.** Transformers through ONNX Runtime 1.32 on the CPU, inputs cut at 512 tokens
  (a full chunk is about 420–440 tokens, so almost nothing is cut); static models as published.

## Quality: the gate

Gate: hybrid finds ≥ 4 more Spanish and ≥ 1 more English meaning-only targets than BM25, and
loses no keyword query BM25 has in its top 5. BM25 found 0/26 Spanish and 0/10 English
meaning-only targets, and 45/45 keyword targets **[measured]**.

| [measured] | Spanish: dense / hybrid (of 26) | English: dense / hybrid (of 10) | Keyword: hybrid (of 45) | Gate |
|---|---|---|---|---|
| **multilingual-e5-small, int8** | 13 / **9** | 8 / **4** | 45 | **pass** |
| bekko-embedding-v1-a8m | 11 / **4** | 7 / **1** | 45 | **pass** (at the threshold) |
| granite-embedding-97m-multilingual-r2, int8 | 11 / 6 | 6 / 3 | 44 (loses «arroz Comunidad Valenciana») | fail |
| bekko-static-embedding-v1, 1024 d | 3 / 1 | 1 / 1 | 45 | fail |
| bekko-static-embedding-v1, 512 d | 2 / 1 | 2 / 0 | 45 | fail |
| potion-multilingual-128M | 2 / 1 | 2 / 1 | 45 | fail |
| OpenAI text-embedding-3-small / Voyage-4-lite | pending (no key) | | | |

"Dense" is the model alone through the same 1-bit scan and int8 rescore.

How to read it:

- **26 Spanish queries are a small sample.** One query is about 4 points; bekko-a8m passes on
  exactly 4, so a single query either way decides its pass. e5-small's 9 is not that fragile.
- **Fusion costs meaning hits.** Most meaning queries have no document with every word, so BM25
  answers with its any-word list (VFS-2 R13, `partial`): 20 documents that share a «de» or a
  «del». RRF ranks that noise level with the dense list, and about a third of the dense hits
  drop out of the top 5 (e5-small: 13 dense, 9 hybrid).
- **Statics fail on meaning, not on keywords.** Dense alone, bekko-a8m finds 43 of the 45
  keyword targets and the bekko statics 42 (potion 29), against e5-small's 24. e5-small is weak on names and
  numbers and strong on paraphrase; BM25 covers the first, so in the fusion that is the right
  trade.
- **On the published scores**, bekko-a8m is ahead of e5-small on multilingual Nano retrieval
  (MNanoBEIR 52.65 vs 51.17) and granite ahead of both on MMTEB retrieval (60.3) **[published]**.
  On this corpus of Spanish household documents the order is the other way around.

### Two findings for 29c (not part of the gate)

| [measured] | Spanish / English hybrid, as R11 | Leaving out BM25 when `partial` | Float32 dense, no quantization |
|---|---|---|---|
| e5-small int8 | 9 / 4 | **13 / 8** (keyword 45) | 18 / 7 |
| bekko-a8m | 4 / 1 | 10 / 6 (keyword 45) | 12 / 7 |
| granite-97m-r2 int8 | 6 / 3 | 11 / 6 (keyword 44) | 13 / 6 |

- **Leave BM25's any-word list out of the fusion.** When no document has every query word,
  fusing only the dense list (BM25's list is still shown where it is not `partial`) raised
  e5-small from 9 to 13 Spanish and 4 to 8 English hits with no keyword loss. D7 allows a
  change to the fusion when the golden set shows a gain; this one does. To decide in 29c.
- **The 1-bit scan loses candidates with e5-small.** Float32 search finds 18 Spanish targets
  against 13 after the 1-bit scan and int8 rescore. e5-small's vectors are all close to one
  another (cosines around 0.8), so plain sign bits carry little. In a one-off check (not in
  the harness), subtracting the corpus mean before taking the sign brought the 1-bit result to
  17 Spanish for e5-small and 13 for granite, but cut bekko-a8m from 11 to 7. 29c should
  measure centring, or rescoring more than 4×k candidates, for the shipped model.

## Cost: time, memory, download

On the development Mac (M1 Pro, 10 cores, 16 GB), 200 full-size chunks (≥ 1,800 characters,
mean 420–440 tokens; 150 Spanish, 50 English), each model in its own process. Load average was
5.3 before and 8.1 after, from other work on the machine; the 2-thread granite run saw 65 % system
CPU, so its figure is the least reliable **[measured]**.

| [measured] | Dims | ms/chunk, 1 thread | ms/chunk, 2 threads | Peak RSS, process (MiB) | Download | Licence |
|---|---|---|---|---|---|---|
| multilingual-e5-small int8 (`onnx/model_qint8_avx512_vnni.onnx`) | 384 | 145 | 77 | 656 | 129 MiB | MIT |
| bekko-embedding-v1-a8m (`onnx/model.onnx`) | 384 | 97 | 51 | 666 | 157 MiB | MIT |
| granite-97m-multilingual-r2 int8 (`onnx/model_quint8_avx2.onnx`) | 384 | 145 | 117 | 637 | 118 MiB | Apache-2.0 |
| bekko-static-embedding-v1 int8, 512 d | 512 | 0.99 | 0.60 | 808 | 103 MiB | MIT |
| bekko-static-embedding-v1 int8, 1024 d | 1024 | 3.5 | 1.4 | 1,468 | 201 MiB | MIT |
| potion-multilingual-128M (float32) | 256 | 0.73 | 0.49 | 1,602 | 506 MiB | MIT |

Licences are as stated on each model card (checked 2026-11-26). Peak RSS is the higher of the two runs, for the whole Python
process (about 57 MB before the model loads) at batch 1 for transformers and 64 for statics. For
the statics it reflects ONNX Runtime and model2vec rather than the model: bekko-static-512 sits
at 238 MB after loading and peaks during the batch. A Rust lookup of an int8 table would need
roughly the file size. The transformer figures are close to what a child process would hold.

On the target PC, one core is 2.5–3× slower than the M1 Pro (research §4) **[estimate]**. At 2 threads:

| [estimate] | per chunk | 60,000 docs (≈ 540,000 chunks) | 500,000 docs (≈ 4.5 M chunks) |
|---|---|---|---|
| multilingual-e5-small int8 | 0.19–0.23 s | **29–35 h** | 10–12 days |
| bekko-embedding-v1-a8m | 0.13–0.15 s | **19–23 h** | 7–8 days |
| granite-97m-r2 int8 | 0.29–0.35 s | 44–53 h | 15–18 days |
| static models | 1.2–1.8 ms | 11–16 min | 1.5–2.3 h |

This is about 1.5–2.5× better than the research's earlier 40–90 h estimate for this class,
which came from a slower measurement (batch 8, word salad, load 14).

## Recommendation

1. **Default: multilingual-e5-small, int8 ONNX** (MIT, 129 MiB download, 384 d). It is the only
   candidate with margin on both languages, and the Spanish margin decides (D14). It needs the
   `query: ` / `passage: ` prefixes and caps inputs at 512 tokens, which fits VFS's chunks.
2. **Fallback: bekko-embedding-v1-a8m** if the target PC's measurement puts e5-small out of
   reach. It is 1.5× faster and better on names and numbers, but it passes only at the
   threshold, it is one author's model, and its card recommends OpenVINO over ONNX Runtime for
   CPU speed.
3. **Not shipped**: granite-97m-r2 (one keyword loss, slowest here), and the static models
   (they fail meaning). bekko-static stays worth re-testing if a later version improves.
4. **In 29c**, consider the two findings above. Leaving BM25's `partial` list out of the fusion
   and fixing the 1-bit loss could take e5-small from 9 to around 13–17 Spanish hits.
5. **Before shipping**: the API model column (needs a key), and the target PC run below.

## What remains to measure on the target PC

`bench/embed-eval/time_models.py` (procedure in the README, no administrator rights needed)
records, on the PC itself:

- time per chunk on 1 and 2 threads for every candidate, and the background-hours figure above
  recomputed from it;
- peak RAM per model process, against the app's budget and VFS-29 R16's child-process cap;
- **the AVX2 check** (`IsProcessorFeaturePresent`): the int8 ONNX files are built for AVX2 or
  AVX-512 VNNI, and some ONNX Runtime builds require AVX2 (research §4);
- system CPU load during the run, with the sync client running as it normally does.

No figure for the target PC is given here beyond the estimates above, which are marked as such.
