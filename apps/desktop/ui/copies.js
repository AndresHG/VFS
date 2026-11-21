// Copies of a file shown as one result (VFS-19): the places a result row stands for. Pure,
// tested in ui-dev/screen.test.mjs.

import { Source } from './contract.js';

/** Every place of a file result: itself, then its copies (same name and size), best first.
 *  Folders and bookmarks have none. */
export function placesOf(hit) {
  if (!hit || hit.source === Source.BOOKMARKS || hit.isDir) return [];
  const self = { id: hit.id, dir: hit.dir, modified: hit.modified, ...(hit.cloudOnly ? { cloudOnly: true } : {}) };
  return [self, ...(hit.copies || [])];
}

/** Whether a result row or one of its places shows the "Solo en la nube" badge (VFS-25 R4):
 *  only a file whose data is in the cloud, never a folder or a bookmark. */
export function showsCloudBadge(item) {
  return !!item?.cloudOnly && !item.isDir && item.source !== Source.BOOKMARKS;
}
