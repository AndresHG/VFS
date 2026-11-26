// «Preguntar» (VFS-28), PR 28d-2: the first-use notice, the question, the streamed answer
// drawn as plain text with numbered markers (R7a, R8), the cap prompt (R10), follow-ups (R9),
// «Copiar respuesta» and «Bibliografía» (R12). Nothing here builds HTML from answer or passage
// text: every string goes through textContent or a text node.

import { AiErrorCode, KeyState, Provider } from './contract.js';
import { aiErrorText, formatUsd, providerName } from './ai.js';

/** «Preguntar» shows only while AI is on, a key is saved and content search has a store (R5). */
export function askVisible(status) {
  return Boolean(status?.settings.enabled && status.key.state === KeyState.SAVED && status.store);
}

/** VFS-47: the answer tier the selector beside «Preguntar» picks. UI-only (the shell stores it
 *  as `AiSettings.cheap`), so it lives here, not in contract.js, which mirrors Rust enums. */
export const ModelTier = Object.freeze({ CHEAP: 'cheap', MID: 'mid' });

/** The answer models the selector offers (VFS-47 R1, D2), cheap first, each with its tier. Names are the providers' own; the hints come from the catalog. */
export function modelChoices(S, provider) {
  const [cheap, mid] = provider === Provider.OPENAI ? ['Luna', 'Sol'] : ['Haiku', 'Sonnet'];
  return [
    { tier: ModelTier.CHEAP, label: S.aiModelCheap(cheap) },
    { tier: ModelTier.MID, label: S.aiModelMid(mid) },
  ];
}

/** A passage's title as the model saw it (D11): the file name and its page or pages. */
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

/** «Copiar respuesta» (R9): the answer with its markers, then its numbered sources. */
export function copyText(S, reply, sources) {
  if (reply.notFound) return S.aiNotFound;
  const text = answerRuns(reply.blocks).map((r) => (r.text ?? ` [${r.marker}]`)).join('');
  const cited = new Map();
  for (const c of reply.blocks.flatMap((b) => b.citations)) {
    if (!cited.has(c.marker)) cited.set(c.marker, sources[c.passage]);
  }
  const lines = [...cited].sort(([a], [b]) => a - b).map(([m, s]) => `[${m}] ${sourceTitle(S, s)}`);
  return lines.length ? `${text.trim()}\n\n${S.aiSources}\n${lines.join('\n')}` : text.trim();
}

/** One bibliography line (R12): `Autor (Año). Título. Ruta`, with «s. a.», the file name and
 *  «(fecha del archivo)» standing in for what the metadata lacks. */
export function bibLine(S, e) {
  const author = e.authors.length ? e.authors.join('; ') : S.aiAnon;
  const year = e.yearFromFile ? `${e.year}, ${S.aiFileDate}` : String(e.year);
  return `${author} (${year}). ${e.title || e.fileName}. ${e.path}`;
}

/**
 * Wires «Preguntar». `filters()` gives the filters on screen; `aiStatus()` the latest AiStatus,
 * `updateAi(status)` hands a newer one to the settings panel and `refreshAi()` reads it again.
 * Returns `setStatus(status)` for the settings panel to call.
 */
export function setupAsk({ api, S, lang, $, el, filters, aiStatus, updateAi, refreshAi, setCheap, revealFile, toast }) {
  const dialog = $('ai-ask');
  const form = $('ai-ask-form');
  const input = $('ai-question');
  const turns = $('ai-turns');
  const cancel = $('ai-cancel');
  const model = $('ai-model');
  let askId = 0;
  let running = 0; // the ask id in flight, 0 when idle
  let thread = null; // {id, sources, documents}
  let last = null; // the latest reply
  let stream = null; // the element the streamed text goes into

  api.onAnswerText(({ askId: id, text }) => {
    if (id === running && stream) stream.textContent += text;
  }).catch(() => {});

  function setStatus(status) {
    $('ask-open').hidden = !askVisible(status);
    if (!status) return;
    // VFS-47 R1, R2, R5: the active provider's models, the saved tier selected.
    model.replaceChildren(...modelChoices(S, status.settings.provider).map((c) => {
      const o = el('option', null, c.label);
      o.value = c.tier;
      return o;
    }));
    model.value = status.settings.cheap ? ModelTier.CHEAP : ModelTier.MID;
  }

  model.addEventListener('change', () => setCheap(model.value === ModelTier.CHEAP));

  function busy(id) {
    running = id;
    cancel.hidden = !id;
    for (const b of form.querySelectorAll('button[type="submit"]')) b.disabled = Boolean(id);
    $('ai-follow-submit').disabled = Boolean(id);
    model.disabled = Boolean(id); // VFS-47 R4
  }

  /** Asks the user once, the first time ever (R6). */
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

  /** The cap prompt (R10): resolves true when the user asks anyway. */
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

  function sourcePanel(source) {
    const box = el('div', 'ai-source');
    const reveal = el('button', 'btn btn-quiet', S.aiReveal);
    reveal.type = 'button';
    reveal.addEventListener('click', () => revealFile(source.fileId, false));
    box.append(el('p', 'ai-source-title', sourceTitle(S, source)), el('p', 'ai-source-text', source.text), reveal);
    return box;
  }

  /** Draws a reply as plain text with marker buttons (R7a, R8). */
  function drawReply(target, reply) {
    const answer = el('div', 'ai-answer');
    const detail = el('div', 'ai-source-slot');
    if (reply.notFound) {
      answer.append(el('p', 'ai-not-found', S.aiNotFound));
      if (thread.documents.length) {
        answer.append(el('p', 'ai-closest', S.aiClosest));
        const list = el('ul', 'ai-closest-list');
        for (const d of thread.documents.slice(0, 5)) {
          const li = el('li');
          const reveal = el('button', 'btn btn-quiet', S.aiReveal);
          reveal.type = 'button';
          reveal.addEventListener('click', () => revealFile(d.fileId, false));
          li.append(el('span', null, d.name), reveal);
          list.append(li);
        }
        answer.append(list);
      }
    } else {
      const p = el('p', 'ai-answer-text');
      for (const r of answerRuns(reply.blocks)) {
        if (r.text !== undefined) {
          p.append(document.createTextNode(r.text));
          continue;
        }
        const source = thread.sources[r.passage];
        const m = el('button', 'ai-marker', `[${r.marker}]`);
        m.type = 'button';
        m.title = `${sourceTitle(S, source)}\n${source.text}`;
        m.setAttribute('aria-label', S.aiMarkerLabel(r.marker, sourceTitle(S, source)));
        m.addEventListener('click', () => detail.replaceChildren(sourcePanel(source)));
        p.append(m);
      }
      answer.append(p);
    }
    target.replaceChildren(answer, detail,
      el('p', 'ai-cost', S.aiCost(formatUsd(reply.costNano, lang), formatUsd(reply.monthNano, lang), formatUsd(reply.capNano, lang))));
  }

  /** Runs one question or follow-up in a new turn of the drawer. */
  async function turn(question, call) {
    const item = el('li', 'ai-turn');
    const body = el('div', 'ai-turn-body');
    stream = el('p', 'ai-stream');
    body.append(el('p', 'ai-status', S.aiAsking), stream);
    item.append(el('p', 'ai-turn-question', question), body);
    turns.append(item);
    const id = ++askId;
    busy(id);
    try {
      let confirmed = false;
      for (;;) {
        try {
          const reply = await call(id, confirmed);
          drawReply(body, reply);
          last = reply;
          refreshAi(); // the month's spend, for the next cap prompt (R10)
          $('ai-after').hidden = false;
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
      body.replaceChildren(el('p', 'ai-error', aiErrorText(S, err, aiStatus()?.settings.provider)));
    } finally {
      busy(0);
      stream = null;
    }
  }

  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const question = input.value.trim();
    if (!question || running || !(await noticeAccepted())) return;
    thread = null;
    last = null;
    turns.replaceChildren();
    $('ai-after').hidden = true;
    await turn(question, async (id, confirmed) => {
      const view = await api.aiAsk(id, question, filters(), confirmed, lang);
      thread = { id: view.threadId, sources: view.sources, documents: view.documents };
      return view.reply;
    });
  });

  $('ai-follow-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const question = $('ai-follow').value.trim();
    if (!question || running || !thread) return;
    $('ai-follow').value = '';
    await turn(question, (id, confirmed) => api.aiFollowUp(thread.id, id, question, confirmed, lang));
  });

  cancel.addEventListener('click', () => { if (running) api.aiCancel(running).catch(() => {}); });

  $('ai-copy').addEventListener('click', async () => {
    if (!last) return;
    try {
      await navigator.clipboard.writeText(copyText(S, last, thread.sources));
      toast(S.aiCopied);
    } catch { /* clipboard refused */ }
  });

  /* «Bibliografía» (R12). */
  const bib = $('ai-bib');
  let entries = [];
  async function drawBib() {
    try {
      entries = await api.aiBibliography(thread.id, $('ai-bib-all').checked);
    } catch (err) {
      entries = [];
      toast(aiErrorText(S, err, aiStatus()?.settings.provider));
    }
    $('ai-bib-empty').hidden = entries.length > 0;
    $('ai-bib-list').replaceChildren(...entries.map((e) => el('li', 'ai-bib-entry', bibLine(S, e))));
  }
  $('ai-bib-open').addEventListener('click', async () => {
    if (!thread) return;
    $('ai-bib-all').checked = false;
    await drawBib();
    bib.showModal();
  });
  $('ai-bib-all').addEventListener('change', drawBib);
  $('ai-bib-copy').addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(entries.map((e) => bibLine(S, e)).join('\n'));
      toast(S.aiBibCopied);
    } catch { /* clipboard refused */ }
  });
  $('ai-bib-export').addEventListener('click', async () => {
    try {
      const path = await api.aiExportRis(thread.id, $('ai-bib-all').checked);
      if (path) toast(S.aiBibSaved(path));
    } catch (err) {
      toast(aiErrorText(S, err, aiStatus()?.settings.provider));
    }
  });

  $('ask-open').addEventListener('click', () => {
    dialog.showModal();
    input.focus();
  });

  return { setStatus };
}
