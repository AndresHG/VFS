# embed-eval: the VFS-29b local embedding benchmark

Picks the light local embedding model VFS ships by default for search by meaning
(VFS-29 R16, D12–D14). Results and the recommendation: `docs/research/embed-eval-29b.md`.

Two parts:

- **Quality** (`bench.py`): the VFS-29 gate. BM25 from the real engine against hybrid search
  (1-bit scan, int8 rescore of the top 4×k, best chunk per document, RRF with k = 60) on the
  content golden set plus 3,097 Wikipedia distractors and 64 synthetic documents.
- **Cost** (`time_models.py`): time per chunk on 1 and 2 threads, peak RAM and download size,
  each model in a fresh process. Meant to be repeated on the slow Windows target PC.

| File | What |
|---|---|
| `synthetic.py` | target and hard-negative documents, and the 36 meaning-only queries |
| `check.py` | proves no meaning-only query shares a content keyword with its target |
| `fetch_corpus.py` | downloads the Wikipedia distractors into `corpus/distractors.jsonl.gz` |
| `models.py` | the candidates and how each one is run |
| `corpus/NOTICE.md` | sources and licences of the corpus |
| `timing_sample.jsonl.gz` | 200 full-size chunks of the corpus (≥ 1,800 characters, 150 Spanish, 50 English), the input of `time_models.py` |

## Run the gate (macOS dev machine)

```bash
cd bench/embed-eval
uv venv -p 3.13 .venv && uv pip install -p .venv/bin/python -r requirements.txt
source "$HOME/.cargo/env" && cargo build --release -p vfs-cli --features content
.venv/bin/python check.py
.venv/bin/python bench.py build --vfs ../../target/release/vfs
.venv/bin/python bench.py bm25 --vfs ../../target/release/vfs
for m in bekko-static-512 bekko-static-1024 potion-multilingual-128M granite-97m-r2-int8 e5-small-int8 bekko-a8m; do
  .venv/bin/python bench.py embed $m
done
.venv/bin/python bench.py eval
.venv/bin/python time_models.py --out work/timing-mac.json
```

Models download from Hugging Face on first use (about 1.3 GB for all six). `work/` holds the
corpus folder, the index and the vectors; it is not committed.

## Repeat the timing on another PC

No Rust, no corpus build and no administrator rights needed; about 1.3 GB of downloads.

1. Copy the `bench/embed-eval` folder to the PC (or clone the repository).
2. Install Python 3.13 for the current user from python.org (tick "Add python.exe to PATH").
3. In a terminal in that folder:

   ```bat
   py -3.13 -m venv .venv
   .venv\Scripts\python -m pip install -r requirements.txt
   .venv\Scripts\python time_models.py --out timing-target.json
   ```

4. Leave the PC alone while it runs (about 10–30 minutes on a slow PC) and keep the sync
   client as it normally is: the JSON records the system CPU load sampled during each model,
   the CPU name, cores, RAM and whether the CPU has AVX2.
5. Send back `timing-target.json`.

`--models` and `--threads` narrow the run (`--threads 1 2 4`), `--chunks` changes the sample
size (default 200).
