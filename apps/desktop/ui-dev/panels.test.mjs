// Side panels close on a click outside them, as on Esc, in every engine (WebKit, the macOS
// shell, has no `closedby`), without losing what was typed (ui/dismiss.js). Real mouse events
// in a headless Chrome against the development fake backend (browser.mjs).
// Run: node --test apps/desktop/ui-dev/panels.test.mjs   (needs Google Chrome or Chromium)

import test from 'node:test';
import assert from 'node:assert/strict';

import { startBrowser } from './browser.mjs';

let page;

test.before(async () => {
  page = await startBrowser();
  await page.send('Emulation.setDeviceMetricsOverride', { width: 1024, height: 600, deviceScaleFactor: 1, mobile: false });
});

test.after(() => page?.close());

const evaluate = (expression) => page.evaluate(expression);
const until = (expression, what) => page.until(expression, what);

/** A press at (x, y) released at (x2, y2): one real click when both are the same. */
async function press(x, y, x2 = x, y2 = y) {
  await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
  await page.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 });
  if (x2 !== x || y2 !== y) await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: x2, y: y2, button: 'left' });
  await page.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: x2, y: y2, button: 'left', clickCount: 1 });
  await evaluate(`new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))`);
}

async function openSettings() {
  await evaluate(`document.getElementById('open-settings').click()`);
  await until(`document.getElementById('settings').open`, 'the settings panel');
}

const isOpen = (id) => evaluate(`document.getElementById(${JSON.stringify(id)}).open`);

/** Ajustes closes on a click outside it, not on a click inside it nor on a press that starts
 *  inside and ends outside (selecting text). Esc still closes it, and so does a click outside
 *  the folders panel. */
test('a click outside a side panel closes it', async () => {
  await page.load();
  await openSettings();
  const panel = await evaluate(`(() => { const r = document.getElementById('settings').getBoundingClientRect(); return [r.left, r.top, r.right, r.bottom]; })()`);
  await press(panel[0] + 40, panel[1] + 140);
  assert.equal(await isOpen('settings'), true, 'a click inside keeps it open');
  // Start on plain text: a form control would take the whole gesture and prove nothing.
  const title = await evaluate(`(() => { const r = document.getElementById('settings-title').getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; })()`);
  await press(title[0], title[1], 20, 300);
  assert.equal(await isOpen('settings'), true, 'a press that starts inside keeps it open');
  await press(20, 300);
  assert.equal(await isOpen('settings'), false, 'a click outside closes it');

  await openSettings();
  await page.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
  await page.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
  await until(`!document.getElementById('settings').open`, 'Esc closes it');

  await evaluate(`document.getElementById('open-folders').click()`);
  await until(`document.getElementById('folders').open`, 'the folders panel');
  await press(20, 300);
  assert.equal(await isOpen('folders'), false, 'the folders panel closes too');
});

/** Closing Ajustes by a click outside loses nothing: a number being edited (the content size
 *  limit, applied on change) is applied, and a filter pattern typed and not yet added is still
 *  in its field when Ajustes opens again. */
test('a click outside Ajustes keeps unsaved input', async () => {
  await page.load('&content=on');
  await until(`!document.getElementById('content-max-row').hidden`, 'the content size limit');
  await openSettings();
  await evaluate(`document.querySelector('#settings .setting-advanced').open = true`);
  await evaluate(`(() => {
    const p = document.getElementById('file-filter-pattern');
    p.value = '*.bak';
    p.dispatchEvent(new Event('input', { bubbles: true }));
    const m = document.getElementById('content-max-mb');
    m.focus();
    m.select();
  })()`);
  // Typed as a person would, so the field's change fires when it loses the focus.
  await page.send('Input.insertText', { text: '250' });
  await press(20, 300);
  assert.equal(await isOpen('settings'), false);
  await until(`import('/ui/api.js').then(({ api }) => api.getContentMaxMb()).then((mb) => mb === 250)`, 'the size limit applied');
  await openSettings();
  assert.equal(await evaluate(`document.getElementById('file-filter-pattern').value`), '*.bak', 'the typed pattern is kept');
});
