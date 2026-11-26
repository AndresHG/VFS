// «Preguntar a sus documentos (IA)» in Ajustes (VFS-28 28d). Run: node --test apps/desktop/ui-dev/ai.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { aiErrorText, formatUsd, keyLine, parseCap, passageTitle, recordLines } from '../ui/ai.js';
import { AiErrorCode, KeyFallback, KeyPlace, KeyState, Provider } from '../ui/contract.js';
import { Lang, localeFor } from '../ui/strings.js';

const { S } = localeFor(Lang.ES, 'es-ES');
const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');
const aiJs = readFileSync(new URL('../ui/ai.js', import.meta.url), 'utf8');

/** VFS-28 AC-1 (UI part, R1): a saved key shows only «termina en …» and its last 4; where it
 *  is kept is said when it is not the system store; an unreadable key asks to re-enter it. */
test('the key line shows only the last four characters', () => {
  assert.equal(keyLine(S, { state: KeyState.SAVED, last4: 'a1b2', place: KeyPlace.CREDENTIAL_STORE }),
    'Clave guardada · termina en …a1b2');
  assert.match(keyLine(S, { state: KeyState.SAVED, last4: 'a1b2', place: KeyPlace.PROTECTED_FILE }), /protección básica/);
  assert.match(keyLine(S, { state: KeyState.SAVED, last4: 'a1b2', place: KeyPlace.SESSION_ONLY }), /esta sesión/);
  assert.equal(keyLine(S, { state: KeyState.UNREADABLE }), 'No se puede leer la clave guardada; vuelva a introducirla.');
  assert.equal(keyLine(S, { state: KeyState.NONE }), S.aiKeyNone);
});

/** VFS-28 R1, R2: the key field is a write-only password field that is never filled by the
 *  screen; the fallback dialog's buttons carry the KeyFallback ids. */
test('the key field is write-only', () => {
  assert.match(html, /<input id="ai-key" type="password" autocomplete="off"/);
  assert.doesNotMatch(aiJs, /keyInput\.value\s*=\s*[^'\s]/, 'the screen only ever clears the key field');
  for (const v of Object.values(KeyFallback)) assert.match(html, new RegExp(`value="${v}"`));
  assert.doesNotMatch(aiJs, /innerHTML/);
});

/** VFS-28 AC-4 (R4, R13): every error code has a sentence; provider names fill the
 *  placeholders; a provider error carries its message. */
test('every AI error has a plain sentence', () => {
  for (const code of Object.values(AiErrorCode)) {
    const text = aiErrorText(S, { code, message: 'overloaded' }, Provider.OPENAI);
    assert.ok(text && text !== S.errors.generic, code);
  }
  assert.equal(aiErrorText(S, { code: AiErrorCode.NO_CREDIT }, Provider.ANTHROPIC), 'Su cuenta de Anthropic no tiene saldo.');
  assert.equal(aiErrorText(S, { code: AiErrorCode.SATURATED }, Provider.OPENAI), 'OpenAI está saturado; pruebe en un momento.');
  assert.equal(aiErrorText(S, { code: AiErrorCode.INVALID_KEY }, Provider.OPENAI), 'La clave no es válida.');
  assert.match(aiErrorText(S, { code: AiErrorCode.PROVIDER, message: 'quota' }, Provider.OPENAI), /quota/);
});

/** VFS-28 R10: the cap is typed in dollars and kept in nano-dollars; costs keep 4 decimals
 *  below a dollar. */
test('cap and costs in dollars', () => {
  assert.equal(parseCap('5'), 5e9);
  assert.equal(parseCap('0,5'), 5e8);
  assert.equal(parseCap('0'), 0);
  for (const bad of ['', '-1', 'cinco']) assert.equal(parseCap(bad), null, bad);
  assert.equal(formatUsd(35_000_000, 'en'), '$0.035');
  assert.equal(formatUsd(5e9, 'en'), '$5.00');
});

/** VFS-28 AC-9 (UI part, R11): a record shows its date, provider and cost, the question, each
 *  passage's file and page, and every call's model, tokens and cost. */
test('a record lists what was sent and what it cost', () => {
  const r = {
    askedAt: '2026-11-23T12:00:00+01:00', provider: Provider.ANTHROPIC, question: '¿fianza?',
    passages: [{ fileName: 'Contrato.pdf', page: 4, text: 'La fianza…' }],
    calls: [
      { rewrite: true, followUp: false, model: 'm1', inputTokens: 800, outputTokens: 60, costNano: 110_000 },
      { rewrite: false, followUp: true, model: 'm2', inputTokens: 9000, outputTokens: 500, costNano: 23_000_000 },
    ],
    costNano: 23_110_000,
  };
  const lines = recordLines(S, r, 'es');
  assert.match(lines.heading, /Anthropic/);
  assert.match(lines.heading, /0,0231/);
  assert.equal(lines.question, '¿fianza?');
  assert.match(lines.calls[0], /^Preparar la búsqueda: m1, 800/);
  assert.match(lines.calls[1], /^Nueva pregunta sobre lo mismo: m2/);
  assert.equal(passageTitle(S, r.passages[0]), 'Contrato.pdf · p. 4');
  assert.equal(passageTitle(S, { fileName: 'nota.txt', page: null }), 'nota.txt');
});

/** VFS-28 R1, R11: Ajustes has the section with its controls, and «Qué se ha enviado» has
 *  «Borrar todo» behind a confirmation. */
test('the Ajustes section and the record are in the page', () => {
  for (const id of ['ai-on', 'ai-providers', 'ai-key-form', 'ai-test', 'ai-replace', 'ai-delete', 'ai-cap',
    'ai-open-record', 'ai-record', 'ai-record-clear', 'confirm-ai-clear', 'confirm-ai-delete', 'ai-fallback']) {
    assert.match(html, new RegExp(`id="${id}"`), id);
  }
  assert.match(html, /<div id="ai-body" class="ai-body" hidden>/, 'off by default');
  assert.match(html, /<section id="ai-section" class="setting" aria-labelledby="ai-set-title" hidden>/,
    'hidden until the shell answers ai_status (VFS-28: a build without AI never shows it)');
});
