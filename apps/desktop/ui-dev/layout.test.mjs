// VFS-35: the window fits small and low-resolution screens. The real page runs in a headless
// Chrome against the development fake backend, at the content size of a maximised window on
// common laptop screens, at normal and large text. Chrome is driven over the DevTools
// protocol with Node built-ins only (no npm packages).
// Run: node --test apps/desktop/ui-dev/layout.test.mjs   (needs Google Chrome or Chromium)

import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('..', import.meta.url)); // apps/desktop

/** Content area of a maximised window on Windows 11 (taskbar 48 px and title bar 32 px, in
 *  CSS px) for each screen and scaling, plus the minimum window of tauri.conf.json. */
function maximised(w, h, scale) {
  return { name: `${w}x${h}@${scale * 100}%`, width: Math.round(w / scale), height: Math.round(h / scale) - 48 - 32, scale };
}
const VIEWPORTS = [
  maximised(1280, 720, 1), maximised(1280, 720, 1.25),
  maximised(1366, 768, 1), maximised(1366, 768, 1.25),
  maximised(1920, 1080, 1), maximised(1920, 1080, 1.25),
  { name: 'minimum window', width: 720, height: 480, scale: 1 },
];
const TEXT = [{ name: 'normal text', root: '100%' }, { name: 'large text', root: '125%' }];

const CHROME = [
  process.env.CHROME_PATH,
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/Applications/Chromium.app/Contents/MacOS/Chromium',
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
  '/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser',
].find((p) => p && existsSync(p));

const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf', '.svg': 'image/svg+xml' };

/** Serves apps/desktop on 127.0.0.1, so ui/api.js loads the fake backend (VFS-1 U5). */
function serve() {
  const server = createServer((req, res) => {
    const path = normalize(join(ROOT, decodeURIComponent(new URL(req.url, 'http://x').pathname)));
    const file = path.endsWith('/') || path.endsWith('\\') ? join(path, 'index.html') : path;
    if (!file.startsWith(ROOT) || !existsSync(file) || !statSync(file).isFile()) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' });
    res.end(readFileSync(file));
  });
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server)));
}

/** Starts headless Chrome and returns a minimal DevTools client for its first page. */
async function launch() {
  const profile = mkdtempSync(join(tmpdir(), 'vfs-layout-'));
  const proc = spawn(CHROME, [
    '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`, '--no-first-run',
    '--no-default-browser-check', '--disable-gpu', '--hide-scrollbars=false', 'about:blank',
  ], { stdio: ['ignore', 'ignore', 'pipe'] });
  const browserUrl = await new Promise((resolve, reject) => {
    let err = '';
    proc.stderr.on('data', (d) => {
      err += d;
      const m = err.match(/DevTools listening on (ws:\/\/\S+)/);
      if (m) resolve(m[1]);
    });
    proc.on('exit', () => reject(new Error(`Chrome exited: ${err}`)));
  });
  const port = new URL(browserUrl).port;
  const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const ws = new WebSocket(pages.find((p) => p.type === 'page').webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let id = 0;
  const pending = new Map();
  const waiters = [];
  ws.onmessage = (event) => {
    const msg = JSON.parse(event.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(msg.error.message));
      else resolve(msg.result);
    } else if (msg.method) {
      for (const w of waiters.splice(0)) if (w.method === msg.method) w.resolve(); else waiters.push(w);
    }
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    pending.set(++id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
  const once = (method) => new Promise((resolve) => waiters.push({ method, resolve }));
  const close = () => {
    ws.close();
    proc.kill();
    try { rmSync(profile, { recursive: true, force: true }); } catch { /* Chrome may still hold it */ }
  };
  return { send, once, close };
}

let server;
let chrome;
let base;

test.before(async () => {
  assert.ok(CHROME, 'Google Chrome or Chromium is needed for the layout tests (set CHROME_PATH)');
  server = await serve();
  base = `http://127.0.0.1:${server.address().port}/ui/?fake`;
  chrome = await launch();
  await chrome.send('Page.enable');
  await chrome.send('Runtime.enable');
});

test.after(() => {
  chrome?.close();
  server?.close();
});

/** Evaluates an expression in the page and returns its value. */
async function evaluate(expression) {
  const { result, exceptionDetails } = await chrome.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (exceptionDetails) throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
  return result.value;
}

/** Waits until `expression` is truthy in the page. */
async function until(expression, what) {
  for (let i = 0; i < 100; i++) {
    if (await evaluate(expression)) return;
    await new Promise((r) => setTimeout(r, 50));
  }
  assert.fail(`timed out waiting for ${what}`);
}

/** Loads the page at a viewport and text size. */
async function open(viewport, text, scenario = '') {
  await chrome.send('Emulation.setDeviceMetricsOverride', {
    width: viewport.width, height: viewport.height, deviceScaleFactor: viewport.scale, mobile: false,
  });
  const loaded = chrome.once('Page.loadEventFired');
  await chrome.send('Page.navigate', { url: scenario ? `${base}&scenario=${scenario}` : base });
  await loaded;
  await until(`document.getElementById('open-folders').textContent.trim() !== ''`, 'the bottom bar');
  // The system text size, as the UI sees it: the root font size its rem units follow.
  await evaluate(`document.documentElement.style.fontSize = '${text.root}'`);
  await evaluate(`new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))`);
}

/** Where an element stands: fully inside the window, and on top at the middle of its top edge, centre and bottom edge. */
const PLACE = `(el) => {
  if (!el || el.offsetParent === null && getComputedStyle(el).position !== 'fixed') return { shown: false };
  const r = el.getBoundingClientRect();
  const inside = r.width > 0 && r.height > 0 && r.left >= 0 && r.top >= 0 && r.right <= innerWidth + 0.5 && r.bottom <= innerHeight + 0.5;
  const points = [r.top + 2, r.top + r.height / 2, r.bottom - 2].map((y) => [r.left + r.width / 2, y]);
  const onTop = points.every(([x, y]) => { const top = document.elementFromPoint(x, y); return !!top && (top === el || el.contains(top)); });
  return { shown: true, inside, onTop, rect: [r.left, r.top, r.right, r.bottom].map(Math.round) };
}`;

/** Asserts an element is fully visible and not covered. */
async function assertVisible(selector, where) {
  const place = await evaluate(`(${PLACE})(document.querySelector(${JSON.stringify(selector)}))`);
  assert.ok(place.shown, `${selector} is shown (${where})`);
  assert.ok(place.inside, `${selector} is inside the window ${JSON.stringify(place.rect)} (${where})`);
  assert.ok(place.onTop, `${selector} is not covered (${where})`);
}

/** VFS-35 AC-5: nothing is wider than the window. */
async function assertNoSideScroll(where) {
  const widths = await evaluate(`[document.documentElement.scrollWidth, document.getElementById('app').scrollWidth, innerWidth]`);
  assert.ok(widths[0] <= widths[2] && widths[1] <= widths[2], `no sideways scroll ${JSON.stringify(widths)} (${where})`);
}

const FOOTER = ['#open-folders', '#open-settings'];
/** The first filter pill: covered or squashed filters fail here (VFS-35 AC-6). */
const FILTER = '#f-type';

for (const viewport of VIEWPORTS) {
  for (const text of TEXT) {
    const where = `${viewport.name}, ${viewport.width}x${viewport.height}, ${text.name}`;

    /** VFS-35 AC-1, AC-2, AC-3, AC-5, AC-6: start view, results view and folders panel. */
    test(`bottom bar, results and add-folder fit at ${where}`, async () => {
      await open(viewport, text);
      for (const s of [...FOOTER, FILTER]) await assertVisible(s, `start view, ${where}`);
      await assertNoSideScroll(`start view, ${where}`);

      await evaluate(`(() => { const q = document.getElementById('q'); q.value = 'foto'; q.dispatchEvent(new Event('input', { bubbles: true })); })()`);
      await until(`document.getElementById('app').dataset.view === 'list' && document.querySelectorAll('#results .row').length > 0`, 'results');
      for (const s of [...FOOTER, FILTER]) await assertVisible(s, `results view, ${where}`);
      // The part of the list in sight: not below the window, not under the bottom bar.
      const [listHeight, rowHeight] = await evaluate(`(() => {
        const list = document.getElementById('results').getBoundingClientRect();
        const bottom = Math.min(list.bottom, innerHeight, document.querySelector('.bottom').getBoundingClientRect().top);
        return [Math.round(bottom - list.top), document.querySelector('#results .row').offsetHeight];
      })()`);
      // The minimum window at large text has room for two rows; every other case for three.
      const rows = viewport.height <= 480 && text.root !== '100%' ? 2 : 3;
      assert.ok(listHeight >= rows * rowHeight, `results list shows ${listHeight}px, ${rows} rows of ${rowHeight}px (${where})`);
      await assertNoSideScroll(`results view, ${where}`);

      const short = await evaluate(`document.getElementById('app').clientHeight < 40 * parseFloat(getComputedStyle(document.documentElement).fontSize)`);
      if (short) {
        const tops = await evaluate(`[...document.querySelectorAll('#filters > .filter, #filters > #f-clear')].filter((el) => el.offsetParent !== null).map((el) => Math.round(el.getBoundingClientRect().top))`);
        assert.equal(new Set(tops).size, 1, `filter pills on one row ${JSON.stringify(tops)} (${where})`);
      }

      await evaluate(`document.getElementById('open-folders').click()`);
      await until(`document.getElementById('folders').open`, 'the folders panel');
      await assertVisible('#add-folder', `folders panel, ${where}`);
      await assertNoSideScroll(`folders panel, ${where}`);
    });

    /** VFS-35 AC-4, AC-5: no folders yet, the empty state offers "Añadir carpeta…". */
    test(`first-run add-folder button fits at ${where}`, async () => {
      await open(viewport, text, 'empty');
      await until(`document.querySelector('#empty .actions .btn-primary')`, 'the empty state');
      await assertVisible('#empty .actions .btn-primary', `no folders, ${where}`);
      for (const s of FOOTER) await assertVisible(s, `no folders, ${where}`);
      await assertNoSideScroll(`no folders, ${where}`);
    });
  }
}
