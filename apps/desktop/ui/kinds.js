// The kind of a result row: its icon in the sprite of index.html and its label for screen
// readers (VFS-36). A new kind of result is one entry here and one symbol in the sprite.
// Pure, tested in ui-dev/kinds.test.mjs.

import { FileKind, Source } from './contract.js';

export const RowKind = Object.freeze({
  ...FileKind,
  BOOKMARK: 'bookmark',
  BOOKMARK_FOLDER: 'bookmark-folder',
});

const fileKind = (kind) => Object.freeze({ icon: kind, label: (S) => S.kindSingular[kind] });

/** `icon`: the sprite symbol `i-<icon>`; `label(S)`: the kind, in the row's accessible name. */
export const ROW_KINDS = Object.freeze({
  [RowKind.DOCUMENT]: fileKind(FileKind.DOCUMENT),
  [RowKind.PHOTO]: fileKind(FileKind.PHOTO),
  [RowKind.VIDEO]: fileKind(FileKind.VIDEO),
  [RowKind.AUDIO]: fileKind(FileKind.AUDIO),
  [RowKind.OTHER]: fileKind(FileKind.OTHER),
  [RowKind.FOLDER]: fileKind(FileKind.FOLDER),
  [RowKind.BOOKMARK]: Object.freeze({ icon: 'bookmark', label: (S) => S.bookmarkKind }),
  [RowKind.BOOKMARK_FOLDER]: Object.freeze({ icon: 'bookmark-folder', label: (S) => S.bookmarkFolderKind }),
});

/** The row kind of a search hit. A shortcut or a cloud-only file keeps its file kind; its
 *  badge carries the rest. */
export function rowKindOf(hit) {
  if (hit.source === Source.BOOKMARKS) return hit.isFolder ? RowKind.BOOKMARK_FOLDER : RowKind.BOOKMARK;
  return hit.isDir ? RowKind.FOLDER : hit.kind;
}
