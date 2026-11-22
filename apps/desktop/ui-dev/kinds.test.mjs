// VFS-36: each kind of result row has its own icon and a label for screen readers.
// Run: node --test apps/desktop/ui-dev/kinds.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { FileKind, LinkKind, Source } from '../ui/contract.js';
import { ROW_KINDS, RowKind, rowKindOf } from '../ui/kinds.js';
import { CATALOGS } from '../ui/strings.js';

const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');

/** VFS-36 AC-1: every row kind maps to a symbol of the sprite and to a label in each catalog. */
test('every row kind has a sprite icon and a label in both languages', () => {
  assert.deepEqual(Object.keys(ROW_KINDS).sort(), Object.values(RowKind).sort());
  for (const [kind, { icon, label }] of Object.entries(ROW_KINDS)) {
    assert.match(html, new RegExp(`<symbol id="i-${icon}"`), `sprite has i-${icon} for ${kind}`);
    for (const [lang, S] of Object.entries(CATALOGS)) {
      assert.equal(typeof label(S), 'string', `${lang} label for ${kind}`);
      assert.ok(label(S).length > 0, `${lang} label for ${kind}`);
    }
  }
});

/** VFS-36 AC-2: a bookmark folder looks unlike a disk folder, and a bookmark unlike a file. */
test('bookmark folders and bookmarks have icons of their own', () => {
  const icons = Object.values(ROW_KINDS).map((k) => k.icon);
  assert.equal(new Set(icons).size, icons.length, 'one icon per kind');
  assert.notEqual(ROW_KINDS[RowKind.BOOKMARK_FOLDER].icon, ROW_KINDS[RowKind.FOLDER].icon);
  for (const S of Object.values(CATALOGS)) {
    assert.notEqual(ROW_KINDS[RowKind.BOOKMARK_FOLDER].label(S), ROW_KINDS[RowKind.FOLDER].label(S));
  }
});

/** VFS-36 AC-3: the kind comes from the result data the UI already receives. */
test('row kind of each result', () => {
  const file = (kind, extra = {}) => ({ source: Source.FILES, isDir: false, kind, ...extra });
  for (const kind of Object.values(FileKind)) if (kind !== FileKind.FOLDER) assert.equal(rowKindOf(file(kind)), kind);
  assert.equal(rowKindOf({ source: Source.FILES, isDir: true, kind: FileKind.OTHER }), RowKind.FOLDER);
  assert.equal(rowKindOf({ source: Source.BOOKMARKS, isFolder: false, url: 'https://a.example' }), RowKind.BOOKMARK);
  assert.equal(rowKindOf({ source: Source.BOOKMARKS, isFolder: true }), RowKind.BOOKMARK_FOLDER);
  // A shortcut and a cloud-only file keep their file kind; their badge says the rest.
  assert.equal(rowKindOf(file(FileKind.PHOTO, { link: { kind: LinkKind.SHORTCUT }, cloudOnly: true })), RowKind.PHOTO);
});
