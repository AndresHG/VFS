// The real page in a headless Chrome, for the tests that need a browser (VFS-35, VFS-38): a
// server for apps/desktop on 127.0.0.1, so ui/api.js loads the development fake backend
// (VFS-1 U5), and a minimal DevTools protocol client with Node built-ins only (no npm packages).
// Needs Google Chrome or Chromium (or CHROME_PATH).

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('..', import.meta.url)); // apps/desktop

const CHROME = [
  process.env.CHROME_PATH,
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/Applications/Chromium.app/Contents/MacOS/Chromium',
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
  '/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser',
].find((p) => p && existsSync(p));

const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf', '.svg': 'image/svg+xml' };

/** Serves apps/desktop on 127.0.0.1. */
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

/** A served page in headless Chrome: `base` is the app on the fake backend; `evaluate` runs an
 *  expression in the page and returns its value; `until` waits for an expression to be truthy;
 *  `load` navigates to the app with extra URL options (Spanish unless `locale=` is given) and
 *  waits for the bottom bar. */
export async function startBrowser() {
  assert.ok(CHROME, 'Google Chrome or Chromium is needed for the browser tests (set CHROME_PATH)');
  const server = await serve();
  const base = `http://127.0.0.1:${server.address().port}/ui/?fake`;
  const chrome = await launch();
  await chrome.send('Page.enable');
  await chrome.send('Runtime.enable');

  async function evaluate(expression) {
    const { result, exceptionDetails } = await chrome.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (exceptionDetails) throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
    return result.value;
  }

  async function until(expression, what) {
    for (let i = 0; i < 100; i++) {
      if (await evaluate(expression)) return;
      await new Promise((r) => setTimeout(r, 50));
    }
    assert.fail(`timed out waiting for ${what}`);
  }

  async function load(options = '') {
    // The tests read Spanish copy unless they ask for a locale: CI's Chrome runs in English.
    const locale = options.includes('locale=') ? '' : '&locale=es-ES';
    const loaded = chrome.once('Page.loadEventFired');
    await chrome.send('Page.navigate', { url: `${base}${options}${locale}` });
    await loaded;
    await until(`document.getElementById('open-folders').textContent.trim() !== ''`, 'the bottom bar');
  }

  /** Types `text` in the search box as a person would (one input event). */
  async function type(text) {
    await evaluate(`(() => { const q = document.getElementById('q'); q.value = ${JSON.stringify(text)}; q.dispatchEvent(new Event('input', { bubbles: true })); })()`);
  }

  /** Sets the select `id` to `value`, as a change by the user. */
  async function choose(id, value) {
    await evaluate(`(() => { const s = document.getElementById(${JSON.stringify(id)}); s.value = ${JSON.stringify(value)}; s.dispatchEvent(new Event('change', { bubbles: true })); })()`);
  }

  /** Ticks exactly the type chips in `kinds`. */
  async function kinds(list) {
    await evaluate(`(() => { for (const i of document.querySelectorAll('#kind-chips input')) {
      const on = ${JSON.stringify(list)}.includes(i.value);
      if (i.checked !== on) { i.checked = on; i.dispatchEvent(new Event('change', { bubbles: true })); }
    } })()`);
  }

  /** Clicks the scope radio for `choice` (VFS-38). */
  async function scope(choice) {
    await evaluate(`document.querySelector('#f-scope [data-scope=${JSON.stringify(choice)}]').click()`);
  }

  return {
    base, send: chrome.send, evaluate, until, load, type, choose, kinds, scope,
    close() { chrome.close(); server.close(); },
  };
}
