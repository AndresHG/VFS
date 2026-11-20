// Copies of a file shown as one result (VFS-19): the places a result row stands for. Pure,
// tested in ui-dev/screen.test.mjs.

import { Source } from './contract.js';

/** Every place of a file result: itself, then its copies (same name and size), best first.
 *  Folders and bookmarks have none. */
export function placesOf(hit) {
  if (!hit || hit.source === Source.BOOKMARKS || hit.isDir) return [];
  return [{ id: hit.id, dir: hit.dir, modified: hit.modified }, ...(hit.copies || [])];
}
