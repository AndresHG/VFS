// VFS-38: the scope group, «Más filtros», the line of filters in use and the remembered choices,
// on the real page in a headless Chrome against the development fake backend (browser.mjs).
// Run: node --test apps/desktop/ui-dev/filters.test.mjs   (needs Google Chrome or Chromium)

import test from 'node:test';
import assert from 'node:assert/strict';

import { startBrowser } from './browser.mjs';
import { CATALOGS, Lang } from '../ui/strings.js';
import { FileKind, SortOrder, Source } from '../ui/contract.js';
import { ScopeChoice, SourceChoice } from '../ui/choices.js';
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

const checkedScope = `document.querySelector('#f-scope [aria-checked="true"]')?.dataset.scope`;
const tags = `[...document.querySelectorAll('#filter-tags .filter-tag')].map((b) => b.textContent)`;
const rowCount = `document.querySelectorAll('#results .row:not(.loading)').length`;

async function key(name, code = name, keyCode = 0) {
  for (const type of ['keyDown', 'keyUp']) {
    await page.send('Input.dispatchKeyEvent', { type, key: name, code, windowsVirtualKeyCode: keyCode });
  }
}

/** VFS-38 AC-5 (R1): the scope group is one tab stop; the arrow keys move the choice and each
 *  move runs the search again; the accessibility tree has one radio group named «Buscar en»
 *  with the chosen radio checked. */
test('the scope group is a radio group moved with the arrow keys', async () => {
  await start();
  await recordSearches();
  await page.type('foto');
  await until(`${rowCount} > 0`, 'results');
  assert.equal(await evaluate(`[...document.querySelectorAll('#f-scope [role="radio"]')].filter((b) => b.tabIndex === 0).length`), 1);

  await evaluate(`document.getElementById('q').focus()`);
  for (let i = 0; i < 4 && !(await evaluate(`!!document.activeElement.closest('#f-scope')`)); i++) await key('Tab', 'Tab', 9);
  assert.equal(await evaluate(`document.activeElement.dataset.scope`), ScopeChoice.ALL, 'Tab reaches the chosen radio');

  await evaluate(`window.searches.length = 0`);
  await key('ArrowRight', 'ArrowRight', 39);
  assert.equal(await evaluate(`document.activeElement.dataset.scope`), ScopeChoice.NAMES);
  assert.equal(await evaluate(checkedScope), ScopeChoice.NAMES);
  await until(`window.searches.some((s) => s.query.scope === ${JSON.stringify(ScopeChoice.NAMES)})`, 'a names-only search');
  await key('ArrowRight', 'ArrowRight', 39);
  assert.equal(await evaluate(checkedScope), ScopeChoice.ALL, 'the arrows wrap');
  await key('ArrowLeft', 'ArrowLeft', 37);
  assert.equal(await evaluate(checkedScope), ScopeChoice.NAMES);
  await until(`window.searches.at(-1).query.scope === ${JSON.stringify(ScopeChoice.NAMES)}`, 'the search run again');

  const { nodes } = await page.send('Accessibility.getFullAXTree');
  const groups = nodes.filter((n) => n.role?.value === 'radiogroup');
  assert.equal(groups.length, 1);
  assert.equal(groups[0].name?.value, S.fScope);
  const checked = nodes.filter((n) => n.role?.value === 'radio'
    && n.properties?.some((p) => p.name === 'checked' && String(p.value?.value) === 'true'));
  assert.deepEqual(checked.map((n) => n.name?.value), [S.scopeOptions[ScopeChoice.NAMES]]);
});

/** VFS-38 AC-6 (R6): scope, source and sort are kept across a restart; type and folder start
 *  empty. Unknown values or storage that throws give the defaults and the app starts; a stored
 *  «Marcadores» while bookmarks are off searches files. */
test('scope, source and sort are remembered; filters are not', async () => {
  await start(BOOKMARKS);
  await page.scope(ScopeChoice.NAMES);
  await page.choose('f-source', SourceChoice.FILES);
  await page.choose('f-sort', SortOrder.NEWEST);
  await page.kinds([FileKind.PHOTO]);
  await page.choose('f-folder', FOLDER);
  await reload(BOOKMARKS);
  assert.equal(await evaluate(checkedScope), ScopeChoice.NAMES);
  assert.equal(await evaluate(`document.getElementById('f-source').value`), SourceChoice.FILES);
  assert.equal(await evaluate(`document.getElementById('f-sort').value`), SortOrder.NEWEST);
  assert.equal(await evaluate(`document.querySelectorAll('#kind-chips input:checked').length`), 0);
  assert.equal(await evaluate(`document.getElementById('f-folder').value`), '');
  // The remembered «Solo nombres» keeps the line of filters in use on screen (R8).
  assert.equal(await evaluate(`document.getElementById('active-filters').hidden`), false);

  const defaults = async (what) => {
    assert.equal(await evaluate(checkedScope), ScopeChoice.ALL, what);
    assert.equal(await evaluate(`document.getElementById('f-source').value`), SourceChoice.BOTH, what);
    assert.equal(await evaluate(`document.getElementById('f-sort').value`), SortOrder.RELEVANCE, what);
  };
  await start(BOOKMARKS, { [PrefKey.SCOPE]: 'everything', [PrefKey.SOURCE]: 'mail', [PrefKey.SORT]: 'sideways' });
  await defaults('unknown values');

  const { identifier } = await page.send('Page.addScriptToEvaluateOnNewDocument', {
    source: `Object.defineProperty(window, 'localStorage', { get() { throw new DOMException('blocked', 'SecurityError'); } });`,
  });
  try {
    await reload(BOOKMARKS);
    await defaults('storage that throws');
    await page.scope(ScopeChoice.NAMES);
    assert.equal(await evaluate(checkedScope), ScopeChoice.NAMES, 'choices still work, unremembered');
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

/** VFS-38 AC-7 (R7, R8): «Más filtros (N)», the chips and «Quitar filtros (N)». */
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
  await page.scope(ScopeChoice.NAMES);
  await page.choose('f-sort', SortOrder.NAME);
  assert.equal(await evaluate(`document.getElementById('f-clear').textContent`), S.clearFiltersN(3));
  await evaluate(`document.getElementById('f-clear').click()`);
  assert.equal(await evaluate(`document.getElementById('f-folder').value`), '');
  assert.equal(await evaluate(`document.getElementById('f-source').value`), SourceChoice.BOTH);
  assert.equal(await evaluate(checkedScope), ScopeChoice.ALL);
  assert.equal(await evaluate(`document.getElementById('f-sort').value`), SortOrder.NAME, 'the sort stays');
  assert.equal(await evaluate(`document.getElementById('active-filters').hidden`), true);

  await page.scope(ScopeChoice.NAMES);
  assert.deepEqual(await evaluate(tags), []);
  assert.equal(await evaluate(`document.getElementById('f-clear').textContent`), S.clearFiltersN(1));
  assert.equal(await evaluate(`document.getElementById('active-filters').hidden`), false);
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

/** VFS-38 AC-9 (R11): «peñalara» is highlighted in names and folder lines under «Todo» and
 *  «Solo nombres», and no row carries a match label. */
test('the words stay highlighted and no row has a match label', async () => {
  await start();
  await page.type('peñalara');
  await until(`${rowCount} === 3`, 'the three «Todo» hits');
  const all = await evaluate(ROWS);
  assert.deepEqual(all.map((r) => r.name), ['Excursión Peñalara.docx', 'IMG_0412.jpg', 'Peñalara julio']);
  assert.deepEqual(all.find((r) => r.name === 'IMG_0412.jpg').pathMarks, ['penalara'], 'a folder-line hit');
  for (const r of all) {
    assert.ok([...r.nameMarks, ...r.pathMarks].length > 0 && [...r.nameMarks, ...r.pathMarks].every((m) => m === 'penalara'), r.name);
    assert.equal(r.badges, 0, `${r.name}: no label`);
  }

  await page.scope(ScopeChoice.NAMES);
  await until(`${rowCount} === 2`, 'the two names-only hits');
  const names = await evaluate(ROWS);
  assert.deepEqual(names.map((r) => r.name), ['Excursión Peñalara.docx', 'Peñalara julio']);
  for (const r of names) {
    assert.deepEqual(r.nameMarks, ['penalara'], r.name);
    assert.equal(r.badges, 0, `${r.name}: no label`);
    assert.deepEqual(r, all.find((x) => x.name === r.name), `${r.name}: drawn as under «Todo»`);
  }
});

/** VFS-38 AC-2 (R3, UI part): with content search on, «Solo nombres» runs no content search,
 *  so the section «En el contenido de los documentos» goes away; «Todo» brings it back. */
test('«Solo nombres» hides the content section; details sit below it', async () => {
  await start('&content=on');
  await page.type('peñalara');
  const shown = `!document.getElementById('content-hits').hidden`;
  await until(shown, 'the content section under «Todo»');
  // VFS-38 R9: the results header adds a grid row; the selected file's details stay below the
  // content section instead of on top of it.
  await until(`${rowCount} > 0`, 'the name hits');
  await evaluate(`document.querySelector('#results .row:not(.loading)').click()`);
  await until(`document.getElementById('details').textContent.trim() !== ''`, 'the details line');
  const gap = await evaluate(`document.getElementById('details').getBoundingClientRect().top
    - document.getElementById('content-hits').getBoundingClientRect().bottom`);
  assert.ok(gap >= 0, `details ${gap}px below the content section`);
  await page.scope(ScopeChoice.NAMES);
  await until(`!(${shown})`, 'no content section under «Solo nombres»');
  await page.scope(ScopeChoice.ALL);
  await until(shown, 'the content section back under «Todo»');
});

/** VFS-38 AC-11 (R13, UI part): a names-only search that finds nothing says so and offers the
 *  «Todo» count; pressing it lists the folder's files with the scope back on «Todo». A
 *  «sin la última palabra» suggestion keeps «Solo nombres». */
test('the empty view names the scope and its suggestion brings back «Todo»', async () => {
  await start();
  await page.scope(ScopeChoice.NAMES);
  await page.type('ordesa');
  await until(`document.querySelector('#empty .actions .btn-primary')?.textContent === ${JSON.stringify(S.suggestDropFilters('2'))}`, 'the «Todo» count');
  assert.ok((await evaluate(`[...document.querySelectorAll('#empty p')].map((p) => p.textContent)`)).includes(S.emptyNamesOnly));
  await evaluate(`document.querySelector('#empty .actions .btn-primary').click()`);
  await until(`${rowCount} === 2`, 'the folder\'s files');
  assert.equal(await evaluate(checkedScope), ScopeChoice.ALL);
  assert.ok((await evaluate(ROWS)).every((r) => r.text.includes('Ordesa 2019')));

  await page.scope(ScopeChoice.NAMES);
  await page.type('peñalara zzzz');
  await until(`document.querySelector('#empty .actions .btn-primary')?.textContent === ${JSON.stringify(S.suggestDropTerm('zzzz', '2'))}`, 'a drop-term suggestion');
  await evaluate(`document.querySelector('#empty .actions .btn-primary').click()`);
  await until(`${rowCount} === 2`, 'the names-only hits');
  assert.equal(await evaluate(checkedScope), ScopeChoice.NAMES, 'the scope stays');
});
