// The Chat tab (VFS-49) and the answers it draws (VFS-28 R7a, R8). Run:
// node --test apps/desktop/ui-dev/chat.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  answerRuns, bibLine, chatReady, ChatGroup, copyText, filterChats, groupChats, modelChoices,
  modelName, ModelTier, scopeChoices, scopeLabel, ScopeKind, sourceTitle,
} from '../ui/chat.js';
import { KeyPlace, KeyState, Provider } from '../ui/contract.js';
import { Lang, localeFor } from '../ui/strings.js';
import { startBrowser } from './browser.mjs';

const { S } = localeFor(Lang.ES, 'es-ES');
const chatJs = readFileSync(new URL('../ui/chat.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');

const contrato = { fileId: 1, name: 'Contrato.pdf', pages: [4], text: 'La fianza asciende a 1.200 euros.' };
const recibo = { fileId: 2, name: 'Recibo.docx', pages: [], text: 'Recibí 1.200 euros.' };
const largo = { fileId: 3, name: 'Acta.pdf', pages: [2, 3], text: '…' };

/** VFS-49 R12: the chat answers only with AI on, a saved key and a content store. */
test('the chat answers only when it can', () => {
  const ok = { settings: { enabled: true }, key: { state: KeyState.SAVED, last4: 'a1b2', place: KeyPlace.CREDENTIAL_STORE }, store: true };
  assert.equal(chatReady(ok), true);
  assert.equal(chatReady({ ...ok, settings: { enabled: false } }), false);
  assert.equal(chatReady({ ...ok, key: { state: KeyState.NONE } }), false);
  assert.equal(chatReady({ ...ok, store: false }), false);
  assert.equal(chatReady(null), false);
});

/** VFS-47 AC-1: the selector offers the active provider's cheap model, then its mid model,
 *  named, with the cost hint from the catalog. */
test('the selector names the provider\'s models, cheap first', () => {
  assert.deepEqual(modelChoices(S, Provider.OPENAI), [
    { tier: ModelTier.CHEAP, label: 'Luna · más barato' }, { tier: ModelTier.MID, label: 'Sol · unas 20 veces más caro' }]);
  assert.deepEqual(modelChoices(S, Provider.ANTHROPIC).map((c) => c.label), ['Haiku · más barato', 'Sonnet · unas 20 veces más caro']);
  const { S: en } = localeFor(Lang.EN, 'en-US');
  assert.deepEqual(modelChoices(en, Provider.OPENAI).map((c) => c.label), ['Luna · cheapest', 'Sol · about 20 times dearer']);
  // VFS-49 R8: an answer names its model as the selector does.
  assert.equal(modelName('claude-haiku-5-5'), 'Haiku');
  assert.equal(modelName('gpt-6.2-sol'), 'Sol');
  assert.equal(modelName('otro-modelo'), 'otro-modelo');
});

/** VFS-28 AC-6 (UI part, R8): markers follow their block, each passage once per block; titles
 *  name the file and page, never a folder. */
test('answers become text runs and markers', () => {
  const blocks = [
    { text: 'Pagó 1.200 euros', citations: [{ passage: 0, marker: 1 }, { passage: 1, marker: 2 }, { passage: 0, marker: 1 }] },
    { text: ', al firmar.', citations: [] },
  ];
  assert.deepEqual(answerRuns(blocks), [
    { text: 'Pagó 1.200 euros' }, { marker: 1, passage: 0 }, { marker: 2, passage: 1 }, { text: ', al firmar.' },
  ]);
  assert.equal(sourceTitle(S, contrato), 'Contrato.pdf · p. 4');
  assert.equal(sourceTitle(S, recibo), 'Recibo.docx');
  assert.equal(sourceTitle(S, largo), 'Acta.pdf · pp. 2–3');
});

/** VFS-28 R9: «Copiar» copies the answer with its numbered sources as plain text; a not-found
 *  answer copies the not-found sentence. */
test('the copied answer carries its sources', () => {
  const turn = {
    notFound: false,
    blocks: [{ text: 'Pagó 1.200 euros', citations: [{ passage: 1, marker: 2 }, { passage: 0, marker: 1 }] }],
    sources: [contrato, recibo],
  };
  assert.equal(copyText(S, turn),
    'Pagó 1.200 euros [2] [1]\n\nFuentes:\n[1] Contrato.pdf · p. 4\n[2] Recibo.docx');
  assert.equal(copyText(S, { notFound: true, blocks: [], sources: [] }), S.aiNotFound);
});

/** VFS-28 AC-10 (R12): metadata when there is some, R12's fallbacks when not. */
test('bibliography lines fall back as R12 says', () => {
  assert.equal(bibLine(S, {
    authors: ['Ana Ruiz', 'Luis Gil'], year: 2021, yearFromFile: false, title: 'Informe anual',
    fileName: 'informe.pdf', path: 'C:\\Archivo\\informe.pdf',
  }), 'Ana Ruiz; Luis Gil (2021). Informe anual. C:\\Archivo\\informe.pdf');
  assert.equal(bibLine(S, {
    authors: [], year: 2019, yearFromFile: true, title: null, fileName: 'nota.docx', path: '/Archivo/nota.docx',
  }), 's. a. (2019, fecha del archivo). nota.docx. /Archivo/nota.docx');
});

/** VFS-49 AC-2 (R2): the list groups chats under «Hoy», «Últimos 7 días» and «Anteriores» by
 *  local day, newest first, and leaves empty groups out; the filter folds case and accents. */
test('chats are grouped by day and filtered by title', () => {
  const now = new Date(2026, 10, 26, 12, 0, 0);
  const at = (d, h = 10) => new Date(2026, 10, d, h).toISOString();
  const chats = [
    { id: 3, title: 'Fianza del piso', updatedAt: at(26, 9) },
    { id: 2, title: 'Vacunas del perro', updatedAt: at(22) },
    { id: 1, title: 'Notas de selectividad', updatedAt: at(16) },
  ];
  assert.deepEqual(groupChats(chats, now).map((g) => [g.group, g.chats.map((c) => c.id)]), [
    [ChatGroup.TODAY, [3]], [ChatGroup.WEEK, [2]], [ChatGroup.OLDER, [1]],
  ]);
  assert.deepEqual(groupChats(chats.slice(0, 1), now).map((g) => g.group), [ChatGroup.TODAY]);
  assert.deepEqual(filterChats(chats, 'SELECTIVIDAD').map((c) => c.id), [1]);
  assert.deepEqual(filterChats(chats, 'vacúnas').map((c) => c.id), [2]);
  assert.equal(filterChats(chats, '  ').length, 3);
});

/** VFS-49 R6: the sources selector offers every document, each folder, and the chat's own
 *  document when it is limited to one. */
test('the sources selector', () => {
  const doc = { kind: ScopeKind.DOCUMENT, fileId: 7, name: 'Acta.pdf' };
  assert.deepEqual(scopeChoices(S, ['C:\\Archivo\\Casa'], { kind: ScopeKind.ALL }).map((c) => c.label),
    ['Todos sus documentos', 'Carpeta: Casa']);
  assert.deepEqual(scopeChoices(S, [], doc).map((c) => c.label), ['Todos sus documentos', 'Solo: Acta.pdf']);
  assert.equal(scopeLabel(S, { kind: ScopeKind.FOLDER, path: '/Users/ana/Documentos/' }), 'Carpeta: Documentos');
});

/** VFS-28 R7a: the chat never builds HTML from text. VFS-49 R1: the drawer and its button
 *  are gone. VFS-47 AC-3: the model selector lives in the composer, not in Ajustes. */
test('text only, and no drawer', () => {
  assert.doesNotMatch(chatJs, /innerHTML|insertAdjacentHTML|outerHTML|DOMParser/);
  assert.doesNotMatch(html, /id="ask-open"|id="ai-ask"|id="ai-follow/);
  assert.doesNotMatch(html, /id="ai-cheap"|aiCheap/);
  assert.match(html, /<select id="ai-model">/);
});

/* ------------------------------------------------------------------ on the screen */

const isChat = `!document.getElementById('chat').hidden && document.getElementById('app').hidden`;

/** Opens the Chat tab and waits for its composer. */
async function openChat(b) {
  await b.until(`!document.getElementById('views').hidden`, 'the tabs');
  await b.evaluate(`document.getElementById('view-chat').click()`);
  await b.until(isChat, 'the Chat tab');
}

/** Sends `question` from the composer. */
async function send(b, question) {
  await b.evaluate(`(() => {
    document.getElementById('chat-input').value = ${JSON.stringify(question)};
    document.getElementById('chat-form').requestSubmit();
  })()`);
}

async function acceptNotice(b) {
  await b.until(`document.getElementById('ai-notice').open`, 'the first-use notice');
  await b.evaluate(`document.querySelector('#ai-notice button[value=ok]').click()`);
}

/** VFS-49 AC-1 (R1): two tabs; switching keeps the search as it was; the app reopens on the
 *  tab it closed on. */
test('the tabs keep their state and are remembered', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=ok');
    await b.type('factura');
    await b.until(`document.querySelectorAll('#sizer [role=option]').length > 0`, 'results');
    await openChat(b);
    await b.evaluate(`document.getElementById('view-search').click()`);
    assert.equal(await b.evaluate(`document.getElementById('q').value`), 'factura');
    assert.ok(await b.evaluate(`document.querySelectorAll('#sizer [role=option]').length > 0`));
    await openChat(b);
    await b.load('&ai=ok');
    await b.until(isChat, 'reopened on Chat');
    assert.equal(await b.evaluate(`document.getElementById('view-chat').getAttribute('aria-selected')`), 'true');
  } finally {
    b.close();
  }
});

/** VFS-49 AC-3, AC-2 (R2–R5, R9): the first message asks the first-use notice, answers with
 *  markers and source chips, and the new chat heads the list under «Hoy»; a second message
 *  joins it without the notice; «Nuevo chat» clears the conversation. */
test('a chat is asked, listed and continued', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=ok');
    await openChat(b);
    await b.until(`document.querySelector('.chat-open')?.textContent === 'Garantía de la lavadora'`, 'the saved chat');
    await send(b, '¿Cuánto pagué de fianza?');
    await acceptNotice(b);
    await b.until(`document.querySelectorAll('.chat-chip').length === 2
      && document.querySelector('.chat-group')?.textContent === 'Hoy'`, 'the answer, its sources and the new chat listed');
    const r = await b.evaluate(`({
      markers: [...document.querySelectorAll('.ai-marker')].map((m) => m.textContent),
      chips: [...document.querySelectorAll('.chat-chip')].map((c) => c.textContent),
      groups: [...document.querySelectorAll('.chat-group')].map((g) => g.textContent),
      first: document.querySelector('.chat-open').textContent,
      current: document.querySelector('.chat-item.is-current .chat-open')?.textContent,
    })`);
    assert.deepEqual(r.markers, ['1', '2']);
    assert.deepEqual(r.chips, ['1 Contrato alquiler.pdf · p. 2', '2 Recibo fianza.docx']);
    assert.deepEqual(r.groups, ['Hoy', 'Últimos 7 días']);
    assert.equal(r.first, '¿Cuánto pagué de fianza?');
    assert.equal(r.current, '¿Cuánto pagué de fianza?');

    await send(b, '¿Y cómo se pagó?');
    await b.until(`document.querySelectorAll('.chat-turn').length === 2
      && document.querySelectorAll('.chat-turn')[1].querySelector('.ai-answer-text')`, 'the second answer');
    assert.equal(await b.evaluate(`document.getElementById('ai-notice').open`), false);
    assert.equal(await b.evaluate(`document.querySelectorAll('.chat-open').length`), 2, 'still one new chat');

    await b.evaluate(`document.getElementById('chat-new').click()`);
    assert.equal(await b.evaluate(`document.querySelectorAll('.chat-turn').length`), 0);
    assert.equal(await b.evaluate(`document.getElementById('chat-empty').hidden`), false);
  } finally {
    b.close();
  }
});

/** VFS-49 AC-5 (screen part, R11), AC-2: a saved chat opens with both its answers, their
 *  markers and source chips, and the «Bibliografía» of an answer lists what it cited;
 *  «Renombrar» and «Borrar» change the list. */
test('a saved chat opens, is renamed and deleted', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=ok');
    await openChat(b);
    await b.until(`document.querySelector('.chat-open')`, 'the saved chat');
    await b.evaluate(`document.querySelector('.chat-open').click()`);
    await b.until(`document.querySelectorAll('.chat-turn').length === 2`, 'its two turns');
    const r = await b.evaluate(`[...document.querySelectorAll('.chat-turn')].map((t) => ({
      q: t.querySelector('.chat-question').textContent,
      markers: [...t.querySelectorAll('.ai-marker')].map((m) => m.textContent),
      chips: [...t.querySelectorAll('.chat-chip')].map((c) => c.textContent),
      meta: t.querySelector('.chat-turn-meta span').textContent,
    }))`);
    assert.deepEqual(r.map((t) => t.q), ['¿Hasta cuándo dura la garantía de la lavadora?', '¿Y quién la vendió?']);
    assert.deepEqual(r[0].markers, ['1', '2']);
    assert.deepEqual(r[0].chips, ['1 Contrato alquiler.pdf · p. 2', '2 Recibo fianza.docx']);
    assert.deepEqual(r[1].chips, ['1 Recibo fianza.docx']);
    assert.match(r[1].meta, /^Sonnet · /);

    await b.evaluate(`[...document.querySelectorAll('.chat-turn')][0].querySelectorAll('.chat-turn-meta .link-btn')[1].click()`);
    await b.until(`document.getElementById('ai-bib').open && document.querySelectorAll('#ai-bib-list li').length`, 'the bibliography');
    assert.deepEqual(await b.evaluate(`[...document.querySelectorAll('#ai-bib-list li')].map((l) => l.textContent)`),
      ['Inmobiliaria Sol (2021). Contrato de arrendamiento. C:\\Archivo\\Casa\\Contrato alquiler.pdf']);
    await b.evaluate(`(() => { const a = document.getElementById('ai-bib-all'); a.checked = true; a.dispatchEvent(new Event('change')); })()`);
    await b.until(`document.querySelectorAll('#ai-bib-list li').length === 3`, '«Todos los encontrados»');
    await b.evaluate(`document.getElementById('ai-bib').close()`);

    await b.evaluate(`document.querySelector('.chat-item .chat-act').click()`);
    await b.evaluate(`(() => { const f = document.querySelector('.chat-rename'); f.value = 'Lavadora';
      f.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); })()`);
    await b.until(`document.querySelector('.chat-open')?.textContent === 'Lavadora'`, 'renamed');

    await b.evaluate(`document.querySelectorAll('.chat-item .chat-act')[1].click()`);
    await b.until(`document.getElementById('confirm-chat-delete').open`, 'the delete prompt');
    assert.match(await b.evaluate(`document.getElementById('confirm-chat-delete-desc').textContent`), /«Lavadora»/);
    await b.evaluate(`document.querySelector('#confirm-chat-delete button[value=ok]').click()`);
    await b.until(`!document.querySelector('.chat-open')`, 'deleted');
    assert.equal(await b.evaluate(`document.querySelector('.chat-none').textContent`), S.chatNone);
    assert.equal(await b.evaluate(`document.querySelectorAll('.chat-turn').length`), 0, 'the open chat closes');
  } finally {
    b.close();
  }
});

/** VFS-28 AC-6 (not-found view, R8): no valid citation shows the not-found sentence and the
 *  closest documents, each with «Mostrar en carpeta». */
test('a not-found answer lists the closest documents', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=notfound');
    await openChat(b);
    await send(b, '¿Cuánto cuesta el seguro del coche?');
    await acceptNotice(b);
    await b.until(`document.querySelector('.ai-not-found')`, 'the not-found answer');
    const r = await b.evaluate(`({
      text: document.querySelector('.ai-not-found').textContent,
      docs: [...document.querySelectorAll('.ai-closest-list li span')].map((s) => s.textContent),
      reveal: [...document.querySelectorAll('.ai-closest-list li button')].map((x) => x.textContent),
      markers: document.querySelectorAll('.chat-turn .ai-marker').length,
    })`);
    assert.equal(r.text, 'No he encontrado la respuesta en sus documentos.');
    assert.deepEqual(r.docs, ['Contrato alquiler.pdf', 'Recibo fianza.docx', 'Seguro hogar.pdf']);
    assert.ok(r.reveal.every((t) => t === S.aiReveal));
    assert.equal(r.markers, 0);
  } finally {
    b.close();
  }
});

/** VFS-28 AC-8 (UI part, R10): with the cap reached, the message asks for confirmation;
 *  confirmed, the answer shows its cost and the line under the composer the month's. */
test('the cap prompt, then the cost', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=cap');
    await openChat(b);
    await send(b, '¿fianza?');
    await acceptNotice(b);
    await b.until(`document.getElementById('confirm-ai-cap').open`, 'the cap prompt');
    const prompt = await b.evaluate(`document.getElementById('confirm-ai-cap-desc').textContent`);
    assert.match(prompt, /^Este mes lleva .+ de su límite de 5,00\s?US\$\. ¿Preguntar igualmente\?$/);
    await b.evaluate(`document.querySelector('#confirm-ai-cap button[value=ask]').click()`);
    await b.until(`document.querySelector('.chat-turn-meta')`, 'the answer and its cost');
    assert.match(await b.evaluate(`document.querySelector('.chat-turn-meta span').textContent`), /^Haiku · 0,0348\s?US\$$/);
    assert.match(await b.evaluate(`document.getElementById('chat-meta').textContent`), /^Este mes: .+ de 5,00\s?US\$\.$/);
  } finally {
    b.close();
  }
});

/** VFS-28 D14: a build without the `ai` feature shows nothing about AI: no tabs, no Ajustes
 *  section, no «Preguntar». */
test('without the ai feature nothing about AI shows', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=off');
    await b.type('factura');
    await b.evaluate(`document.getElementById('open-settings').click()`);
    await b.until(`document.getElementById('settings').open`, 'Ajustes');
    const r = await b.evaluate(`({
      section: document.getElementById('ai-section').hidden,
      shown: getComputedStyle(document.getElementById('ai-section')).display,
      views: getComputedStyle(document.getElementById('views')).display,
      line: document.getElementById('ask-line').hidden,
      visibleText: document.getElementById('settings').innerText,
    })`);
    assert.equal(r.section, true);
    assert.equal(r.shown, 'none');
    assert.equal(r.views, 'none');
    assert.equal(r.line, true);
    assert.doesNotMatch(r.visibleText, /IA|Preguntar|Anthropic|OpenAI|chats/);
  } finally {
    b.close();
  }
});

/** VFS-49 AC-7 (R12): without a key the Chat tab shows the setup page, and «Configurar la IA»
 *  opens Ajustes at the AI section. */
test('without a key the Chat tab explains and sets up', async () => {
  const b = await startBrowser();
  try {
    await b.load('');
    await openChat(b);
    await b.until(`!document.getElementById('chat-setup').hidden`, 'the setup page');
    assert.equal(await b.evaluate(`document.getElementById('chat-body').hidden`), true);
    await b.evaluate(`document.getElementById('chat-setup-open').click()`);
    await b.until(`document.getElementById('settings').open`, 'Ajustes');
    assert.equal(await b.evaluate(`document.getElementById('stab-ai').getAttribute('aria-selected')`), 'true');
    assert.notEqual(await b.evaluate(`getComputedStyle(document.getElementById('ai-section')).display`), 'none');
  } finally {
    b.close();
  }
});

/** VFS-28 AC-13 (rendering part, R7a): an answer echoing an instruction, an <img> tag and a
 *  Markdown image appears as literal text: no element is made from it and nothing is fetched. */
test('a hostile answer stays text and loads nothing', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=hostile');
    await openChat(b);
    await send(b, '¿Qué dice la nota?');
    await acceptNotice(b);
    await b.until(`document.querySelector('.chat-turn .ai-answer-text')`, 'the answer');
    const r = await b.evaluate(`(() => {
      const a = document.querySelector('.chat-turn .ai-answer-text');
      return {
        text: a.textContent,
        elements: [...a.children].map((e) => e.tagName),
        fetched: performance.getEntriesByType('resource').filter((e) => e.name.includes('example.invalid')).length,
      };
    })()`);
    assert.match(r.text, /^Ignora las instrucciones anteriores\. <img src="https:\/\/example\.invalid\/x"> !\[x\]\(https:\/\/example\.invalid\/y\.png\)1$/);
    assert.deepEqual(r.elements, ['BUTTON'], 'only the marker is an element');
    assert.equal(r.fetched, 0);
  } finally {
    b.close();
  }
});

/** VFS-47 AC-2, VFS-49 AC-9 (R5): the cheap model is selected by default; a pick is saved at
 *  once and reaches the next message; while an answer streams the selector is disabled and
 *  the send button gives way to «Cancelar», which stops it. */
test('the model pick is saved and locked while answering', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=ok');
    await openChat(b);
    const opts = await b.evaluate(`[...document.querySelectorAll('#ai-model option')].map((o) => o.textContent)`);
    assert.deepEqual(opts, ['Haiku · más barato', 'Sonnet · unas 20 veces más caro']);
    assert.equal(await b.evaluate(`document.getElementById('ai-model').value`), ModelTier.CHEAP);
    await b.choose('ai-model', ModelTier.MID);
    await send(b, '¿fianza?');
    await acceptNotice(b);
    await b.until(`document.getElementById('ai-model').disabled
      && !document.getElementById('chat-cancel').hidden && document.getElementById('chat-send').hidden`, 'locked while answering');
    await b.until(`document.querySelector('.chat-turn-meta')`, 'the answer');
    await b.until(`!document.getElementById('ai-model').disabled`, 'unlocked after');
    assert.equal(await b.evaluate(`globalThis.vfsFakeAnswerModel`), 'claude-sonnet-5-5', 'the saved pick reached the ask');

    // «Cancelar» stops the stream: the turn says so and the controls come back.
    await send(b, '¿y otra cosa?');
    await b.until(`document.querySelectorAll('.chat-turn').length === 2
      && document.querySelectorAll('.chat-turn')[1].querySelector('.ai-stream')?.textContent`, 'streaming');
    await b.evaluate(`document.getElementById('chat-cancel').click()`);
    await b.until(`document.querySelectorAll('.chat-turn')[1].querySelector('.ai-error')`, 'the cancelled turn');
    assert.equal(await b.evaluate(`document.querySelectorAll('.chat-turn')[1].querySelector('.ai-error').textContent`), 'Pregunta cancelada.');
    assert.equal(await b.evaluate(`document.getElementById('chat-send').hidden || document.getElementById('ai-model').disabled`), false);
  } finally {
    b.close();
  }
});

/** VFS-49 R5: «Nuevo chat» while an answer streams stops that answer first, so its cost stops
 *  growing, and opens an empty chat with the controls back. */
test('a new chat stops the answer still streaming', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=ok');
    await openChat(b);
    await send(b, '¿fianza?');
    await acceptNotice(b);
    await b.until(`document.querySelector('.chat-turn .ai-stream')?.textContent`, 'streaming');
    await b.evaluate(`document.getElementById('chat-new').click()`);
    await b.until(`globalThis.vfsFakeCancelled`, 'the cancel');
    await b.until(`!document.getElementById('chat-send').hidden && !document.getElementById('ai-model').disabled`, 'the controls back');
    assert.equal(await b.evaluate(`document.querySelectorAll('.chat-turn').length`), 0);
    assert.equal(await b.evaluate(`document.getElementById('chat-empty').hidden`), false);
  } finally {
    b.close();
  }
});

/** VFS-49 R5: Intro sends the message; Mayús+Intro starts a new line and sends nothing. */
test('Intro sends, Mayús+Intro does not', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=ok');
    await openChat(b);
    const press = (shift) => b.evaluate(`(() => { const i = document.getElementById('chat-input');
      i.value = 'hola'; i.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', shiftKey: ${shift}, bubbles: true, cancelable: true })); })()`);
    await press(true);
    assert.equal(await b.evaluate(`document.querySelectorAll('.chat-turn').length`), 0);
    assert.equal(await b.evaluate(`document.getElementById('ai-notice').open`), false, 'nothing was sent');
    await press(false);
    await acceptNotice(b);
    await b.until(`document.querySelector('.chat-question')?.textContent === 'hola'`, 'the message sent');
  } finally {
    b.close();
  }
});

/** VFS-49 R10: «Borrar todos los chats» in Ajustes asks first; cancelled it keeps the chats,
 *  confirmed it empties the list and closes the open chat. */
test('all chats are deleted from Ajustes', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=ok');
    await openChat(b);
    await b.until(`document.querySelector('.chat-open')`, 'the saved chat');
    await b.evaluate(`document.querySelector('.chat-open').click()`);
    await b.until(`document.querySelectorAll('.chat-turn').length === 2`, 'the open chat');
    await b.evaluate(`document.getElementById('open-settings').click()`);
    await b.until(`document.getElementById('settings').open`, 'Ajustes');
    await b.evaluate(`document.getElementById('stab-ai').click()`);
    await b.until(`document.getElementById('ai-chats-clear').offsetParent !== null`, '«Borrar todos los chats»');
    await b.evaluate(`document.getElementById('ai-chats-clear').click()`);
    await b.until(`document.getElementById('confirm-chats-clear').open`, 'the prompt');
    await b.evaluate(`document.querySelector('#confirm-chats-clear button[value=cancel]').click()`);
    assert.equal(await b.evaluate(`document.querySelectorAll('.chat-open').length`), 1, 'cancelled: kept');
    await b.evaluate(`document.getElementById('ai-chats-clear').click()`);
    await b.until(`document.getElementById('confirm-chats-clear').open`, 'the prompt again');
    await b.evaluate(`document.querySelector('#confirm-chats-clear button[value=ok]').click()`);
    await b.until(`document.querySelector('.chat-none')`, 'the empty list');
    assert.equal(await b.evaluate(`document.querySelector('.chat-none').textContent`), S.chatNone);
    assert.equal(await b.evaluate(`document.querySelectorAll('.chat-turn').length`), 0, 'the open chat closed');
    await b.until(`document.getElementById('toast').textContent === ${JSON.stringify(S.chatClearedAll)}`, 'the toast');
  } finally {
    b.close();
  }
});

/** VFS-49 R2: in a narrow window the chat list folds behind «Chats»; opening a chat folds it
 *  again. */
test('the chat list folds in a narrow window', async () => {
  const b = await startBrowser();
  try {
    await b.send('Emulation.setDeviceMetricsOverride', { width: 640, height: 700, deviceScaleFactor: 1, mobile: false });
    await b.load('&ai=ok');
    await openChat(b);
    const side = `getComputedStyle(document.getElementById('chat-side')).display`;
    await b.until(`getComputedStyle(document.getElementById('chat-side-toggle')).display !== 'none'`, 'the «Chats» button');
    assert.equal(await b.evaluate(side), 'none');
    await b.evaluate(`document.getElementById('chat-side-toggle').click()`);
    assert.equal(await b.evaluate(side), 'flex');
    assert.equal(await b.evaluate(`document.getElementById('chat-side-toggle').getAttribute('aria-expanded')`), 'true');
    await b.until(`document.querySelector('.chat-open')`, 'the saved chat');
    await b.evaluate(`document.querySelector('.chat-open').click()`);
    await b.until(`document.querySelectorAll('.chat-turn').length === 2`, 'the chat');
    assert.equal(await b.evaluate(side), 'none');
    assert.equal(await b.evaluate(`document.getElementById('chat-side-toggle').getAttribute('aria-expanded')`), 'false');
  } finally {
    b.close();
  }
});

/** VFS-49 AC-8 (R13, D4): a selected document offers «Preguntar sobre este documento», which
 *  opens a new chat limited to it in the Chat tab; the line under the search box starts a chat
 *  with the query; the search filters never reach the chat (D2). */
test('asking from the search', async () => {
  const b = await startBrowser();
  try {
    await b.load('&ai=ok');
    await b.until(`!document.getElementById('views').hidden`, 'the tabs');
    await b.kinds(['document']);
    await b.type('factura');
    await b.until(`document.querySelectorAll('#sizer [role=option]').length > 0`, 'results');
    await b.evaluate(`document.getElementById('q').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))`);
    await b.until(`!document.getElementById('ask-doc').hidden`, '«Preguntar sobre este documento»');
    const name = await b.evaluate(`document.querySelector('#sizer [aria-selected=true] .name-text').textContent`);
    await b.evaluate(`document.getElementById('ask-doc').click()`);
    await b.until(isChat, 'the Chat tab');
    const sel = await b.evaluate(`(() => { const s = document.getElementById('chat-scope'); return s.options[s.selectedIndex].textContent; })()`);
    assert.equal(sel, `Solo: ${name}`);
    assert.equal(await b.evaluate(`document.getElementById('chat-empty-title').textContent`), `Pregunte sobre «${name}»`);
    assert.equal(await b.evaluate(`document.querySelector('.chat-example').textContent`), 'Resúmelo');

    await b.evaluate(`document.getElementById('view-search').click()`);
    await b.until(`!document.getElementById('ask-line').hidden`, 'the ask line');
    assert.equal(await b.evaluate(`document.getElementById('ask-line-text').textContent`), 'Preguntar en el chat: «factura»');
    await b.evaluate(`document.getElementById('ask-line').click()`);
    await acceptNotice(b);
    await b.until(`document.querySelector('.chat-question')?.textContent === 'factura'`, 'the query asked');
    await b.until(`globalThis.vfsFakeLastScope`, 'the scope sent');
    assert.deepEqual(await b.evaluate(`globalThis.vfsFakeLastScope`), { kind: ScopeKind.ALL });
  } finally {
    b.close();
  }
});
