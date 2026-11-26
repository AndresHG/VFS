#!/usr/bin/env python3
"""VFS-29b gate run: BM25 (the shipped engine) against hybrid search with each local model.

    bench.py build  --vfs PATH/TO/vfs    # corpus folder, `vfs index`, `vfs content-index`
    bench.py bm25   --vfs PATH/TO/vfs    # BM25 ranking of every query from `vfs content-search`
    bench.py embed  MODEL [--threads N]  # vectors for every chunk of content.db and every query
    bench.py eval   [MODEL ...]          # gate table; writes work/results.json

Everything is written under work/ (not committed). The chunks are the ones the real engine
stored in content.db (VFS-2 R11, ~2,000 characters, never crossing a page), and the BM25
ranking is the real `retrieve` through `vfs content-search` (the same call: Searcher::search
wraps retrieve), so only the dense half and the fusion are re-implemented here, as VFS-29 R11
specifies them: scan 1-bit vectors, rescore the top 4×k chunks with int8, best chunk per
document, reciprocal rank fusion with BM25 (k = 60).

The `vfs` binary must be built with the content feature:
    cargo build --release -p vfs-cli --features content
"""

import argparse
import gzip
import json
import re
import shutil
import subprocess
import sys
import time
import zlib
import sqlite3
from pathlib import Path

import numpy as np

from synthetic import MEANING, NEGATIVES, TARGETS

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
GOLDEN = REPO / "crates/vfs-content/tests/golden"
WORK = HERE / "work"
CORPUS = WORK / "Archivo"
DB = WORK / "index.db"

TOP = 5          # the gate: expected document in the first 5 (VFS-2 AC-17, VFS-29 gate)
K = 20           # documents each list contributes to the fusion (retrieve's k)
RESCORE = 4 * K  # chunks rescored with int8 after the 1-bit scan (R11: 4 × k)
RRF_K = 60       # reciprocal rank fusion constant (R11, D7)


def keyword_cases():
    """The keyword subset: every case of the content golden set, read from golden.rs."""
    src = (REPO / "crates/vfs-content/tests/golden.rs").read_text(encoding="utf-8")
    pairs = re.findall(r'case\(\s*"((?:[^"\\]|\\.)*)",\s*"([^"]+)",?\s*\)', src)
    return [(q.replace('\\"', '"'), f) for q, f in pairs]


def queries():
    out = [{"id": f"kw{i:02d}", "set": "keyword", "lang": "es", "q": q, "want": f}
           for i, (q, f) in enumerate(keyword_cases())]
    out += [{"id": f"mo{i:02d}", "set": "meaning", "lang": lang, "q": q, "want": f}
            for i, (q, f, lang) in enumerate(MEANING)]
    return out


# ------------------------------------------------------------------ build


def cmd_build(a):
    if WORK.exists():
        shutil.rmtree(WORK / "Archivo", ignore_errors=True)
        for f in WORK.glob("*.db*"):
            f.unlink()
    CORPUS.mkdir(parents=True, exist_ok=True)
    n = {"golden": 0, "synthetic": 0, "es": 0, "en": 0}
    for f in GOLDEN.iterdir():
        if f.name != "NOTICE.md":
            shutil.copy(f, CORPUS / f.name)
            n["golden"] += 1
    for name, text in {**TARGETS, **NEGATIVES}.items():
        (CORPUS / name).write_text(text + "\n", encoding="utf-8")
        n["synthetic"] += 1
    with gzip.open(HERE / "corpus/distractors.jsonl.gz", "rt", encoding="utf-8") as f:
        for line in f:
            r = json.loads(line)
            n[r["lang"]] += 1
            name = f"dz_{r['lang']}_{n[r['lang']]:05d}.txt"
            (CORPUS / name).write_text(r["title"] + "\n\n" + r["text"] + "\n", encoding="utf-8")
    print("corpus:", n, flush=True)
    t = time.time()
    subprocess.run([a.vfs, "index", "--db", str(DB), str(CORPUS)], check=True)
    subprocess.run([a.vfs, "content-index", "--db", str(DB)], check=True)
    print(f"indexed in {time.time() - t:.1f} s")
    chunks = load_chunks()
    print(f"{len(chunks)} chunks in content.db")


def load_chunks():
    """(name, ordinal, text) of every stored chunk, in a fixed order."""
    con = sqlite3.connect(WORK / "content.db")
    texts = {}
    for fid, name, blob in con.execute("SELECT file_id, name, text FROM docs WHERE text IS NOT NULL"):
        texts[fid] = (name, zlib.decompress(blob, -15))
    out = []
    for fid, ordinal, sb, eb in con.execute(
            "SELECT file_id, ordinal, start_byte, end_byte FROM chunks ORDER BY file_id, ordinal"):
        if fid in texts:
            name, raw = texts[fid]
            out.append((name, ordinal, raw[sb:eb].decode("utf-8")))
    return out


# ------------------------------------------------------------------ BM25


def cmd_bm25(a):
    rank, partial = {}, {}
    for q in queries():
        r = subprocess.run([a.vfs, "content-search", "--db", str(DB), "--json", "--limit", str(K),
                            q["q"]], check=True, capture_output=True, text=True)
        content = json.loads(r.stdout)["content"]
        rank[q["id"]] = [h["name"] for h in content["hits"]]
        partial[q["id"]] = content["partial"]
    (WORK / "bm25.json").write_text(json.dumps(rank, ensure_ascii=False, indent=1))
    (WORK / "bm25_partial.json").write_text(json.dumps(partial, indent=1))
    print(f"BM25 ranking of {len(rank)} queries -> work/bm25.json")


# ------------------------------------------------------------------ embed


def cmd_embed(a):
    from models import Encoder

    chunks = load_chunks()
    enc = Encoder(a.model, a.threads)
    t = time.time()
    batch = 64 if enc.spec["kind"] != "transformer" else 1
    vecs = enc.embed([c[2] for c in chunks], batch=batch)
    secs = time.time() - t
    qs = queries()
    qv = enc.embed([q["q"] for q in qs], query=True, batch=batch)
    out = WORK / "emb"
    out.mkdir(exist_ok=True)
    np.savez(out / f"{a.model}.npz", docs=vecs, queries=qv,
             names=np.array([c[0] for c in chunks]), qids=np.array([q["id"] for q in qs]))
    print(f"{a.model}: {len(chunks)} chunks in {secs:.1f} s "
          f"({1000 * secs / len(chunks):.1f} ms/chunk, {a.threads} threads)")


# ------------------------------------------------------------------ eval


def int8(v, scale):
    return np.clip(np.round(v / scale), -127, 127).astype(np.int8)


def dense_rank(docs, names, q, k=K):
    """R11: 1-bit scan, rescore the top 4×k chunks with int8, best chunk per document."""
    bits = np.packbits(docs > 0, axis=1)
    qbits = np.packbits(q > 0)
    ham = np.unpackbits(bits ^ qbits, axis=1).sum(1)
    cand = np.argsort(ham, kind="stable")[:RESCORE]
    scale = np.abs(docs).max(0) / 127.0           # per-dimension int8 calibration
    scale[scale == 0] = 1.0
    scores = (int8(docs[cand], scale).astype(np.float32) * scale) @ q
    out = []
    for i in cand[np.argsort(-scores, kind="stable")]:
        if names[i] not in out:
            out.append(names[i])
        if len(out) == k:
            break
    return out


def exact_rank(docs, names, q, k=K):
    """Reference only: float32 cosine over every chunk, best chunk per document."""
    out = []
    for i in np.argsort(-(docs @ q), kind="stable"):
        if names[i] not in out:
            out.append(names[i])
        if len(out) == k:
            break
    return out


def rrf(*lists):
    score = {}
    for lst in lists:
        for r, name in enumerate(lst):
            score[name] = score.get(name, 0.0) + 1.0 / (RRF_K + r + 1)
    return sorted(score, key=lambda n: (-score[n], n))


def hit(lst, want):
    return want in lst[:TOP]


def cmd_eval(a):
    bm25 = json.loads((WORK / "bm25.json").read_text())
    partial = json.loads((WORK / "bm25_partial.json").read_text())
    qs = queries()
    models = a.models or sorted(p.stem for p in (WORK / "emb").glob("*.npz"))
    res = {"top": TOP, "k": K, "rescore": RESCORE, "rrf_k": RRF_K, "queries": len(qs),
           "bm25": {}, "models": {}}

    def count(sel, ranks):
        return sum(hit(ranks[q["id"]], q["want"]) for q in qs if sel(q))

    def es(q):
        return q["set"] == "meaning" and q["lang"] == "es"

    def en(q):
        return q["set"] == "meaning" and q["lang"] == "en"

    def kw(q):
        return q["set"] == "keyword"

    res["bm25"] = {"meaning_es": count(es, bm25), "meaning_en": count(en, bm25),
                   "keyword": count(kw, bm25),
                   "n_meaning_es": sum(map(es, qs)), "n_meaning_en": sum(map(en, qs)),
                   "n_keyword": sum(map(kw, qs))}
    for m in models:
        z = np.load(WORK / "emb" / f"{m}.npz")
        docs, names, qv = z["docs"], z["names"], z["queries"]
        qidx = {qid: i for i, qid in enumerate(z["qids"])}
        dense, hyb, exact, strict = {}, {}, {}, {}
        for q in qs:
            v = qv[qidx[q["id"]]]
            dense[q["id"]] = dense_rank(docs, names, v)
            hyb[q["id"]] = rrf(bm25[q["id"]], dense[q["id"]])
            exact[q["id"]] = exact_rank(docs, names, v)
            # Variant, not the gate: when no document has every query word (VFS-2 R13,
            # `partial`), BM25's any-word list is left out of the fusion.
            strict[q["id"]] = rrf([] if partial[q["id"]] else bm25[q["id"]], dense[q["id"]])
        losses = [q["q"] for q in qs if kw(q) and hit(bm25[q["id"]], q["want"])
                  and not hit(hyb[q["id"]], q["want"])]
        r = {"dense_es": count(es, dense), "dense_en": count(en, dense),
             "hybrid_es": count(es, hyb), "hybrid_en": count(en, hyb),
             "dense_keyword": count(kw, dense), "hybrid_keyword": count(kw, hyb),
             "exact_es": count(es, exact), "exact_en": count(en, exact),
             "strict_es": count(es, strict), "strict_en": count(en, strict),
             "strict_keyword": count(kw, strict),
             "keyword_losses": losses,
             "meaning_hits": {q["q"]: [hit(bm25[q["id"]], q["want"]), hit(dense[q["id"]], q["want"]),
                                       hit(hyb[q["id"]], q["want"])] for q in qs if not kw(q)}}
        r["passes"] = (r["hybrid_es"] - res["bm25"]["meaning_es"] >= 4
                       and r["hybrid_en"] - res["bm25"]["meaning_en"] >= 1 and not losses)
        res["models"][m] = r
    (WORK / "results.json").write_text(json.dumps(res, ensure_ascii=False, indent=1))
    b = res["bm25"]
    print(f"BM25: meaning es {b['meaning_es']}/{b['n_meaning_es']}, en {b['meaning_en']}/"
          f"{b['n_meaning_en']}, keyword {b['keyword']}/{b['n_keyword']}")
    print(f"{'model':26s} {'dense es':>8s} {'hyb es':>6s} {'dense en':>8s} {'hyb en':>6s} "
          f"{'hyb kw':>6s} {'losses':>6s}  gate  | f32 es/en  strict es/en/kw")
    for m, r in res["models"].items():
        print(f"{m:26s} {r['dense_es']:8d} {r['hybrid_es']:6d} {r['dense_en']:8d} "
              f"{r['hybrid_en']:6d} {r['hybrid_keyword']:6d} {len(r['keyword_losses']):6d}  "
              f"{'PASS' if r['passes'] else 'fail'}  | {r['exact_es']:3d}/{r['exact_en']:<3d}    "
              f"{r['strict_es']:3d}/{r['strict_en']}/{r['strict_keyword']}")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    for c in ("build", "bm25"):
        p = sub.add_parser(c)
        p.add_argument("--vfs", required=True)
    p = sub.add_parser("embed")
    p.add_argument("model")
    p.add_argument("--threads", type=int, default=4)
    p = sub.add_parser("eval")
    p.add_argument("models", nargs="*")
    a = ap.parse_args()
    {"build": cmd_build, "bm25": cmd_bm25, "embed": cmd_embed, "eval": cmd_eval}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
