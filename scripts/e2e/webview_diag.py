#!/usr/bin/env python3
"""TEMPORARY (VFS-5 investigation): launches the installed app with a WebView2 switch set,
waits, screenshots the desktop and lists the app's processes and data folder."""
import json, os, subprocess, sys, time
from PIL import ImageGrab
import psutil

app, out, label, extra = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
os.makedirs(out, exist_ok=True)
env = dict(os.environ)
if extra:
    env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = extra
p = subprocess.Popen([app], env=env)
time.sleep(25)
ImageGrab.grab().save(os.path.join(out, f"{label}.png"))
procs = []
for c in [psutil.Process(p.pid)] + psutil.Process(p.pid).children(recursive=True):
    try:
        procs.append({"name": c.name(), "rss_mb": round(c.memory_info().rss / 2**20, 1),
                      "cmd": " ".join(c.cmdline())[:400]})
    except Exception as e:
        procs.append(str(e))
data = os.path.join(os.environ["LOCALAPPDATA"], "app.vfs.desktop")
files = []
for d, _, fs in os.walk(data):
    files += [os.path.relpath(os.path.join(d, f), data) for f in fs]
json.dump({"extra": extra, "procs": procs, "files": files[:80]},
          open(os.path.join(out, f"{label}.json"), "w"), indent=1)
for c in psutil.Process(p.pid).children(recursive=True):
    c.kill()
p.kill()
time.sleep(3)
