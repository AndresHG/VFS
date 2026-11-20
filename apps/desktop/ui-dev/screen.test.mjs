// The one Spanish screen (VFS-1 AC-12), checked without a browser: the copy in strings.js
// and the markup of index.html. Run: node --test apps/desktop/ui-dev/screen.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { S } from '../ui/strings.js';
import { FileKind } from '../ui/contract.js';
import { DateChoice, SizeChoice } from '../ui/choices.js';

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
