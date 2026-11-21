// "Enviar comentarios" (VFS-20), checked without a browser: the copy in strings.js, the markup
// of index.html and the message after saving. Run: node --test apps/desktop/ui-dev/feedback.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { S } from '../ui/strings.js';
import { savedMessage } from '../ui/feedback.js';

const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');
const api = readFileSync(new URL('../ui/api.js', import.meta.url), 'utf8');

/** VFS-20 AC-7: the bottom bar opens a dialog with the text box, the names box unticked,
 * the "Ver el informe" preview and both actions, every label in the catalog. */
test('feedback dialog markup and copy', () => {
  assert.match(html, /<footer class="bottom">[\s\S]*id="open-feedback"[\s\S]*<\/footer>/);
  for (const id of ['feedback', 'feedback-text', 'feedback-names', 'feedback-details', 'feedback-report',
    'feedback-github', 'feedback-export', 'feedback-msg']) {
    assert.match(html, new RegExp(`id="${id}"`), id);
  }
  assert.doesNotMatch(html, /id="feedback-names"[^>]*checked/);
  for (const [, key] of html.matchAll(/data-s(?:-label)?="(feedback\w+)"/g)) assert.ok(S[key], key);
  assert.equal(S.feedbackButton, 'Enviar comentarios');
  assert.equal(S.feedbackPreview, 'Ver el informe');
  assert.match(S.feedbackExport, /Exportar diagnóstico/);
  assert.match(api, /feedbackReport: \(description, includeNames\) => invoke\('feedback_report'/);
  assert.match(api, /feedbackSave: \(description, includeNames, github\) => invoke\('feedback_save'/);
});

/** VFS-20 AC-4, AC-5: after saving, the message names the folder and the file, and says what
 * to do with it: give it to whoever installed the program, or drag it into GitHub. */
test('saved message says where the report is and what to do', () => {
  const saved = { folder: 'C:\\Users\\ana\\Downloads', file: 'vfs-informe-2026-11-21-1030.zip' };
  const local = savedMessage(S, saved, false);
  const gh = savedMessage(S, saved, true);
  for (const m of [local, gh]) {
    assert.ok(m.includes(saved.folder) && m.includes(saved.file), m);
  }
  assert.match(local, /quien le instaló/);
  assert.match(gh, /GitHub/);
});
