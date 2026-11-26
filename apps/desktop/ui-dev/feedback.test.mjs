// "Enviar comentarios" (VFS-20), checked without a browser: the copy in the catalogs, the markup
// of index.html and the message after saving. Run: node --test apps/desktop/ui-dev/feedback.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { CATALOGS, Lang } from '../ui/strings.js';
import { savedMessage, setupFeedback } from '../ui/feedback.js';

const S = CATALOGS[Lang.ES];
const EN = CATALOGS[Lang.EN];
const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');

/** VFS-20 AC-7, VFS-48 AC-4: the bottom bar opens a dialog with the text box, the "Ver el
 * informe" preview and both actions, every label in the catalog, and no names box. */
test('feedback dialog markup and copy', () => {
  assert.match(html, /<footer class="bottom">[\s\S]*id="open-feedback"[\s\S]*<\/footer>/);
  for (const id of ['feedback', 'feedback-text', 'feedback-details', 'feedback-report',
    'feedback-github', 'feedback-export', 'feedback-msg']) {
    assert.match(html, new RegExp(`id="${id}"`), id);
  }
  assert.doesNotMatch(html, /feedback-names/);
  for (const [, key] of html.matchAll(/data-s(?:-label)?="(feedback\w+)"/g)) assert.ok(S[key], key);
  assert.equal(S.feedbackButton, 'Enviar comentarios');
  assert.equal(S.feedbackPreview, 'Ver el informe');
  assert.match(S.feedbackExport, /Exportar diagnóstico/);
});

/** VFS-30 AC-5: "Ver el informe" says it shows only the newest log lines and the .zip holds
 * all, in both catalogs. */
test('the preview says the .zip holds the whole log', () => {
  assert.match(html, /<details id="feedback-details">[\s\S]*data-s="feedbackPreviewNote"[\s\S]*<\/details>/);
  assert.match(S.feedbackPreviewNote, /\.zip/);
  assert.match(EN.feedbackPreviewNote, /\.zip/);
});

/** VFS-20 R6, VFS-48 AC-4: the dialog says the report is anonymous and the GitHub issue is
 * public, in both catalogs. */
test('feedback dialog warns that the GitHub page is public', () => {
  assert.match(html, /data-s="feedbackPublic"/);
  assert.match(S.feedbackPublic, /anónimo[\s\S]*pública/);
  assert.match(EN.feedbackPublic, /anonymous[\s\S]*public/);
  assert.equal(S.feedbackNames, undefined);
  assert.equal(EN.feedbackNames, undefined);
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
  // VFS-22: the English catalog says the same.
  const enLocal = savedMessage(EN, saved, false);
  assert.ok(enLocal.includes(saved.folder) && enLocal.includes(saved.file), enLocal);
  assert.match(enLocal, /whoever installed/);
  assert.match(savedMessage(EN, saved, true), /GitHub/);
});

/** A stand-in for the few DOM elements the dialog touches. */
function fakeDom() {
  const els = new Map();
  const $ = (id) => {
    if (!els.has(id)) {
      const listeners = {};
      els.set(id, {
        value: '', checked: false, open: false, hidden: true, disabled: false, className: '', textContent: '',
        addEventListener: (type, fn) => { (listeners[type] ??= []).push(fn); },
        fire: async (type) => { for (const fn of listeners[type] ?? []) await fn(); },
        focus() {}, scrollIntoView() {}, showModal() { this.open = true; },
      });
    }
    return els.get(id);
  };
  return $;
}

/** VFS-20 AC-7, VFS-48 AC-4: opening "Ver el informe" asks for the report with the user's text
 * only (no names choice); saving reports where it went. VFS-30 AC-5: typing asks for no report. */
test('dialog asks for the report when the preview opens', async () => {
  const $ = fakeDom();
  const calls = [];
  const api = {
    feedbackReport: async (...args) => { calls.push(['report', ...args]); return 'informe'; },
    feedbackSave: async (...args) => {
      calls.push(['save', ...args]);
      return { folder: 'C:\\Descargas', file: 'vfs-informe-x.zip' };
    },
  };
  setupFeedback({ api, S, $, errorText: () => 'error' });
  await $('open-feedback').fire('click');
  assert.equal($('feedback').open, true);

  $('feedback-text').value = 'Va lento';
  $('feedback-details').open = true;
  await $('feedback-details').fire('toggle');
  assert.equal($('feedback-report').value, 'informe');
  assert.deepEqual(calls, [['report', 'Va lento']]);
  // Editing the text never rebuilds the report (it reads the whole log on a slow PC).
  $('feedback-text').value = 'Va muy lento';
  await $('feedback-text').fire('input');
  await $('feedback-text').fire('change');
  assert.equal(calls.length, 1);

  await $('feedback-github').fire('click');
  assert.deepEqual(calls.at(-1), ['save', 'Va muy lento', true]);
  assert.equal($('feedback-msg').hidden, false);
  assert.ok($('feedback-msg-text').textContent.includes('vfs-informe-x.zip'));
  assert.equal($('feedback-github').disabled, false);
});
