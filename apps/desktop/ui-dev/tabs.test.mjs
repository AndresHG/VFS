// VFS-42: the «Nombres» and «Documentos» tabs of the results header, on the real page in a
// headless Chrome against the development fake backend (browser.mjs). The fake counts the
// name searches, content searches and content offers it is sent (`api.fakeCalls`).
// Run: node --test apps/desktop/ui-dev/tabs.test.mjs   (needs Google Chrome or Chromium)

import test from 'node:test';
import assert from 'node:assert/strict';

import { startBrowser } from './browser.mjs';
import { CATALOGS, Lang } from '../ui/strings.js';
import { FileKind, SuggestionKind } from '../ui/contract.js';
import { ResultsTab, SourceChoice } from '../ui/choices.js';

const S = CATALOGS[Lang.ES];
/** A root folder of the fake backend, used as the «Carpeta» filter. */
const FOLDER = 'C:\\Users\\Carmen\\Documents';
const BOOKMARKS = '&bookmarks';
const CONTENT = '&content=on';

let page;

test.before(async () => {
  page = await startBrowser();
  await page.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
});

test.after(() => page?.close());

const evaluate = (expression) => page.evaluate(expression);
const until = (expression, what) => page.until(expression, what);

/** Loads the app with empty storage (or the given stored values) and waits for the folders
 *  and, with content search on, for its status. */
async function start(options = '', stored = {}) {
  await page.load(options);
  await evaluate(`(() => { localStorage.clear(); for (const [k, v] of Object.entries(${JSON.stringify(stored)})) localStorage.setItem(k, v); })()`);
  await page.load(options);
  await until(`[...document.getElementById('f-folder').options].some((o) => o.value === ${JSON.stringify(FOLDER)})`, 'the folders');
  if (options.includes('content=on')) await until(`document.getElementById('content-on').checked`, 'the content status');
}

/** The fake backend's call counts. */
const calls = `(async () => (await import('/ui/api.js')).api.fakeCalls)()`;
async function resetCalls() {
  await evaluate(`(async () => { const c = (await import('/ui/api.js')).api.fakeCalls; for (const k of Object.keys(c)) c[k] = 0; })()`);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const chosenTab = `document.querySelector('#tabs [aria-selected="true"]')?.dataset.tab`;
const rowCount = `document.querySelectorAll('#results .row:not(.loading)').length`;
const hitCount = `document.querySelectorAll('#content-list .chit').length`;
const emptyBox = `(() => { const e = document.getElementById('empty'); return e.hidden ? null
  : { title: e.querySelector('h3')?.textContent, body: [...e.querySelectorAll('p')].map((p) => p.textContent),
    actions: [...e.querySelectorAll('.actions .btn')].map((b) => b.textContent) }; })()`;
const shown = (id) => `(document.getElementById(${JSON.stringify(id)}).offsetParent !== null)`;

async function key(name, code = name, keyCode = 0) {
  for (const type of ['keyDown', 'keyUp']) {
    await page.send('Input.dispatchKeyEvent', { type, key: name, code, windowsVirtualKeyCode: keyCode });
  }
}

/** VFS-42 AC-1 (R1): the start view shows no tabs; the results header is a tablist named
 *  «Buscar en» with «Nombres» selected; Tab reaches one tab; the arrow keys, Home and End move
 *  and choose; the results panel is labelled by the chosen tab; the filter line has no scope
 *  buttons. */
test('the tabs are a tablist with one tab stop, moved with the arrow keys', async () => {
  await start(CONTENT);
  assert.equal(await evaluate(`document.getElementById('app').dataset.view`), 'start');
  assert.equal(await evaluate(shown('tabs')), false, 'no tabs on the start view');
  assert.equal(await evaluate(`document.querySelectorAll('#filters [role="radio"], #filters [role="radiogroup"], #f-scope').length`), 0, 'no scope buttons');
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('#filters > button, #filters > label')].filter((e) => e.offsetParent).map((e) => e.id || e.querySelector('select').id)`),
    ['f-type', 'f-date', 'f-more'], '«Tipo», «Fecha» and «Más filtros» only');

  await page.type('foto');
  await until(`${rowCount} > 0`, 'results');
  assert.equal(await evaluate(shown('tabs')), true);
  assert.equal(await evaluate(chosenTab), ResultsTab.NAMES);
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('#tabs [role="tab"]')].map((t) => [t.textContent, t.tabIndex, t.getAttribute('aria-controls')])`),
    [[S.tabNames, 0, 'results-panel'], [S.tabDocuments, -1, 'results-panel']]);

  await evaluate(`document.getElementById('q').focus()`);
  for (let i = 0; i < 12 && !(await evaluate(`!!document.activeElement.closest('#tabs')`)); i++) await key('Tab', 'Tab', 9);
  assert.equal(await evaluate(`document.activeElement.dataset.tab`), ResultsTab.NAMES, 'Tab reaches the chosen tab');
  await key('Tab', 'Tab', 9);
  assert.ok(!(await evaluate(`!!document.activeElement.closest('#tabs')`)), 'one tab stop');
  await evaluate(`document.getElementById('tab-names').focus()`);

  await key('ArrowRight', 'ArrowRight', 39);
  assert.equal(await evaluate(`document.activeElement.dataset.tab`), ResultsTab.DOCUMENTS);
  assert.equal(await evaluate(chosenTab), ResultsTab.DOCUMENTS, 'the arrow chooses');
  await key('ArrowRight', 'ArrowRight', 39);
  assert.equal(await evaluate(chosenTab), ResultsTab.NAMES, 'the arrows wrap');
  await key('End', 'End', 35);
  assert.equal(await evaluate(chosenTab), ResultsTab.DOCUMENTS);
  await key('Home', 'Home', 36);
  assert.equal(await evaluate(chosenTab), ResultsTab.NAMES);
  await key('ArrowLeft', 'ArrowLeft', 37);
  assert.equal(await evaluate(chosenTab), ResultsTab.DOCUMENTS);

  const { nodes } = await page.send('Accessibility.getFullAXTree');
  const lists = nodes.filter((n) => n.role?.value === 'tablist');
  assert.equal(lists.length, 1);
  assert.equal(lists[0].name?.value, S.tabsLabel);
  const selected = nodes.filter((n) => n.role?.value === 'tab'
    && n.properties?.some((p) => p.name === 'selected' && String(p.value?.value) === 'true'));
  assert.deepEqual(selected.map((n) => n.name?.value), [S.tabDocuments]);
  const panels = nodes.filter((n) => n.role?.value === 'tabpanel');
  assert.deepEqual(panels.map((n) => n.name?.value), [S.tabDocuments], 'the panel is labelled by the chosen tab');
});

/** VFS-42 AC-2 (R2): with content search and bookmarks on, «Nombres» lists name and path hits
 *  (a file found only by its folder path included) and sends no content search; a document
 *  hit with an unread reason keeps its label. */
test('«Nombres» searches names and paths and never content', async () => {
  await start(`${BOOKMARKS}${CONTENT}`);
  await resetCalls();
  await page.type('foto');
  await until(`${rowCount} > 0`, 'name hits');
  await sleep(700); // past the content search's 300 ms debounce
  const rows = await evaluate(`[...document.querySelectorAll('#results .row:not(.loading)')].map((r) => ({
    name: r.querySelectorAll('.name mark').length, path: r.querySelectorAll('.path mark').length }))`);
  assert.ok(rows.some((r) => r.name > 0), 'a name hit');
  assert.ok(rows.some((r) => r.name === 0 && r.path > 0), 'a hit found only by its folder path');
  const c = await evaluate(calls);
  assert.ok(c.searchSources > 0, 'a name search');
  assert.equal(c.contentSearch, 0, 'no content search');
  assert.equal(await evaluate(`document.getElementById('content-hits').hidden`), true);

  await page.type('testamento');
  await until(`document.querySelector('#results .badge-unread')`, 'an unread label on a document hit');
  assert.equal((await evaluate(calls)).contentSearch, 0, 'still no content search');
});

/** VFS-42 AC-3 (R3): «Documentos» sends one content search and no name search; hits show with
 *  highlights, the count and «Ver más»; «Origen» and «Orden» are hidden; a short query asks for
 *  3 letters; no match offers «Buscar en los nombres» and, with a type set, «Quitar filtros»,
 *  below the reading note; a failed query shows the failure. */
test('«Documentos» runs only the content search', async () => {
  await start(`${BOOKMARKS}${CONTENT}`);
  await page.type('foto');
  await until(`${rowCount} > 0`, 'name hits');
  await until(shown('f-sort-pill'), '«Orden» on «Nombres»');
  await resetCalls();
  await page.tab(ResultsTab.DOCUMENTS);
  await until(`${hitCount} > 0`, 'the content hits');
  let c = await evaluate(calls);
  assert.equal(c.contentSearch, 1, 'one content search');
  // A name request sent before the switch may still land; a new text on «Documentos» must
  // start no name search.
  await resetCalls();
  await page.type('fotos');
  await until(`(async () => (await import('/ui/api.js')).api.fakeCalls.contentSearch >= 1)()`, 'the new content search');
  await until(`${hitCount} > 0`, 'the content hits');
  c = await evaluate(calls);
  assert.equal(c.searchSources, 0, 'no name search on «Documentos»');
  await page.type('foto');
  await until(`(() => { const m = [...document.querySelectorAll('#content-list mark')]; return m.length > 0 && m.every((x) => x.textContent === 'foto'); })()`, 'the content hits for «foto» again');
  assert.equal(await evaluate(rowCount), 0, 'no name list');
  assert.ok(await evaluate(`[...document.querySelectorAll('#content-list mark')].every((m) => m.textContent === 'foto') && document.querySelectorAll('#content-list mark').length > 0`), 'highlights');
  const n = await evaluate(hitCount);
  assert.equal(await evaluate(`document.getElementById('summary').textContent`), S.contentCount(n, String(n), false), 'the count');
  assert.equal(await evaluate(shown('f-sort-pill')), false, 'no «Orden»');
  await evaluate(`document.getElementById('f-more').click()`);
  assert.equal(await evaluate(`document.getElementById('f-source-field').hidden`), true, 'no «Origen»');
  await evaluate(`document.getElementById('f-more-pop').hidePopover()`);
  assert.equal(await evaluate(`document.getElementById('empty').hidden`), true, 'no empty view over the hits');

  // The fake has fewer than a page of hits while it reads: report more, so «Ver más» shows.
  await evaluate(`(async () => {
    const { api } = await import('/ui/api.js');
    const run = api.contentSearch;
    api.contentSearch = async (...a) => { const r = await run.apply(api, a); return { ...r, total: r.total + 20 }; };
  })()`);
  await page.type('fotos');
  await until(`!document.getElementById('content-more').hidden`, '«Ver más»');
  const sent = (await evaluate(calls)).contentSearch;
  await evaluate(`document.getElementById('content-more').click()`);
  await until(`${calls}.then((c) => c.contentSearch === ${sent + 1})`, '«Ver más» asks for the next page');

  await page.type('fo');
  await until(`${emptyBox}?.title === ${JSON.stringify(S.emptyDocumentsShort)}`, 'the 3-letter message');

  await page.type('zzzz');
  await until(`${emptyBox}?.actions.length === 1`, 'the no-match view');
  assert.deepEqual(await evaluate(emptyBox), { title: S.emptyNoHitsTitle('zzzz'), body: [S.contentNone], actions: [S.searchNames] });
  const note = await evaluate(`(() => { const n = document.querySelector('#content-notes p'); const e = document.getElementById('empty');
    return n && { text: n.textContent, above: n.getBoundingClientRect().bottom <= e.querySelector('h3').getBoundingClientRect().top }; })()`);
  assert.ok(note?.text.startsWith(S.contentStillReading('', '').slice(0, 8)) && note.above, `the reading note above it ${JSON.stringify(note)}`);
  await page.kinds([FileKind.PHOTO]);
  await until(`${emptyBox}?.actions.length === 2`, '«Quitar filtros» with a type set');
  assert.deepEqual((await evaluate(emptyBox)).actions, [S.searchNames, S.clearFilters]);
  await evaluate(`document.querySelector('#empty .actions .btn-primary').click()`);
  assert.equal(await evaluate(chosenTab), ResultsTab.NAMES, '«Buscar en los nombres» chooses «Nombres»');
  await page.kinds([]);

  await page.tab(ResultsTab.DOCUMENTS);
  await page.type('fallo');
  await until(`document.getElementById('content-notes').textContent.includes(${JSON.stringify(S.contentFailed)})`, 'the failure');
  assert.equal(await evaluate(emptyBox), null, 'never the no-match view');
});

/** VFS-42 AC-3 (R3): with a bookmark folder chosen, «Documentos» says bookmark folders have no
 *  documents, offers «Quitar la carpeta» and sends no query; «Carpeta» offers no bookmark
 *  folders there. */
test('a bookmark folder on «Documentos» has no documents', async () => {
  await start(`${BOOKMARKS}${CONTENT}`);
  await page.type('foto');
  await until(`${rowCount} > 0`, 'name hits');
  const bookmarkOptions = `[...document.querySelectorAll('#f-folder optgroup option')].map((o) => o.value)`;
  const roots = await evaluate(bookmarkOptions);
  assert.ok(roots.length > 0, 'bookmark roots on «Nombres»');
  await page.tab(ResultsTab.DOCUMENTS);
  assert.deepEqual(await evaluate(bookmarkOptions), [], 'no bookmark folders on «Documentos»');

  await page.tab(ResultsTab.NAMES);
  await page.choose('f-folder', roots[0]);
  await resetCalls();
  await page.tab(ResultsTab.DOCUMENTS);
  await until(`${emptyBox}?.title === ${JSON.stringify(S.documentsBookmarkFolder)}`, 'the bookmark-folder message');
  assert.deepEqual((await evaluate(emptyBox)).actions, [S.removeFolder]);
  assert.deepEqual(await evaluate(bookmarkOptions), [roots[0]], 'the chosen one stays chosen');
  assert.equal(await evaluate(`document.getElementById('f-folder').value`), roots[0]);
  await sleep(700);
  assert.equal((await evaluate(calls)).contentSearch, 0, 'no query');

  await evaluate(`document.querySelector('#empty .actions .btn-primary').click()`);
  assert.equal(await evaluate(`document.getElementById('f-folder').value`), '');
  await until(`${hitCount} > 0`, 'the content hits without the folder');
});

/** VFS-42 AC-4 (R4): with content search off, «Documentos» is shown; choosing it twice sends no
 *  search and asks for the offer's numbers at most once; its button turns content search on and
 *  a search runs. Turning content search off on «Documentos» keeps the tab and the invitation. */
test('«Documentos» invites to turn content search on', async () => {
  await start(BOOKMARKS);
  await page.type('foto');
  await until(`${rowCount} > 0`, 'name hits');
  assert.equal(await evaluate(shown('tab-documents')), true, 'shown with content search off');
  await resetCalls();
  await page.tab(ResultsTab.DOCUMENTS);
  await until(`${emptyBox}?.title === ${JSON.stringify(S.documentsOffTitle)}`, 'the invitation');
  assert.deepEqual((await evaluate(emptyBox)).actions, [S.contentOfferYes]);
  await page.tab(ResultsTab.NAMES);
  await page.tab(ResultsTab.DOCUMENTS);
  await page.type('fotos');
  await sleep(700);
  const c = await evaluate(calls);
  assert.equal(c.contentSearch, 0, 'no content search');
  assert.ok(c.contentOffer <= 1, `the offer asked at most once (${c.contentOffer})`);
  assert.equal(c.searchSources, 1, 'only the name search of the «Nombres» visit');
  assert.equal(await evaluate(chosenTab), ResultsTab.DOCUMENTS);

  await evaluate(`document.querySelector('#empty .actions .btn-primary').click()`);
  await until(`document.getElementById('content-on').checked`, 'content search on');
  await until(`${calls}.then((c) => c.contentSearch === 1)`, 'a content search once on');

  await evaluate(`(() => { const t = document.getElementById('content-on'); t.checked = false; t.dispatchEvent(new Event('change', { bubbles: true })); })()`);
  await until(`${emptyBox}?.title === ${JSON.stringify(S.documentsOffTitle)}`, 'the invitation again');
  assert.equal(await evaluate(chosenTab), ResultsTab.DOCUMENTS, 'the tab stays');
  assert.equal(await evaluate(`document.getElementById('content-hits').hidden`), true);
});

/** VFS-42 AC-4 (R4): the invitation's numbers come from the offer once known; while they are
 *  not (a failing offer), the text and button show without them. */
test('the invitation shows the offer numbers, or none', async () => {
  await start(BOOKMARKS);
  await page.type('foto');
  await until(`${rowCount} > 0`, 'name hits');
  await page.tab(ResultsTab.DOCUMENTS);
  await until(`${emptyBox}?.body[0]?.includes('1.200')`, 'the document count in the invitation');

  await start(BOOKMARKS);
  await evaluate(`(async () => { const { api } = await import('/ui/api.js'); api.contentOffer = async () => { throw { code: 'io' }; }; })()`);
  await page.type('foto');
  await until(`${rowCount} > 0`, 'name hits');
  await page.tab(ResultsTab.DOCUMENTS);
  await until(`${emptyBox}?.title === ${JSON.stringify(S.documentsOffTitle)}`, 'the invitation');
  await sleep(200);
  assert.deepEqual(await evaluate(emptyBox), { title: S.documentsOffTitle, body: [S.documentsOffBody], actions: [S.contentOfferYes] });
});

/** VFS-42 AC-5 (R5): a stored `vfs.scope` of any value opens on «Nombres»; with a type and a
 *  folder set on «Documentos», «Quitar filtros (2)» counts no tab and keeps «Documentos»; a
 *  non-default source counts only on «Nombres» and applies again there. */
test('the tab is not remembered and is not a filter', async () => {
  for (const stored of ['documents', 'all', 'names', 'junk']) {
    await start(CONTENT, { 'vfs.scope': stored });
    await page.type('foto');
    await until(`${rowCount} > 0`, `name hits (${stored})`);
    assert.equal(await evaluate(chosenTab), ResultsTab.NAMES, `opens on «Nombres» with vfs.scope=${stored}`);
  }

  await start(`${BOOKMARKS}${CONTENT}`);
  await page.type('foto');
  await until(`${rowCount} > 0`, 'name hits');
  await page.choose('f-source', SourceChoice.FILES);
  assert.equal(await evaluate(`document.getElementById('f-clear').textContent`), S.clearFiltersN(1));
  await page.tab(ResultsTab.DOCUMENTS);
  assert.equal(await evaluate(`document.getElementById('active-filters').hidden`), true, 'the source counts nowhere on «Documentos»');
  await page.kinds([FileKind.DOCUMENT]);
  await page.choose('f-folder', FOLDER);
  assert.equal(await evaluate(`document.getElementById('f-clear').textContent`), S.clearFiltersN(2));
  assert.equal(await evaluate(`document.getElementById('f-more-text').textContent`), S.moreFiltersN(1));
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('#filter-tags .filter-tag')].map((b) => b.textContent)`), [S.chipFolder('Documents')], 'no tab chip');
  await evaluate(`document.getElementById('f-clear').click()`);
  assert.equal(await evaluate(chosenTab), ResultsTab.DOCUMENTS, '«Quitar filtros» keeps the tab');
  assert.equal(await evaluate(`document.querySelectorAll('#kind-chips input:checked').length`), 0);
  assert.equal(await evaluate(`document.getElementById('f-folder').value`), '');
  assert.equal(await evaluate(`localStorage.getItem('vfs.scope')`), null, 'nothing kept for the tab');
});

/** VFS-42 AC-6 (R6): a «Nombres» search with a type that finds nothing offers `dropFilters`
 *  with no scope wording; pressing it clears the type and keeps a chosen bookmark folder. */
test('«Nombres» with nothing found offers to drop the filters', async () => {
  await start();
  await page.kinds([FileKind.AUDIO]);
  await page.type('ordesa');
  await until(`document.querySelector('#empty .actions .btn-primary')?.textContent === ${JSON.stringify(S.suggestDropFilters('2'))}`, 'the drop-filters suggestion');
  const body = await evaluate(`[...document.querySelectorAll('#empty p')].map((p) => p.textContent)`);
  assert.deepEqual(body, [S.emptyNoHitsFilters], 'no scope wording');
  await evaluate(`document.querySelector('#empty .actions .btn-primary').click()`);
  await until(`${rowCount} === 2`, 'the hits without filters');
  assert.equal(await evaluate(`document.querySelectorAll('#kind-chips input:checked').length`), 0);
  assert.equal(await evaluate(chosenTab), ResultsTab.NAMES);

  // Inside a bookmark folder the fake suggests nothing; a stand-in suggestion takes the same path.
  await start(BOOKMARKS);
  const root = await evaluate(`document.querySelector('#f-folder optgroup option').value`);
  await evaluate(`(async () => {
    const { api } = await import('/ui/api.js');
    api.suggest = async (q) => ({ kind: ${JSON.stringify(SuggestionKind.DROP_FILTERS)}, text: q.text, clearFilters: true, term: null, replacement: null, count: 1, capped: false });
  })()`);
  await page.choose('f-folder', root);
  await page.choose('f-source', SourceChoice.FILES); // a suggestion is asked only when files are searched
  await page.kinds([FileKind.AUDIO]);
  await page.type('recetas zz');
  await until(`document.querySelector('#empty .actions .btn-primary')?.textContent === ${JSON.stringify(S.suggestDropFilters('1'))}`, 'the suggestion');
  await evaluate(`document.querySelector('#empty .actions .btn-primary').click()`);
  assert.equal(await evaluate(`document.querySelectorAll('#kind-chips input:checked').length`), 0, 'the type cleared');
  await until(`document.getElementById('f-folder').value === ${JSON.stringify(root)}`, 'the bookmark folder kept');
});
