// The bookmark source on the screen (VFS-6 AC-11, AC-13): which sources each "Buscar en"
// choice covers, bookmark folders in the "Carpeta" select, the folder path of a bookmark row,
// and Spanish copy for every new closed-set value.
// Run: node --test apps/desktop/ui-dev/sources.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { S } from '../ui/strings.js';
import { BookmarkRoot, ReadState, SortOrder, Source } from '../ui/contract.js';
import { SourceChoice } from '../ui/choices.js';
import {
  PATH_SEPARATOR, bookmarkFolderOf, bookmarkFolderValue, bookmarkPathParts, site, sizeFilterApplies, sourcesFor,
} from '../ui/sources.js';

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
  const label = S.bookmarkRowLabel('Flan', S.bookmarkKind, 'Menú de marcadores', 'cocina.es', '1 mar 2020');
  assert.match(label, /cocina\.es/);
  assert.match(label, /guardado el 1 mar 2020/);
});

/** VFS-6 AC-11: a bookmark row shows the site of its address. */
test('site of a bookmark address', () => {
  assert.equal(site('https://www.cocina.es/flan?x=1'), 'cocina.es');
  assert.equal(site('http://user:pw@Ejemplo.es:8080/a'), 'Ejemplo.es');
  assert.equal(site('HTTPS://WWW.banco.es'), 'banco.es');
  for (const none of ['file:///C:/a.pdf', 'data:text/html,hola', '', null]) assert.equal(site(none), '', String(none));
});

/** VFS-6 AC-11: every choice, root, read state and sort order has Spanish copy; "Buscar en"
 *  and "Orden" start hidden (shown only while the setting is on). */
test('copy and markup for the bookmark source', () => {
  assert.deepEqual(Object.keys(S.sourceOptions).sort(), Object.values(SourceChoice).sort());
  assert.deepEqual(Object.keys(S.sortOptions).sort(), Object.values(SortOrder).sort());
  for (const r of Object.values(BookmarkRoot)) assert.ok(S.bookmarkRoots[r], r);
  assert.deepEqual(Object.keys(S.bookmarksState).sort(), Object.values(ReadState).sort());
  assert.match(html, /<label class="filter filter-select" id="f-source-pill" hidden>/);
  assert.match(html, /<label class="filter filter-select" id="f-sort-pill" hidden>/);
  assert.match(html, /id="bookmarks-on"/);
  for (const key of ['bookmarksToggle', 'folderContentsButton', 'bookmarkKind', 'bookmarkFolderKind', 'fSource', 'fSort']) {
    assert.equal(typeof S[key], 'string', key);
  }
});
