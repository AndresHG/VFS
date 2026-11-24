// VFS-38: «Más filtros», the line of filters in use and the remembered choices, on the real page
// in a headless Chrome against the development fake backend (browser.mjs). The scope group
// became the results tabs (VFS-42, tabs.test.mjs).
// Run: node --test apps/desktop/ui-dev/filters.test.mjs   (needs Google Chrome or Chromium)

import test from 'node:test';
import assert from 'node:assert/strict';

import { startBrowser } from './browser.mjs';
import { CATALOGS, Lang } from '../ui/strings.js';
import { FileKind, SortOrder, Source } from '../ui/contract.js';
import { ResultsTab, SourceChoice } from '../ui/choices.js';
import { PrefKey } from '../ui/prefs.js';

const S = CATALOGS[Lang.ES];
/** A root folder of the fake backend, used as the «Carpeta» filter. */
const FOLDER = 'C:\\Users\\Carmen\\Documents';
const BOOKMARKS = '&bookmarks';

let page;

test.before(async () => {
  page = await startBrowser();
  await page.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
});

test.after(() => page?.close());

const evaluate = (expression) => page.evaluate(expression);
const until = (expression, what) => page.until(expression, what);

/** Loads the app with empty storage (or the given remembered values) and waits until the
 *  folders, and with them the remembered choices, are in. */
async function start(options = '', stored = {}) {
  await page.load(options);
  await evaluate(`(() => { localStorage.clear(); for (const [k, v] of Object.entries(${JSON.stringify(stored)})) localStorage.setItem(k, v); })()`);
  await reload(options);
}

async function reload(options = '') {
  await page.load(options);
  await until(`[...document.getElementById('f-folder').options].some((o) => o.value === ${JSON.stringify(FOLDER)})`, 'the folders');
}

/** Records the query and options of every search the screen sends. */
async function recordSearches() {
  await evaluate(`(async () => {
    const { api } = await import('/ui/api.js');
    window.searches = [];
    const run = api.searchSources;
    api.searchSources = (query, options) => { window.searches.push({ query, options }); return run.call(api, query, options); };
  })()`);
}

const tags = `[...document.querySelectorAll('#filter-tags .filter-tag')].map((b) => b.textContent)`;
const rowCount = `document.querySelectorAll('#results .row:not(.loading)').length`;

async function key(name, code = name, keyCode = 0) {
  for (const type of ['keyDown', 'keyUp']) {
    await page.send('Input.dispatchKeyEvent', { type, key: name, code, windowsVirtualKeyCode: keyCode });
  }
}

/** VFS-38 AC-6 (R6, minus the scope: VFS-42 R5): source and sort are kept across a restart;
 *  type and folder start empty. Unknown values or storage that throws give the defaults and
 *  the app starts; a stored «Marcadores» while bookmarks are off searches files. */
test('source and sort are remembered; filters are not', async () => {
  await start(BOOKMARKS);
  await page.choose('f-source', SourceChoice.FILES);
  await page.choose('f-sort', SortOrder.NEWEST);
  await page.kinds([FileKind.PHOTO]);
  await page.choose('f-folder', FOLDER);
  await reload(BOOKMARKS);
  assert.equal(await evaluate(`document.getElementById('f-source').value`), SourceChoice.FILES);
  assert.equal(await evaluate(`document.getElementById('f-sort').value`), SortOrder.NEWEST);
  assert.equal(await evaluate(`document.querySelectorAll('#kind-chips input:checked').length`), 0);
  assert.equal(await evaluate(`document.getElementById('f-folder').value`), '');
  // The remembered «Solo archivos» keeps the line of filters in use on screen (R8).
  assert.equal(await evaluate(`document.getElementById('active-filters').hidden`), false);

  const defaults = async (what) => {
    assert.equal(await evaluate(`document.getElementById('f-source').value`), SourceChoice.BOTH, what);
    assert.equal(await evaluate(`document.getElementById('f-sort').value`), SortOrder.RELEVANCE, what);
  };
  await start(BOOKMARKS, { [PrefKey.SOURCE]: 'mail', [PrefKey.SORT]: 'sideways' });
  await defaults('unknown values');

  const { identifier } = await page.send('Page.addScriptToEvaluateOnNewDocument', {
    source: `Object.defineProperty(window, 'localStorage', { get() { throw new DOMException('blocked', 'SecurityError'); } });`,
  });
  try {
    await reload(BOOKMARKS);
    await defaults('storage that throws');
    await page.choose('f-sort', SortOrder.NEWEST);
    assert.equal(await evaluate(`document.getElementById('f-sort').value`), SortOrder.NEWEST, 'choices still work, unremembered');
    await page.type('foto');
    await until(`${rowCount} > 0`, 'results without storage');
  } finally {
    await page.send('Page.removeScriptToEvaluateOnNewDocument', { identifier });
  }

  await start('', { [PrefKey.SOURCE]: SourceChoice.BOOKMARKS });
  await recordSearches();
  await page.type('foto');
  await until(`${rowCount} > 0`, 'file results');
  assert.deepEqual(await evaluate(`window.searches.at(-1).options.sources`), [Source.FILES]);
});

/** VFS-38 AC-7 (R7, R8, minus the scope: VFS-42 R5): «Más filtros (N)», the chips and
 *  «Quitar filtros (N)», which keeps the sort. */
test('«Más filtros», its chips and «Quitar filtros»', async () => {
  await start(BOOKMARKS);
  assert.equal(await evaluate(`document.getElementById('active-filters').hidden`), true, 'no line without filters');
  await evaluate(`document.getElementById('f-more').click()`);
  await until(`document.getElementById('f-more-pop').matches(':popover-open')`, 'the popover');
  assert.equal(await evaluate(`document.querySelector('#f-source-field label').textContent`), S.fSource);
  await page.choose('f-folder', FOLDER);
  await page.choose('f-source', SourceChoice.BOOKMARKS);
  assert.equal(await evaluate(`document.getElementById('f-more-text').textContent`), S.moreFiltersN(2));
  assert.deepEqual(await evaluate(tags), [S.chipFolder('Documents'), S.sourceChips[SourceChoice.BOOKMARKS]]);
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('#filter-tags .filter-tag')].map((b) => [b.getAttribute('aria-label'), b.querySelector('use').getAttribute('href')])`),
    [[S.removeFilter(S.chipFolder('Documents')), '#i-close'], [S.removeFilter(S.sourceChips[SourceChoice.BOOKMARKS]), '#i-close']]);
  assert.equal(await evaluate(`document.getElementById('f-clear').textContent`), S.clearFiltersN(2));

  await evaluate(`document.querySelector('#filter-tags .filter-tag').click()`);
  assert.equal(await evaluate(`document.getElementById('f-folder').value`), '', 'the folder chip clears the folder');
  assert.equal(await evaluate(`document.getElementById('f-source').value`), SourceChoice.BOOKMARKS, 'and leaves the source');
  assert.deepEqual(await evaluate(tags), [S.sourceChips[SourceChoice.BOOKMARKS]]);

  await page.choose('f-folder', FOLDER);
  await page.choose('f-sort', SortOrder.NAME);
  assert.equal(await evaluate(`document.getElementById('f-clear').textContent`), S.clearFiltersN(2));
  await evaluate(`document.getElementById('f-clear').click()`);
  assert.equal(await evaluate(`document.getElementById('f-folder').value`), '');
  assert.equal(await evaluate(`document.getElementById('f-source').value`), SourceChoice.BOTH);
  assert.equal(await evaluate(`document.getElementById('f-sort').value`), SortOrder.NAME, 'the sort stays');
  assert.equal(await evaluate(`document.getElementById('active-filters').hidden`), true);
});

/** VFS-38 AC-7 (R7): without the Popover API, «Más filtros» unfolds a row below the filter
 *  line and its filters work the same. */
test('without the Popover API «Más filtros» unfolds inline', async () => {
  const { identifier } = await page.send('Page.addScriptToEvaluateOnNewDocument', { source: 'delete HTMLElement.prototype.popover;' });
  try {
    await start(BOOKMARKS);
    assert.equal(await evaluate(`document.getElementById('filters').dataset.more`), 'inline');
    assert.equal(await evaluate(`document.getElementById('f-more-pop').hidden`), true);
    await evaluate(`document.getElementById('f-more').click()`);
    assert.equal(await evaluate(`document.getElementById('f-more').getAttribute('aria-expanded')`), 'true');
    const place = await evaluate(`(() => {
      const pop = document.getElementById('f-more-pop').getBoundingClientRect();
      const line = document.getElementById('f-more').getBoundingClientRect();
      return { below: pop.top >= line.bottom, height: pop.height, inFlow: getComputedStyle(document.getElementById('f-more-pop')).position };
    })()`);
    assert.ok(place.below && place.height > 0 && place.inFlow === 'static', JSON.stringify(place));
    await page.choose('f-folder', FOLDER);
    await page.choose('f-source', SourceChoice.FILES);
    assert.equal(await evaluate(`document.getElementById('f-more-text').textContent`), S.moreFiltersN(2));
    assert.deepEqual(await evaluate(tags), [S.chipFolder('Documents'), S.sourceChips[SourceChoice.FILES]]);
    await evaluate(`document.getElementById('f-more').click()`);
    assert.equal(await evaluate(`document.getElementById('f-more-pop').hidden`), true, 'the same button folds it');
  } finally {
    await page.send('Page.removeScriptToEvaluateOnNewDocument', { identifier });
  }
});

/** The rows on screen: name, the folded text of every highlight in the name and in the
 *  folder line, and how many badges. */
const ROWS = `[...document.querySelectorAll('#results .row:not(.loading)')].map((row) => {
  const fold = (s) => s.normalize('NFD').replace(/\\p{M}/gu, '').toLowerCase();
  return {
    name: row.querySelector('.name-text').textContent,
    nameMarks: [...row.querySelectorAll('.name mark')].map((m) => fold(m.textContent)),
    pathMarks: [...row.querySelectorAll('.path mark')].map((m) => fold(m.textContent)),
    badges: row.querySelectorAll('.badge').length,
    text: row.querySelector('.main').textContent,
  };
}).sort((a, b) => a.name.localeCompare(b.name))`;

/** VFS-38 AC-9 (R11): «peñalara» is highlighted in names and folder lines on «Nombres»
 *  (VFS-42 R2: names and paths), and no row carries a match label. */
test('the words stay highlighted and no row has a match label', async () => {
  await start();
  await page.type('peñalara');
  await until(`${rowCount} === 3`, 'the three hits');
  const all = await evaluate(ROWS);
  assert.deepEqual(all.map((r) => r.name), ['Excursión Peñalara.docx', 'IMG_0412.jpg', 'Peñalara julio']);
  assert.deepEqual(all.find((r) => r.name === 'IMG_0412.jpg').pathMarks, ['penalara'], 'a folder-line hit');
  for (const r of all) {
    assert.ok([...r.nameMarks, ...r.pathMarks].length > 0 && [...r.nameMarks, ...r.pathMarks].every((m) => m === 'penalara'), r.name);
    assert.equal(r.badges, 0, `${r.name}: no label`);
  }
});

/** Sets the window size of the next loads (CSS px) and the root text size. */
async function viewport(width, height, scale = 1) {
  await page.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: scale, mobile: false });
}

/** VFS-38 R7, VFS-35 R5: the «Más filtros» popover opens under its button and stays inside
 *  the window: moved left at the right edge (its width was read before it showed, so it hung
 *  off the minimum window), and as tall as its fields, scrolling when the room below the
 *  button is short (WebKit stretched it to the window's height). Esc closes it and gives the
 *  focus back to the button. */
test('the «Más filtros» popover stays inside the window', async () => {
  try {
    for (const [w, h, root] of [[720, 480, '100%'], [720, 480, '125%'], [1024, 496, '100%'], [1093, 534, '100%']]) {
      const where = `${w}x${h}, text ${root}`;
      await viewport(w, h, w > 1000 ? 1.25 : 1);
      await start(BOOKMARKS);
      await evaluate(`document.documentElement.style.fontSize = '${root}'`);
      await page.type('foto');
      await until(`${rowCount} > 0`, 'results');
      // As from the keyboard: the button has the focus when it opens the popover.
      await evaluate(`(() => { const b = document.getElementById('f-more'); b.focus(); b.click(); })()`);
      await until(`document.getElementById('f-more-pop').matches(':popover-open')`, 'the popover');
      const box = await evaluate(`(() => {
        const pop = document.getElementById('f-more-pop');
        const r = pop.getBoundingClientRect();
        const button = document.getElementById('f-more').getBoundingClientRect();
        const last = document.getElementById('f-source').getBoundingClientRect();
        return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, w: innerWidth, h: innerHeight, below: r.top >= button.bottom,
          fits: pop.scrollHeight <= pop.clientHeight + 1, scrolls: getComputedStyle(pop).overflowY === 'auto', lastBottom: last.bottom,
          fieldsHeight: [...pop.children].filter((c) => !c.hidden).reduce((s, c) => s + c.offsetHeight, 0) };
      })()`);
      assert.ok(box.left >= 0 && box.right <= box.w, `inside the window sideways ${JSON.stringify(box)} (${where})`);
      assert.ok(box.bottom <= box.h + 0.5 && box.below, `under its button and above the window's bottom ${JSON.stringify(box)} (${where})`);
      assert.ok(box.fits ? box.lastBottom <= box.bottom : box.scrolls, `every field in sight or reachable by scrolling ${JSON.stringify(box)} (${where})`);
      assert.ok(box.bottom - box.top <= box.fieldsHeight + 80, `as tall as its fields ${JSON.stringify(box)} (${where})`);
      await key('Escape', 'Escape', 27);
      await until(`!document.getElementById('f-more-pop').matches(':popover-open')`, 'Esc closes it');
      assert.equal(await evaluate(`document.activeElement?.id`), 'f-more', `the focus goes back to the button (${where})`);
    }
  } finally {
    await evaluate(`document.documentElement.style.fontSize = ''`);
    await viewport(1280, 800);
  }
});

/** VFS-6 R7: the size field is off while only bookmarks are searched, and looks it. */
test('the size field looks off while only bookmarks are searched', async () => {
  await start(BOOKMARKS);
  await evaluate(`document.getElementById('f-more').click()`);
  // The page's own style, not the engine's default for a disabled select (WebKit draws none).
  const look = `(() => { const s = getComputedStyle(document.getElementById('f-size')); return [s.opacity, s.cursor]; })()`;
  assert.deepEqual(await evaluate(look), ['1', 'pointer']);
  await page.choose('f-source', SourceChoice.BOOKMARKS);
  assert.equal(await evaluate(`document.getElementById('f-size').disabled`), true);
  assert.deepEqual(await evaluate(look), ['0.5', 'not-allowed'], 'dimmed by the page');
});

/** VFS-42 R3, VFS-27 R12: on «Documentos» with content search on, the hits take the name
 *  list's room with their count in the results header, bookmarks on and every source included
 *  (the source does not apply there, so «Solo marcadores» leaves them in), and the first hit is
 *  whole in sight at 1280×720 at 125 % (1024×496 CSS px). */
test('content hits show on «Documentos» with bookmarks on', async () => {
  try {
    for (const options of [`&content=on`, `${BOOKMARKS}&content=on`]) {
      await viewport(1024, 496, 1.25);
      await start(options);
      await until(`document.getElementById('content-on').checked`, 'the content status');
      await page.type('foto');
      await until(`${rowCount} > 0`, `name hits (${options})`);
      await page.tab(ResultsTab.DOCUMENTS);
      await until(`document.querySelectorAll('#content-list .chit').length > 0`, `content hits (${options})`);
      const first = await evaluate(`(() => {
        const r = document.querySelector('#content-list .chit').getBoundingClientRect();
        const bottom = Math.min(innerHeight, document.querySelector('.bottom').getBoundingClientRect().top);
        return { inside: r.top >= 0 && r.bottom <= bottom + 0.5, list: document.querySelector('.list-wrap').offsetParent,
          count: document.getElementById('summary').textContent, n: document.querySelectorAll('#content-list .chit').length };
      })()`);
      assert.ok(first.inside, `the first hit is whole in sight ${JSON.stringify(first)} (${options})`);
      assert.equal(first.list, null, `the hits take the name list's room (${options})`);
      assert.equal(first.count, S.contentCount(first.n, String(first.n), false), `the count (${options})`);
    }
    await page.tab(ResultsTab.NAMES);
    await page.choose('f-source', SourceChoice.BOOKMARKS);
    await page.tab(ResultsTab.DOCUMENTS);
    await until(`document.querySelectorAll('#content-list .chit').length > 0`, 'content hits with «Solo marcadores» chosen on «Nombres»');
  } finally {
    await viewport(1280, 800);
  }
});
