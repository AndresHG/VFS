// The bookmark source on the screen (VFS-6 AC-11): which sources each "Buscar en" choice
// covers, the bookmark row text, and that every new closed-set value has Spanish copy.
// Run: node --test apps/desktop/ui-dev/sources.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { S } from '../ui/strings.js';
import { BookmarkRoot, ReadState, Source } from '../ui/contract.js';
import { SourceChoice } from '../ui/choices.js';
import { bookmarkPath, fileFiltersApply, site, sourcesFor } from '../ui/sources.js';

const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');

/** VFS-6 AC-11, R1, D7: while bookmarks are off every choice searches files only; "Todo"
 *  (the default) covers every source. */
test('each choice covers its sources', () => {
  for (const choice of Object.values(SourceChoice)) {
    assert.deepEqual(sourcesFor(choice, false), [Source.FILES], choice);
    assert.equal(fileFiltersApply(choice, false), true, choice);
  }
  assert.deepEqual(sourcesFor(SourceChoice.ALL, true), []);
  assert.deepEqual(sourcesFor(SourceChoice.FILES, true), [Source.FILES]);
  assert.deepEqual(sourcesFor(SourceChoice.BOOKMARKS, true), [Source.BOOKMARKS]);
  assert.equal(fileFiltersApply(SourceChoice.BOOKMARKS, true), false);
  assert.equal(fileFiltersApply(SourceChoice.ALL, true), true);
});

/** VFS-6 AC-11: a bookmark row shows its Firefox folder path (profile first when several)
 *  and its site. */
test('bookmark row text', () => {
  const hit = { root: BookmarkRoot.MENU, folder: ['Recetas', 'Postres'], profile: null };
  assert.equal(bookmarkPath(hit, S.bookmarkRoots), 'Menú de marcadores › Recetas › Postres');
  assert.equal(bookmarkPath({ ...hit, profile: 'Casa' }, S.bookmarkRoots), 'Casa › Menú de marcadores › Recetas › Postres');
  assert.equal(site('https://www.cocina.es/flan?x=1'), 'cocina.es');
  assert.equal(site('http://user:pw@ejemplo.es:8080/a'), 'ejemplo.es:8080');
  assert.equal(site('file:///C:/a.pdf'), '');
  assert.match(S.bookmarkRowLabel('Flan', 'Menú de marcadores', 'cocina.es', '1 mar 2020'), /guardado el 1 mar 2020/);
});

/** VFS-6 AC-11: every source choice, bookmark folder and read state has Spanish copy, and the
 *  "Buscar en" control starts hidden (shown only while the setting is on). */
test('copy and markup for the bookmark source', () => {
  assert.deepEqual(Object.keys(S.sourceOptions).sort(), Object.values(SourceChoice).sort());
  for (const r of Object.values(BookmarkRoot)) assert.ok(S.bookmarkRoots[r], r);
  assert.deepEqual(Object.keys(S.bookmarksState).sort(), Object.values(ReadState).sort());
  assert.match(html, /<fieldset class="sources" id="sources" hidden>/);
  assert.match(html, /id="bookmarks-on"/);
  for (const key of ['bookmarksToggle', 'openButton', 'bookmarkKind', 'fSource']) assert.equal(typeof S[key], 'string', key);
});
