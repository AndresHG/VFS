// Remembered choices (VFS-38 R6) and the results tabs (VFS-42 R1, R5), without a browser.
// Run: node --test apps/desktop/ui-dev/prefs.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';

import { PrefKey, readPref, writePref } from '../ui/prefs.js';
import { ResultsTab, SourceChoice } from '../ui/choices.js';
import { CATALOGS, Lang } from '../ui/strings.js';

/** A Storage stand-in over a Map. */
function memory(entries = {}) {
  const m = new Map(Object.entries(entries));
  return { getItem: (k) => (m.has(k) ? m.get(k) : null), setItem: (k, v) => m.set(k, String(v)), map: m };
}

const throwing = {
  getItem() { throw new Error('SecurityError'); },
  setItem() { throw new Error('QuotaExceededError'); },
};

/** VFS-38 AC-6 (R6): a kept value is used only when it is offered; missing, unknown, not
 *  offered, unreadable or absent storage give the default; writing never throws. */
test('a remembered choice is read only when it is offered', () => {
  const sources = Object.values(SourceChoice);
  const store = memory({ [PrefKey.SOURCE]: SourceChoice.FILES, [PrefKey.SORT]: 'sideways' });
  assert.equal(readPref(store, PrefKey.SOURCE, sources, SourceChoice.BOTH), SourceChoice.FILES);
  assert.equal(readPref(store, PrefKey.SORT, ['relevance'], 'relevance'), 'relevance', 'unknown value');
  assert.equal(readPref(memory(), PrefKey.SORT, ['relevance'], 'relevance'), 'relevance', 'missing value');
  assert.equal(readPref(memory({ [PrefKey.SOURCE]: SourceChoice.BOOKMARKS }), PrefKey.SOURCE, [SourceChoice.BOTH], SourceChoice.BOTH),
    SourceChoice.BOTH, 'a choice not offered at start');
  assert.equal(readPref(throwing, PrefKey.SOURCE, sources, SourceChoice.BOTH), SourceChoice.BOTH, 'storage that throws');
  assert.equal(readPref(null, PrefKey.SOURCE, sources, SourceChoice.BOTH), SourceChoice.BOTH, 'no storage at all');
  assert.doesNotThrow(() => writePref(throwing, PrefKey.SOURCE, SourceChoice.FILES));
  assert.doesNotThrow(() => writePref(null, PrefKey.SOURCE, SourceChoice.FILES));
  const kept = memory();
  writePref(kept, PrefKey.SOURCE, SourceChoice.FILES);
  assert.equal(readPref(kept, PrefKey.SOURCE, sources, SourceChoice.BOTH), SourceChoice.FILES);
});

/** VFS-42 AC-5 (R5): the tab is not remembered: no key is kept for it, `vfs.scope` included. */
test('no remembered choice for the tab', () => {
  assert.ok(!Object.values(PrefKey).includes('vfs.scope'), 'vfs.scope is no longer read');
  assert.ok(!Object.keys(PrefKey).some((k) => /scope|tab/i.test(k)), 'no key for the scope or the tab');
});

/** VFS-42 R1, R3: two tabs with their labels in both languages, the tablist named «Buscar en»;
 *  the copy of «Documentos» exists in both catalogs. */
test('the tabs and the «Documentos» messages have their copy', () => {
  assert.deepEqual(Object.values(ResultsTab), ['names', 'documents']);
  for (const lang of Object.values(Lang)) {
    const S = CATALOGS[lang];
    for (const k of ['tabsLabel', 'tabNames', 'tabDocuments', 'emptyDocumentsShort', 'searchNames', 'documentsBookmarkFolder',
      'removeFolder', 'documentsOffTitle', 'documentsOffBody']) assert.ok(S[k], `${lang}: ${k}`);
    assert.deepEqual(Object.keys(S.sourceChips).sort(), [SourceChoice.BOOKMARKS, SourceChoice.FILES], lang);
  }
  const ES = CATALOGS[Lang.ES];
  assert.deepEqual([ES.tabsLabel, ES.tabNames, ES.tabDocuments], ['Buscar en', 'Nombres', 'Documentos']);
  assert.deepEqual([CATALOGS[Lang.EN].tabNames, CATALOGS[Lang.EN].tabDocuments], ['Names', 'Documents']);
  assert.equal(ES.searchNames, 'Buscar en los nombres');
  assert.equal(ES.documentsBookmarkFolder, 'Las carpetas de marcadores no tienen documentos');
  assert.equal(ES.removeFolder, 'Quitar la carpeta');
  assert.equal(ES.emptyDocumentsShort, 'Escribe al menos 3 letras para buscar dentro de los documentos.');
  assert.equal(ES.fSource, 'Origen');
  assert.equal(CATALOGS[Lang.EN].fSource, 'Source');
});
