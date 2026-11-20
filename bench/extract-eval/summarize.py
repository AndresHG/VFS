#!/usr/bin/env python3
"""Summarise `extract-eval run` output: one table per backend, by format, plus hostile files.

    summarize.py RESULTS_DIR            # every <backend>.jsonl in the folder
"""

import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path


def pct(x):
    return "—" if x is None else f"{100 * x:.1f}"


def mean(xs):
    xs = [x for x in xs if x is not None]
    return statistics.mean(xs) if xs else None


def main():
    d = Path(sys.argv[1])
    for f in sorted(d.glob("*.jsonl")):
        rows = [json.loads(l) for l in f.open()]
        good = [r for r in rows if not r["path"].startswith("hostile")]
        print(f"\n## {f.stem}\n")
        print("| format | files | ok | unsupported | error/crash/timeout | token recall % | accent recall % | marker in order % | hyphen words % | MB/s | docs/s | p50 ms | max ms | max RSS MB |")
        print("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
        by = defaultdict(list)
        for r in good:
            key = r["format"] + ("/" + r["producer"] if r["format"] == "pdf" and r.get("producer") else "")
            by[key].append(r)
        by["ALL"] = [r for r in good if r["outcome"] != "unsupported"]
        for k in sorted(by, key=lambda k: (k == "ALL", k)):
            rs = by[k]
            ok = [r for r in rs if r["outcome"] == "ok"]
            uns = sum(r["outcome"] == "unsupported" for r in rs)
            bad = sum(r["outcome"] in ("error", "crash", "timeout") for r in rs)
            done = [r for r in rs if r["outcome"] != "unsupported"]
            secs = sum(r["ms"] for r in done) / 1000 or 1e-9
            mb = sum(r["bytes"] for r in done) / 1e6
            ms = sorted(r["ms"] for r in done) or [0]
            marker = [r["marker_sentence"] for r in ok if r["marker_sentence"] is not None]
            two = [r for r in ok if r.get("two_columns")]
            print(f"| {k} | {len(rs)} | {len(ok)} | {uns} | {bad} | {pct(mean([r['token_recall'] for r in ok]))} | "
                  f"{pct(mean([r['accent_recall'] for r in ok]))} | {pct(mean(marker) if marker else None)} | "
                  f"{pct(mean([r['hyphen_words_whole'] for r in ok]))} | {mb / secs:.1f} | {len(done) / secs:.0f} | "
                  f"{ms[len(ms) // 2]:.0f} | {ms[-1]:.0f} | {max((r['peak_rss_kb'] for r in done), default=0) / 1024:.0f} |")
            if two and k.startswith("pdf"):
                tm = [r["marker_sentence"] for r in two if r["marker_sentence"] is not None]
                print(f"| {k} two-column | {len(two)} | | | | {pct(mean([r['token_recall'] for r in two]))} | | {pct(mean(tm) if tm else None)} | | | | | | |")
        hostile = [r for r in rows if r["path"].startswith("hostile")]
        if hostile:
            print("\n| hostile file | expect | outcome | chars | ms | RSS MB | error |")
            print("|---|---|---|---|---|---|---|")
            for r in hostile:
                err = (r.get("error") or "").replace("|", "/").replace("\n", " ")[:70]
                print(f"| {r['path'][8:]} | {r.get('expect')} | {r['outcome']} | {r['chars']} | {r['ms']:.0f} | {r['peak_rss_kb'] / 1024:.0f} | {err} |")


if __name__ == "__main__":
    main()
