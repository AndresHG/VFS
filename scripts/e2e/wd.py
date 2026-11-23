"""VFS-8: the few W3C WebDriver calls the journey needs, over plain HTTP (standard library
only). It talks to the server embedded in the `webdriver` build, on Windows and on macOS
(spec decision 1)."""

import base64
import json
import time
import urllib.error
import urllib.request

ELEMENT_PREFIX = "element-6066-11e4-a52e-"


def element_id(ref):
    """The id inside a W3C element reference. Matched by prefix: tauri-plugin-wdio-webdriver
    1.5.1 answers with a misspelt key (`...cecf` instead of `...cecb`)."""
    for k, v in ref.items():
        if k.startswith(ELEMENT_PREFIX):
            return v
    raise WebDriverError(f"not an element reference: {ref}")


class WebDriverError(Exception):
    pass


class WebDriver:
    def __init__(self, url, timeout=60):
        self.url = url.rstrip("/")
        self.timeout = timeout
        self.session = None

    def _call(self, method, path, body=None, timeout=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.url + path, data=data, method=method,
                                     headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout or self.timeout) as r:
                payload = json.loads(r.read() or b"{}")
        except urllib.error.HTTPError as e:
            raw = e.read()
            try:
                payload = json.loads(raw)
            except ValueError:
                raise WebDriverError(f"{method} {path}: HTTP {e.code} {raw[:300]!r}") from None
        value = payload.get("value")
        if isinstance(value, dict) and value.get("error"):
            raise WebDriverError(f"{method} {path}: {value.get('error')}: "
                                 f"{str(value.get('message'))[:500]}")
        return value

    def wait_ready(self, seconds=60):
        end = time.time() + seconds
        last = None
        while time.time() < end:
            try:
                return self._call("GET", "/status", timeout=5)
            except Exception as e:  # server not listening yet
                last = e
                time.sleep(0.5)
        raise WebDriverError(f"driver at {self.url} not ready: {last}")

    def start(self, capabilities, timeout=180):
        value = self._call("POST", "/session",
                           {"capabilities": {"alwaysMatch": capabilities}}, timeout=timeout)
        self.session = value["sessionId"]
        return value

    def quit(self):
        if self.session:
            try:
                self._call("DELETE", f"/session/{self.session}", timeout=30)
            finally:
                self.session = None

    def s(self, method, path, body=None, timeout=None):
        return self._call(method, f"/session/{self.session}{path}", body, timeout)

    # Elements -------------------------------------------------------------------------------
    def find(self, css):
        return element_id(self.s("POST", "/element", {"using": "css selector", "value": css}))

    def find_all(self, css):
        return [element_id(e) for e in
                self.s("POST", "/elements", {"using": "css selector", "value": css})]

    def click(self, el):
        self.s("POST", f"/element/{el}/click", {})

    def clear(self, el):
        self.s("POST", f"/element/{el}/clear", {})

    def type(self, el, text):
        self.s("POST", f"/element/{el}/value", {"text": text})

    def text(self, el):
        return self.s("GET", f"/element/{el}/text")

    # Scripts --------------------------------------------------------------------------------
    def js(self, script, *args, timeout=None):
        return self.s("POST", "/execute/sync", {"script": script, "args": list(args)},
                      timeout=timeout)

    def js_async(self, script, *args, timeout=None):
        return self.s("POST", "/execute/async", {"script": script, "args": list(args)},
                      timeout=timeout)

    def invoke(self, cmd, args=None, timeout=120):
        """Calls a Tauri command from inside the page; returns its value or raises."""
        r = self.js_async(
            "const done = arguments[arguments.length - 1];"
            "window.__TAURI__.core.invoke(arguments[0], arguments[1])"
            ".then(v => done({ok: v === undefined ? null : v}))"
            ".catch(e => done({err: e}));",
            cmd, args or {}, timeout=timeout)
        if r is None or "err" in r:
            raise WebDriverError(f"invoke {cmd}: {r}")
        return r["ok"]

    def screenshot(self):
        return base64.b64decode(self.s("GET", "/screenshot", timeout=60))

    def set_script_timeout(self, ms):
        self.s("POST", "/timeouts", {"script": ms})
