#!/usr/bin/env python3
"""VFS-8 AC-2, AC-3, AC-4, AC-5: drives the real VFS window through the user's journey and
records a screenshot of the window and of the whole desktop at every step, plus timings and
memory, into --out.

The app is a `--features webdriver` build (on Windows, installed from its NSIS installer): this
script launches it with TAURI_WEBDRIVER_PORT set and talks W3C WebDriver to the server embedded
in it. Edge WebDriver through `tauri-driver` could not attach to the release build on a runner
(see specs/vfs-8-windows-e2e/spec.md, decision 1); the release build gets windows_smoke.py.

The tree comes from gen_tree.py (--manifest). Steps never stop the run: a failed step is
recorded with its error and screenshot and the next one runs; the exit code is non-zero if any
step failed. Read-only towards everything except the generated tree, where the live-change
steps create, rename and delete their own files.
"""

import argparse
import json
import ntpath
import os
import platform
import shutil
import subprocess
import sys
import time
import traceback
import unicodedata

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from wd import WebDriver, WebDriverError  # noqa: E402
import gen_tree  # noqa: E402

IS_WIN = os.name == "nt"
IS_MAC = sys.platform == "darwin"
# Seconds after minimising at which memory is sampled (measurement plan step 9: 5 minutes).
MINIMISED_SAMPLES_S = (15, 60, 300)

# Serde values of `IndexerState` and `RootStatus` (crates/vfs-core/src/types.rs), as
# `get_progress` reports them.
STATE_IDLE = "idle"
STATE_SCANNING = "scanning"
ROOT_READY = "ready"

# Title of the native folder picker (`pick_folder`, apps/desktop/src-tauri/src/commands.rs).
FOLDER_DIALOG_TITLE = "Elegir carpeta"

# A `SearchQuery.filters` with no filter set.
NO_FILTERS = {"kinds": [], "modifiedFrom": None, "modifiedTo": None, "sizeMin": None,
              "sizeMax": None, "folder": None}

# Values of the date and size selects (`DateChoice`, `SizeChoice` in apps/desktop/ui/choices.js).
DATE_WEEK = "week"
SIZE_MEDIUM, SIZE_LARGE, SIZE_HUGE = "medium", "large", "huge"

# What "Mostrar en carpeta" must show: the item selected (also past 260 chars, VFS-24 AC-1),
# or an ancestor (the fallback when the Shell cannot select the item).
EXPECT_SELECT = "select"
EXPECT_PARENT = "parent"

# VFS-27 AC-16: documents of the VFS-2 golden corpus, each with a query that must find it
# through the app's `content_search` command (from crates/vfs-content/tests/golden.rs).
GOLDEN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "crates",
                      "vfs-content", "tests", "golden")
CONTENT_CASES = [
    ("fianza dos mensualidades", "escaneo0034.docx"),
    ("potencia contratada", "doc_2020_07.txt"),
    ("caldera presión", "nota_rapida.txt"),
    ("cocido garbanzos", "receta_abuela.docx"),
    ("derrama ascensor", "acta.pdf"),
    ("renovación póliza hogar", "carta_seguro.docx"),
    ("declaración renta devolver", "renta2022.pdf"),
    ("euríbor amortización anticipada", "hipoteca_bbva.pdf"),
    ("metformina glucosa", "medico.txt"),
    ("Covadonga Cares", "viaje_asturias.docx"),
    ("garantía lavadora Bosch", "garantia_lavadora.pdf"),
    ("excursión Segovia autorización", "colegio.txt"),
    ("impuesto bienes inmuebles", "ibi.pdf"),
    ("vacunas perro", "perro.docx"),
    ("pensión jubilación cotizados", "jubilacion.txt"),
]
# The content section shows the top hits first; the expected document must be among these.
CONTENT_TOP = 5


def scan_done(p):
    """True when a `get_progress` answer says every root is scanned and the indexer is idle."""
    roots = p.get("roots") or []
    return p["state"] == STATE_IDLE and bool(roots) and all(
        r["status"] == ROOT_READY for r in roots)


def nfc(s):
    return unicodedata.normalize("NFC", s or "")


def now():
    return time.perf_counter()


def wait_for(fn, timeout, interval=0.25, what="condition"):
    """Polls `fn` until it returns a truthy value; returns (value, seconds) or raises."""
    t0 = now()
    last_err = None
    while now() - t0 < timeout:
        try:
            v = fn()
            if v:
                return v, now() - t0
        except Exception as e:  # transient while the page re-renders
            last_err = e
        time.sleep(interval)
    raise TimeoutError(f"timed out after {timeout}s waiting for {what}"
                       + (f" (last error: {last_err})" if last_err else ""))


# ---------------------------------------------------------------------------------------------
# App lifecycle


class App:
    """Starts and stops the app and owns the WebDriver session."""

    def __init__(self, args):
        self.args = args
        self.proc = None
        self.wd = None

    def start(self):
        t0 = now()
        env = dict(os.environ, TAURI_WEBDRIVER_PORT=str(self.args.port))
        log = open(os.path.join(self.args.out, "app-stdout.log"), "ab")
        self.proc = subprocess.Popen([self.args.app], env=env, stdout=log, stderr=log)
        self.wd = WebDriver(f"http://127.0.0.1:{self.args.port}")
        self.wd.wait_ready(90)
        # The server answers before the webview exists; WebView2's first start can be slow.
        wait_for(lambda: self.try_session(), 180, 1.0, "the webview window")
        self.wd.set_script_timeout(120_000)
        # The page is ready when main.js has rendered the status line.
        wait_for(lambda: self.wd.js(
            "return !!window.__TAURI__ && document.querySelector('#status-text') !== null"
            " && document.querySelector('#open-folders').textContent.length > 0"),
            60, what="UI ready")
        # VFS-22: the steps below read Spanish copy and the Spanish folder picker title, and
        # the runner's OS may be English: pin the app to Spanish and wait for the redraw.
        # 'vfs.language' is LANGUAGE_KEY in apps/desktop/ui/main.js. A page already in
        # Spanish is not reloaded, so the wait below never sees a page about to go away.
        self.wd.js("localStorage.setItem('vfs.language', 'es');"
                   " if (document.documentElement.lang !== 'es') setTimeout(() => location.reload(), 0);"
                   " return true")
        wait_for(lambda: self.wd.js(
            "return document.documentElement.lang === 'es'"
            " && document.querySelector('#open-folders') !== null"
            " && document.querySelector('#open-folders').textContent.length > 0"),
            60, what="UI ready in Spanish")
        return now() - t0

    def try_session(self):
        try:
            self.wd.start({})
            return True
        except WebDriverError as e:
            if "no such window" in str(e):
                return False
            raise

    def pids(self):
        """The app process and its descendants (WebView2 on Windows)."""
        import psutil
        roots = []
        exe = os.path.normcase(os.path.abspath(self.args.app))
        for p in psutil.process_iter(["exe"]):
            try:
                if p.info["exe"] and os.path.normcase(p.info["exe"]) == exe:
                    roots.append(p)
            except Exception:
                pass
        out = []
        for r in roots:
            out.append(r)
            try:
                out.extend(r.children(recursive=True))
            except Exception:
                pass
        return out

    def memory(self):
        """Working set and private bytes in MB, for the app and for its WebView processes."""
        import psutil
        app = {"rss_mb": 0.0, "private_mb": 0.0, "processes": 0}
        web = {"rss_mb": 0.0, "private_mb": 0.0, "processes": 0}
        for p in self.pids():
            try:
                name = p.name().lower()
                mi = p.memory_info()
                private = getattr(mi, "private", None)
                if private is None:
                    try:
                        private = p.memory_full_info().uss
                    except Exception:
                        private = 0
                bucket = web if "webview" in name or "webkit" in name else app
                bucket["rss_mb"] += mi.rss / 2**20
                bucket["private_mb"] += private / 2**20
                bucket["processes"] += 1
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                pass
        if IS_MAC:  # WKWebView's processes are XPC services, not children of the app
            for p in psutil.process_iter(["name"]):
                if (p.info["name"] or "").startswith("com.apple.WebKit"):
                    try:
                        web["rss_mb"] += p.memory_info().rss / 2**20
                        web["processes"] += 1
                    except Exception:
                        pass
            web["note"] = "all com.apple.WebKit.* processes on the machine"
        for b in (app, web):
            for k in ("rss_mb", "private_mb"):
                b[k] = round(b[k], 1)
        return {"app": app, "webview": web,
                "total_rss_mb": round(app["rss_mb"] + web["rss_mb"], 1)}

    def stop(self):
        if self.wd:
            try:
                self.wd.quit()
            except Exception:
                pass
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(20)
            except subprocess.TimeoutExpired:
                self.proc.kill()
            self.proc = None
        for p in self.pids():  # nothing of the app may outlive a stop
            try:
                p.terminate()
            except Exception:
                pass
        time.sleep(2)


# ---------------------------------------------------------------------------------------------
# Journey


class Journey:
    def __init__(self, args):
        self.args = args
        self.app = App(args)
        self.m = json.load(open(args.manifest, encoding="utf-8"))
        self.t = self.m["targets"]
        self.root = self.m["root"]
        self.steps = []
        self.metrics = {"platform": platform.platform(), "machine": platform.machine(),
                        "cpu_count": os.cpu_count(), "tree_files": self.m["files"],
                        "tree_dirs": self.m["dirs"]}
        self.shots = os.path.join(args.out, "screens")
        os.makedirs(self.shots, exist_ok=True)

    @property
    def wd(self):
        return self.app.wd

    # -- recording --------------------------------------------------------------------------
    def shoot(self, label):
        n = len(self.steps) + 1
        base = os.path.join(self.shots, f"{n:02d}-{label}")
        try:
            with open(base + "-window.png", "wb") as f:
                f.write(self.wd.screenshot())
        except Exception as e:
            print(f"  window screenshot failed: {e}")
        try:
            from PIL import ImageGrab
            ImageGrab.grab().save(base + "-desktop.png")
        except Exception as e:
            print(f"  desktop screenshot failed: {e}")

    def step(self, name, fn, critical=False):
        print(f"== {name}")
        rec = {"step": name, "ok": False}
        t0 = now()
        try:
            notes = fn()
            rec["ok"] = True
            if notes:
                rec["notes"] = notes
        except Exception as e:
            rec["error"] = f"{type(e).__name__}: {e}"
            print(traceback.format_exc())
        rec["seconds"] = round(now() - t0, 2)
        self.shoot(name)
        self.steps.append(rec)
        print(f"   {'ok' if rec['ok'] else 'FAILED'} in {rec['seconds']} s"
              + (f": {rec.get('notes')}" if rec.get("notes") else "")
              + (f": {rec.get('error')}" if rec.get("error") else ""))
        self.save()
        if critical and not rec["ok"]:
            raise SystemExit(f"critical step {name} failed")
        return rec

    def save(self):
        with open(os.path.join(self.args.out, "journey.json"), "w", encoding="utf-8") as f:
            json.dump({"steps": self.steps, "metrics": self.metrics}, f, ensure_ascii=False,
                      indent=2)

    # -- page helpers -----------------------------------------------------------------------
    def text_of(self, css):
        return self.wd.js("const e = document.querySelector(arguments[0]);"
                          "return e ? e.textContent : null;", css)

    def visible(self, css):
        return self.wd.js("const e = document.querySelector(arguments[0]);"
                          "return !!e && !e.hidden && e.getClientRects().length > 0;", css)

    def rows(self):
        return self.wd.js(
            "return [...document.querySelectorAll('#results .row:not(.loading)')]"
            ".filter(r => r.style.display !== 'none')"
            ".map(r => ({i: +r.dataset.index,"
            " name: r.querySelector('.name-text')?.textContent || '',"
            " kind: r.querySelector('.kind')?.textContent || '',"
            " dir: r.querySelector('.path')?.textContent || ''}))"
            ".sort((a, b) => a.i - b.i);") or []

    def progress(self):
        return self.wd.invoke("get_progress")

    def type_query(self, text):
        q = self.wd.find("#q")
        self.wd.clear(q)
        # clear() does not fire `input`; select-all + backspace does, so the UI resets.
        self.wd.type(q, "\ue009a\ue003")
        if text:
            self.wd.type(q, text)
        return now()

    def search_ui(self, query, expect_name, timeout=20, absent=False):
        """Types `query`, waits until `expect_name` is (or is no longer) among the visible
        rows, and returns the rank and the seconds from the last key to that render."""
        t_typed = self.type_query(query)
        want = nfc(expect_name)

        def check():
            if nfc(self.text_of("#summary")) == "Buscando…":
                return None
            names = [nfc(r["name"]) for r in self.rows()]
            if absent:
                return {"rank": None} if want not in names else None
            if want in names:
                return {"rank": names.index(want) + 1}
            return None
        found, _ = wait_for(check, timeout, 0.05, f"«{expect_name}» for «{query}»")
        found["ui_ms"] = round((now() - t_typed) * 1000)
        found["summary"] = self.text_of("#summary")
        return found

    def engine_ms(self, query, filters=None):
        r = self.wd.invoke("search", {"query": {"text": query, "filters": filters or NO_FILTERS,
                                                "limit": 200, "offset": 0}})
        return r["elapsedMs"], r["total"]

    def set_select(self, css, value):
        self.wd.js("const s = document.querySelector(arguments[0]); s.value = arguments[1];"
                   "s.dispatchEvent(new Event('change', {bubbles: true}));", css, value)

    def clear_filters(self):
        if self.visible("#f-clear"):
            self.wd.click(self.wd.find("#f-clear"))

    # -- steps ------------------------------------------------------------------------------
    def s_first_run(self):
        secs = self.app.start()
        self.metrics["launch_to_ui_s"] = round(secs, 2)
        # The empty state renders after the first list_roots answer.
        wait_for(lambda: "Añada una carpeta" in nfc(self.text_of("#empty")), 30,
                 what="first-run empty state")
        return {"launch_to_ui_s": round(secs, 2), "status": self.text_of("#status-text"),
                "viewport": self.wd.js("return [innerWidth, innerHeight, devicePixelRatio]")}

    def s_open_folders(self):
        self.wd.click(self.wd.find("#open-folders"))
        wait_for(lambda: self.wd.js("return document.querySelector('#folders').open"), 10,
                 what="folders drawer")

    def s_add_folder(self):
        path = self.root
        how = "native dialog"
        self.metrics["scan_t0"] = None
        if IS_WIN and not self.args.no_dialog:
            self.wd.click(self.wd.find("#add-folder"))
            try:
                drive_windows_folder_dialog(path, self)
            except Exception as e:
                how = f"invoke add_root (dialog automation failed: {e})"
                close_windows_folder_dialog()
                self.metrics["scan_t0"] = now()
                self.wd.invoke("add_root", {"path": path})
        else:
            how = "invoke add_root (native dialog not automatable here)"
            self.metrics["scan_t0"] = now()
            self.wd.invoke("add_root", {"path": path})
        if self.metrics["scan_t0"] is None:
            self.metrics["scan_t0"] = now()
        wait_for(lambda: self.wd.invoke("list_roots"), 20, what="root listed")
        self.metrics["add_folder_via"] = how
        return {"via": how}

    def s_scan_progress(self):
        def scanning():
            p = self.progress()
            return p if p["filesIndexed"] > 0 or p["state"] == STATE_SCANNING else None
        p, _ = wait_for(scanning, 60, 0.2, "scan to start")
        # close the drawer so the main window shows the progress
        self.wd.js("document.querySelector('#folders').close()")
        time.sleep(1.0)
        return {"files_indexed": p["filesIndexed"], "status": self.text_of("#status-text"),
                "summary": self.text_of("#summary"), "empty": self.text_of("#empty")}

    def s_search_during_scan(self):
        p = self.progress()
        if p["state"] != STATE_SCANNING:
            return {"note": "scan already finished; searched after it instead"}
        self.type_query("factura")
        rows, secs = wait_for(lambda: self.rows(), 30, 0.1, "any hit during the scan")
        p2 = self.progress()
        return {"hits_visible": len(rows), "first": rows[0]["name"],
                "files_indexed_at_search": p2["filesIndexed"], "state": p2["state"],
                "summary": self.text_of("#summary"), "ui_ms": round(secs * 1000)}

    def s_scan_finishes(self):
        samples = []
        mem_peak = {"total_rss_mb": 0}

        def done():
            p = self.progress()
            samples.append((round(now() - self.metrics["scan_t0"], 1), p["filesIndexed"]))
            try:
                mem = self.app.memory()
                if mem["total_rss_mb"] > mem_peak["total_rss_mb"]:
                    mem_peak.update(mem)
            except Exception:
                pass
            return scan_done(p)
        _, _ = wait_for(done, self.args.scan_timeout, 1.0, "first scan to finish")
        secs = now() - self.metrics["scan_t0"]
        files = self.wd.invoke("list_roots")[0]["files"]
        self.metrics["first_scan_s"] = round(secs, 1)
        self.metrics["first_scan_files"] = files
        self.metrics["first_scan_files_per_s"] = round(files / secs) if secs else None
        self.metrics["scan_samples"] = samples[:: max(1, len(samples) // 60)]
        self.metrics["memory_peak_during_scan"] = mem_peak
        self.type_query("")
        time.sleep(1)
        return {"seconds": round(secs, 1), "files": files,
                "files_per_s": self.metrics["first_scan_files_per_s"],
                "status": self.text_of("#status-text"), "empty": self.text_of("#empty")}

    def search_case(self, label, query, key):
        def run():
            target = self.t[key]
            r = self.search_ui(query, target["name"])
            ms, total = self.engine_ms(query)
            r.update({"query": query, "engine_ms": ms, "total": total})
            self.metrics.setdefault("searches", {})[label] = r
            if target["kind"] == gen_tree.KIND_FOLDER:
                kinds = [row["kind"] for row in self.rows() if nfc(row["name"]) == nfc(
                    target["name"])]
                r["kind_shown"] = kinds[0] if kinds else None
                if r["kind_shown"] != "Carpeta":
                    raise AssertionError(f"folder hit shown as {r['kind_shown']!r}")
            return r
        return run

    def filter_case(self, label, apply, query, key):
        def run():
            self.clear_filters()
            self.type_query("")
            apply()
            r = self.search_ui(query, self.t[key]["name"])
            r.update({"query": query, "filter": label, "summary": self.text_of("#summary")})
            self.metrics.setdefault("filters", {})[label] = r
            return r
        return run

    def apply_kind(self, kind_value):
        def run():
            self.wd.click(self.wd.find("#f-type"))
            time.sleep(0.3)
            self.wd.click(self.wd.find(f"#kind-chips input[value='{kind_value}']"))
            time.sleep(0.3)
            self.shoot("type-popover")
            self.wd.js("document.querySelector('#f-type-pop').hidePopover?.()")
        return run

    def s_reveal(self, key, expect):
        def run():
            target = self.t[key]
            self.clear_filters()
            query = {gen_tree.T_NAME: "notaria valdepenas",
                     gen_tree.T_LONG: "informe pericial ibanez",
                     gen_tree.T_FOLDER: "boda inigo ainhoa"}[key]
            self.search_ui(query, target["name"])
            idx = [nfc(r["name"]) for r in self.rows()].index(nfc(target["name"]))
            row = self.wd.find(f"#results .row[data-index='{idx}']")
            self.wd.click(row)
            close_file_manager_windows()
            t0 = now()
            try:
                self.wd.click(self.wd.find(f"#results .row[data-index='{idx}'] .reveal"))
                how = "reveal button"
            except WebDriverError:
                self.wd.type(self.wd.find("#q"), "\ue007")  # Enter on the selected row
                how = "Enter key"
            res = check_file_manager(target, expect, timeout=15)
            res["seconds"] = round(now() - t0, 2)
            res["via"] = how
            res["toast"] = self.text_of("#toast") if self.visible("#toast") else None
            res["error_banner"] = (self.text_of("#banner-error-text")
                                   if self.visible("#banner-error") else None)
            self.metrics.setdefault("reveal", {})[key] = res
            if not res.get("ok"):
                raise AssertionError(f"file manager check failed: {res}")
            return res
        return run

    def s_live_create(self):
        d = os.path.join(self.root, "Documentos")
        self.live_path = os.path.join(d, self.m["live_name"])
        self.type_query("")
        t0 = now()
        gen_tree.write_file(self.live_path, 0, time.time())
        r = self.search_ui("carta urgente muñoz", self.m["live_name"], timeout=60)
        r["seconds_from_create"] = round(now() - t0, 2)
        self.metrics["live_create_s"] = r["seconds_from_create"]
        return r

    def s_live_rename(self):
        new = os.path.join(os.path.dirname(self.live_path), self.m["live_renamed"])
        t0 = now()
        os.rename(gen_tree.fs_path(self.live_path), gen_tree.fs_path(new))
        self.live_path = new
        r = self.search_ui("carta respondida muñoz", self.m["live_renamed"], timeout=60)
        gone = self.search_ui("carta urgente muñoz", self.m["live_name"], timeout=60,
                              absent=True)
        r["old_name_gone"] = True
        r["old_name_ms"] = gone["ui_ms"]
        r["seconds_from_rename"] = round(now() - t0, 2)
        self.metrics["live_rename_s"] = r["seconds_from_rename"]
        return r

    def s_live_delete(self):
        self.search_ui("carta respondida muñoz", self.m["live_renamed"])
        t0 = now()
        os.remove(gen_tree.fs_path(self.live_path))
        r = self.search_ui("carta respondida muñoz", self.m["live_renamed"], timeout=60,
                           absent=True)
        r["seconds_from_delete"] = round(now() - t0, 2)
        self.metrics["live_delete_s"] = r["seconds_from_delete"]
        return r

    def s_memory_idle(self):
        self.type_query("notaria")
        time.sleep(5)
        mem = self.app.memory()
        self.metrics["memory_idle_after_search"] = mem
        return mem

    def s_memory_minimised(self):
        """VFS-24 AC-2, VFS-5 AC-12: memory with the window open, minimised
        (WebView2 suspended) and restored."""
        import ctypes
        user32 = ctypes.windll.user32
        hwnd = main_window(self.app.proc.pid)
        samples = {"open": self.app.memory()}
        user32.ShowWindow(hwnd, 6)  # SW_MINIMIZE
        t0 = now()
        for at in MINIMISED_SAMPLES_S:
            time.sleep(max(0.0, at - (now() - t0)))
            samples[f"minimised_{at}s"] = self.app.memory()
        # Only the app's own log proves WebView2 suspended: Windows trims any minimised
        # process's working set anyway.
        log = os.path.join(os.environ.get("LOCALAPPDATA", ""), "app.vfs.desktop", "logs",
                           "vfs.log")
        with open(log, encoding="utf-8", errors="replace") as f:
            samples["suspend_log"] = [l.strip() for l in f if "webview suspended" in l][-3:]
        user32.ShowWindow(hwnd, 9)  # SW_RESTORE
        time.sleep(10)
        samples["restored_10s"] = self.app.memory()
        # The resumed page must still search and render (raises if it does not).
        samples["restored_search"] = self.search_ui("notaria valdepenas",
                                                    self.t[gen_tree.T_NAME]["name"])
        self.metrics["memory_minimised"] = samples
        if not any("webview suspended: true" in l for l in samples["suspend_log"]):
            raise AssertionError(f"WebView2 did not report a suspend: {samples['suspend_log']}")
        return samples

    def s_accessibility(self):
        """VFS-24 AC-3: names Narrator reads through UI Automation, and the page
        following Windows high contrast and "show animations" off."""
        from pywinauto import Desktop
        self.search_ui("notaria valdepenas", self.t[gen_tree.T_NAME]["name"])
        want = self.wd.js(
            "return {search: document.querySelector('label[for=q]').textContent,"
            " results: document.querySelector('#results').getAttribute('aria-label'),"
            " row: document.querySelector('#results [role=option]').getAttribute('aria-label')}")
        win = Desktop(backend="uia").window(handle=main_window(self.app.proc.pid))

        def uia_names():
            found = {}
            for el in win.descendants():
                try:
                    info = el.element_info
                    found.setdefault(info.control_type, []).append(info.name)
                except Exception:
                    pass
            return found
        # WebView2 builds its UI Automation tree on the first request: ask until it is there.
        names, exposed = {}, {}
        for _ in range(10):
            names = uia_names()
            seen = set(sum(names.values(), []))
            exposed = {k: v in seen for k, v in want.items()}
            if all(exposed.values()):
                break
            time.sleep(1)
        out = {"expected": want, "exposed": exposed,
               "uia_counts": {k: len(v) for k, v in names.items()},
               "uia_sample": {k: [n for n in v if n][:5] for k, v in names.items()}}
        self.metrics["accessibility"] = out
        out["high_contrast"] = self.with_setting(
            set_high_contrast, "(forced-colors: active)", "high-contrast")
        out["reduced_motion"] = self.with_setting(
            set_animations_off, "(prefers-reduced-motion: reduce)", "reduced-motion")
        self.metrics["accessibility"] = out
        missing = [k for k, ok in exposed.items() if not ok]
        unfollowed = [k for k in ("high_contrast", "reduced_motion") if not out[k]["follows"]]
        if missing or unfollowed:
            raise AssertionError(f"not exposed: {missing}; settings not followed: {unfollowed}")
        return out

    def with_setting(self, toggle, media, label):
        """Starts from the Windows setting off (the runner may have it on already), checks
        the page does not match `media`, turns the setting on, waits for the page to match,
        takes a screenshot and turns it off again: the page must follow both ways."""
        def matches(want):
            try:
                wait_for(lambda: self.wd.js(f"return matchMedia('{media}').matches") == want,
                         15, 0.5, f"{media} = {want}")
                return True
            except TimeoutError:
                return False
        initially = self.wd.js(f"return matchMedia('{media}').matches")
        toggle(False)
        off = matches(False)
        toggle(True)
        try:
            on = matches(True)
            self.shoot(label)
        finally:
            toggle(False)
        back = matches(False)
        return {"initially": initially, "off": off, "on": on, "back_off": back,
                "follows": off and on and back}

    def s_close_and_change(self):
        self.app.stop()
        d = os.path.join(self.root, "Escritorio", "Cambios mientras cerrado")
        os.makedirs(gen_tree.fs_path(d), exist_ok=True)
        for i in range(50):
            gen_tree.write_file(os.path.join(d, f"Cambio mientras cerrado {i:03d}.txt"), 0,
                                time.time())
        gone = self.t[gen_tree.T_AUDIO]["path"]
        os.remove(gen_tree.fs_path(gone))
        return {"created": 50, "deleted": os.path.basename(gone)}

    def s_reopen_catch_up(self):
        t0 = now()
        launch = self.app.start()

        def caught_up():
            ms, total = self.engine_ms("cambio mientras cerrado")
            return total >= 50
        _, _ = wait_for(caught_up, self.args.scan_timeout, 0.5, "catch-up to find new files")
        found_s = now() - t0
        self.shoot("catch-up-found")

        def idle():
            return scan_done(self.progress())
        wait_for(idle, self.args.scan_timeout, 0.5, "catch-up to finish")
        idle_s = now() - t0
        gone = self.engine_ms("cancion de cuna maria")[1]
        r = self.search_ui("cambio mientras cerrado", "Cambio mientras cerrado 000.txt")
        self.metrics["catch_up"] = {"launch_s": round(launch, 2),
                                    "new_files_found_s": round(found_s, 1),
                                    "idle_s": round(idle_s, 1),
                                    "deleted_still_found": gone}
        if gone:
            raise AssertionError("file deleted while closed is still found after catch-up")
        r.update(self.metrics["catch_up"])
        return r

    def s_content_search(self):
        """VFS-27 AC-16: a folder of the golden documents is added, content search is
        accepted from the offer, the run finishes, and each document is found by a word in
        its text. The big tree is removed first, so the run reads only these documents."""
        for r in self.wd.invoke("list_roots"):
            self.wd.invoke("remove_root", {"id": r["id"]})
        dest = os.path.join(os.path.dirname(os.path.abspath(self.args.manifest)), "Contenido")
        os.makedirs(dest, exist_ok=True)
        for _, name in CONTENT_CASES:
            shutil.copy(os.path.join(GOLDEN, name), os.path.join(dest, name))
        self.wd.invoke("add_root", {"path": dest})
        wait_for(lambda: scan_done(self.progress()), self.args.scan_timeout, 0.5, "scan of the documents")
        # R4: the offer shows once every folder has finished its first scan; AC-16 accepts it
        # there, so a missing offer fails the step.
        wait_for(lambda: self.visible("#banner-content-offer"), 30, 0.5, "content offer")
        self.shoot("content-offer")
        self.wd.click(self.wd.find("#content-offer-yes"))

        def finished():
            st = self.wd.invoke("content_status")
            return st if st["phase"]["kind"] == "finished" and st["done"] >= st["total"] else None
        st, read_s = wait_for(finished, 900, 1, "content run to finish")
        self.shoot("content-finished")
        missing = []
        for query, name in CONTENT_CASES:
            r = self.wd.invoke("content_search", {"query": query, "filters": NO_FILTERS, "offset": 0})
            names = [h["name"] for h in r["hits"][:CONTENT_TOP]]
            if name not in names:
                missing.append(f"{query!r} -> want {name}, got {names}")
        self.type_query(CONTENT_CASES[0][0])
        wait_for(lambda: self.visible("#content-hits") and self.wd.js(
            "return document.querySelectorAll('#content-list .chit').length") > 0,
            20, 0.25, "content section on screen")
        self.shoot("content-section")
        result = {"documents": st["total"], "read_s": round(read_s, 1),
                  "missing": missing}
        if missing:
            raise AssertionError(f"content queries missed: {missing}")
        return result

    def run(self):
        short = self.args.short
        self.step("first-run", self.s_first_run, critical=True)
        self.step("folders-drawer", self.s_open_folders)
        self.step("add-folder", self.s_add_folder, critical=True)
        self.step("scan-progress", self.s_scan_progress)
        self.step("search-during-scan", self.s_search_during_scan)
        self.step("scan-finished", self.s_scan_finishes, critical=True)
        self.step("search-name",
                  self.search_case("name", "Notaría Valdepeñas", gen_tree.T_NAME))
        self.step("search-middle-segment",
                  self.search_case("middle", "remedios extracto", gen_tree.T_MIDDLE))
        self.step("search-no-accents",
                  self.search_case("no_accents", "notaria valdepenas", gen_tree.T_NAME))
        self.step("search-typo",
                  self.search_case("typo", "notaria valdepeñaz", gen_tree.T_NAME))
        self.step("search-folder",
                  self.search_case("folder", "boda iñigo ainhoa", gen_tree.T_FOLDER))
        self.step("filter-type-audio", self.filter_case(
            "type_audio", self.apply_kind(gen_tree.KIND_AUDIO), "cuna", gen_tree.T_AUDIO))
        self.step("filter-date-week", self.filter_case(
            "date_week", lambda: self.set_select("#f-date", DATE_WEEK), "peñiscola playa",
            gen_tree.T_WEEK))
        if not short:
            self.step("filter-size-medium", self.filter_case(
                "size_medium", lambda: self.set_select("#f-size", SIZE_MEDIUM), "jubilacion",
                gen_tree.T_MEDIUM))
            self.step("filter-size-large", self.filter_case(
                "size_large", lambda: self.set_select("#f-size", SIZE_LARGE), "covadonga",
                gen_tree.T_LARGE))
            self.step("filter-size-huge", self.filter_case(
                "size_huge", lambda: self.set_select("#f-size", SIZE_HUGE), "",
                gen_tree.T_HUGE))
            self.step("filter-folder", self.filter_case(
                "folder",
                lambda: self.set_select("#f-folder", self.wd.invoke("list_roots")[0]["path"]),
                "extracto", gen_tree.T_MIDDLE))
        self.step("reveal-file", self.s_reveal(gen_tree.T_NAME, EXPECT_SELECT))
        if not short:
            self.step("reveal-folder", self.s_reveal(gen_tree.T_FOLDER, EXPECT_SELECT))
            # VFS-24 AC-1: a path over 260 characters is selected too.
            self.step("reveal-long-path", self.s_reveal(gen_tree.T_LONG, EXPECT_SELECT))
            self.step("live-create", self.s_live_create)
            self.step("live-rename", self.s_live_rename)
            self.step("live-delete", self.s_live_delete)
        self.step("memory-idle", self.s_memory_idle)
        if IS_WIN:
            self.step("memory-minimised", self.s_memory_minimised)
            self.step("accessibility", self.s_accessibility)
        if not short:
            self.step("closed-and-changed", self.s_close_and_change)
            self.step("reopen-catch-up", self.s_reopen_catch_up)
        self.step("content-search", self.s_content_search)
        self.app.stop()
        self.save()
        failed = [s["step"] for s in self.steps if not s["ok"]]
        print("failed steps:", failed or "none")
        return 1 if failed else 0


# ---------------------------------------------------------------------------------------------
# Native UI outside the webview: the folder dialog and the file manager


def main_window(pid):
    """The visible top-level window of process `pid` whose title starts with "VFS"."""
    import ctypes
    from ctypes import wintypes
    user32 = ctypes.windll.user32
    found = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def visit(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        title = ctypes.create_unicode_buffer(64)
        user32.GetWindowTextW(hwnd, title, 64)
        if owner.value == pid and user32.IsWindowVisible(hwnd) and title.value.startswith("VFS"):
            found.append(hwnd)
        return True

    user32.EnumWindows(visit, 0)
    if not found:
        raise AssertionError(f"no visible VFS window of process {pid}")
    return found[0]


def set_high_contrast(on):
    """Windows high contrast for this user (SPI_SETHIGHCONTRAST; no administrator needed)."""
    import ctypes
    from ctypes import wintypes

    class HIGHCONTRASTW(ctypes.Structure):
        _fields_ = [("cbSize", wintypes.UINT), ("dwFlags", wintypes.DWORD),
                    ("lpszDefaultScheme", wintypes.LPWSTR)]
    hc = HIGHCONTRASTW(ctypes.sizeof(HIGHCONTRASTW), 1 if on else 0, None)  # HCF_HIGHCONTRASTON
    # SPI_SETHIGHCONTRAST, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE
    if not ctypes.windll.user32.SystemParametersInfoW(0x0043, ctypes.sizeof(hc),
                                                      ctypes.byref(hc), 0x3):
        raise ctypes.WinError()


def set_animations_off(off):
    """Windows "show animations in Windows" (SPI_SETCLIENTAREAANIMATION), per user."""
    import ctypes
    if not ctypes.windll.user32.SystemParametersInfoW(0x1043, 0, ctypes.c_void_p(0 if off else 1),
                                                      0x3):
        raise ctypes.WinError()


def drive_windows_folder_dialog(path, journey):
    """Fills the Windows folder picker ("Elegir carpeta") with UI Automation."""
    from pywinauto import Desktop
    dlg = Desktop(backend="uia").window(title=FOLDER_DIALOG_TITLE)
    dlg.wait("visible", timeout=20)
    journey.shoot("folder-dialog")
    edit = dlg.child_window(auto_id="1152", control_type="Edit")
    edit.set_edit_text(path)
    journey.metrics["scan_t0"] = now()
    for _ in range(3):
        dlg.child_window(auto_id="1", control_type="Button").click_input()
        time.sleep(1.5)
        if not dlg.exists(timeout=0.5):
            return
    raise RuntimeError("folder dialog still open after three clicks on its OK button")


def close_windows_folder_dialog():
    try:
        from pywinauto import Desktop
        dlg = Desktop(backend="uia").window(title=FOLDER_DIALOG_TITLE)
        if dlg.exists(timeout=1):
            dlg.close()
    except Exception:
        pass


PS_EXPLORER = r"""
$ErrorActionPreference = 'SilentlyContinue'
$out = @()
foreach ($w in (New-Object -ComObject Shell.Application).Windows()) {
  if ($w.FullName -notlike '*explorer.exe') { continue }
  $sel = @(); foreach ($i in $w.Document.SelectedItems()) { $sel += $i.Path }
  $out += [pscustomobject]@{ folder = $w.Document.Folder.Self.Path; selected = $sel }
}
ConvertTo-Json -InputObject @($out) -Compress -Depth 4
"""

PS_CLOSE_EXPLORER = r"""
foreach ($w in (New-Object -ComObject Shell.Application).Windows()) {
  if ($w.FullName -like '*explorer.exe') { $w.Quit() }
}
"""


def powershell(script):
    r = subprocess.run(["powershell", "-NoProfile", "-Command", script], capture_output=True,
                       text=True, encoding="utf-8", timeout=60)
    return r.stdout.strip()


def close_file_manager_windows():
    if IS_WIN:
        powershell(PS_CLOSE_EXPLORER)


def win_path_key(p):
    """A Windows path in the form two spellings of the same path share: no `\\\\?\\`
    prefix, 8.3 short names expanded (Explorer reports long paths with them, VFS-24 AC-1),
    NFC, normalised separators, case-folded."""
    if not p:
        return ""
    p = long_name(p)
    if p.startswith("\\\\?\\"):
        p = p[4:]
    return ntpath.normcase(ntpath.normpath(nfc(p)))


def long_name(p):
    """`p` with any 8.3 short names expanded (GetLongPathNameW), when it exists."""
    if not IS_WIN:
        return p
    import ctypes
    q = p if p.startswith("\\\\?\\") else "\\\\?\\" + p
    buf = ctypes.create_unicode_buffer(32768)
    n = ctypes.windll.kernel32.GetLongPathNameW(q, buf, len(buf))
    return buf.value if 0 < n < len(buf) else p


def explorer_match(windows, target_path, expect):
    """The first `PS_EXPLORER` window that shows the target: on its folder with the target
    selected (`expect=select`), or on one of its ancestors (`expect=parent`). Selected items
    are compared by full path, never by display name, which drops the extension when
    Explorer hides extensions (Windows' default)."""
    path = win_path_key(target_path)
    folder = ntpath.dirname(path)
    for w in windows:
        wf = win_path_key(w.get("folder"))
        sel = [win_path_key(s) for s in (w.get("selected") or [])]
        if expect == EXPECT_SELECT and wf == folder and path in sel:
            return w
        if expect == EXPECT_PARENT and wf and folder.startswith(wf) and wf != folder:
            return w
    return None


def check_file_manager(target, expect, timeout):
    """Windows: an Explorer window that shows the target (`explorer_match`). macOS: Finder's
    front window, when AppleScript may ask Finder."""
    name = nfc(target["name"])
    if IS_WIN:
        windows = []

        def look():
            nonlocal windows
            raw = powershell(PS_EXPLORER)
            windows = json.loads(raw) if raw else []
            return explorer_match(windows, target["path"], expect)
        try:
            w, secs = wait_for(look, timeout, 0.5, "Explorer window")
            return {"ok": True, "explorer": w, "explorer_after_s": round(secs, 2)}
        except TimeoutError:
            res = {"ok": False, "explorer_windows": windows}
        # Diagnostic: does Explorer select the item when asked directly, outside the app?
        close_file_manager_windows()
        subprocess.Popen(f'explorer.exe /select,"{target["path"]}"')
        try:
            w, _ = wait_for(look, timeout, 0.5, "Explorer window (direct)")
            res["explorer_direct_selects"] = True
        except TimeoutError:
            res["explorer_direct_selects"] = False
            res["explorer_direct_windows"] = windows
        close_file_manager_windows()
        return res
    if IS_MAC:
        time.sleep(3)

        def ask(script):
            try:
                r = subprocess.run(["osascript", "-e", script], capture_output=True, text=True,
                                   timeout=10)
                return r.returncode, r.stdout.strip(), r.stderr.strip()[:200]
            except subprocess.TimeoutExpired:
                return None, "", "osascript timed out (Automation permission prompt?)"
        code, folder_out, err = ask('tell application "Finder" to get POSIX path of '
                                    '(target of front Finder window as alias)')
        _, sel, _ = ask('tell application "Finder" to get name of item 1 of (get selection)')
        res = {"finder_folder": folder_out, "finder_error": err, "finder_selected": sel}
        if code != 0:
            # Not a failure of the app: the runner may not let a script ask Finder.
            res["ok"] = True
            res["note"] = "Finder state not readable here; judged from the desktop screenshot"
        else:
            res["ok"] = nfc(sel) == name or expect == EXPECT_PARENT
        return res
    return {"ok": True, "note": "no file manager check on this platform"}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--app", required=True, help="path to the app binary (webdriver build)")
    ap.add_argument("--port", type=int, default=4445, help="embedded server port")
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--short", action="store_true", help="fewer steps (macOS)")
    ap.add_argument("--no-dialog", action="store_true", help="add the folder via add_root")
    ap.add_argument("--scan-timeout", type=int, default=1800)
    args = ap.parse_args(argv)
    os.makedirs(args.out, exist_ok=True)
    j = Journey(args)
    try:
        return j.run()
    finally:
        j.app.stop()
        j.save()
        shutil.copy(args.manifest, os.path.join(args.out, "manifest.json"))


if __name__ == "__main__":
    sys.exit(main())
