// The one Spanish screen (VFS-1 AC-12), checked without a browser: the copy in strings.js
// and the markup of index.html. Run: node --test apps/desktop/ui-dev/screen.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { S } from '../ui/strings.js';
import { FileKind, Source } from '../ui/contract.js';
import { DateChoice, SizeChoice } from '../ui/choices.js';
import { placesOf } from '../ui/copies.js';

const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');

/** Every user-visible string in `S`: nested tables walked, functions called with samples. */
function allStrings(value, out = []) {
  if (typeof value === 'string') out.push(value);
  else if (typeof value === 'function') allStrings(value('1', '1', '/a', '1 mar', '1 KB', true), out);
  else if (value && typeof value === 'object') for (const v of Object.values(value)) allStrings(v, out);
  return out;
}

/** VFS-1 AC-12, AC-11: every type chip and every filter has a Spanish label. */
test('every type chip and filter option has a label', () => {
  for (const kind of Object.values(FileKind)) {
    assert.ok(S.kindsPlural[kind], `chip label for ${kind}`);
    assert.ok(S.kindSingular[kind], `row label for ${kind}`);
  }
  assert.deepEqual(Object.keys(S.dateOptions).sort(), Object.values(DateChoice).sort());
  assert.deepEqual(Object.keys(S.sizeOptions).sort(), Object.values(SizeChoice).sort());
  for (const key of ['fType', 'fDate', 'fSize', 'fFolder', 'fFrom', 'fTo', 'folderAll', 'folderPick']) {
    assert.ok(S[key], key);
  }
});

/** VFS-1 AC-12, decision 16: no user-facing text says "indexar", "índice" or "reindexar". */
test('plain words only', () => {
  const texts = allStrings(S);
  assert.ok(texts.length > 50);
  for (const s of texts) assert.doesNotMatch(s, /index|índice|indice/i, s);
});

/** VFS-1 AC-12: the page is Spanish, every text slot it names exists in strings.js, and it is
 *  one screen with one search box. */
test('one Spanish screen with one search box', () => {
  assert.match(html, /<html lang="es">/);
  const slots = [...html.matchAll(/data-s(?:-label)?="([A-Za-z]+)"/g)].map((m) => m[1]);
  assert.ok(slots.length > 10);
  for (const key of slots) assert.equal(typeof S[key], 'string', `strings.js has ${key}`);
  assert.equal([...html.matchAll(/type="search"/g)].length, 1);
  assert.match(html, /<input id="q" type="search"[^>]*maxlength="\d+"/s);
  for (const id of ['kind-chips', 'f-date', 'f-size', 'f-folder']) assert.match(html, new RegExp(`id="${id}"`));
});

/** VFS-1 AC-11: the type filter names what it narrows to, so a chosen filter is visible
 *  without opening it. */
test('type button names the chosen types', () => {
  assert.equal(S.typeButton(), 'Todos los tipos');
  assert.equal(S.typeButton('Fotos'), 'Fotos');
  assert.equal(S.typeButton('Fotos', 'Vídeos'), 'Fotos, Vídeos');
  assert.equal(S.typeButton('Documentos', 'Fotos', 'Vídeos'), 'Documentos y 2 más');
});

/** VFS-1 AC-11, AC-12: the type checklist is a popover opened by its own button (keyboard and
 *  light dismiss come from the platform), and the chips live inside it. */
test('type checklist is a popover owned by the type button', () => {
  assert.match(html, /<button id="f-type"[^>]*popovertarget="f-type-pop"/);
  assert.match(html, /<div id="f-type-pop"[^>]*popover>\s*<fieldset[^>]*>[\s\S]*?id="kind-chips"[\s\S]*?<\/fieldset>\s*<\/div>/);
});

/** VFS-19 AC-5: a result with copies stands for every place, itself first; folders,
 *  bookmarks and results without copies stand for one place or none. */
test('a result with copies lists every place', () => {
  const hit = { id: 1, dir: '/a', modified: 3, isDir: false, copies: [{ id: 2, dir: '/b', modified: 2 }] };
  assert.deepEqual(placesOf(hit), [{ id: 1, dir: '/a', modified: 3 }, { id: 2, dir: '/b', modified: 2 }]);
  assert.equal(placesOf({ id: 1, dir: '/a', modified: 3, isDir: false }).length, 1);
  assert.deepEqual(placesOf({ id: 1, dir: '/a', isDir: true, copies: [] }), []);
  assert.deepEqual(placesOf({ id: 1, source: Source.BOOKMARKS }), []);
  assert.deepEqual(placesOf(undefined), []);
});

/** VFS-19 AC-5: the "en N sitios" mark, the row label and the list of places are in Spanish,
 *  and the list is a labelled dialog. */
test('copies have their Spanish copy and dialog', () => {
  assert.equal(S.copiesBadge(3), 'en 3 sitios');
  assert.match(S.copiesTitle('Boda.mp4', 3), /Boda\.mp4.*3 sitios/);
  assert.match(S.rowLabel('a', 'Vídeo', '/d', '1 mar', '1 KB', false, 3), /está en 3 sitios/);
  assert.doesNotMatch(S.rowLabel('a', 'Vídeo', '/d', '1 mar', '1 KB', false, 1), /sitios/);
  assert.ok(S.copiesIntro && S.copiesBadgeLabel(2) && S.copiesModified('1 mar'));
  assert.match(html, /<dialog id="copies"[^>]*aria-labelledby="copies-title"/);
  assert.match(html, /<ul class="roots" id="copies-list"/);
});
