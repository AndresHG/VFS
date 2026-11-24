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

const scopeButtons = `[...document.querySelectorAll('#f-scope [role="radio"]')].map((b) => b.dataset.scope)`;
const emptyBox = `(() => { const e = document.getElementById('empty'); return e.hidden ? null
  : { title: e.querySelector('h3')?.textContent, body: [...e.querySelectorAll('p')].map((p) => p.textContent),
    action: e.querySelector('.actions .btn-primary')?.textContent ?? null }; })()`;

/** VFS-38 AC-4 (R1, R4, R5): «Solo documentos» is offered only while content search is on. It
 *  shows the content section alone: no name list, no «Origen», no «Orden», and no name search
 *  sent. A short query asks for 3 letters; no match offers «Buscar en todo», which brings back
 *  «Todo». Turning content search off, or a restart with it off, brings back «Todo». */
test('«Solo documentos» shows only the content results', async () => {
  await start(BOOKMARKS);
  assert.deepEqual(await evaluate(scopeButtons), [ScopeChoice.ALL, ScopeChoice.NAMES], 'content search off');

  await start(`${BOOKMARKS}&content=on`);
  await until(`${scopeButtons}.length === 3`, 'the third button once content search is on');
  assert.deepEqual(await evaluate(scopeButtons), [ScopeChoice.ALL, ScopeChoice.NAMES, ScopeChoice.DOCUMENTS]);
  await until(`!document.getElementById('f-source-field').hidden`, '«Origen» under «Todo» with bookmarks on');
  await recordSearches();
  await page.scope(ScopeChoice.DOCUMENTS);
  assert.equal(await evaluate(checkedScope), ScopeChoice.DOCUMENTS);
  await page.type('peñalara');
  await until(`document.querySelectorAll('#content-list .chit').length > 0`, 'the content hits');
  assert.equal(await evaluate(rowCount), 0, 'no name list');
  assert.deepEqual(await evaluate('window.searches'), [], 'no name or path search');
  assert.equal(await evaluate(`document.getElementById('f-source-field').hidden`), true, 'no «Origen»');
  assert.equal(await evaluate(`document.getElementById('f-sort-pill').hidden`), true, 'no «Orden»');
  assert.equal(await evaluate(emptyBox), null, 'no empty view over the hits');
  assert.equal(await evaluate(`document.querySelector('.list-wrap').offsetParent`), null, 'the section takes the name list\'s room');
  assert.equal(await evaluate(`document.getElementById('content-title').textContent`), S.contentTitle);

  await page.type('pe');
  await until(`${emptyBox}?.title === ${JSON.stringify(S.emptyDocumentsShort)}`, 'the 3-letter message');
  assert.equal(await evaluate(`document.getElementById('content-hits').hidden`), true);

  await page.type('zzzz');
  await until(`${emptyBox}?.action === ${JSON.stringify(S.searchEverything)}`, 'the no-match view');
  assert.deepEqual(await evaluate(emptyBox), {
    title: S.emptyNoHitsTitle('zzzz'), body: [S.contentNone], action: S.searchEverything,
  });
  assert.ok(!(await evaluate(`document.getElementById('content-notes').textContent`)).includes(S.contentNone), 'said once');
  await evaluate(`document.querySelector('#empty .actions .btn-primary').click()`);
  assert.equal(await evaluate(checkedScope), ScopeChoice.ALL, '«Buscar en todo» sets «Todo»');
  await until('window.searches.length > 0', 'the name search under «Todo»');

  await page.scope(ScopeChoice.DOCUMENTS);
  await evaluate(`(() => { const t = document.getElementById('content-on'); t.checked = false; t.dispatchEvent(new Event('change', { bubbles: true })); })()`);
  await until(`${scopeButtons}.length === 2`, 'two buttons once content search is off');
  assert.equal(await evaluate(checkedScope), ScopeChoice.ALL, 'off brings back «Todo»');
  assert.equal(await evaluate(`localStorage.getItem(${JSON.stringify(PrefKey.SCOPE)})`), ScopeChoice.ALL);

  await start(BOOKMARKS, { [PrefKey.SCOPE]: ScopeChoice.DOCUMENTS });
  await until(`${checkedScope} === ${JSON.stringify(ScopeChoice.ALL)}`, '«Todo» after a restart with content search off');
  assert.deepEqual(await evaluate(scopeButtons), [ScopeChoice.ALL, ScopeChoice.NAMES]);

  await start(`${BOOKMARKS}&content=on`, { [PrefKey.SCOPE]: ScopeChoice.DOCUMENTS });
  await until(`${scopeButtons}.length === 3`, 'the content status');
  assert.equal(await evaluate(checkedScope), ScopeChoice.DOCUMENTS, 'kept while content search is on');
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

/** VFS-38 R2, VFS-27 R12: under «Todo» with content search on, the content section shows
 *  below the name hits with its count, bookmarks on and «Archivos y marcadores» included (the
 *  default source sends no source list, which read as "no files" and hid the section), and
 *  its heading is in sight at 1280×720 at 125 % (1024×496 CSS px) even with thousands of name hits. Only
 *  «Solo marcadores» leaves it out. */
test('content hits show under «Todo» with bookmarks on', async () => {
  try {
    for (const options of [`&content=on`, `${BOOKMARKS}&content=on`]) {
      await viewport(1024, 496, 1.25);
      await start(options);
      await until(`document.querySelectorAll('#f-scope [role="radio"]').length === 3`, 'the content status');
      assert.equal(await evaluate(checkedScope), ScopeChoice.ALL);
      await page.type('foto');
      await until(`${rowCount} > 0 && document.querySelectorAll('#content-list .chit').length > 0`, `name and content hits (${options})`);
      const head = await evaluate(`(() => {
        const sec = document.getElementById('content-hits');
        const r = document.querySelector('.content-head').getBoundingClientRect();
        const top = document.elementFromPoint(r.left + 20, r.top + r.height / 2);
        return { hidden: sec.hidden, inside: r.top >= 0 && r.bottom <= innerHeight, onTop: !!top && document.querySelector('.content-head').contains(top),
          count: document.getElementById('content-count').textContent, n: document.querySelectorAll('#content-list .chit').length };
      })()`);
      assert.ok(!head.hidden && head.inside && head.onTop, `the heading is in sight ${JSON.stringify(head)} (${options})`);
      assert.equal(head.count, S.contentCount(head.n, String(head.n), false), `the count (${options})`);
    }
    await page.choose('f-source', SourceChoice.BOOKMARKS);
    await until(`document.getElementById('content-hits').hidden`, 'no content section for bookmarks alone');
  } finally {
    await viewport(1280, 800);
  }
});
