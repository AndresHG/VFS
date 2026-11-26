#!/usr/bin/env python3
"""Cost of each VFS-29b candidate on this PC: time per chunk, peak RAM, download size (R16).

Self-contained apart from models.py and timing_sample.jsonl.gz next to it (200 full-size
VFS chunks of the benchmark corpus, 150 Spanish and 50 English). Runs each model in a fresh child process per thread count,
so the peak RAM of one model never includes another's. Built to be repeated unchanged on the
slow Windows target PC: see README.md, "Repeat the timing on another PC".

    time_models.py                           # every model, 1 and 2 threads
    time_models.py --models bekko-static-512 granite-97m-r2-int8 --threads 1 2 4
    time_models.py --out timing-target.json

Prints a table and writes the JSON with the machine description (CPU, cores, RAM, AVX2) and
the system CPU load sampled while each model ran, so a busy machine is visible in the record.
"""

import argparse
import gzip
import json
import os
import platform
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
WARMUP = 5


def peak_rss_bytes():
    if sys.platform == "win32":
        import psutil
        return psutil.Process().memory_info().peak_wset
    import resource
    v = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return v if sys.platform == "darwin" else v * 1024


def rss_bytes():
    import psutil
    return psutil.Process().memory_info().rss


def sample_chunks(n):
    with gzip.open(HERE / "timing_sample.jsonl.gz", "rt", encoding="utf-8") as f:
        rows = [json.loads(line)["text"] for line in f]
    return rows[:n]


def child(model, threads, n):
    """Runs in its own process: load one model, embed the sample, report as one JSON line."""
    import numpy  # noqa: F401  (imported before the baseline, like onnxruntime below)
    import onnxruntime  # noqa: F401
    base = rss_bytes()
    from models import Encoder

    texts = sample_chunks(n)
    t0 = time.perf_counter()
    enc = Encoder(model, threads)
    load_s = time.perf_counter() - t0
    after_load = rss_bytes()
    static = enc.spec["kind"] != "transformer"
    enc.embed(texts[:WARMUP], batch=64 if static else 1)
    c0, t0 = time.process_time(), time.perf_counter()
    enc.embed(texts, batch=64 if static else 1)
    wall = time.perf_counter() - t0
    cpu = time.process_time() - c0
    toks = [enc.tokens(t) for t in texts]
    print(json.dumps({
        "model": model, "threads": threads, "chunks": len(texts),
        "ms_per_chunk": 1000 * wall / len(texts), "cpu_ms_per_chunk": 1000 * cpu / len(texts),
        "load_s": load_s, "baseline_rss": base, "rss_after_load": after_load,
        "peak_rss": peak_rss_bytes(), "download_bytes": enc.download_bytes,
        "dim": enc.spec["dim"], "licence": enc.spec["licence"],
        "mean_tokens": sum(toks) / len(toks),
    }))


def has_avx2():
    if sys.platform == "win32":
        import ctypes
        return bool(ctypes.windll.kernel32.IsProcessorFeaturePresent(40))  # PF_AVX2_INSTRUCTIONS_AVAILABLE
    if sys.platform.startswith("linux"):
        return "avx2" in Path("/proc/cpuinfo").read_text()
    if platform.machine() in ("arm64", "aarch64"):
        return None  # not an x86 CPU
    out = subprocess.run(["sysctl", "-n", "machdep.cpu.leaf7_features"],
                         capture_output=True, text=True).stdout
    return "AVX2" in out


def cpu_name():
    if sys.platform == "darwin":
        return subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"],
                              capture_output=True, text=True).stdout.strip()
    if sys.platform == "win32":
        import winreg
        k = winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                           r"HARDWARE\DESCRIPTION\System\CentralProcessor\0")
        return winreg.QueryValueEx(k, "ProcessorNameString")[0].strip()
    for line in Path("/proc/cpuinfo").read_text().splitlines():
        if line.startswith("model name"):
            return line.split(":", 1)[1].strip()
    return platform.processor()


def machine():
    import onnxruntime
    import psutil
    return {
        "platform": platform.platform(), "cpu": cpu_name(),
        "logical_cores": psutil.cpu_count(), "physical_cores": psutil.cpu_count(logical=False),
        "ram_bytes": psutil.virtual_memory().total, "avx2": has_avx2(),
        "python": platform.python_version(), "onnxruntime": onnxruntime.__version__,
    }


def run_one(model, threads, n):
    """Starts the child and samples whole-system CPU use while it runs."""
    import psutil
    samples, done = [], threading.Event()

    def sample():
        psutil.cpu_percent(None)
        while not done.wait(1.0):
            samples.append(psutil.cpu_percent(None))

    th = threading.Thread(target=sample, daemon=True)
    th.start()
    r = subprocess.run([sys.executable, __file__, "--child", model, str(threads), str(n)],
                       capture_output=True, text=True, cwd=HERE)
    done.set()
    th.join()
    if r.returncode != 0:
        return {"model": model, "threads": threads, "error": r.stderr.strip()[-2000:]}
    out = json.loads(r.stdout.strip().splitlines()[-1])
    out["system_cpu_percent_mean"] = sum(samples) / len(samples) if samples else None
    return out


def main():
    from models import MODELS
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", nargs="*", default=list(MODELS))
    ap.add_argument("--threads", nargs="*", type=int, default=[1, 2])
    ap.add_argument("--chunks", type=int, default=200)
    ap.add_argument("--out", default="timing.json")
    ap.add_argument("--child", nargs=3, help=argparse.SUPPRESS)
    a = ap.parse_args()
    if a.child:
        return child(a.child[0], int(a.child[1]), int(a.child[2]))
    for m in a.models:  # download first, so download time never counts as run time
        from models import fetch
        fetch(m)
    info = {"machine": machine(), "runs": []}
    if hasattr(os, "getloadavg"):
        info["loadavg_before"] = os.getloadavg()
    print(json.dumps(info["machine"]))
    print(f"{'model':26s} {'thr':>3s} {'ms/chunk':>9s} {'cpu ms':>7s} {'peak RSS':>9s} "
          f"{'model RAM':>9s} {'download':>9s} {'sys CPU':>7s}")
    for m in a.models:
        for t in a.threads:
            r = run_one(m, t, a.chunks)
            info["runs"].append(r)
            if "error" in r:
                print(f"{m:26s} {t:3d}  ERROR: {r['error'][-300:]}")
                continue
            print(f"{m:26s} {t:3d} {r['ms_per_chunk']:9.2f} {r['cpu_ms_per_chunk']:7.2f} "
                  f"{r['peak_rss'] / 2**20:7.0f}MB {(r['peak_rss'] - r['baseline_rss']) / 2**20:7.0f}MB "
                  f"{r['download_bytes'] / 2**20:7.0f}MB {r['system_cpu_percent_mean'] or 0:6.0f}%",
                  flush=True)
    if hasattr(os, "getloadavg"):
        info["loadavg_after"] = os.getloadavg()
    Path(a.out).write_text(json.dumps(info, indent=1))
    print(f"-> {a.out}")


if __name__ == "__main__":
    main()
