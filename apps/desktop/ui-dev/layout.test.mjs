// VFS-35: the window fits small and low-resolution screens. The real page runs in a headless
// Chrome against the development fake backend, at the content size of a maximised window on
// common laptop screens, at normal and large text. Chrome is driven over the DevTools
// protocol (browser.mjs). VFS-38 AC-8: the filter line stays one row. VFS-42 AC-7: the results
// tabs stay one row, with the count and «Orden» beside them or wrapped below.
// Run: node --test apps/desktop/ui-dev/layout.test.mjs   (needs Google Chrome or Chromium)

import test from 'node:test';
import assert from 'node:assert/strict';

import { startBrowser } from './browser.mjs';
import { CATALOGS, Lang } from '../ui/strings.js';
import { FileKind } from '../ui/contract.js';
import { DateChoice, ResultsTab, SizeChoice, SourceChoice } from '../ui/choices.js';

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
        const tops = await evaluate(`[...document.querySelectorAll('#filters > .filter')].filter((el) => el.offsetParent !== null).map((el) => Math.round(el.getBoundingClientRect().top))`);
        assert.equal(new Set(tops).size, 1, `filter pills on one row ${JSON.stringify(tops)} (${where})`);
      }

      await evaluate(`document.getElementById('open-folders').click()`);
      await until(`document.getElementById('folders').open`, 'the folders panel');
      await assertVisible('#add-folder', `folders panel, ${where}`);
      await assertNoSideScroll(`folders panel, ${where}`);
    });

    /** VFS-35 R1, VFS-27 R8, R12: while documents are being read (bookmarks on, results
     *  shown), the bottom bar keeps every button on one row: on a short window from 56 rem
     *  wide at normal text the whole bar is one row, the reading progress cut to one line
     *  with its whole text as tooltip (it wrapped to three lines and pushed «Ajustes» onto a
     *  row of its own). Then at 1280×720 at 125 % (1024×496 CSS px) «Documentos» shows its
     *  first whole hit (VFS-42 R3). */
    test(`bottom bar one row while reading documents at ${where}`, async () => {
      for (const locale of ['', '&locale=en-US']) {
        await open(viewport, text, `&bookmarks&content=on${locale}`);
        await evaluate(`(() => { const q = document.getElementById('q'); q.value = 'foto'; q.dispatchEvent(new Event('input', { bubbles: true })); })()`);
        await until(`document.querySelectorAll('#results .row').length > 0`, 'name hits');
        await page.tab(ResultsTab.DOCUMENTS); // VFS-42 R3: content hits only on «Documentos»
        await until(`document.querySelectorAll('#content-list .chit').length > 0 && !document.getElementById('content-line').hidden`, 'content hits');
        const bar = await evaluate(`(() => {
          const shown = (el) => el.offsetParent !== null;
          const mid = (el) => { const r = el.getBoundingClientRect(); return Math.round(r.top + r.height / 2); };
          const buttons = [...document.querySelectorAll('.bottom .btn')].filter(shown);
          const all = [...document.querySelector('.bottom').children].filter(shown);
          const line = document.getElementById('content-line');
          return { buttons: buttons.map(mid), all: all.map(mid), title: line.title, textContent: line.textContent };
        })()`);
        const at = `${where}${locale}`;
        // The minimum window at large text has no room for five buttons in a row (VFS-35).
        if (viewport.width > 720 || text.root === '100%') {
          assert.ok(Math.max(...bar.buttons) - Math.min(...bar.buttons) <= 4, `every button on one row ${JSON.stringify(bar)} (${at})`);
        }
        const short = await evaluate(`document.getElementById('app').clientHeight < 40 * parseFloat(getComputedStyle(document.documentElement).fontSize)`);
        const wide = viewport.width >= 56 * 16 * (text.root === '100%' ? 1 : 1.25);
        if (short && wide && text.root === '100%') {
          assert.ok(Math.max(...bar.all) - Math.min(...bar.all) <= 4, `the whole bar on one row ${JSON.stringify(bar)} (${at})`);
          assert.equal(bar.title, bar.textContent, `the whole progress text is its tooltip (${at})`);
        }
        if (viewport.width === 1024 && viewport.height === 496 && text.root === '100%') {
          const hit = await evaluate(`(() => {
            const sec = document.getElementById('content-hits').getBoundingClientRect();
            const first = document.querySelector('#content-list .chit').getBoundingClientRect();
            const bottom = Math.min(sec.bottom, document.querySelector('.bottom').getBoundingClientRect().top);
            return { top: first.top, bottom: first.bottom, visibleTo: bottom, secTop: sec.top };
          })()`);
          assert.ok(hit.top >= hit.secTop && hit.bottom <= hit.visibleTo + 0.5, `the first content hit is whole in sight ${JSON.stringify(hit)} (${at})`);
        }
      }
    });

    /** VFS-35 R4, VFS-6: with bookmarks on, the folders panel keeps every folder reachable:
     *  the list scrolls on its own between the intro and the folded «Marcadores de Firefox»
     *  line, so its first and last folders, the bookmarks line and «Añadir carpeta…» are all
     *  in sight without scrolling the panel (the bookmarks section took the height before). */
    test(`folder list reachable with bookmarks on at ${where}`, async () => {
      await open(viewport, text, '&bookmarks');
      await evaluate(`document.getElementById('open-folders').click()`);
      await until(`document.getElementById('folders').open && document.querySelectorAll('#roots .root').length >= 3`, 'the folders panel');
      await until(`document.getElementById('folders').getAnimations().every((a) => a.playState !== 'running')`, 'the panel slid in');
      for (const s of ['#roots .root:first-child .root-path', '#bookmarks-setting summary', '#add-folder']) await assertVisible(s, `folders panel, ${where}`);
      const panel = await evaluate(`(() => { const d = document.getElementById('folders'); return [d.scrollHeight, d.clientHeight]; })()`);
      assert.ok(panel[0] <= panel[1] + 1, `the panel itself does not scroll ${JSON.stringify(panel)} (${where})`);
      await evaluate(`document.querySelector('#roots .root:last-child').scrollIntoView({ block: 'nearest' })`);
      await assertVisible('#roots .root:last-child .root-path', `last folder, ${where}`);
      await evaluate(`document.querySelector('#roots .root:last-child .root-actions .btn').scrollIntoView({ block: 'nearest' })`);
      await assertVisible('#roots .root:last-child .root-actions .btn', `last folder's actions, ${where}`);
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
 *  longest date and size choices, a folder and files only (so size applies). */
function filterStates(S) {
  const typeSets = [[FileKind.DOCUMENT, FileKind.FOLDER], [FileKind.DOCUMENT, FileKind.PHOTO, FileKind.VIDEO], [FileKind.VIDEO, FileKind.FOLDER]];
  const types = typeSets.reduce((a, b) => (S.typeButton(...b.map((k) => S.kindsPlural[k])).length > S.typeButton(...a.map((k) => S.kindsPlural[k])).length ? b : a));
  const date = widest(S.dateOptions, [DateChoice.WEEK, DateChoice.MONTH, DateChoice.YEAR]);
  const size = widest(S.sizeOptions, [SizeChoice.SMALL, SizeChoice.MEDIUM, SizeChoice.LARGE, SizeChoice.HUGE]);
  const some = { types, date };
  return [
    { name: 'no filters', types: [], date: DateChoice.ANY, size: SizeChoice.ANY, folder: '', source: SourceChoice.BOTH },
    { name: 'type and date', ...some, size: SizeChoice.ANY, folder: '', source: SourceChoice.BOTH },
    { name: 'every filter', ...some, size, folder: 'D:\\Archivo familiar', source: SourceChoice.FILES },
  ];
}

/** Opens the results view with bookmarks on and content search on. */
async function openFilterLine(viewport, text, lang) {
  await open(viewport, text, `${lang === Lang.EN ? '&bookmarks&locale=en-US' : '&bookmarks'}&content=on`);
  await until(`[...document.getElementById('f-folder').options].some((o) => o.value === 'D:\\\\Archivo familiar')
    && !document.getElementById('f-source-field').hidden
    && document.getElementById('content-on').checked`, 'roots, bookmarks and the content status');
  await page.type('a');
  await until(`document.getElementById('app').dataset.view === 'list'`, 'results view');
}

async function applyState(state) {
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
  const controls = ['f-type', 'f-date', 'f-more'].map((id) => document.getElementById(id))
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
    measure: parseFloat(getComputedStyle(document.documentElement).fontSize) * 60,
  };
})()`;

const LINE_VIEWPORTS = [maximised(1280, 720, 1.25), maximised(1366, 768, 1.25), maximised(1920, 1080, 1)];

for (const viewport of LINE_VIEWPORTS) {
  for (const lang of [Lang.ES, Lang.EN]) {
    const where = `${viewport.name}, ${viewport.width}x${viewport.height}, normal text, ${lang}`;

    /** VFS-38 AC-8 (R9, R10): with bookmarks on and content search on, the filter line is one
     *  row inside the 60rem column with no sideways scroll in every filter state, and «Orden»
     *  sits in the results header. */
    test(`filter line is one row and «Orden» is in the results header at ${where}`, async (t) => {
      await openFilterLine(viewport, TEXT[0], lang);
      for (const state of filterStates(CATALOGS[lang])) {
        await applyState(state);
        const line = await evaluate(LINE);
        t.diagnostic(`${state.name}: line ${line.width}px of ${line.room}px (measure ${line.measure}px)`);
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

/** VFS-35 R5: the bottom bar wraps rather than widening the page when its buttons need more
 *  room than the row has, as with large text and wider system fonts (CI's macOS Chrome drew
 *  this bar 55 px wider than the window). A button made wider stands in for those fonts. */
test('a bottom bar wider than its row wraps instead of widening the page', async () => {
  const viewport = VIEWPORTS[1];
  await openFilterLine(viewport, TEXT[1], Lang.ES);
  await evaluate(`document.getElementById('open-settings').style.paddingInline = '14rem'`);
  await assertNoSideScroll(`${viewport.name}, large text, a wide button`);
  await evaluate(`document.getElementById('open-settings').style.paddingInline = ''`);
});

/** VFS-38 AC-8 (R10, VFS-35 R5): at the minimum window and with large text, every filter set,
 *  bookmarks and content search on, nothing is wider than the window. */
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

/* ---------------------------------------------------------------- VFS-42 AC-7 */

/** How the results header sits: the tabs' boxes, whether each tab shows its whole word, and
 *  where the count and «Orden» are. */
const TAB_ROW = `(() => {
  const tabs = [...document.querySelectorAll('#tabs [role="tab"]')].map((t) => {
    const r = t.getBoundingClientRect();
    return { top: Math.round(r.top), left: r.left, right: r.right, whole: t.scrollWidth <= t.clientWidth + 0.5 && t.scrollHeight <= t.clientHeight + 0.5 };
  });
  const meta = document.querySelector('.results-meta').getBoundingClientRect();
  return { tabs, metaTop: Math.round(meta.top), metaLeft: meta.left, tabsBottom: Math.round(document.getElementById('tabs').getBoundingClientRect().bottom), w: innerWidth,
    list: getComputedStyle(document.getElementById('tabs')).overflowX };
})()`;

/** VFS-42 AC-7: the tabs are one row inside the window, each showing its whole word; the count
 *  (and «Orden» when `sort`) are visible beside them or on the line below; nothing scrolls
 *  sideways. */
async function assertTabRow(where, sort) {
  const row = await evaluate(TAB_ROW);
  assert.equal(new Set(row.tabs.map((t) => t.top)).size, 1, `the tabs on one row ${JSON.stringify(row)} (${where})`);
  assert.ok(row.tabs.every((t) => t.left >= 0 && t.right <= row.w + 0.5 && t.whole), `the tabs whole and inside the window ${JSON.stringify(row)} (${where})`);
  assert.equal(row.list, 'visible', `the tabs do not scroll (${where})`);
  const beside = row.metaTop < row.tabsBottom - 1;
  assert.ok(beside ? row.metaLeft >= row.tabs.at(-1).right : row.metaTop >= row.tabsBottom - 1, `the count beside the tabs or below them ${JSON.stringify(row)} (${where})`);
  await assertVisible('#summary', where);
  if (sort) await assertVisible('#f-sort-pill', where);
  else assert.equal(await evaluate(`document.getElementById('f-sort-pill').offsetParent`), null, `no «Orden» (${where})`);
  await assertNoSideScroll(where);
}

const TAB_VIEWPORTS = [maximised(1366, 768, 1.25), maximised(1280, 720, 1.25), maximised(1920, 1080, 1), VIEWPORTS[6]];

for (const viewport of TAB_VIEWPORTS) {
  for (const text of TEXT) {
    for (const lang of [Lang.ES, Lang.EN]) {
      const where = `${viewport.name}, ${viewport.width}x${viewport.height}, ${text.name}, ${lang}`;

      /** VFS-42 AC-7 (R7, R8): with bookmarks on, on «Nombres» and «Documentos», the tab row
       *  holds as assertTabRow says. */
      test(`the tab row fits at ${where}`, async () => {
        await openFilterLine(viewport, text, lang);
        await page.type('foto');
        await until(`document.querySelectorAll('#results .row').length > 0 && document.getElementById('summary').textContent !== ''`, 'name hits');
        await assertTabRow(`«Nombres», ${where}`, true);
        await page.tab(ResultsTab.DOCUMENTS);
        await until(`document.querySelectorAll('#content-list .chit').length > 0 && document.getElementById('summary').textContent !== ''`, 'content hits');
        await assertTabRow(`«Documentos», ${where}`, false);
      });
    }
  }
}
