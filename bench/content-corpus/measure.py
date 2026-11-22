#!/usr/bin/env python3
"""VFS-2 scale measurement: name-index a corpus, content-index it, time a query set.

    measure.py --vfs target/release/vfs --corpus DIR --work DIR [--queries FILE]

`vfs` must be built with `--features content`. Writes `<work>/measure.json` and prints a
summary. macOS dev-machine tool (uses /usr/bin/time -l for peak memory).
"""

import argparse
import json
import random
import re
import sqlite3
import statistics
import subprocess
import time
from pathlib import Path

BASE_QUERIES = [
    "factura", "contrato", "calefacción", "calefaccion", "facturas", "niño", "PEÑALARA",
    "hipoteca", "testamento", "cigüeña", "contrato alquiler", "factura banco",
    "señor muñoz", "\"pagó la factura\"", "\"contrato de alquiler\"", "\"el señor muñoz\"",
    "amortización préstamo", "indemnización seguro", "receta tortilla", "junta propietarios",
    "arrendamiento", "comunicación", "declaración renta hacienda", "xyzzy", "notaría escritura",
]


def timed(cmd):
    """Runs cmd under /usr/bin/time -l; returns (stdout, wall s, peak RSS bytes)."""
    t = time.time()
    p = subprocess.run(["/usr/bin/time", "-l", *cmd], capture_output=True, text=True)
    wall = time.time() - t
    if p.returncode != 0:
        raise SystemExit(f"{cmd} failed: {p.stderr[-2000:]}")
    m = re.search(r"(\d+)\s+maximum resident set size", p.stderr)
    return p.stdout, wall, int(m.group(1)) if m else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vfs", required=True)
    ap.add_argument("--corpus", required=True, type=Path)
    ap.add_argument("--work", required=True, type=Path)
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--queries-only", action="store_true",
                    help="reuse the index in --work; only time the queries")
    a = ap.parse_args()
    a.work.mkdir(parents=True, exist_ok=True)
    db = a.work / "index.db"
    truth = [json.loads(l) for l in open(a.corpus / "truth.jsonl", encoding="utf-8")]
    if a.queries_only:
        out = json.loads((a.work / "measure.json").read_text())
        return time_queries(a, db, truth, out)
    for f in a.work.glob("*.db*"):
        f.unlink()
    source_bytes = sum((a.corpus / t["path"]).stat().st_size for t in truth)
    out = {"documents": len(truth), "source_bytes": source_bytes}

    _, wall, rss = timed([a.vfs, "index", "--db", str(db), str(a.corpus)])
    out["name_index"] = {"wall_s": wall, "peak_rss": rss, "db_bytes": db.stat().st_size}

    so, wall, rss = timed([a.vfs, "content-index", "--db", str(db), "--json"])
    run = json.loads(so)
    out["content_index"] = {"wall_s": wall, "peak_rss_time_l": rss, **run}
    store = a.work / "content.db"
    c = sqlite3.connect(store)
    c.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    blob = c.execute("SELECT coalesce(sum(length(text)),0), count(*) FROM docs WHERE state=2").fetchone()
    out["store"] = {"bytes": store.stat().st_size, "text_blob_bytes": blob[0], "indexed": blob[1],
                    "chunks": c.execute("SELECT count(*) FROM chunks").fetchone()[0],
                    "by_reason": dict(c.execute("SELECT coalesce(reason,'indexed'), count(*) FROM docs GROUP BY 1").fetchall())}
    c.close()
    # Saved now: the query phase below must not be able to lose the indexing figures.
    (a.work / "measure.json").write_text(json.dumps(out, indent=2, ensure_ascii=False))

    time_queries(a, db, truth, out)


def time_queries(a, db, truth, out):
    # Rare-word queries: markers of 15 random documents (each shared by only a few documents).
    rng = random.Random(3)
    markers = [t["marker"] for t in rng.sample([t for t in truth if t.get("marker")], 15)]
    queries = BASE_QUERIES + markers
    lat, per, cold_lat, cpu_lat = [], [], [], []
    for q in queries:
        # Cold: a fresh process and connection (first query after start). Warm: the median
        # of --runs repeats on one connection, as the app runs them.
        so = subprocess.run([a.vfs, "content-search", "--db", str(db), "--json", q],
                            capture_output=True, text=True, check=True).stdout
        cold = json.loads(so)["content"]["elapsedMs"]
        so = subprocess.run([a.vfs, "content-search", "--db", str(db), "--json",
                             "--runs", str(a.runs), q],
                            capture_output=True, text=True, check=True).stdout
        full = json.loads(so)
        r = full["content"]
        med = r["elapsedMs"]
        cold_lat.append(cold)
        cpu_lat.append(full["medianCpuMs"])
        lat.append(med)
        per.append({"q": q, "warm_median_ms": med, "cold_ms": cold, "cpu_ms": full["medianCpuMs"],
                    "total": r["total"],
                    "capped": r["totalCapped"]})
    def pcts(xs):
        s = sorted(xs)
        return {"p50_ms": s[len(s) // 2], "p95_ms": s[min(len(s) - 1, int(round(0.95 * len(s))) - 1)],
                "max_ms": s[-1]}
    out["queries"] = {"n": len(queries), "warm": pcts(lat), "cold": pcts(cold_lat),
                      "warm_cpu": pcts(cpu_lat), "per_query": per}
    (a.work / "measure.json").write_text(json.dumps(out, indent=2, ensure_ascii=False))
    ci = out["content_index"]
    print(json.dumps({k: out[k] for k in ("documents", "source_bytes", "name_index", "store")}, indent=2, ensure_ascii=False))
    print(f"content-index: {ci['wall_s']:.0f} s, peak RSS {ci['peakRssBytes'] / 2**20:.0f} MB indexer, "
          f"{ci['workerPeakRssBytes'] / 2**20:.0f} MB worker; queries warm {out['queries']['warm']}, "
          f"cold {out['queries']['cold']}, warm CPU {out['queries']['warm_cpu']}")


if __name__ == "__main__":
    main()
