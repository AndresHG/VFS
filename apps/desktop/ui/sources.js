// Search sources on the screen (VFS-6): which sources a "Buscar en" choice covers, and the
// text of a bookmark row. Pure functions, tested in ui-dev/sources.test.mjs.

import { Source } from './contract.js';
import { SourceChoice } from './choices.js';

/** The `sources` argument of `search_sources`; files only while bookmarks are off (R1). */
export function sourcesFor(choice, bookmarksOn) {
  if (!bookmarksOn || choice === SourceChoice.FILES) return [Source.FILES];
  if (choice === SourceChoice.BOOKMARKS) return [Source.BOOKMARKS];
  return [];
}

/** The type, size and folder filters only apply to files (R7). */
export function fileFiltersApply(choice, bookmarksOn) {
  return !bookmarksOn || choice !== SourceChoice.BOOKMARKS;
}

/** "Menú de marcadores › Recetas › Postres", led by the profile name when there are several. */
export function bookmarkPath(hit, rootLabels) {
  const parts = [rootLabels[hit.root] ?? hit.root, ...hit.folder];
  if (hit.profile) parts.unshift(hit.profile);
  return parts.join(' › ');
}

/** The site of an address, without `www.`: "https://www.cocina.es/flan" -> "cocina.es". */
export function site(url) {
  const m = /^[a-z][a-z0-9+.-]*:\/\/([^/?#]*)/i.exec(url);
  if (!m) return '';
  return m[1].replace(/^[^@]*@/, '').replace(/^www\./i, '');
}
