// The no-hits suggestion on screen (VFS-13). Run: node --test apps/desktop/ui-dev/suggest.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';

import { wantsSuggestion, suggestionLabel } from '../ui/suggest.js';
import { localeFor, Lang } from '../ui/strings.js';
import { Source, SuggestionKind, TruncatedBy } from '../ui/contract.js';

const es = localeFor(Lang.ES, 'es-ES');
const s = (over) => ({ kind: SuggestionKind.SPELLING, text: 'boda', clearFilters: false,
  term: 'Bdoa', replacement: 'boda', count: 3, capped: false, ...over });

/** VFS-13 R1: only a finished search with no hits, not cut by the deadline, over files. */
test('a suggestion is asked only for an empty file search', () => {
  assert.ok(wantsSuggestion(0, null, [Source.FILES, Source.BOOKMARKS]));
  assert.ok(wantsSuggestion(0, TruncatedBy.CAP, [Source.FILES]));
  assert.ok(!wantsSuggestion(1, null, [Source.FILES]));
  assert.ok(!wantsSuggestion(0, TruncatedBy.DEADLINE, [Source.FILES]));
  assert.ok(!wantsSuggestion(0, null, [Source.BOOKMARKS]));
});

/** VFS-13 AC-7: the three button copies, and a capped count as «más de 2.000». */
test('the button reads as the spec says', () => {
  assert.equal(suggestionLabel(es.S, es.N, s({ kind: SuggestionKind.DROP_FILTERS, count: 12 })),
    'Ningún resultado con estos filtros. ¿Buscar sin filtros? (12)');
  assert.equal(suggestionLabel(es.S, es.N, s()), '¿Quería decir «boda»? (3)');
  assert.equal(suggestionLabel(es.S, es.N, s({ kind: SuggestionKind.DROP_TERM, term: '2019', count: 2000, capped: true })),
    '¿Buscar sin «2019»? (más de 2.000)');
});
