// «Filtro de archivos» (VFS-34), checked without a browser: the add flow, the copy in both
// catalogs and the markup. Run: node --test apps/desktop/ui-dev/filefilter.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { addPattern, patternErrorText, parseLimit, showContentLimit, CONFIRM_ABOVE } from '../ui/filefilter.js';
import { ErrorCode, PatternReason } from '../ui/contract.js';
import { CATALOGS, Lang } from '../ui/strings.js';

const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');

/** A backend whose count answers `count`, recording what is called. */
function fakeApi(count) {
  const calls = [];
  return {
    calls,
    async countFileExclusion(p) { calls.push(['count', p]); return count; },
    async addFileExclusion(p) { calls.push(['add', p]); return ['*.tmp', p]; },
  };
}

/** VFS-34 AC-1, R2: a pattern matching 1,500 indexed files asks first; refused, nothing is
 *  added; accepted, it is added. */
test('more than 1,000 matches asks for confirmation before adding', async () => {
  const asked = [];
  let api = fakeApi(1500);
  let r = await addPattern(api, '*.bak', async (n) => { asked.push(n); return false; });
  assert.deepEqual(asked, [1500]);
  assert.deepEqual(r, { cancelled: true, count: 1500 });
  assert.deepEqual(api.calls, [['count', '*.bak']]);

  api = fakeApi(1500);
  r = await addPattern(api, '*.bak', async () => true);
  assert.deepEqual(r, { list: ['*.tmp', '*.bak'], count: 1500 });
  assert.deepEqual(api.calls, [['count', '*.bak'], ['add', '*.bak']]);
});

/** VFS-34 AC-1, R2: 1,000 matches or fewer are added without asking, with the count. */
test('up to 1,000 matches is added without asking', async () => {
  const api = fakeApi(CONFIRM_ABOVE);
  const r = await addPattern(api, '*.swp', async () => assert.fail('asked'));
  assert.equal(r.count, CONFIRM_ABOVE);
  assert.deepEqual(api.calls.map((c) => c[0]), ['count', 'add']);
});

/** VFS-34 AC-1, R2: a refused pattern is never added, and each reason has a message in both
 *  languages. */
test('refused patterns are worded in both languages', async () => {
  const api = {
    async countFileExclusion() { throw { code: ErrorCode.INVALID_PATTERN, message: PatternReason.DUPLICATE }; },
    async addFileExclusion() { assert.fail('added'); },
  };
  await assert.rejects(addPattern(api, '*.tmp', async () => true), (e) => e.message === PatternReason.DUPLICATE);
  for (const lang of Object.values(Lang)) {
    const S = CATALOGS[lang];
    for (const reason of Object.values(PatternReason)) {
      const text = patternErrorText({ code: ErrorCode.INVALID_PATTERN, message: reason }, S);
      assert.ok(text && text.length > 5, `${lang} ${reason}`);
    }
    assert.equal(patternErrorText({ code: ErrorCode.IO, message: 'x' }, S), null);
  }
});

/** VFS-34 AC-6, R1: the section's copy exists in both catalogs, and the built-in line names
 *  the system and lock files. */
test('the copy exists in both catalogs', () => {
  const keys = ['fileFilterTitle', 'fileFilterIntro', 'fileFilterPlaceholder', 'fileFilterAdd', 'fileFilterRemove',
    'fileFilterReset', 'fileFilterBuiltIn', 'fileFilterAdded', 'fileFilterComeBack', 'fileFilterConfirmTitle',
    'fileFilterConfirmBody', 'fileFilterConfirmAdd', 'contentMaxLabel', 'contentMaxHint', 'contentMaxInvalid'];
  for (const lang of Object.values(Lang)) {
    for (const k of keys) assert.ok(CATALOGS[lang][k], `${lang}.${k}`);
  }
  const es = CATALOGS[Lang.ES];
  assert.equal(es.fileFilterTitle, 'Filtro de archivos');
  assert.equal(es.fileFilterPlaceholder, 'Nombre o extensión (por ejemplo *.tmp)');
  assert.equal(es.fileFilterReset, 'Restablecer la lista');
  assert.equal(es.contentMaxLabel, 'Tamaño máximo para leer el contenido (MB)');
  assert.equal(es.fileFilterComeBack, 'Los archivos volverán a aparecer tras la próxima revisión completa.');
  assert.match(es.fileFilterBuiltIn, /Thumbs\.db, \.DS_Store, desktop\.ini, ~\$/);
  assert.match(es.fileFilterConfirmBody('*.bak', '1.500'), /1\.500/);
});

/** VFS-34 AC-6, R1, R6: the section has the list, the add field and button, reset and the
 *  built-in line; the size field starts hidden and follows content search. */
test('the section markup, and the size field follows content search', () => {
  for (const id of ['file-filter-list', 'file-filter-pattern', 'file-filter-add', 'file-filter-reset', 'content-max-mb']) {
    assert.match(html, new RegExp(`id="${id}"`), id);
  }
  assert.match(html, /data-s="fileFilterBuiltIn"/);
  assert.match(html, /<div id="content-max-row"[^>]*\bhidden\b/);
  // The filter sits in «Avanzado», after VFS-33's switch.
  assert.ok(html.indexOf('id="include-all"') < html.indexOf('id="file-filter-list"'));
  assert.ok(html.indexOf('id="file-filter-list"') < html.indexOf('</details>', html.indexOf('id="include-all"')));
  const row = { hidden: true };
  showContentLimit(row, true);
  assert.equal(row.hidden, false);
  showContentLimit(row, false);
  assert.equal(row.hidden, true);
});

/** VFS-34 R6: the size field takes whole MB from 0 (no limit) to 100,000. */
test('the size limit is a whole number of MB', () => {
  assert.equal(parseLimit('100'), 100);
  assert.equal(parseLimit(' 0 '), 0);
  assert.equal(parseLimit('100000'), 100000);
  for (const bad of ['', '-1', '1.5', 'abc', '100001', '1e3']) assert.equal(parseLimit(bad), null, bad);
});
