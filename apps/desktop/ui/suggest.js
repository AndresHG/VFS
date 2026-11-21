// A suggestion for a search with no hits (VFS-13): when the screen asks for one and how its
// button reads. Tested in ui-dev/suggest.test.mjs.

import { Source, SuggestionKind, TruncatedBy } from './contract.js';
import { bookmarkFolderOf } from './sources.js';

/** R1: ask only after a finished search with no hits, not cut by the deadline, that covered
 *  files (a search of bookmarks alone gets no suggestion). */
export function wantsSuggestion(total, truncatedBy, sources) {
  return total === 0 && truncatedBy !== TruncatedBy.DEADLINE && sources.includes(Source.FILES);
}

/** R5: the button's copy, with the count formatted by `N` («más de 2.000» when capped). */
export function suggestionLabel(S, N, s) {
  const n = s.capped ? S.suggestCapped(N.count(s.count)) : N.count(s.count);
  switch (s.kind) {
    case SuggestionKind.DROP_FILTERS: return S.suggestDropFilters(n);
    case SuggestionKind.SPELLING: return S.suggestSpelling(s.replacement, n);
    case SuggestionKind.DROP_TERM: return S.suggestDropTerm(s.term, n);
    default: return '';
  }
}

/** The folder choice after clearing filters: a suggestion keeps a bookmark folder (the
 *  Contract: sources and bookmark folder stay); "Quitar filtros" clears any folder. */
export function folderAfterClear(folder, keepBookmarkFolder) {
  return keepBookmarkFolder && bookmarkFolderOf(folder) != null ? folder : '';
}
