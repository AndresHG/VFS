// Search sources on the screen (VFS-6): which sources a "Buscar en" choice covers, bookmark
// folders in the "Carpeta" select, and the folder path of a bookmark row. Pure functions,
// tested in ui-dev/sources.test.mjs.

import { Source } from './contract.js';
import { SourceChoice } from './choices.js';

/** The `sources` of `search_sources`; files only while bookmarks are off (R1). */
export function sourcesFor(choice, bookmarksOn) {
  if (!bookmarksOn || choice === SourceChoice.FILES) return [Source.FILES];
  if (choice === SourceChoice.BOOKMARKS) return [Source.BOOKMARKS];
  return [];
}

/** The size filter only applies to files (R7). */
export function sizeFilterApplies(choice, bookmarksOn) {
  return !bookmarksOn || choice !== SourceChoice.BOOKMARKS;
}

/** "Carpeta" select values: an absolute path is a folder on disk; this prefix and an id, a
 *  bookmark folder (R12). A path never starts with it. */
const BOOKMARK_FOLDER = 'bm:';

export function bookmarkFolderValue(id) {
  return `${BOOKMARK_FOLDER}${id}`;
}

/** The bookmark folder id of a select value, or null for a folder on disk or none. */
export function bookmarkFolderOf(value) {
  return value?.startsWith(BOOKMARK_FOLDER) ? Number(value.slice(BOOKMARK_FOLDER.length)) : null;
}

/** Where a bookmark or bookmark folder is saved, outermost first: the profile name when
 *  there are several, the root's name in the active language, then the folders. */
export function bookmarkPathParts(hit, rootLabels) {
  const parts = [rootLabels[hit.root] ?? hit.root, ...hit.folder];
  if (hit.profile) parts.unshift(hit.profile);
  return parts;
}

/** The site of an address, without user, `www.` or port: "https://www.cocina.es/flan" ->
 *  "cocina.es"; "" for an address with no host (`file:`, `data:`). */
export function site(url) {
  const m = /^[a-z][a-z0-9+.-]*:\/\/([^/?#]*)/i.exec(url || '');
  if (!m) return '';
  return m[1].replace(/^[^@]*@/, '').replace(/:\d+$/, '').replace(/^www\./i, '');
}

/** The separator between folder names in a bookmark path. No-break spaces: the row splits the
 *  path into a head and a tail for middle truncation, and a plain space at either edge would
 *  collapse. */
export const PATH_SEPARATOR = '\u00a0›\u00a0';
