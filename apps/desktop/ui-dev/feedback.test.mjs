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
});

/** VFS-20 R6: the dialog says the GitHub issue is public, in both catalogs. */
test('feedback dialog warns that the GitHub page is public', () => {
  assert.match(html, /data-s="feedbackPublic"/);
  assert.match(S.feedbackPublic, /pública/);
  assert.match(EN.feedbackPublic, /public/);
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

/** VFS-20 AC-7: the dialog opens with the names box unticked; opening "Ver el informe" asks
 * for the report, and ticking the box asks again with names; saving reports where it went. */
test('dialog asks for the report and refreshes it when the names box changes', async () => {
  const $ = fakeDom();
  const calls = [];
  const api = {
    feedbackReport: async (text, names) => { calls.push(['report', text, names]); return `informe ${names}`; },
    feedbackSave: async (text, names, github) => {
      calls.push(['save', text, names, github]);
      return { folder: 'C:\\Descargas', file: 'vfs-informe-x.zip' };
    },
  };
  setupFeedback({ api, S, $, errorText: () => 'error' });
  $('feedback-names').checked = true;
  await $('open-feedback').fire('click');
  assert.equal($('feedback').open, true);
  assert.equal($('feedback-names').checked, false);

  $('feedback-text').value = 'Va lento';
  $('feedback-details').open = true;
  await $('feedback-details').fire('toggle');
  assert.equal($('feedback-report').value, 'informe false');
  $('feedback-names').checked = true;
  await $('feedback-names').fire('change');
  assert.equal($('feedback-report').value, 'informe true');
  assert.deepEqual(calls, [['report', 'Va lento', false], ['report', 'Va lento', true]]);

  await $('feedback-github').fire('click');
  assert.deepEqual(calls.at(-1), ['save', 'Va lento', true, true]);
  assert.equal($('feedback-msg').hidden, false);
  assert.ok($('feedback-msg-text').textContent.includes('vfs-informe-x.zip'));
  assert.equal($('feedback-github').disabled, false);
});
