// «Preguntar a sus documentos (IA)» (VFS-28), PR 28d: the Ajustes section and «Qué se ha
// enviado». The key goes to the shell once, when typed, and never comes back (R2): the screen
// only ever sees its last 4 characters. Everything is drawn with textContent.

import { AiErrorCode, KeyFallback, KeyPlace, KeyState, Provider } from './contract.js';

/** Nano-dollars per dollar, as the shell counts costs (VFS-28 R10). */
export const NANO_PER_USD = 1e9;

/** A cost in dollars for `lang`; under a dollar it keeps 4 decimals, so a question never reads 0. */
export function formatUsd(nano, lang) {
  const usd = nano / NANO_PER_USD;
  const digits = usd !== 0 && Math.abs(usd) < 1 ? 4 : 2;
  return new Intl.NumberFormat(lang, {
    style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: digits,
  }).format(usd);
}

/** The cap typed in dollars as nano-dollars, or null when it is not a number ≥ 0. */
export function parseCap(text) {
  const usd = Number(String(text).trim().replace(',', '.'));
  if (!String(text).trim() || !Number.isFinite(usd) || usd < 0) return null;
  return Math.round(usd * NANO_PER_USD);
}

/** The provider's name as shown (brand names, the same in every language). */
export function providerName(S, provider) {
  return S.aiProviders[provider] || provider;
}

/** The line under the key field (R1, R2, R3): «Clave guardada · termina en …1234». */
export function keyLine(S, key) {
  if (key.state === KeyState.SAVED) {
    const where = {
      [KeyPlace.CREDENTIAL_STORE]: '',
      [KeyPlace.PROTECTED_FILE]: S.aiKeyPlaceFile,
      [KeyPlace.SESSION_ONLY]: S.aiKeyPlaceSession,
    }[key.place] || '';
    return S.aiKeySaved(key.last4) + (where ? ` ${where}` : '');
  }
  return key.state === KeyState.UNREADABLE ? S.aiKeyUnreadable : S.aiKeyNone;
}

/** The sentence for a rejected AI call (R4, R13); `provider` names the provider in it. */
export function aiErrorText(S, err, provider) {
  const code = err?.code;
  const name = providerName(S, provider);
  if (code === AiErrorCode.PROVIDER) return S.aiErrors.provider(name, err.message || '');
  const text = S.aiErrors[code];
  if (typeof text === 'function') return text(name);
  return text || S.errors.generic;
}

/** The title of a sent passage: the file name and page (D11). */
export function passageTitle(S, p) {
  return p.page ? S.aiPassageTitle(p.fileName, p.page) : p.fileName;
}

/** One record of «Qué se ha enviado» as plain lines (R11): a heading, the question, the calls. */
export function recordLines(S, r, lang) {
  const when = new Intl.DateTimeFormat(lang, { dateStyle: 'medium', timeStyle: 'short' })
    .format(new Date(r.askedAt));
  return {
    heading: S.aiRecordHeading(when, providerName(S, r.provider), formatUsd(r.costNano, lang)),
    question: r.question,
    calls: r.calls.map((c) => S.aiRecordCall(
      c.rewrite ? S.aiCallRewrite : c.followUp ? S.aiCallFollowUp : S.aiCallAnswer,
      c.model, c.inputTokens, c.outputTokens, formatUsd(c.costNano, lang),
    )),
  };
}

/**
 * Wires the Ajustes section and «Qué se ha enviado». `onStatus(status)` hears every new
 * AiStatus, so the search screen can show or hide «Preguntar».
 */
export function setupAiSettings({ api, S, lang, $, el, toast, onStatus }) {
  let status = null;
  let pendingKey = ''; // the key typed while the fallback choice is open; cleared after
  const on = $('ai-on');
  const keyInput = $('ai-key');
  const keyForm = $('ai-key-form');
  const msg = $('ai-msg');

  const radios = Object.values(Provider).map((p) => {
    const input = el('input');
    input.type = 'radio';
    input.name = 'ai-provider';
    input.value = p;
    const label = el('label', 'ai-provider-choice');
    label.append(input, ` ${providerName(S, p)}`);
    $('ai-providers').append(label);
    input.addEventListener('change', () => saveSettings());
    return input;
  });

  function say(text) {
    msg.textContent = text;
    msg.hidden = !text;
  }

  function render(next) {
    status = next;
    const s = next.settings;
    on.checked = s.enabled;
    $('ai-body').hidden = !s.enabled;
    for (const r of radios) r.checked = r.value === s.provider;
    $('ai-cheap').checked = s.cheap;
    $('ai-key-line').textContent = keyLine(S, next.key);
    const saved = next.key.state === KeyState.SAVED;
    keyForm.hidden = saved;
    $('ai-test').hidden = !saved;
    $('ai-replace').hidden = !saved;
    $('ai-delete').hidden = next.key.state === KeyState.NONE;
    if (document.activeElement !== $('ai-cap')) {
      $('ai-cap').value = String(next.capNano / NANO_PER_USD);
    }
    $('ai-month').textContent = S.aiMonth(formatUsd(next.monthNano, lang), formatUsd(next.capNano, lang));
    onStatus?.(next);
  }

  async function call(fn) {
    say('');
    try {
      render(await fn());
      return true;
    } catch (err) {
      say(aiErrorText(S, err, status?.settings.provider));
      return false;
    }
  }

  function saveSettings() {
    const provider = radios.find((r) => r.checked)?.value || Provider.ANTHROPIC;
    return call(() => api.aiSetSettings(on.checked, provider, $('ai-cheap').checked));
  }

  on.addEventListener('change', saveSettings);
  $('ai-cheap').addEventListener('change', saveSettings);

  keyForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const key = keyInput.value;
    if (!key.trim()) return;
    const provider = status.settings.provider;
    try {
      render(await api.aiSaveKey(provider, key));
      keyInput.value = '';
      say('');
    } catch (err) {
      if (err?.code !== AiErrorCode.STORE_UNAVAILABLE) {
        say(aiErrorText(S, err, provider));
        return;
      }
      pendingKey = key;
      keyInput.value = '';
      const d = $('ai-fallback');
      d.returnValue = '';
      d.showModal();
    }
  });

  $('ai-fallback').addEventListener('close', async () => {
    const choice = $('ai-fallback').returnValue;
    const key = pendingKey;
    pendingKey = '';
    if (choice !== KeyFallback.PROTECTED_FILE && choice !== KeyFallback.SESSION_ONLY) return;
    await call(() => api.aiSaveKeyFallback(status.settings.provider, key, choice));
  });

  $('ai-replace').addEventListener('click', () => {
    keyForm.hidden = false;
    keyInput.focus();
  });

  $('ai-test').addEventListener('click', async () => {
    const provider = status.settings.provider;
    $('ai-test').disabled = true;
    say(S.aiTesting);
    try {
      await api.aiTestKey(provider);
      say(S.aiTestOk);
    } catch (err) {
      say(aiErrorText(S, err, provider));
    } finally {
      $('ai-test').disabled = false;
    }
  });

  const deleteDialog = $('confirm-ai-delete');
  $('ai-delete').addEventListener('click', () => {
    deleteDialog.returnValue = '';
    deleteDialog.showModal();
  });
  deleteDialog.addEventListener('close', async () => {
    if (deleteDialog.returnValue !== 'delete') return;
    if (await call(() => api.aiDeleteKey(status.settings.provider))) toast(S.aiKeyDeleted);
  });

  $('ai-cap').addEventListener('change', () => {
    const cap = parseCap($('ai-cap').value);
    if (cap === null) {
      say(S.aiCapInvalid);
      return;
    }
    call(() => api.aiSetCap(cap));
  });

  /* «Qué se ha enviado» (R11). */
  const recordDialog = $('ai-record');
  async function showRecord() {
    const list = $('ai-record-list');
    let records = [];
    try {
      records = await api.aiRecords();
    } catch (err) {
      say(aiErrorText(S, err, status?.settings.provider));
    }
    $('ai-record-empty').hidden = records.length > 0;
    $('ai-record-clear').hidden = records.length === 0;
    list.replaceChildren(...records.slice().reverse().map((r) => {
      const lines = recordLines(S, r, lang);
      const item = el('li', 'ai-record-item');
      item.append(el('p', 'ai-record-heading', lines.heading), el('p', 'ai-record-question', lines.question));
      if (r.rewrite) {
        const rw = el('details', 'ai-record-rewrite');
        rw.append(el('summary', null, S.aiRecordRewrite), el('p', 'ai-record-text', r.rewrite));
        item.append(rw);
      }
      const passages = el('details', 'ai-record-passages');
      passages.append(el('summary', null, S.aiRecordPassages(r.passages.length)));
      for (const p of r.passages) {
        passages.append(el('p', 'ai-record-file', passageTitle(S, p)), el('p', 'ai-record-text', p.text));
      }
      item.append(passages);
      for (const c of lines.calls) item.append(el('p', 'ai-record-call', c));
      return item;
    }));
  }
  $('ai-open-record').addEventListener('click', async () => {
    await showRecord();
    recordDialog.showModal();
  });

  const clearDialog = $('confirm-ai-clear');
  $('ai-record-clear').addEventListener('click', () => {
    clearDialog.returnValue = '';
    clearDialog.showModal();
  });
  clearDialog.addEventListener('close', async () => {
    if (clearDialog.returnValue !== 'clear') return;
    try {
      await api.aiClearRecords();
      toast(S.aiRecordCleared);
    } catch (err) {
      say(aiErrorText(S, err, status?.settings.provider));
    }
    showRecord();
  });

  /** Reads the status again (when Ajustes opens). */
  async function refresh() {
    try {
      render(await api.aiStatus());
    } catch (err) {
      say(aiErrorText(S, err, status?.settings.provider));
    }
  }
  refresh();
  return { refresh, status: () => status };
}
