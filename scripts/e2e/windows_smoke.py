#!/usr/bin/env python3
"""VFS-5 AC-1: launch-and-screenshot smoke test of the installed release build on Windows,
which WebDriver cannot drive (spec decision 1).

1. Launch the installed exe as a user would; wait for its window; screenshot the desktop and
   the window; record time to window, window size against the screen, and memory of the app
   and its WebView2 processes; close the window and check the process exits.
2. Diagnostic for decision 1: launch it again with the variable Edge WebDriver relies on
   (WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=...) and record whether the
   DevTools endpoint answers.

Usage: windows_smoke.py --app <installed exe> --out <dir>
"""

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from journey import App, now  # noqa: E402

TITLE = "VFS — Buscador de archivos"


def find_window(timeout):
    from pywinauto import Desktop
    t0 = now()
    while now() - t0 < timeout:
        for w in Desktop(backend="uia").windows():
            try:
                if w.window_text() == TITLE:
                    return w, now() - t0
            except Exception:
                pass
        time.sleep(0.25)
    raise TimeoutError(f"no window titled {TITLE!r} after {timeout}s")


def launch(app, env=None):
    return subprocess.Popen([app], env=dict(os.environ, **(env or {})))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--app", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    os.makedirs(args.out, exist_ok=True)
    res = {}
    from PIL import ImageGrab
    tracker = App(argparse.Namespace(app=args.app, out=args.out, port=0))
    try:
        p = launch(args.app)
        w, secs = find_window(60)
        res["time_to_window_s"] = round(secs, 2)
        time.sleep(5)  # let the page render and the first status arrive
        w.set_focus()
        time.sleep(1)
        ImageGrab.grab().save(os.path.join(args.out, "release-first-run-desktop.png"))
        w.capture_as_image().save(os.path.join(args.out, "release-first-run-window.png"))
        r = w.rectangle()
        screen = ImageGrab.grab().size
        res["window"] = {"left": r.left, "top": r.top, "width": r.width(),
                         "height": r.height(), "screen": list(screen),
                         "fits_screen": r.left >= 0 and r.top >= 0 and r.right <= screen[0]
                         and r.bottom <= screen[1]}
        res["memory_first_run"] = tracker.memory()
        w.close()
        t0 = now()
        try:
            p.wait(15)
            res["exit_after_close_s"] = round(now() - t0, 2)
            res["exit_code"] = p.returncode
        except subprocess.TimeoutExpired:
            res["exit_after_close_s"] = None
            p.kill()
        tracker.stop()

        # Decision 1 diagnostic: does the release build honour the DevTools switch?
        port = 9333
        p = launch(args.app, {"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS":
                              f"--remote-debugging-port={port}"})
        find_window(60)
        time.sleep(5)
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=5) as r:
                res["devtools_with_env_var"] = json.loads(r.read())
        except Exception as e:
            res["devtools_with_env_var"] = f"not reachable: {e}"
        tracker.stop()
    except Exception as e:
        res["error"] = f"{type(e).__name__}: {e}"
        try:
            ImageGrab.grab().save(os.path.join(args.out, "release-error-desktop.png"))
        except Exception:
            pass
    finally:
        tracker.stop()
        with open(os.path.join(args.out, "smoke.json"), "w", encoding="utf-8") as f:
            json.dump(res, f, ensure_ascii=False, indent=2)
    print(json.dumps(res, ensure_ascii=False)[:1500])
    return 1 if "error" in res else 0


if __name__ == "__main__":
    sys.exit(main())
