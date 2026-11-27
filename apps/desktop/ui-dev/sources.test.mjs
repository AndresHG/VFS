// The bookmark source on the screen (VFS-6 AC-11, AC-13; VFS-44 AC-6): which sources each «Origen»
// choice covers, bookmark folders in the "Carpeta" select, the folder path of a bookmark row,
// and Spanish copy for every new closed-set value.
// Run: node --test apps/desktop/ui-dev/sources.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { CATALOGS, Lang } from '../ui/strings.js';
import { BookmarkRoot, Browser, ReadState, SortOrder, Source } from '../ui/contract.js';
import { SourceChoice } from '../ui/choices.js';
import {
  PATH_SEPARATOR, bookmarkFolderOf, bookmarkFolderValue, bookmarkPathParts, namesBrowsers, site, sizeFilterApplies, sortChoices, sortFor,
  sourcesFor,
} from '../ui/sources.js';

const S = CATALOGS[Lang.ES];

const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');

/** VFS-6 AC-11, R1, D7: while bookmarks are off every choice searches files only; both
 *  sources (the default) is the empty list. */
test('each choice covers its sources', () => {
  for (const choice of Object.values(SourceChoice)) {
    assert.deepEqual(sourcesFor(choice, false), [Source.FILES], choice);
    assert.equal(sizeFilterApplies(choice, false), true, choice);
  }
  assert.deepEqual(sourcesFor(SourceChoice.BOTH, true), []);
  assert.deepEqual(sourcesFor(SourceChoice.FILES, true), [Source.FILES]);
  assert.deepEqual(sourcesFor(SourceChoice.BOOKMARKS, true), [Source.BOOKMARKS]);
  assert.equal(sizeFilterApplies(SourceChoice.BOOKMARKS, true), false);
});

/** VFS-6 AC-13 (R12): a bookmark folder in the "Carpeta" select is told from a path. */
test('bookmark folders in the folder select', () => {
  assert.equal(bookmarkFolderOf(bookmarkFolderValue(42)), 42);
  for (const path of ['', 'C:\\Users\\Carmen', '/Users/carmen/Documentos', null]) {
    assert.equal(bookmarkFolderOf(path), null, String(path));
  }
});

/** VFS-6 AC-11, AC-13: a bookmark row's path is the Spanish root name and its folders, led by
 *  the profile name when there are several. */
test('bookmark row path', () => {
  const hit = { root: BookmarkRoot.MENU, folder: ['Recetas', 'Postres'], profile: null };
  assert.deepEqual(bookmarkPathParts(hit, S.bookmarkRoots), ['Menú de marcadores', 'Recetas', 'Postres']);
  assert.match(PATH_SEPARATOR, /›/);
  assert.deepEqual(bookmarkPathParts({ ...hit, profile: 'Casa' }, S.bookmarkRoots).slice(0, 2), ['Casa', 'Menú de marcadores']);
  assert.deepEqual(bookmarkPathParts({ ...hit, profile: 'Casa' }, S.bookmarkRoots, 'Chrome').slice(0, 3), ['Chrome', 'Casa', 'Menú de marcadores']);
  const label = S.bookmarkRowLabel('Flan', S.bookmarkKind, 'Menú de marcadores', 'cocina.es', '1 mar 2020');
  assert.match(label, /cocina\.es/);
  assert.match(label, /guardado el 1 mar 2020/);
});

/** VFS-44 AC-6 (R7): browsers are named only while more than one has bookmarks stored. */
test('browsers are named while several have bookmarks', () => {
  const b = (browser, count) => ({ browser, enabled: true, state: ReadState.OK, count, profiles: 1, lastRead: 1 });
  assert.equal(namesBrowsers(null), false);
  assert.equal(namesBrowsers({ browsers: [b(Browser.FIREFOX, 3), b(Browser.CHROME, 0)] }), false);
  assert.equal(namesBrowsers({ browsers: [b(Browser.FIREFOX, 3), b(Browser.CHROME, 2)] }), true);
  for (const x of Object.values(Browser)) {
    assert.ok(S.browserNames[x], x);
    assert.match(S.bookmarksToggle(S.browserNames[x]), new RegExp(S.browserNames[x]));
    assert.match(S.openBookmarkButton(S.browserNames[x]), new RegExp(S.browserNames[x]));
  }
});

/** VFS-6 AC-11: a bookmark row shows the site of its address. */
test('site of a bookmark address', () => {
  assert.equal(site('https://www.cocina.es/flan?x=1'), 'cocina.es');
  assert.equal(site('http://user:pw@Ejemplo.es:8080/a'), 'Ejemplo.es');
  assert.equal(site('HTTPS://WWW.banco.es'), 'banco.es');
  for (const none of ['file:///C:/a.pdf', 'data:text/html,hola', '', null]) assert.equal(site(none), '', String(none));
});

/** VFS-6 AC-11: every choice, root, read state and sort order has Spanish copy; the source
 *  choice («Origen» in «Más filtros», VFS-38 R7) starts hidden (shown only while the setting is
 *  on). VFS-38 R9: "Orden" sits in the results header. */
test('copy and markup for the bookmark source', () => {
  assert.deepEqual(Object.keys(S.sourceOptions).sort(), Object.values(SourceChoice).sort());
  assert.deepEqual(Object.keys(S.sortOptions).sort(), Object.values(SortOrder).sort());
  for (const r of Object.values(BookmarkRoot)) assert.ok(S.bookmarkRoots[r], r);
  assert.deepEqual(Object.keys(S.bookmarksState).sort(), Object.values(ReadState).sort());
  assert.match(html, /<div id="f-more-pop"[^>]*>[\s\S]*<div class="field" id="f-source-field" hidden>[\s\S]*?<\/div>\s*<\/div>/);
  assert.match(html, /<div class="results-head" id="results-head" hidden>[\s\S]*?<label class="filter filter-select" id="f-sort-pill">/);
  assert.match(html, /id="bookmarks-browsers"/);
  for (const key of ['folderContentsButton', 'bookmarkKind', 'bookmarkFolderKind', 'fSource', 'fSort']) {
    assert.equal(typeof S[key], 'string', key);
  }
});

/** VFS-12 AC-5: five orders always, in display order, plus "by source" while bookmarks are
 *  included; a choice no longer offered is sent as relevance. */
test('sort choices follow the bookmark setting', () => {
  const plain = [SortOrder.RELEVANCE, SortOrder.NEWEST, SortOrder.OLDEST, SortOrder.NAME, SortOrder.SIZE];
  assert.deepEqual(sortChoices(false), plain);
  assert.deepEqual(sortChoices(true), [...plain, SortOrder.SOURCE]);
  assert.deepEqual(Object.keys(S.sortOptions), [...plain, SortOrder.SOURCE]);
  assert.equal(sortFor(SortOrder.SOURCE, false), SortOrder.RELEVANCE);
  assert.equal(sortFor(SortOrder.SOURCE, true), SortOrder.SOURCE);
  assert.equal(sortFor(SortOrder.SIZE, false), SortOrder.SIZE);
  assert.match(S.resultsApproximate('2.000'), /ordenarlos todos/);
});
