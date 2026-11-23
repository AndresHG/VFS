// Remembered choices and the scope choices (VFS-38 R6, R14), without a browser.
// Run: node --test apps/desktop/ui-dev/prefs.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';

import { PrefKey, readPref, writePref } from '../ui/prefs.js';
import { ScopeChoice, SourceChoice, scopeFor, scopesFor } from '../ui/choices.js';
import { Scope } from '../ui/contract.js';
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
  const store = memory({ [PrefKey.SOURCE]: SourceChoice.FILES, [PrefKey.SCOPE]: 'everything' });
  assert.equal(readPref(store, PrefKey.SOURCE, sources, SourceChoice.BOTH), SourceChoice.FILES);
  assert.equal(readPref(store, PrefKey.SCOPE, scopesFor(true), ScopeChoice.ALL), ScopeChoice.ALL, 'unknown value');
  assert.equal(readPref(store, PrefKey.SORT, ['relevance'], 'relevance'), 'relevance', 'missing value');
  assert.equal(readPref(memory({ [PrefKey.SCOPE]: ScopeChoice.DOCUMENTS }), PrefKey.SCOPE, scopesFor(false), ScopeChoice.ALL),
    ScopeChoice.ALL, 'a choice not offered at start');
  assert.equal(readPref(throwing, PrefKey.SOURCE, sources, SourceChoice.BOTH), SourceChoice.BOTH, 'storage that throws');
  assert.equal(readPref(null, PrefKey.SOURCE, sources, SourceChoice.BOTH), SourceChoice.BOTH, 'no storage at all');
  assert.doesNotThrow(() => writePref(throwing, PrefKey.SCOPE, ScopeChoice.NAMES));
  assert.doesNotThrow(() => writePref(null, PrefKey.SCOPE, ScopeChoice.NAMES));
  const kept = memory();
  writePref(kept, PrefKey.SCOPE, ScopeChoice.NAMES);
  assert.equal(readPref(kept, PrefKey.SCOPE, scopesFor(false), ScopeChoice.ALL), ScopeChoice.NAMES);
});

/** VFS-38 R1, R14, AC-4: the screen's scope choices map to the core's `Scope`; the group offers
 *  «Todo» first, and «Solo documentos» last and only while content search is on; every choice
 *  has its label. */
test('scope choices map to the core scope and have labels', () => {
  assert.equal(scopeFor(ScopeChoice.ALL), Scope.ALL);
  assert.equal(scopeFor(ScopeChoice.NAMES), Scope.NAMES);
  for (const s of Object.values(Scope)) assert.ok(Object.values(ScopeChoice).includes(s), s);
  assert.deepEqual(scopesFor(false), [ScopeChoice.ALL, ScopeChoice.NAMES]);
  assert.deepEqual(scopesFor(true), [ScopeChoice.ALL, ScopeChoice.NAMES, ScopeChoice.DOCUMENTS]);
  assert.equal(scopeFor(ScopeChoice.DOCUMENTS), Scope.ALL, '«Solo documentos» runs no name search at all');
  for (const lang of Object.values(Lang)) {
    const S = CATALOGS[lang];
    assert.deepEqual(Object.keys(S.scopeOptions).sort(), Object.values(ScopeChoice).sort(), lang);
    assert.deepEqual(Object.keys(S.sourceChips).sort(), [SourceChoice.BOOKMARKS, SourceChoice.FILES], lang);
  }
  assert.equal(CATALOGS[Lang.ES].fScope, 'Buscar en');
  assert.equal(CATALOGS[Lang.ES].scopeOptions[ScopeChoice.DOCUMENTS], 'Solo documentos');
  assert.equal(CATALOGS[Lang.ES].searchEverything, 'Buscar en todo');
  assert.equal(CATALOGS[Lang.ES].emptyDocumentsShort, 'Escribe al menos 3 letras para buscar dentro de los documentos.');
  assert.equal(CATALOGS[Lang.ES].fSource, 'Origen');
  assert.equal(CATALOGS[Lang.EN].fSource, 'Source');
});
