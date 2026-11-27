// «Relacionados» on the screen (VFS-53). Run: node --test apps/desktop/ui-dev/related.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { reasonsFor, relatedAvailable } from '../ui/related.js';
import { CATALOGS, Lang, localeFor } from '../ui/strings.js';

const { S } = localeFor(Lang.ES, 'es-ES');
const mainJs = readFileSync(new URL('../ui/main.js', import.meta.url), 'utf8');
const chatJs = readFileSync(new URL('../ui/chat.js', import.meta.url), 'utf8');

/// VFS-53 AC-7 (R6): every related document says «mismo tema»; one in the same folder also
/// says «misma carpeta».
test('reasons: same topic, and same folder when it is', () => {
  assert.deepEqual(reasonsFor(S, { dir: '/a' }, '/b'), ['mismo tema']);
  assert.deepEqual(reasonsFor(S, { dir: '/a' }, '/a'), ['mismo tema', 'misma carpeta']);
  assert.deepEqual(reasonsFor(S, { dir: '/a' }, null), ['mismo tema']);
});

/// VFS-53 AC-7 (R4): «Relacionados» only while search by meaning serves.
test('available only while search by meaning serves', () => {
  assert.equal(relatedAvailable(null), false);
  assert.equal(relatedAvailable({ settings: { enabled: true }, searchable: false }), false);
  assert.equal(relatedAvailable({ settings: { enabled: false }, searchable: true }), false);
  assert.equal(relatedAvailable({ settings: { enabled: true }, searchable: true }), true);
});

/// VFS-53 AC-7, AC-8 (R4): the «Documentos» hit and the Chat source panel offer it, gated.
test('the hit and the chat source panel offer it', () => {
  assert.match(mainJs, /if \(relatedAvailable\(state\.meaning\)\)/);
  assert.match(mainJs, /relatedOn: \(\) => relatedAvailable\(state\.meaning\)/);
  assert.match(chatJs, /if \(relatedOn\(\)\)/);
});

/// VFS-22: the copy is in both catalogs.
test('copy in both catalogs', () => {
  for (const key of ['relatedButton', 'relatedOpen', 'relatedTitle', 'relatedLoading', 'relatedNone', 'relatedFailed', 'relatedSameTopic', 'relatedSameFolder']) {
    for (const [lang, cat] of Object.entries(CATALOGS)) assert.ok(cat[key], `${key} missing in ${lang}`);
  }
});
