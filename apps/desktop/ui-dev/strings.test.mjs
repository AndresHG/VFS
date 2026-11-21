// The string catalogs and the language choice (VFS-22). Run:
// node --test apps/desktop/ui-dev/strings.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

import { CATALOGS, Lang, intlLocale, languageFor, localeFor } from '../ui/strings.js';
import * as F from '../ui/format.js';

const UI = new URL('../ui/', import.meta.url).pathname;
const CATALOG_FILE = /^strings(\.\w+)?\.js$/;

/** Every leaf of a catalog as [dotted key, value]. */
function leaves(obj, prefix = '') {
  return Object.entries(obj).flatMap(([k, v]) => (v && typeof v === 'object'
    ? leaves(v, `${prefix}${k}.`)
    : [[`${prefix}${k}`, v]]));
}

/** VFS-22 AC-1: both catalogs have the same keys; a function takes the same number of
 *  arguments in both and returns text; a plain string is empty in both or in neither. */
test('es and en catalogs have the same keys and shapes', () => {
  const es = new Map(leaves(CATALOGS[Lang.ES]));
  const en = new Map(leaves(CATALOGS[Lang.EN]));
  assert.deepEqual([...en.keys()].sort(), [...es.keys()].sort());
  for (const [key, a] of es) {
    const b = en.get(key);
    assert.equal(typeof b, typeof a, key);
    if (typeof a === 'function') {
      assert.equal(b.length, a.length, `${key} arity`);
      const args = Array.from({ length: Math.max(a.length, 2) }, (_, i) => String(i + 2));
      for (const [lang, f] of [['es', a], ['en', b]]) {
        const out = f(...args);
        assert.ok(typeof out === 'string' && out.trim(), `${lang}.${key}(…) is text`);
      }
    } else {
      assert.equal(Boolean(b), Boolean(a), `${key} empty in one language only`);
    }
  }
});

/** Code with comments removed, so a Spanish comment is not taken for copy. */
function stripComments(src) {
  return src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|\s)\/\/.*$/gm, '$1');
}

/** VFS-22 AC-2: every key the UI names exists, and no visible text bypasses the catalogs. */
test('no UI string bypasses the catalog', () => {
  const keys = CATALOGS[Lang.ES];
  const scripts = readdirSync(UI).filter((f) => f.endsWith('.js') && !CATALOG_FILE.test(f));
  for (const f of scripts) {
    const src = stripComments(readFileSync(join(UI, f), 'utf8'));
    for (const [, key] of src.matchAll(/\bS\.(\w+)/g)) assert.ok(key in keys, `${f}: S.${key}`);
    // Copy is Spanish or English with typographic marks; code literals are plain ASCII.
    const literal = src.match(/(['"`])(?:(?!\1)[^\\\n]|\\.)*[À-ÿ¿¡«»“”‘’—…](?:(?!\1)[^\\\n]|\\.)*\1/);
    assert.ok(!literal, `${f} holds copy outside the catalogs: ${literal?.[0]}`);
  }

  const html = readFileSync(join(UI, 'index.html'), 'utf8');
  for (const [, key] of html.matchAll(/data-s(?:-label)?="(\w+)"/g)) assert.ok(key in keys, `index.html: ${key}`);
  const body = html
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<(script|style|svg)\b[\s\S]*?<\/\1>/g, '');
  const text = body.replace(/<[^>]*>/g, ' ').replace(/&\w+;/g, ' ').trim();
  assert.ok(!text, `index.html has text outside the catalogs: ${text.slice(0, 80)}`);
  const attr = body.match(/\s(?:placeholder|title|alt|aria-label)="[^"]*[A-Za-z][^"]*"/);
  assert.ok(!attr, `index.html has a copy attribute: ${attr?.[0]}`);
});

/** VFS-22 AC-3: with no saved choice, Spanish for es and es-*, English for anything else. */
test('the system locale picks the language', () => {
  for (const l of ['es-ES', 'es-MX', 'es', 'es_ES.UTF-8', 'ES-es']) assert.equal(languageFor(l, null), Lang.ES, l);
  for (const l of ['en-US', 'fr-FR', 'ca-ES', 'eu-ES', 'est', '', undefined]) assert.equal(languageFor(l, null), Lang.EN, String(l));
});

/** VFS-22 AC-4: a saved choice wins over the system; anything else saved means "system". */
test('a saved choice wins over the system locale', () => {
  assert.equal(languageFor('es-ES', 'en'), Lang.EN);
  assert.equal(languageFor('en-US', 'es'), Lang.ES);
  for (const saved of ['system', 'fr', 'toString', '', null]) assert.equal(languageFor('es-ES', saved), Lang.ES, String(saved));
});

/** VFS-22 AC-5: numbers and dates follow the language; English keeps the system's
 *  English order (en-GB day first), and a system in another language gets the default. */
test('numbers and dates follow the language', () => {
  const secs = new Date(2021, 2, 12, 9, 14).getTime() / 1000;

  const us = localeFor(Lang.EN, 'en-US');
  assert.equal(us.S.revealButton, 'Show in folder');
  assert.equal(us.N.count(1234567), '1,234,567');
  assert.equal(us.N.count(1234), '1,234');
  assert.equal(us.N.date(secs), 'Mar 12, 2021');
  assert.equal(us.N.size(1.5 * 1024 * 1024), '1.5 MB');

  assert.equal(localeFor(Lang.EN, 'en-GB').N.date(secs), '12 Mar 2021');
  assert.equal(intlLocale(Lang.EN, 'es-ES'), 'en-US');

  const es = localeFor(Lang.ES, 'en-US');
  assert.equal(es.S.revealButton, 'Mostrar en carpeta');
  assert.equal(es.N.count(1234567), '1.234.567');
  assert.equal(es.N.date(secs), '12 mar 2021');
  assert.equal(es.N.size(1.5 * 1024 * 1024), '1,5 MB');

  assert.equal(localeFor('fr', 'fr-FR').lang, Lang.ES);
});

/** VFS-22 AC-8: highlighting folds accents and case without any language input, so it is
 *  the same in both; search folding lives in vfs-core and is untouched. */
test('accent folding does not depend on the language', () => {
  assert.deepEqual(F.matchRanges('Información.pdf', F.terms('informacion')), [[0, 11]]);
});
