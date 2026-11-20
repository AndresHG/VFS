#!/usr/bin/env python3
"""VFS-5 AC-6: measures the installed app on one disk condition, through `tauri-driver`:
first scan (seconds, files/s, memory peak), searches while it runs, in-app query latency after
it, memory at rest, and the start-up catch-up after changes made while the app was closed.

Usage: measure.py --app <exe> --manifest <manifest.json> --label quiet|busy --out <dir>
                  [--between "<command run while the app is closed, e.g. a disk remount>"]
Writes <out>/measure-<label>.json.
"""

import argparse
import json
import os
import statistics
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from journey import App, now, wait_for  # noqa: E402
import gen_tree  # noqa: E402

QUERIES = ["notaria valdepenas", "remedios extracto", "factura", "img", "boda iñigo ainhoa",
           "notaria valdepeñaz", "peñiscola", "2019", "copia de seguridad", "cumpleaños abuela"]
NO_FILTERS = {"kinds": [], "modifiedFrom": None, "modifiedTo": None, "sizeMin": None,
              "sizeMax": None, "folder": None}


def search(wd, text):
    t = now()
    r = wd.invoke("search", {"query": {"text": text, "filters": NO_FILTERS, "limit": 100,
                                       "offset": 0}})
    return {"engine_ms": r["elapsedMs"], "roundtrip_ms": round((now() - t) * 1000, 1),
            "total": r["total"], "truncated_by": r.get("truncatedBy")}


def scanned(wd):
    p = wd.invoke("get_progress")
    roots = p.get("roots") or []
    return p["state"] == "idle" and roots and all(r["status"] == "ready" for r in roots)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--app", required=True)
    ap.add_argument("--driver", default="http://127.0.0.1:4444")
    ap.add_argument("--tauri-driver", help="start this tauri-driver binary on :4444")
    ap.add_argument("--native-driver", help="msedgedriver.exe for tauri-driver")
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--between", default="")
    ap.add_argument("--timeout", type=int, default=3600)
    args = ap.parse_args(argv)
    args.mode = "tauri-driver"
    os.makedirs(args.out, exist_ok=True)
    m = json.load(open(args.manifest, encoding="utf-8"))
    res = {"label": args.label, "tree_files": m["files"], "tree_dirs": m["dirs"]}
    path = os.path.join(args.out, f"measure-{args.label}.json")

    def save():
        with open(path, "w", encoding="utf-8") as f:
            json.dump(res, f, ensure_ascii=False, indent=2)

    app = App(args)
    try:
        res["launch_to_ui_s"] = round(app.start(), 2)
        res["memory_start"] = app.memory()
        wd = app.wd
        victim = os.path.join(m["root"], "Documentos", f"Para borrar {args.label}.txt")
        gen_tree.write_file(victim, 0, time.time())
        t0 = now()
        wd.invoke("add_root", {"path": m["root"]})
        during, peak, samples = [], {"total_rss_mb": 0}, []
        last_q = 0.0
        while now() - t0 < args.timeout:
            if scanned(wd):
                break
            el = now() - t0
            p = wd.invoke("get_progress")
            samples.append([round(el, 1), p["filesIndexed"]])
            mem = app.memory()
            if mem["total_rss_mb"] > peak["total_rss_mb"]:
                peak = mem
            if el - last_q >= 10:
                last_q = el
                q = search(wd, "factura")
                q["at_s"] = round(el, 1)
                q["files_indexed"] = p["filesIndexed"]
                during.append(q)
            time.sleep(1)
        secs = now() - t0
        files = wd.invoke("list_roots")[0]["files"]
        res["first_scan"] = {"seconds": round(secs, 1), "files": files,
                             "files_per_s": round(files / secs) if secs else None,
                             "finished": scanned(wd), "memory_peak": peak,
                             "progress_samples": samples[:: max(1, len(samples) // 60)],
                             "search_while_scanning": during}
        save()
        time.sleep(5)
        res["memory_idle_after_scan"] = app.memory()
        lat = {}
        for q in QUERIES:
            runs = [search(wd, q) for _ in range(6)]
            first, rest = runs[0], runs[1:]
            lat[q] = {"first_engine_ms": first["engine_ms"],
                      "first_roundtrip_ms": first["roundtrip_ms"],
                      "median_engine_ms": statistics.median(r["engine_ms"] for r in rest),
                      "median_roundtrip_ms": statistics.median(r["roundtrip_ms"] for r in rest),
                      "max_roundtrip_ms": max(r["roundtrip_ms"] for r in runs),
                      "total": first["total"], "truncated_by": first["truncated_by"]}
        res["queries"] = lat
        save()
        app.stop()

        # Changes while the app is closed: 1,000 new files, one indexed file deleted.
        d = os.path.join(m["root"], "Escritorio", f"Nuevos {args.label}")
        os.makedirs(gen_tree.fs_path(d), exist_ok=True)
        for i in range(1000):
            gen_tree.write_file(os.path.join(d, f"Nuevo mientras cerrado {i:04d}.txt"), 0,
                                time.time())
        os.remove(gen_tree.fs_path(victim))
        if args.between:
            r = subprocess.run(args.between, shell=True, capture_output=True, text=True)
            res["between"] = {"cmd": args.between, "exit": r.returncode,
                              "out": (r.stdout + r.stderr)[-500:]}
        t0 = now()
        launch = app.start()
        wd = app.wd
        wait_for(lambda: search(wd, "nuevo mientras cerrado")["total"] >= 1000,
                            args.timeout, 0.5, "catch-up to find the new files")
        found = now() - t0
        wait_for(lambda: scanned(wd), args.timeout, 0.5, "catch-up to finish")
        idle = now() - t0
        res["catch_up"] = {"launch_s": round(launch, 2), "new_files_found_s": round(found, 1),
                           "idle_s": round(idle, 1),
                           "deleted_still_found": search(wd, f"para borrar {args.label}")["total"]}
        time.sleep(5)
        res["memory_idle_after_catch_up"] = app.memory()
    except Exception as e:
        res["error"] = f"{type(e).__name__}: {e}"
        raise
    finally:
        save()
        app.stop()
        app.stop_driver()
    print(json.dumps({k: res.get(k) for k in ("first_scan", "catch_up")}, default=str)[:2000])
    return 0


if __name__ == "__main__":
    sys.exit(main())
