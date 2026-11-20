#!/usr/bin/env python3
"""VFS-8: a background workload that keeps a tree busy the way a sync client and an antivirus
do on the reference machine, at a configurable rate, until it is killed or --seconds pass.

Per second, on average:
  --scan-rate  files opened, stat-ed and read (up to 64 KiB), as an on-access scanner does;
  --touch-rate files whose modification time is bumped, as a sync client re-stamping files;
  --churn-rate temporary files created, written (--churn-kb) and deleted in a ".sync-tmp"
               folder under the tree, as a sync client downloading.
Stops when --stop-file exists, on SIGTERM, or after --seconds; then writes a JSON summary to
--report.
"""

import argparse
import json
import os
import random
import signal
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gen_tree import fs_path  # noqa: E402


def all_files(root):
    out = []
    for d, _, files in os.walk(fs_path(root)):
        out.extend(os.path.join(d, f) for f in files)
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("root")
    ap.add_argument("--scan-rate", type=float, default=200)
    ap.add_argument("--touch-rate", type=float, default=5)
    ap.add_argument("--churn-rate", type=float, default=2)
    ap.add_argument("--churn-kb", type=int, default=2048)
    ap.add_argument("--seconds", type=float, default=7200)
    ap.add_argument("--report", default="")
    ap.add_argument("--stop-file", default="")
    args = ap.parse_args(argv)
    rng = random.Random(7)
    files = all_files(args.root)
    tmp = os.path.join(fs_path(args.root), ".sync-tmp")
    os.makedirs(tmp, exist_ok=True)
    stats = {"scanned": 0, "touched": 0, "churned": 0, "errors": 0, "files_known": len(files),
             "args": vars(args)}
    stop = {"now": False}
    signal.signal(signal.SIGTERM, lambda *_: stop.update(now=True))
    if hasattr(signal, "SIGBREAK"):
        signal.signal(signal.SIGBREAK, lambda *_: stop.update(now=True))
    blob = os.urandom(args.churn_kb * 1024)
    t0 = time.time()
    tick = 0.1
    while not stop["now"] and time.time() - t0 < args.seconds:
        if args.stop_file and os.path.exists(args.stop_file):
            break
        t = time.time()
        for _ in range(int(args.scan_rate * tick + rng.random())):
            p = rng.choice(files)
            try:
                os.stat(p)
                with open(p, "rb") as f:
                    f.read(65536)
                stats["scanned"] += 1
            except OSError:
                stats["errors"] += 1
        for _ in range(int(args.touch_rate * tick + rng.random())):
            p = rng.choice(files)
            try:
                os.utime(p, None)
                stats["touched"] += 1
            except OSError:
                stats["errors"] += 1
        for _ in range(int(args.churn_rate * tick + rng.random())):
            p = os.path.join(tmp, "~dl%08d.tmp" % rng.randrange(10**8))
            try:
                with open(p, "wb") as f:
                    f.write(blob)
                    f.flush()
                    os.fsync(f.fileno())
                os.remove(p)
                stats["churned"] += 1
            except OSError:
                stats["errors"] += 1
        time.sleep(max(0.0, tick - (time.time() - t)))
    stats["seconds"] = round(time.time() - t0, 1)
    if args.report:
        with open(args.report, "w") as f:
            json.dump(stats, f, indent=2)
    print(json.dumps(stats))
    return 0


if __name__ == "__main__":
    sys.exit(main())
