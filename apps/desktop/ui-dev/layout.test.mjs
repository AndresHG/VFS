// VFS-35: the window fits small and low-resolution screens. The real page runs in a headless
// Chrome against the development fake backend, at the content size of a maximised window on
// common laptop screens, at normal and large text. Chrome is driven over the DevTools
// protocol (browser.mjs). VFS-38 AC-8: the filter line stays one row.
// Run: node --test apps/desktop/ui-dev/layout.test.mjs   (needs Google Chrome or Chromium)

import test from 'node:test';
import assert from 'node:assert/strict';

import { startBrowser } from './browser.mjs';
import { CATALOGS, Lang } from '../ui/strings.js';
import { FileKind } from '../ui/contract.js';
import { DateChoice, ScopeChoice, SizeChoice, SourceChoice } from '../ui/choices.js';

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

let page;

test.before(async () => {
  page = await startBrowser();
});

test.after(() => page?.close());

const evaluate = (expression) => page.evaluate(expression);
const until = (expression, what) => page.until(expression, what);

/** Loads the page at a viewport and text size, with extra URL options (`&scenario=…`). */
async function open(viewport, text, options = '') {
  await page.send('Emulation.setDeviceMetricsOverride', {
    width: viewport.width, height: viewport.height, deviceScaleFactor: viewport.scale, mobile: false,
  });
  await page.load(options);
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
const FILTER = '#f-scope';

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
        const tops = await evaluate(`[...document.querySelectorAll('#filters > .filter, #filters > .scope')].filter((el) => el.offsetParent !== null).map((el) => Math.round(el.getBoundingClientRect().top))`);
        assert.equal(new Set(tops).size, 1, `filter pills on one row ${JSON.stringify(tops)} (${where})`);
      }

      await evaluate(`document.getElementById('open-folders').click()`);
      await until(`document.getElementById('folders').open`, 'the folders panel');
      await assertVisible('#add-folder', `folders panel, ${where}`);
      await assertNoSideScroll(`folders panel, ${where}`);
    });

    /** VFS-34 AC-6, VFS-35 AC-5: in Ajustes › Avanzado, the file filter shows the five
     *  defaults (none a shortcut), its add field, add button and reset button fit and are
     *  not covered once scrolled to, and the size field is hidden while content search is
     *  off. */
    test(`file filter fits in the settings at ${where}`, async () => {
      await open(viewport, text);
      await evaluate(`document.getElementById('open-settings').click()`);
      await until(`document.getElementById('settings').open`, 'the settings panel');
      // The drawer slides in from 2rem to the right (160 ms): measure once it stops.
      await until(`document.getElementById('settings').getAnimations().every((a) => a.playState === 'finished')`, 'the slide-in');
      await evaluate(`document.querySelector('#settings .setting-advanced').open = true`);
      await until(`document.querySelectorAll('#file-filter-list li').length === 5`, 'the default list');
      const items = await evaluate(`[...document.querySelectorAll('#file-filter-list code')].map((c) => c.textContent)`);
      for (const p of items) assert.doesNotMatch(p, /lnk|url/i, 'no default hides a shortcut');
      for (const s of ['#file-filter-pattern', '#file-filter-add', '#file-filter-reset', '#file-filter-list li .icon-btn']) {
        await evaluate(`document.querySelector(${JSON.stringify(s)}).scrollIntoView({ block: 'center' })`);
        await assertVisible(s, `settings, ${where}`);
      }
      assert.equal(await evaluate(`document.getElementById('content-max-row').offsetParent === null`), true, 'size field hidden');
      await assertNoSideScroll(`settings, ${where}`);
    });

    /** VFS-35 AC-4, AC-5: no folders yet, the empty state offers "Añadir carpeta…". */
    test(`first-run add-folder button fits at ${where}`, async () => {
      await open(viewport, text, '&scenario=empty');
      await until(`document.querySelector('#empty .actions .btn-primary')`, 'the empty state');
      await assertVisible('#empty .actions .btn-primary', `no folders, ${where}`);
      for (const s of FOOTER) await assertVisible(s, `no folders, ${where}`);
      await assertNoSideScroll(`no folders, ${where}`);
    });
  }
}

/* ---------------------------------------------------------------- VFS-38 AC-8 */

/** The widest label of a set of choices, to test the line at its widest. */
function widest(labels, keys) {
  return keys.reduce((a, b) => (labels[b].length > labels[a].length ? b : a));
}

/** The filter states of AC-8 for one language: the type set whose button reads longest, the
 *  longest date and size choices, a folder, files only (so size applies) and «Solo nombres». */
function filterStates(S) {
  const typeSets = [[FileKind.DOCUMENT, FileKind.FOLDER], [FileKind.DOCUMENT, FileKind.PHOTO, FileKind.VIDEO], [FileKind.VIDEO, FileKind.FOLDER]];
  const types = typeSets.reduce((a, b) => (S.typeButton(...b.map((k) => S.kindsPlural[k])).length > S.typeButton(...a.map((k) => S.kindsPlural[k])).length ? b : a));
  const date = widest(S.dateOptions, [DateChoice.WEEK, DateChoice.MONTH, DateChoice.YEAR]);
  const size = widest(S.sizeOptions, [SizeChoice.SMALL, SizeChoice.MEDIUM, SizeChoice.LARGE, SizeChoice.HUGE]);
  const some = { types, date, scope: ScopeChoice.NAMES };
  return [
    { name: 'no filters', types: [], date: DateChoice.ANY, scope: ScopeChoice.ALL, size: SizeChoice.ANY, folder: '', source: SourceChoice.BOTH },
    { name: 'type, date and names only', ...some, size: SizeChoice.ANY, folder: '', source: SourceChoice.BOTH },
    { name: 'every filter', ...some, size, folder: 'D:\\Archivo familiar', source: SourceChoice.FILES },
  ];
}

/** Opens the results view with bookmarks on and a third scope button, as VFS-27 27c will add. */
async function openFilterLine(viewport, text, lang) {
  await open(viewport, text, lang === Lang.EN ? '&bookmarks&locale=en-US' : '&bookmarks');
  await until(`[...document.getElementById('f-folder').options].some((o) => o.value === 'D:\\\\Archivo familiar')
    && !document.getElementById('f-source-field').hidden`, 'roots and bookmarks');
  const label = JSON.stringify(CATALOGS[lang].scopeOptions[ScopeChoice.DOCUMENTS]);
  await evaluate(`(() => {
    const b = document.createElement('button');
    b.type = 'button'; b.className = 'scope-btn'; b.textContent = ${label};
    b.setAttribute('role', 'radio'); b.setAttribute('aria-checked', 'false'); b.tabIndex = -1;
    b.dataset.scope = ${JSON.stringify(ScopeChoice.DOCUMENTS)};
    document.getElementById('f-scope').append(b);
  })()`);
  await page.type('a');
  await until(`document.getElementById('app').dataset.view === 'list'`, 'results view');
}

async function applyState(state) {
  await page.scope(state.scope);
  await page.kinds(state.types);
  await page.choose('f-date', state.date);
  await page.choose('f-source', state.source);
  await page.choose('f-size', state.size);
  await page.choose('f-folder', state.folder);
  await until(`document.querySelectorAll('#results .row').length > 0 || !document.getElementById('empty').hidden`, 'the search');
  await evaluate(`new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))`);
}

/** The filter line's controls and how they sit: their tops, the line's width, the room. */
const LINE = `(() => {
  const form = document.getElementById('filters');
  const controls = ['f-scope', 'f-type', 'f-date', 'f-more'].map((id) => document.getElementById(id))
    .map((el) => (el.tagName === 'SELECT' ? el.parentElement : el)).filter((el) => el.offsetParent !== null);
  const rects = controls.map((el) => el.getBoundingClientRect());
  const style = getComputedStyle(form);
  const room = form.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
  return {
    tops: rects.map((r) => Math.round(r.top)),
    width: Math.round(rects.at(-1).right - rects[0].left),
    room: Math.round(room),
    fits: rects.at(-1).right <= form.getBoundingClientRect().right - parseFloat(style.paddingRight) + 0.5,
    scrolls: form.scrollWidth > form.clientWidth,
    label: getComputedStyle(document.getElementById('f-scope-label')).position !== 'absolute',
    measure: parseFloat(getComputedStyle(document.documentElement).fontSize) * 60,
  };
})()`;

const LINE_VIEWPORTS = [maximised(1280, 720, 1.25), maximised(1366, 768, 1.25), maximised(1920, 1080, 1)];

for (const viewport of LINE_VIEWPORTS) {
  for (const lang of [Lang.ES, Lang.EN]) {
    const where = `${viewport.name}, ${viewport.width}x${viewport.height}, normal text, ${lang}`;

    /** VFS-38 AC-8 (R9, R10): with bookmarks on and three scope buttons, the filter line is one
     *  row inside the 60rem column with no sideways scroll in every filter state, and «Orden»
     *  sits in the results header. */
    test(`filter line is one row and «Orden» is in the results header at ${where}`, async (t) => {
      await openFilterLine(viewport, TEXT[0], lang);
      for (const state of filterStates(CATALOGS[lang])) {
        await applyState(state);
        const line = await evaluate(LINE);
        t.diagnostic(`${state.name}: line ${line.width}px of ${line.room}px (measure ${line.measure}px), «${CATALOGS[lang].fScope}» shown: ${line.label}`);
        assert.equal(new Set(line.tops).size, 1, `one row ${JSON.stringify(line.tops)} (${state.name}, ${where})`);
        assert.ok(line.fits && !line.scrolls, `inside the line, no sideways scroll: ${line.width}px of ${line.room}px (${state.name}, ${where})`);
        assert.ok(line.room <= line.measure + 0.5, `the line keeps to the 60rem column (${where})`);
        if (await evaluate(`document.querySelectorAll('#results .row').length > 0`)) {
          assert.ok(await evaluate(`!!document.getElementById('f-sort-pill').closest('.results-head')`), `«Orden» in the results header (${where})`);
          await assertVisible('#f-sort-pill', `${state.name}, ${where}`);
          await assertVisible('#summary', `${state.name}, ${where}`);
        }
        await assertNoSideScroll(`${state.name}, ${where}`);
      }
    });
  }
}

/** VFS-38 AC-8 (R10, VFS-35 R5): at the minimum window and with large text, every filter set,
 *  bookmarks on and three scope buttons, nothing is wider than the window. */
for (const [viewport, text] of [[VIEWPORTS[6], TEXT[0]], [VIEWPORTS[6], TEXT[1]], [VIEWPORTS[1], TEXT[1]], [VIEWPORTS[4], TEXT[1]]]) {
  const where = `${viewport.name}, ${viewport.width}x${viewport.height}, ${text.name}`;
  test(`every filter set does not widen the page at ${where}`, async () => {
    await openFilterLine(viewport, text, Lang.ES);
    await applyState(filterStates(CATALOGS[Lang.ES]).at(-1));
    await assertVisible(FILTER, where);
    await assertVisible('#f-clear', where);
    await assertNoSideScroll(where);
  });
}
