#!/usr/bin/env python3
"""VFS-8 AC-1: launch-and-screenshot smoke test of the installed release build on Windows,
which WebDriver cannot drive (spec decision 1).

1. Launch the installed exe as a user would, twice (the first launch creates WebView2's
   profile); for each, record time to window and time until the page is painted (the window
   stays blank white until then), screenshot the desktop and the window, record the window
   size against the screen and the memory of the app and its WebView2 processes, close the
   window and check that the process exits. The main window stays hidden until its page has
   loaded, with a native splash meanwhile (issue 108): each launch also records when the
   splash first shows and when it goes, and screenshots it.
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
# Window class of the native splash (`SPLASH_CLASS`, apps/desktop/src-tauri/src/splash.rs).
SPLASH_CLASS = "VfsSplash"


def watch_splash(t0, out, run, timeout):
    """Polls for the splash window from launch (`t0`) until it has shown and gone, or the
    main window is visible without it, or `timeout`. Screenshots the desktop and the splash
    the first time it is seen, and the splash again 1 s later (the first capture can precede
    its first paint). Times are seconds since launch."""
    import ctypes
    from ctypes import wintypes
    from PIL import ImageGrab
    user32 = ctypes.windll.user32
    user32.FindWindowW.restype = wintypes.HWND

    def visible(cls, title):
        h = user32.FindWindowW(cls, title)
        return h if h and user32.IsWindowVisible(h) else None

    r = {"splash_seen": False, "time_to_splash_s": None, "splash_gone_s": None,
         "time_to_main_visible_s": None}
    box = None
    while now() - t0 < timeout:
        h = visible(SPLASH_CLASS, None)
        main = visible(None, TITLE)
        t = round(now() - t0, 2)
        if main and r["time_to_main_visible_s"] is None:
            r["time_to_main_visible_s"] = t
        if h and not r["splash_seen"]:
            r["splash_seen"], r["time_to_splash_s"] = True, t
            try:
                ImageGrab.grab().save(os.path.join(out, f"release-{run}-splash-desktop.png"))
                rect = wintypes.RECT()
                user32.GetWindowRect(h, ctypes.byref(rect))
                box = (rect.left, rect.top, rect.right, rect.bottom)
                r["splash_rect"] = list(box)
                ImageGrab.grab(bbox=box, all_screens=True).save(
                    os.path.join(out, f"release-{run}-splash.png"))
            except Exception as e:
                r["splash_screenshot_error"] = f"{type(e).__name__}: {e}"
        if h and box and t >= r["time_to_splash_s"] + 1:
            try:
                ImageGrab.grab(bbox=box, all_screens=True).save(
                    os.path.join(out, f"release-{run}-splash-1s.png"))
            except Exception as e:
                r["splash_screenshot_error"] = f"{type(e).__name__}: {e}"
            box = None
        if r["splash_seen"] and not h and r["splash_gone_s"] is None:
            r["splash_gone_s"] = t
        if main and (r["splash_gone_s"] is not None or not r["splash_seen"]):
            break
        time.sleep(0.05)
    return r


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


def painted(w):
    """True once the client area is no longer blank white (the page background is not pure
    white)."""
    img = w.capture_as_image().convert("RGB")
    width, height = img.size
    box = img.crop((0, 40, width, height)).resize((200, 120))
    px = list(box.getdata())
    return sum(1 for p in px if p != (255, 255, 255)) > 0.5 * len(px)


def wait_painted(w, timeout):
    t0 = now()
    while now() - t0 < timeout:
        try:
            if painted(w):
                return now() - t0
        except Exception:
            pass
        time.sleep(0.25)
    return None


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
        for run in ("first", "second"):
            t0 = now()
            p = launch(args.app)
            splash = watch_splash(t0, args.out, run, 60)
            w, _ = find_window(60)
            secs = now() - t0
            paint = wait_painted(w, 120)
            r = {"time_to_window_s": round(secs, 2),
                 "time_to_paint_s": None if paint is None else round(now() - t0, 2),
                 "splash": splash}
            time.sleep(3)  # let the first status arrive
            w.set_focus()
            time.sleep(1)
            ImageGrab.grab().save(os.path.join(args.out, f"release-{run}-launch-desktop.png"))
            w.capture_as_image().save(os.path.join(args.out, f"release-{run}-launch-window.png"))
            rect = w.rectangle()
            screen = ImageGrab.grab().size
            r["window"] = {"left": rect.left, "top": rect.top, "width": rect.width(),
                           "height": rect.height(), "screen": list(screen),
                           "fits_screen": rect.left >= 0 and rect.top >= 0
                           and rect.right <= screen[0] and rect.bottom <= screen[1]}
            r["memory"] = tracker.memory()
            w.close()
            t1 = now()
            try:
                p.wait(20)
                r["exit_after_close_s"] = round(now() - t1, 2)
                r["exit_code"] = p.returncode
            except subprocess.TimeoutExpired:
                r["exit_after_close_s"] = None
            tracker.stop()
            res[f"{run}_launch"] = r

        # Decision 1 diagnostic: does the release build honour the DevTools switch?
        port = 9333
        p = launch(args.app, {"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS":
                              f"--remote-debugging-port={port}"})
        w, _ = find_window(60)
        wait_painted(w, 120)
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
