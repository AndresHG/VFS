// The Chat tab (VFS-49): the chats on the left, a threaded conversation that always answers
// from the user's documents, the composer with the sources and model selectors, and the setup
// page while AI cannot answer. Answers are drawn as plain text with numbered markers (VFS-28
// R7a, R8): nothing here builds HTML from answer, passage or title text; every string goes
// through textContent or a text node.

import { AiErrorCode, KeyState, Provider } from './contract.js';
import { aiErrorText, formatUsd, providerName } from './ai.js';

/** The chat can answer: AI on, a key saved and a content store (VFS-28 R5, VFS-49 R12). */
export function chatReady(status) {
  return Boolean(status?.settings.enabled && status.key.state === KeyState.SAVED && status.store);
}

/** VFS-47: the answer tier the model selector picks. UI-only (the shell stores it as
 *  `AiSettings.cheap`), so it lives here, not in contract.js, which mirrors Rust enums. */
export const ModelTier = Object.freeze({ CHEAP: 'cheap', MID: 'mid' });

/** Each provider's answer models by tier, as the providers name them (VFS-47 D2). */
export const MODEL_NAMES = Object.freeze({
  [Provider.ANTHROPIC]: Object.freeze(['Haiku', 'Sonnet']),
  [Provider.OPENAI]: Object.freeze(['Luna', 'Sol']),
});

/** The answer models the selector offers (VFS-47 R1, D2), cheap first, each with its tier.
 *  Names are the providers' own; the hints come from the catalog. */
export function modelChoices(S, provider) {
  const [cheap, mid] = MODEL_NAMES[provider === Provider.OPENAI ? Provider.OPENAI : Provider.ANTHROPIC];
  return [
    { tier: ModelTier.CHEAP, label: S.aiModelCheap(cheap) },
    { tier: ModelTier.MID, label: S.aiModelMid(mid) },
  ];
}

/** `Scope.kind` of the shell's `vfs_ai::ask::Scope` (VFS-49 R6), a tagged object:
 *  `{kind: 'all'}`, `{kind: 'folder', path}` or `{kind: 'document', fileId, name}`. */
export const ScopeKind = Object.freeze({ ALL: 'all', FOLDER: 'folder', DOCUMENT: 'document' });

/** «claude-haiku-5-5» → «Haiku»: an answer's model as the selector names it; any other id as
 *  it came (VFS-49 R8). */
export function modelName(id) {
  const known = Object.values(MODEL_NAMES).flat();
  return known.find((n) => String(id).toLowerCase().includes(n.toLowerCase())) || id;
}

/** What the sources selector says for `scope` (VFS-49 R6). */
export function scopeLabel(S, scope) {
  switch (scope?.kind) {
    case ScopeKind.FOLDER: return S.chatScopeFolder(lastSegment(scope.path));
    case ScopeKind.DOCUMENT: return S.chatScopeOnly(scope.name);
    default: return S.chatScopeAll;
  }
}

function lastSegment(path) {
  const parts = String(path).split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] || path;
}

/** The selector's choices (VFS-49 R6): every document, each of the user's folders, and the
 *  chat's own document when it is limited to one. */
export function scopeChoices(S, folders, current) {
  const choices = [{ kind: ScopeKind.ALL }, ...folders.map((path) => ({ kind: ScopeKind.FOLDER, path }))];
  if (current?.kind === ScopeKind.DOCUMENT) choices.push(current);
  if (current?.kind === ScopeKind.FOLDER && !folders.includes(current.path)) choices.push(current);
  return choices.map((scope) => ({ scope, label: scopeLabel(S, scope) }));
}

export function sameScope(a, b) {
  if ((a?.kind ?? ScopeKind.ALL) !== (b?.kind ?? ScopeKind.ALL)) return false;
  if (a?.kind === ScopeKind.FOLDER) return a.path === b.path;
  if (a?.kind === ScopeKind.DOCUMENT) return a.fileId === b.fileId;
  return true;
}

/** Text compared after case and accent folding, for the chat list filter (VFS-49 R2). */
function fold(text) {
  return String(text).normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
}

/** The chats whose title contains `text`, folded; all of them for a blank filter. */
export function filterChats(chats, text) {
  const needle = fold(text.trim());
  return needle ? chats.filter((c) => fold(c.title).includes(needle)) : chats;
}

/** `ChatGroup`: the list's headings (VFS-49 R2). */
export const ChatGroup = Object.freeze({ TODAY: 'today', WEEK: 'week', OLDER: 'older' });

/** The chats under «Hoy», «Últimos 7 días» and «Anteriores», by their last activity in local
 *  time, in the order given (newest first); empty groups are left out. */
export function groupChats(chats, now = new Date()) {
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const weekAgo = startOfToday - 6 * 24 * 3600 * 1000;
  const groups = new Map([[ChatGroup.TODAY, []], [ChatGroup.WEEK, []], [ChatGroup.OLDER, []]]);
  for (const c of chats) {
    const t = Date.parse(c.updatedAt);
    const g = t >= startOfToday ? ChatGroup.TODAY : t >= weekAgo ? ChatGroup.WEEK : ChatGroup.OLDER;
    groups.get(g).push(c);
  }
  return [...groups].filter(([, list]) => list.length).map(([group, list]) => ({ group, chats: list }));
}

/** A passage's title as the model saw it (VFS-28 D11): the file name and its page or pages. */
export function sourceTitle(S, source) {
  const p = source.pages;
  if (!p.length) return source.name;
  if (p.length === 1) return S.aiPassageTitle(source.name, p[0]);
  return S.aiPagesTitle(source.name, p[0], p[p.length - 1]);
}

/** An answer as runs: `{text}`, and after each block its markers, each passage once (R8). */
export function answerRuns(blocks) {
  const runs = [];
  for (const b of blocks) {
    if (b.text) runs.push({ text: b.text });
    const seen = new Set();
    for (const c of b.citations) {
      if (seen.has(c.marker)) continue;
      seen.add(c.marker);
      runs.push({ marker: c.marker, passage: c.passage });
    }
  }
  return runs;
}

/** The cited sources of an answer, once each, by marker: the chips under it (VFS-49 R8). */
export function citedSources(turn) {
  const cited = new Map();
  for (const c of turn.blocks.flatMap((b) => b.citations)) {
    if (!cited.has(c.marker) && turn.sources[c.passage]) cited.set(c.marker, turn.sources[c.passage]);
  }
  return [...cited].sort(([a], [b]) => a - b).map(([marker, source]) => ({ marker, source }));
}

/** «Copiar» (VFS-28 R9): the answer with its markers, then its numbered sources. */
export function copyText(S, turn) {
  if (turn.notFound) return S.aiNotFound;
  const text = answerRuns(turn.blocks).map((r) => (r.text ?? ` [${r.marker}]`)).join('');
  const lines = citedSources(turn).map(({ marker, source }) => `[${marker}] ${sourceTitle(S, source)}`);
  return lines.length ? `${text.trim()}\n\n${S.aiSources}\n${lines.join('\n')}` : text.trim();
}

/** One bibliography line (VFS-28 R12): `Autor (Año). Título. Ruta`, with «s. a.», the file
 *  name and «(fecha del archivo)» standing in for what the metadata lacks. */
export function bibLine(S, e) {
  const author = e.authors.length ? e.authors.join('; ') : S.aiAnon;
  const year = e.yearFromFile ? `${e.year}, ${S.aiFileDate}` : String(e.year);
  return `${author} (${year}). ${e.title || e.fileName}. ${e.path}`;
}

/**
 * Wires the Chat tab. `aiStatus()` gives the latest AiStatus, `updateAi(status)` hands a newer
 * one to Ajustes and `refreshAi()` reads it again; `folders()` the user's folder paths;
 * `showChat()` brings the Chat tab forward; `openAiSettings()` opens Ajustes at the AI section.
 * Returns `setStatus(status)` for Ajustes to call, `newChat(scope)`, `askNew(text)` and
 * `askAboutDocument(fileId, name)`.
 */
export function setupChat({
  api, S, lang, $, el, aiStatus, updateAi, refreshAi, setCheap, revealFile, toast, folders,
  showChat, openAiSettings,
}) {
  const input = $('chat-input');
  const form = $('chat-form');
  const turnsEl = $('chat-turns');
  const model = $('ai-model');
  const scopeSel = $('chat-scope');
  const send = $('chat-send');
  const cancel = $('chat-cancel');
  let askId = 0;
  let running = 0; // the ask id in flight, 0 when idle
  let stream = null; // the element the streamed text goes into
  let chats = [];
  let current = { id: null, scope: { kind: ScopeKind.ALL }, turns: [] };
  let choices = [];

  api.onAnswerText(({ askId: id, text }) => {
    if (id === running && stream) stream.textContent += text;
  }).catch(() => {});

  /* ------------------------------------------------------------ status and setup page */

  function setStatus(status) {
    const ready = chatReady(status);
    $('chat-setup').hidden = ready;
    $('chat-body').hidden = !ready;
    if (!status) return;
    $('chat-setup-store').hidden = Boolean(status.store);
    // VFS-47 R1, R2, R5: the active provider's models, the saved tier selected.
    model.replaceChildren(...modelChoices(S, status.settings.provider).map((c) => {
      const o = el('option', null, c.label);
      o.value = c.tier;
      return o;
    }));
    model.value = status.settings.cheap ? ModelTier.CHEAP : ModelTier.MID;
  }

  model.addEventListener('change', () => setCheap(model.value === ModelTier.CHEAP));
  $('chat-setup-open').addEventListener('click', () => openAiSettings());

  /* ------------------------------------------------------------ the chat list */

  async function loadChats() {
    try {
      chats = await api.aiChats();
    } catch {
      chats = [];
    }
    drawList();
  }

  function drawList() {
    const list = $('chat-list');
    const shown = filterChats(chats, $('chat-filter').value);
    const heading = { [ChatGroup.TODAY]: S.chatToday, [ChatGroup.WEEK]: S.chatWeek, [ChatGroup.OLDER]: S.chatOlder };
    const parts = [];
    for (const { group, chats: items } of groupChats(shown)) {
      parts.push(el('h3', 'chat-group', heading[group]));
      const ul = el('ul', 'chat-items');
      ul.setAttribute('role', 'list');
      for (const c of items) ul.append(chatItem(c));
      parts.push(ul);
    }
    if (!parts.length) parts.push(el('p', 'chat-none', chats.length ? S.chatNoMatch : S.chatNone));
    list.replaceChildren(...parts);
  }

  function chatItem(c) {
    const li = el('li', 'chat-item');
    if (c.id === current.id) li.classList.add('is-current');
    const open = el('button', 'chat-open', c.title);
    open.type = 'button';
    if (c.id === current.id) open.setAttribute('aria-current', 'true');
    open.addEventListener('click', () => openChat(c.id));
    const rename = el('button', 'icon-btn chat-act');
    rename.type = 'button';
    rename.setAttribute('aria-label', S.chatRenameLabel(c.title));
    rename.title = S.chatRename;
    rename.append(iconUse('i-edit'));
    rename.addEventListener('click', () => startRename(li, c));
    const del = el('button', 'icon-btn chat-act');
    del.type = 'button';
    del.setAttribute('aria-label', S.chatDeleteLabel(c.title));
    del.title = S.chatDelete;
    del.append(iconUse('i-trash'));
    del.addEventListener('click', () => deleteChat(c));
    li.append(open, rename, del);
    return li;
  }

  function iconUse(id) {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('aria-hidden', 'true');
    const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
    use.setAttribute('href', `#${id}`);
    svg.append(use);
    return svg;
  }

  /** «Renombrar» in place (VFS-49 R2): Intro saves, Escape keeps the old title. */
  function startRename(li, c) {
    const field = el('input', 'chat-rename');
    field.value = c.title;
    field.maxLength = 120;
    field.setAttribute('aria-label', S.chatRename);
    li.replaceChildren(field);
    field.focus();
    field.select();
    let done = false;
    const finish = async (save) => {
      if (done) return;
      done = true;
      if (save && field.value.trim() && field.value.trim() !== c.title) {
        try { await api.aiChatRename(c.id, field.value); } catch (err) { toast(aiErrorText(S, err)); }
      }
      await loadChats();
    };
    field.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') { e.preventDefault(); finish(true); }
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); finish(false); }
    });
    field.addEventListener('blur', () => finish(true));
  }

  function confirm(dialogId, text) {
    const d = $(dialogId);
    if (text !== undefined) $(`${dialogId}-desc`).textContent = text;
    d.returnValue = '';
    d.showModal();
    return new Promise((resolve) => {
      d.addEventListener('close', () => resolve(d.returnValue === 'ok'), { once: true });
    });
  }

  async function deleteChat(c) {
    if (!(await confirm('confirm-chat-delete', S.chatDeleteBody(c.title)))) return;
    try { await api.aiChatDelete(c.id); } catch (err) { toast(aiErrorText(S, err)); return; }
    if (current.id === c.id) newChat();
    await loadChats();
  }

  $('chat-filter').addEventListener('input', drawList);
  // A message still streaming is stopped first: a new chat never runs an old one's answer.
  $('chat-new').addEventListener('click', () => {
    if (running) api.aiCancel(running).catch(() => {});
    newChat();
    input.focus();
  });
  $('chat-side-toggle').addEventListener('click', () => {
    const open = $('chat').classList.toggle('side-open');
    $('chat-side-toggle').setAttribute('aria-expanded', String(open));
  });

  /** «Borrar todos los chats» in Ajustes (VFS-49 R10). */
  $('ai-chats-clear').addEventListener('click', async () => {
    if (!(await confirm('confirm-chats-clear'))) return;
    try { await api.aiChatsClear(); } catch (err) { toast(aiErrorText(S, err)); return; }
    newChat();
    await loadChats();
    toast(S.chatClearedAll);
  });

  /* ------------------------------------------------------------ one chat */

  function drawScope() {
    choices = scopeChoices(S, folders(), current.scope);
    scopeSel.replaceChildren(...choices.map((c, i) => {
      const o = el('option', null, c.label);
      o.value = String(i);
      return o;
    }));
    scopeSel.value = String(Math.max(0, choices.findIndex((c) => sameScope(c.scope, current.scope))));
  }
  scopeSel.addEventListener('change', () => {
    current.scope = choices[Number(scopeSel.value)]?.scope ?? { kind: ScopeKind.ALL };
    if (!current.turns.length) drawEmpty();
  });

  function drawEmpty() {
    const doc = current.scope.kind === ScopeKind.DOCUMENT;
    $('chat-empty').hidden = current.turns.length > 0;
    $('chat-empty-title').textContent = doc ? S.chatDocTitle(current.scope.name) : S.chatEmptyTitle;
    $('chat-empty-body').textContent = S.chatEmptyBody;
    const examples = doc ? S.chatDocExamples : S.chatExamples;
    $('chat-examples').replaceChildren(...examples.map((text) => {
      const b = el('button', 'chat-example', text);
      b.type = 'button';
      b.addEventListener('click', () => { input.value = text; input.focus(); });
      return b;
    }));
  }

  function newChat(scope = { kind: ScopeKind.ALL }) {
    current = { id: null, scope, turns: [] };
    turnsEl.replaceChildren();
    input.value = '';
    $('chat-meta').textContent = '';
    drawScope();
    drawEmpty();
    drawList();
  }

  async function openChat(id) {
    if (running) return;
    try {
      const chat = await api.aiChat(id);
      current = { id: chat.id, scope: chat.scope, turns: chat.turns };
    } catch (err) {
      toast(aiErrorText(S, err, aiStatus()?.settings.provider));
      await loadChats();
      return;
    }
    turnsEl.replaceChildren(...current.turns.map((t, ord) => {
      const item = questionItem(t.question);
      drawAnswer(item.body, t, ord);
      return item.li;
    }));
    $('chat').classList.remove('side-open');
    drawScope();
    drawEmpty();
    drawList();
    turnsEl.lastElementChild?.scrollIntoView({ block: 'start' });
  }

  function questionItem(question) {
    const li = el('li', 'chat-turn');
    const body = el('div', 'chat-answer');
    li.append(el('p', 'chat-question', question), body);
    return { li, body };
  }

  function sourcePanel(source) {
    const box = el('div', 'ai-source');
    const reveal = el('button', 'btn btn-quiet', S.aiReveal);
    reveal.type = 'button';
    reveal.addEventListener('click', () => revealFile(source.fileId, false));
    box.append(el('p', 'ai-source-title', sourceTitle(S, source)), el('p', 'ai-source-text', source.text), reveal);
    return box;
  }

  /** Draws an answered turn (VFS-49 R8): plain text with marker buttons, a chip per cited
   *  source, then the model and cost, «Copiar» and «Bibliografía». */
  function drawAnswer(target, turn, ord) {
    const detail = el('div', 'ai-source-slot');
    const parts = [];
    if (turn.notFound) {
      parts.push(el('p', 'ai-not-found', S.aiNotFound));
      if (turn.documents.length) {
        parts.push(el('p', 'ai-closest', S.aiClosest));
        const list = el('ul', 'ai-closest-list');
        for (const d of turn.documents.slice(0, 5)) {
          const li = el('li');
          const reveal = el('button', 'btn btn-quiet', S.aiReveal);
          reveal.type = 'button';
          reveal.addEventListener('click', () => revealFile(d.fileId, false));
          li.append(el('span', null, d.name), reveal);
          list.append(li);
        }
        parts.push(list);
      }
    } else {
      const p = el('p', 'ai-answer-text');
      for (const r of answerRuns(turn.blocks)) {
        if (r.text !== undefined) {
          p.append(document.createTextNode(r.text));
          continue;
        }
        const source = turn.sources[r.passage];
        const m = el('button', 'ai-marker', String(r.marker));
        m.type = 'button';
        m.title = `${sourceTitle(S, source)}\n${source.text}`;
        m.setAttribute('aria-label', S.aiMarkerLabel(r.marker, sourceTitle(S, source)));
        m.addEventListener('click', () => detail.replaceChildren(sourcePanel(source)));
        p.append(m);
      }
      parts.push(p);
      const chips = el('div', 'chat-chips');
      for (const { marker, source } of citedSources(turn)) {
        const chip = el('button', 'chat-chip');
        chip.type = 'button';
        chip.append(el('b', null, String(marker)), ` ${sourceTitle(S, source)}`);
        chip.addEventListener('click', () => detail.replaceChildren(sourcePanel(source)));
        chips.append(chip);
      }
      if (chips.childElementCount) parts.push(chips);
    }
    const meta = el('div', 'chat-turn-meta');
    meta.append(el('span', null, S.chatTurnCost(modelName(turn.model), formatUsd(turn.costNano, lang))));
    if (!turn.notFound) {
      const copy = el('button', 'link-btn', S.chatCopy);
      copy.type = 'button';
      copy.addEventListener('click', async () => {
        try {
          await navigator.clipboard.writeText(copyText(S, turn));
          toast(S.aiCopied);
        } catch { /* clipboard refused */ }
      });
      meta.append(copy);
    }
    const bib = el('button', 'link-btn', S.aiBibOpen);
    bib.type = 'button';
    bib.addEventListener('click', () => openBib(ord));
    meta.append(bib);
    parts.push(detail, meta);
    target.replaceChildren(...parts);
  }


  /* ------------------------------------------------------------ asking */

  function busy(id) {
    running = id;
    cancel.hidden = !id;
    send.hidden = Boolean(id);
    model.disabled = Boolean(id); // VFS-47 R4
    scopeSel.disabled = Boolean(id);
  }

  /** Asks the user once, the first time ever (VFS-28 R6, VFS-49 R9). */
  function noticeAccepted() {
    if (aiStatus()?.settings.noticeSeen) return Promise.resolve(true);
    const d = $('ai-notice');
    $('ai-notice-body').textContent = S.aiNoticeBody(providerName(S, aiStatus()?.settings.provider));
    d.returnValue = '';
    d.showModal();
    return new Promise((resolve) => {
      d.addEventListener('close', async () => {
        if (d.returnValue !== 'ok') { resolve(false); return; }
        try { updateAi(await api.aiNoticeSeen()); } catch { /* asked again next time */ }
        resolve(true);
      }, { once: true });
    });
  }

  /** The cap prompt (VFS-28 R10): resolves true when the user asks anyway. */
  function capConfirmed() {
    const s = aiStatus();
    const d = $('confirm-ai-cap');
    $('confirm-ai-cap-desc').textContent = S.aiCapAsk(formatUsd(s?.monthNano ?? 0, lang), formatUsd(s?.capNano ?? 0, lang));
    d.returnValue = '';
    d.showModal();
    return new Promise((resolve) => {
      d.addEventListener('close', () => resolve(d.returnValue === 'ask'), { once: true });
    });
  }

  async function ask(question) {
    if (!question || running || !chatReady(aiStatus()) || !(await noticeAccepted())) return;
    input.value = '';
    $('chat-empty').hidden = true;
    const item = questionItem(question);
    stream = el('p', 'ai-stream');
    item.body.replaceChildren(el('p', 'ai-status', S.aiAsking), stream);
    turnsEl.append(item.li);
    item.li.scrollIntoView({ block: 'start' });
    const id = ++askId;
    busy(id);
    const chatId = current.id;
    const scope = current.scope;
    try {
      let confirmed = false;
      for (;;) {
        try {
          const sent = await api.aiChatSend(id, chatId, scope, question, confirmed, lang);
          if (current.id !== chatId || !item.li.isConnected) return; // the user moved on
          current.id = sent.chatId;
          current.turns.push(sent.turn);
          drawAnswer(item.body, sent.turn, current.turns.length - 1);
          $('chat-meta').textContent = S.chatMonth(formatUsd(sent.monthNano, lang), formatUsd(sent.capNano, lang));
          refreshAi(); // the month's spend, for the next cap prompt
          await loadChats();
          return;
        } catch (err) {
          if (err?.code === AiErrorCode.CAP_REACHED && !confirmed && await capConfirmed()) {
            confirmed = true;
            stream.textContent = '';
            continue;
          }
          throw err;
        }
      }
    } catch (err) {
      item.body.replaceChildren(el('p', 'ai-error', aiErrorText(S, err, aiStatus()?.settings.provider)));
      if (err?.code === AiErrorCode.NO_CHAT) { await loadChats(); }
    } finally {
      busy(0);
      stream = null;
    }
  }

  form.addEventListener('submit', (e) => {
    e.preventDefault();
    ask(input.value.trim());
  });
  // VFS-49 R5: Intro sends, Mayús+Intro starts a new line.
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      form.requestSubmit();
    }
  });
  cancel.addEventListener('click', () => { if (running) api.aiCancel(running).catch(() => {}); });

  /* ------------------------------------------------------------ «Bibliografía» (VFS-28 R12) */

  const bibDialog = $('ai-bib');
  let bibOrd = 0;
  let entries = [];
  async function drawBib() {
    try {
      entries = await api.aiChatBibliography(current.id, bibOrd, $('ai-bib-all').checked);
    } catch (err) {
      entries = [];
      toast(aiErrorText(S, err, aiStatus()?.settings.provider));
    }
    $('ai-bib-empty').hidden = entries.length > 0;
    $('ai-bib-list').replaceChildren(...entries.map((e) => el('li', 'ai-bib-entry', bibLine(S, e))));
  }
  async function openBib(ord) {
    if (current.id == null) return;
    bibOrd = ord;
    $('ai-bib-all').checked = false;
    await drawBib();
    bibDialog.showModal();
  }
  $('ai-bib-all').addEventListener('change', drawBib);
  $('ai-bib-copy').addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(entries.map((e) => bibLine(S, e)).join('\n'));
      toast(S.aiBibCopied);
    } catch { /* clipboard refused */ }
  });
  $('ai-bib-export').addEventListener('click', async () => {
    try {
      const path = await api.aiChatExportRis(current.id, bibOrd, $('ai-bib-all').checked);
      if (path) toast(S.aiBibSaved(path));
    } catch (err) {
      toast(aiErrorText(S, err, aiStatus()?.settings.provider));
    }
  });

  /* ------------------------------------------------------------ from the search (R13) */

  /** A new chat limited to one document, in the Chat tab (VFS-49 R13, D4). */
  function askAboutDocument(fileId, name) {
    if (running) return;
    showChat();
    newChat({ kind: ScopeKind.DOCUMENT, fileId, name });
    input.focus();
  }

  /** A new chat over every document whose first message is `text` (VFS-49 R13). */
  function askNew(text) {
    if (running) return;
    showChat();
    newChat();
    ask(text);
  }

  newChat();
  loadChats();
  return { setStatus, newChat, askAboutDocument, askNew, refreshScopes: drawScope, focus: () => input.focus() };
}
