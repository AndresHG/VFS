// One bookmark switch per browser in the folders panel (VFS-44 AC-1 on the screen): the
// browsers the status lists, each with its own switch and status line, in a headless Chrome
// against the development fake backend (browser.mjs; Firefox holds the sample bookmarks,
// Chrome is found and holds none).
// Run: node --test apps/desktop/ui-dev/bookmarks.test.mjs   (needs Google Chrome or Chromium)

import test from 'node:test';
import assert from 'node:assert/strict';

import { startBrowser } from './browser.mjs';

let page;

test.before(async () => {
  page = await startBrowser();
});

test.after(() => page?.close());

const evaluate = (expression) => page.evaluate(expression);
const until = (expression, what) => page.until(expression, what);

const switches = () => evaluate(`[...document.querySelectorAll('#bookmarks-browsers input')]
  .map((i) => [i.dataset.browser, i.checked, i.parentElement.textContent.trim()])`);

/** VFS-44 AC-1, R9: Firefox (on) and Chrome (off) each have a switch; turning Chrome on reads
 *  it and says no Chrome bookmarks were found, leaving Firefox on; the focused switch keeps
 *  the focus through the status update. */
test('one switch per browser', async () => {
  await page.load('&bookmarks');
  await evaluate(`document.getElementById('open-folders').click()`);
  await until(`document.getElementById('folders').open`, 'the folders panel');
  await evaluate(`document.getElementById('bookmarks-setting').open = true`);
  await until(`document.querySelectorAll('#bookmarks-browsers input').length === 2`, 'two switches');
  assert.deepEqual(await switches(), [
    ['firefox', true, 'Incluir marcadores de Firefox'],
    ['chrome', false, 'Incluir marcadores de Chrome'],
  ]);

  await evaluate(`(() => {
    const c = document.querySelector('#bookmarks-browsers input[data-browser="chrome"]');
    c.focus();
    c.click();
  })()`);
  await until(`document.getElementById('bookmarks-browsers').textContent.includes('No se han encontrado marcadores de Chrome')`,
    'Chrome read');
  assert.deepEqual((await switches()).map(([b, on]) => [b, on]), [['firefox', true], ['chrome', true]]);
  assert.equal(await evaluate(`document.activeElement?.dataset.browser`), 'chrome');
});
