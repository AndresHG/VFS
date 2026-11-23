// Remembered choices and the scope choices (VFS-38 R6, R14), without a browser.
// Run: node --test apps/desktop/ui-dev/prefs.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';

import { PrefKey, readPref, writePref } from '../ui/prefs.js';
import { OFFERED_SCOPES, ScopeChoice, SourceChoice, scopeFor } from '../ui/choices.js';
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
  assert.equal(readPref(store, PrefKey.SCOPE, OFFERED_SCOPES, ScopeChoice.ALL), ScopeChoice.ALL, 'unknown value');
  assert.equal(readPref(store, PrefKey.SORT, ['relevance'], 'relevance'), 'relevance', 'missing value');
  assert.equal(readPref(memory({ [PrefKey.SCOPE]: ScopeChoice.DOCUMENTS }), PrefKey.SCOPE, OFFERED_SCOPES, ScopeChoice.ALL),
    ScopeChoice.ALL, 'a choice not offered at start');
  assert.equal(readPref(throwing, PrefKey.SOURCE, sources, SourceChoice.BOTH), SourceChoice.BOTH, 'storage that throws');
  assert.equal(readPref(null, PrefKey.SOURCE, sources, SourceChoice.BOTH), SourceChoice.BOTH, 'no storage at all');
  assert.doesNotThrow(() => writePref(throwing, PrefKey.SCOPE, ScopeChoice.NAMES));
  assert.doesNotThrow(() => writePref(null, PrefKey.SCOPE, ScopeChoice.NAMES));
  const kept = memory();
  writePref(kept, PrefKey.SCOPE, ScopeChoice.NAMES);
  assert.equal(readPref(kept, PrefKey.SCOPE, OFFERED_SCOPES, ScopeChoice.ALL), ScopeChoice.NAMES);
});

/** VFS-38 R1, R14: the screen's scope choices map to the core's `Scope`; the group offers
 *  «Todo» first and «Solo documentos» not yet (VFS-27 27c); every choice has its label. */
test('scope choices map to the core scope and have labels', () => {
  assert.equal(scopeFor(ScopeChoice.ALL), Scope.ALL);
  assert.equal(scopeFor(ScopeChoice.NAMES), Scope.NAMES);
  for (const s of Object.values(Scope)) assert.ok(Object.values(ScopeChoice).includes(s), s);
  assert.equal(OFFERED_SCOPES[0], ScopeChoice.ALL);
  assert.ok(!OFFERED_SCOPES.includes(ScopeChoice.DOCUMENTS));
  for (const lang of Object.values(Lang)) {
    const S = CATALOGS[lang];
    assert.deepEqual(Object.keys(S.scopeOptions).sort(), Object.values(ScopeChoice).sort(), lang);
    assert.deepEqual(Object.keys(S.sourceChips).sort(), [SourceChoice.BOOKMARKS, SourceChoice.FILES], lang);
  }
  assert.equal(CATALOGS[Lang.ES].fScope, 'Buscar en');
  assert.equal(CATALOGS[Lang.ES].fSource, 'Origen');
  assert.equal(CATALOGS[Lang.EN].fSource, 'Source');
});
