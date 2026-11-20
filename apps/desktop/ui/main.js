// The one screen (VFS-1 R9): search box, filters, virtualised results, folder panel, status.

import { api } from './api.js';
import { S } from './strings.js';
import * as F from './format.js';
import { createVirtualList } from './vlist.js';
import { statusText, firstScanRunning as firstScanOf } from './status.js';

const $ = (id) => document.getElementById(id);
const PAGE = 100;
const DEBOUNCE_MS = 60;
const KINDS = ['document', 'photo', 'video', 'audio', 'folder', 'other'];
const MiB = 1024 * 1024;
const SIZE_RANGES = {
  any: [null, null],
  small: [null, MiB - 1],
  medium: [MiB, 100 * MiB],
  large: [100 * MiB, 1024 * MiB],
  huge: [1024 * MiB + 1, null],
};
const DATE_DAYS = { week: 7, month: 30, year: 365 };

const state = {
  text: '',
  kinds: new Set(),
  date: 'any',
  size: 'any',
  folder: '', // '' = all; otherwise an absolute path
  pickedFolder: null,
  roots: [],
  progress: null,
  seq: 0,
  total: 0,
  truncatedBy: null,
  pages: new Map(),
  pagesPending: new Set(),
  active: -1,
  terms: [],
  searchState: 'idle', // idle | loading | done | error
  unavailableDismissed: '',
};

/* ---------------------------------------------------------------- static copy */

for (const el of document.querySelectorAll('[data-s]')) el.textContent = S[el.dataset.s];
for (const el of document.querySelectorAll('[data-s-label]')) el.setAttribute('aria-label', S[el.dataset.sLabel]);
$('q').placeholder = S.searchPlaceholder;
if (api.isFake) $('dev-badge').hidden = false;

function icon(name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
  use.setAttribute('href', `#i-${name}`);
  svg.append(use);
  return svg;
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

/* ---------------------------------------------------------------- filters UI */

const chips = $('kind-chips');
for (const k of KINDS) {
  const label = el('label', 'chip');
  const input = el('input');
  input.type = 'checkbox';
  input.value = k;
  const span = el('span');
  span.append(icon(k), S.kindsPlural[k]);
  label.append(input, span);
  chips.append(label);
}

function fillSelect(select, options) {
  select.replaceChildren(...Object.entries(options).map(([v, t]) => new Option(t, v)));
}
fillSelect($('f-date'), S.dateOptions);
fillSelect($('f-size'), S.sizeOptions);

function renderFolderSelect() {
  const sel = $('f-folder');
  const opts = [new Option(S.folderAll, '')];
  for (const r of state.roots) {
    const o = new Option(F.baseName(r.path), r.path);
    o.title = r.path;
    opts.push(o);
  }
  const p = state.pickedFolder;
  if (p && !state.roots.some((r) => r.path === p)) {
    const o = new Option(S.folderInside(F.baseName(p)), p);
    o.title = p;
    opts.push(o);
  }
  opts.push(new Option(S.folderPick, '__pick'));
  sel.replaceChildren(...opts);
  if (state.folder && ![...sel.options].some((o) => o.value === state.folder)) state.folder = '';
  sel.value = state.folder;
}

function activeFilterCount() {
  return state.kinds.size + (state.date !== 'any') + (state.size !== 'any') + (state.folder !== '');
}

function renderClearButton() {
  const n = activeFilterCount();
  const b = $('f-clear');
  b.hidden = n === 0; // nothing to clear
  b.textContent = n ? S.clearFiltersN(n) : S.clearFilters;
}

/** Local midnight of a yyyy-mm-dd value, in unix seconds. */
function dayStart(v) {
  const [y, m, d] = v.split('-').map(Number);
  return Math.floor(new Date(y, m - 1, d).getTime() / 1000);
}

function dateRange() {
  if (DATE_DAYS[state.date]) return [Math.floor(Date.now() / 1000) - DATE_DAYS[state.date] * 86400, null];
  if (state.date === 'custom') {
    const from = $('f-from').value;
    const to = $('f-to').value;
    const a = from ? dayStart(from) : null;
    const b = to ? dayStart(to) + 86400 : null; // "Hasta" includes that whole day
    return [a, b];
  }
  return [null, null];
}

function rangeValid() {
  const [a, b] = dateRange();
  const ok = a == null || b == null || a < b;
  $('range-error').textContent = ok ? '' : S.rangeInverted;
  return ok;
}

function filters() {
  const [modifiedFrom, modifiedTo] = dateRange();
  const [sizeMin, sizeMax] = SIZE_RANGES[state.size];
  return {
    kinds: [...state.kinds],
    modifiedFrom, modifiedTo, sizeMin, sizeMax,
    folder: state.folder || null,
  };
}

chips.addEventListener('change', (e) => {
  const k = e.target.value;
  if (e.target.checked) state.kinds.add(k); else state.kinds.delete(k);
  filtersChanged();
});
$('f-date').addEventListener('change', (e) => {
  state.date = e.target.value;
  $('date-range').hidden = state.date !== 'custom';
  if (state.date === 'custom') { $('f-from').focus(); return; }
  $('range-error').textContent = '';
  filtersChanged();
});
$('f-from').addEventListener('change', filtersChanged);
$('f-to').addEventListener('change', filtersChanged);
$('f-size').addEventListener('change', (e) => { state.size = e.target.value; filtersChanged(); });
$('f-folder').addEventListener('change', async (e) => {
  if (e.target.value !== '__pick') { state.folder = e.target.value; filtersChanged(); return; }
  e.target.value = state.folder;
  try {
    const p = await api.pickFolder();
    if (p) { state.pickedFolder = p; state.folder = p; renderFolderSelect(); filtersChanged(); }
  } catch (err) { showError(errorText(err)); }
});
$('f-clear').addEventListener('click', () => {
  state.kinds.clear();
  for (const c of chips.querySelectorAll('input')) c.checked = false;
  state.date = 'any'; $('f-date').value = 'any';
  $('date-range').hidden = true; $('f-from').value = ''; $('f-to').value = '';
  $('range-error').textContent = '';
  state.size = 'any'; $('f-size').value = 'any';
  state.folder = ''; $('f-folder').value = '';
  filtersChanged();
  $('q').focus();
});
$('filters').addEventListener('submit', (e) => e.preventDefault());

function filtersChanged() {
  renderClearButton();
  if (state.date === 'custom' && !rangeValid()) return;
  runSearch();
}

/* ---------------------------------------------------------------- search */

const q = $('q');
let debounce = 0;

q.addEventListener('input', () => {
  state.text = q.value;
  $('q-clear').hidden = !q.value;
  clearTimeout(debounce);
  debounce = setTimeout(runSearch, DEBOUNCE_MS);
});
$('q-clear').addEventListener('click', () => { clearQuery(); q.focus(); });

function clearQuery() {
  q.value = '';
  state.text = '';
  $('q-clear').hidden = true;
  clearTimeout(debounce);
  runSearch();
}

function hasQuery() {
  return state.text.trim() !== '' || activeFilterCount() > 0;
}

let loadingTimer = 0;

/** Runs page 0 of the current query. `quiet` keeps the list and selection (live refresh). */
async function runSearch({ quiet = false } = {}) {
  const seq = ++state.seq;
  clearTimeout(loadingTimer);
  if (!hasQuery()) {
    state.searchState = 'idle';
    setResults(0, null, new Map());
    return;
  }
  state.terms = F.terms(state.text);
  if (!quiet) {
    // Only say "Buscando…" if the answer is slow; fast answers should not flicker.
    loadingTimer = setTimeout(() => { if (seq === state.seq) { state.searchState = 'loading'; renderSummary(); } }, 350);
  }
  try {
    const res = await api.search({ text: state.text.trim(), filters: filters(), limit: PAGE, offset: 0 });
    if (seq !== state.seq) return; // a newer query is on its way: drop this stale answer
    clearTimeout(loadingTimer);
    state.searchState = 'done';
    // A live refresh keeps the same file selected, so Enter never reveals a different one.
    let active = 0;
    if (quiet && state.active >= 0) {
      const prev = hitAt(state.active);
      const same = prev ? res.hits.findIndex((h) => h.id === prev.id && h.isDir === prev.isDir) : -1;
      active = same >= 0 ? same : state.active;
    }
    setResults(res.total, res.truncatedBy, new Map([[0, res.hits]]), active);
  } catch (err) {
    if (seq !== state.seq) return;
    clearTimeout(loadingTimer);
    state.searchState = 'error';
    state.lastError = err;
    setResults(0, null, new Map());
  }
}

function setResults(total, truncatedBy, pages, active = 0) {
  state.total = total;
  state.truncatedBy = truncatedBy;
  state.pages = pages;
  state.pagesPending = new Set();
  state.active = total ? Math.min(Math.max(active, 0), total - 1) : -1;
  list.reset(total);
  if (state.active > 0) list.reveal(state.active);
  q.setAttribute('aria-expanded', String(total > 0));
  syncActive();
  renderSummary();
  renderEmpty();
  announceSoon();
}

function hitAt(i) {
  return state.pages.get(Math.floor(i / PAGE))?.[i % PAGE];
}

async function ensurePage(p) {
  if (state.pages.has(p) || state.pagesPending.has(p)) return;
  const seq = state.seq;
  state.pagesPending.add(p);
  try {
    const res = await api.search({ text: state.text.trim(), filters: filters(), limit: PAGE, offset: p * PAGE });
    if (seq !== state.seq) return;
    state.pages.set(p, res.hits);
    const idx = [];
    for (let i = p * PAGE; i < Math.min(state.total, (p + 1) * PAGE); i++) idx.push(i);
    list.refresh(idx);
    if (state.active >= p * PAGE && state.active < (p + 1) * PAGE) syncActive();
  } catch {
    // The row stays as a placeholder; scrolling away and back retries.
  } finally {
    if (seq === state.seq) state.pagesPending.delete(p);
  }
}

/* ---------------------------------------------------------------- rows */

function unavailableRootFor(dir) {
  return state.roots.find((r) => r.status === 'unavailable' && F.isUnder(dir, r.path));
}

function buildRow() {
  const row = el('div', 'row');
  row.setAttribute('role', 'option');
  const ico = el('span', 'ico');
  ico.setAttribute('aria-hidden', 'true');
  const main = el('span', 'main');
  const name = el('span', 'name');
  name.append(el('span', 'name-text', 'Ejemplo de nombre.pdf'));
  const path = el('span', 'path');
  path.append(el('span', 'p-head', 'C:\\'), el('span', 'p-tail', 'Carpeta'));
  main.append(name, path);
  const reveal = el('button', 'reveal');
  reveal.type = 'button';
  reveal.tabIndex = -1; // keyboard and screen readers use Enter on the selected row instead
  reveal.setAttribute('aria-hidden', 'true');
  reveal.append(icon('reveal'), el('span', 'reveal-text', S.revealButton));
  row.append(ico, main, el('span', 'kind', 'Documento'), el('span', 'date', '12 sept 2026'), el('span', 'size', '999,9 MB'), reveal);
  return row;
}

function fillRow(row, i) {
  row.id = `r-${i}`;
  row.dataset.index = i;
  row.setAttribute('aria-posinset', i + 1);
  row.setAttribute('aria-setsize', state.total);
  row.setAttribute('aria-selected', String(i === state.active));
  const [ico, main, kind, date, size] = row.children;
  const hit = hitAt(i);
  if (!hit) {
    row.classList.add('loading');
    row.setAttribute('aria-label', S.loadingRow);
    ico.replaceChildren();
    ensurePage(Math.floor(i / PAGE));
    return;
  }
  row.classList.remove('loading');
  const kindKey = hit.isDir ? 'folder' : hit.kind;
  ico.dataset.kind = kindKey;
  ico.replaceChildren(icon(kindKey));

  const [nameEl, pathEl] = main.children;
  const nameText = el('span', 'name-text');
  F.appendHighlighted(nameText, hit.name, F.matchRanges(hit.name, state.terms));
  const nameParts = [nameText];
  const unavailable = unavailableRootFor(hit.dir);
  if (unavailable) nameParts.push(el('span', 'badge badge-unavailable', S.unavailableBadge));
  nameEl.replaceChildren(...nameParts);

  const ranges = F.matchRanges(hit.dir, state.terms);
  const cut = F.lastSegmentStart(hit.dir);
  const head = el('span', 'p-head');
  const tail = el('span', 'p-tail');
  F.appendHighlighted(head, hit.dir, ranges, 0, cut);
  F.appendHighlighted(tail, hit.dir, ranges, cut, hit.dir.length);
  pathEl.replaceChildren(head, tail);
  pathEl.title = hit.dir;

  const kindLabel = S.kindSingular[kindKey];
  const dateText = F.date(hit.modified);
  const sizeText = hit.isDir ? '' : F.size(hit.size);
  kind.textContent = kindLabel;
  date.textContent = dateText;
  size.textContent = sizeText;
  row.setAttribute('aria-label', S.rowLabel(hit.name, kindLabel, hit.dir, dateText, sizeText, !!unavailable));
}

const list = createVirtualList({ scroller: $('results'), sizer: $('sizer'), buildRow, fillRow });

/* ---------------------------------------------------------------- selection + keyboard */

function syncActive() {
  const i = state.active;
  if (i >= 0) q.setAttribute('aria-activedescendant', `r-${i}`);
  else q.removeAttribute('aria-activedescendant');
  for (const row of $('sizer').children) {
    if (row.dataset.index != null) row.setAttribute('aria-selected', String(Number(row.dataset.index) === i));
  }
  renderDetails();
}

function setActive(i) {
  if (!state.total) return;
  state.active = Math.max(0, Math.min(state.total - 1, i));
  list.reveal(state.active);
  syncActive();
}

q.addEventListener('keydown', (e) => {
  if (e.isComposing) return;
  switch (e.key) {
    case 'ArrowDown': e.preventDefault(); setActive(state.active + 1); break;
    case 'ArrowUp': e.preventDefault(); setActive(state.active - 1); break;
    case 'PageDown': e.preventDefault(); setActive(state.active + list.pageSize()); break;
    case 'PageUp': e.preventDefault(); setActive(state.active - list.pageSize()); break;
    case 'Enter': e.preventDefault(); if (state.active >= 0) reveal(state.active); break;
    case 'Escape':
      e.preventDefault();
      if (q.value) clearQuery();
      break;
    default:
  }
});

document.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') {
    e.preventDefault();
    q.focus();
    q.select();
  }
});

const listEl = $('results');
listEl.addEventListener('mousedown', (e) => {
  const row = e.target.closest('.row');
  if (!row) return;
  e.preventDefault(); // keep focus in the search box so the arrow keys keep working
  if (document.activeElement !== q) q.focus({ preventScroll: true });
  setActive(Number(row.dataset.index));
});
listEl.addEventListener('click', (e) => {
  const btn = e.target.closest('.reveal');
  if (btn) reveal(Number(btn.closest('.row').dataset.index));
});
listEl.addEventListener('dblclick', (e) => {
  const row = e.target.closest('.row');
  if (row && !e.target.closest('.reveal')) reveal(Number(row.dataset.index));
});

async function reveal(i) {
  const hit = hitAt(i);
  if (!hit) return;
  try {
    const outcome = await api.reveal(hit.id, hit.isDir);
    if (outcome === 'opened_parent') toast(S.revealedParent);
  } catch (err) {
    showError(errorText(err, 'reveal'));
  }
}

/* ---------------------------------------------------------------- summary, empty, details */

function renderSummary() {
  const s = $('summary');
  const fuzzy = $('fuzzy-note');
  fuzzy.hidden = true;
  if (state.searchState === 'loading') { s.textContent = S.searching; return; }
  if (state.searchState !== 'done' || !state.total) { s.textContent = ''; return; }
  const n = F.count(state.total);
  if (state.truncatedBy === 'cap') s.textContent = S.resultsCapped(n);
  else if (state.truncatedBy === 'deadline') s.textContent = S.resultsPartial(n);
  else s.textContent = S.results(n, state.total === 1);
  fuzzy.hidden = hitAt(0)?.matched !== 'fuzzy';
}

function firstScanRunning() {
  return firstScanOf(state.roots);
}

function renderEmpty() {
  const box = $('empty');
  const parts = [];
  let actions = [];
  let mode;
  const filesIndexed = state.progress?.filesIndexed ?? state.roots.reduce((s, r) => s + r.files, 0);

  if (api.missing) {
    mode = 'missing';
    parts.push(icon('warn'), el('h3', null, S.backendMissing));
  } else if (state.searchState === 'error') {
    const code = state.lastError?.code;
    mode = `error:${code}`;
    parts.push(icon('warn'), el('h3', null, S.emptyErrorTitle),
      el('p', null, S.errors.any[code] || S.emptyErrorBody));
    actions = [button(S.retry, 'btn btn-primary', () => runSearch())];
  } else if (!state.roots.length && state.rootsLoaded) {
    mode = 'no-roots';
    parts.push(icon('folder'), el('h3', null, S.emptyNoRootsTitle), el('p', null, S.emptyNoRootsBody));
    actions = [button(S.addFolderEllipsis, 'btn btn-primary', () => addFolder())];
  } else if (state.searchState === 'idle') {
    if (!state.rootsLoaded) { box.hidden = true; return; }
    if (firstScanRunning()) {
      mode = 'first-scan';
      const bar = document.createElement('progress');
      parts.push(icon('search'), el('h3', null, S.emptyFirstScanTitle), bar,
        el('p', null, S.emptyFirstScanBody(F.count(filesIndexed))));
    } else {
      mode = 'idle';
      parts.push(icon('search'), el('h3', null, S.emptyIdleTitle(F.count(filesIndexed))), el('p', null, S.emptyIdleBody));
    }
  } else if (state.searchState === 'done' && !state.total) {
    if (state.truncatedBy === 'deadline') {
      mode = 'deadline';
      parts.push(icon('info'), el('h3', null, S.emptyDeadlineTitle), el('p', null, S.emptyDeadlineBody));
      actions = [button(S.retry, 'btn btn-primary', () => runSearch())];
    } else {
      mode = `no-hits:${activeFilterCount() > 0}`;
      parts.push(icon('search'), el('h3', null, S.emptyNoHitsTitle(state.text.trim())));
      if (activeFilterCount()) {
        parts.push(el('p', null, S.emptyNoHitsFilters));
        actions = [button(S.clearFilters, 'btn btn-primary', () => $('f-clear').click())];
      } else {
        parts.push(el('p', null, S.emptyNoHitsBody));
      }
    }
  } else {
    box.hidden = true;
    return;
  }
  // Same state as on screen: refresh the words only, so buttons under the pointer and the
  // progress animation are not rebuilt on every progress tick.
  if (!box.hidden && box.dataset.mode === mode) {
    for (const tag of ['h3', 'p']) {
      const now = box.querySelector(tag);
      const next = parts.find((x) => x.tagName === tag.toUpperCase());
      if (now && next && now.textContent !== next.textContent) now.textContent = next.textContent;
    }
    return;
  }
  box.dataset.mode = mode;
  parts[0].classList.add('empty-art');
  if (actions.length) {
    const a = el('div', 'actions');
    a.append(...actions);
    parts.push(a);
  }
  box.replaceChildren(...parts);
  box.hidden = false;
}

function button(text, cls, onClick) {
  const b = el('button', cls, text);
  b.type = 'button';
  b.addEventListener('click', onClick);
  return b;
}

function renderDetails() {
  const d = $('details');
  const hit = state.active >= 0 ? hitAt(state.active) : null;
  if (!hit) { d.replaceChildren(); return; }
  d.replaceChildren(el('strong', null, `${S.detailsLabel} `), F.joinPath(hit.dir, hit.name));
}

/* Screen readers hear the result count once typing pauses, not on every keystroke. */
let announceTimer = 0;
function announceSoon() {
  clearTimeout(announceTimer);
  announceTimer = setTimeout(() => {
    const a = $('announcer');
    if (state.searchState === 'done') {
      a.textContent = state.total ? $('summary').textContent : ($('empty').querySelector('h3')?.textContent || '');
    } else if (state.searchState === 'error') {
      a.textContent = S.emptyErrorTitle;
    }
  }, 900);
}

/* ---------------------------------------------------------------- notices */

function errorText(err, action) {
  const code = err?.code;
  if (code === 'backend_missing') return S.backendMissing;
  return (action && S.errors[action]?.[code]) || S.errors.any[code] || S.errors.generic;
}

function showError(text) {
  // While a modal is open the page is inert, so the message goes inside the folder panel.
  if ($('folders').open) { folderMessage(text); return; }
  $('banner-error-text').textContent = text;
  $('banner-error').hidden = false;
}
$('banner-error-close').addEventListener('click', () => { $('banner-error').hidden = true; q.focus(); });

function renderUnavailableBanner() {
  const missing = state.roots.filter((r) => r.status === 'unavailable').map((r) => r.path);
  const key = missing.join('|');
  const banner = $('banner-unavailable');
  if (!missing.length || key === state.unavailableDismissed) { banner.hidden = true; return; }
  $('banner-unavailable-text').textContent = missing.length === 1
    ? S.unavailableOne(missing[0])
    : S.unavailableMany(missing.map((p) => `«${p}»`).join(', '));
  banner.hidden = false;
}
$('banner-unavailable-close').addEventListener('click', () => {
  state.unavailableDismissed = state.roots.filter((r) => r.status === 'unavailable').map((r) => r.path).join('|');
  renderUnavailableBanner();
  q.focus();
});

let toastTimer = 0;
function toast(text) {
  const t = $('toast');
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { t.hidden = true; }, 6000);
}

/* ---------------------------------------------------------------- status bar */

function renderStatus() {
  const p = state.progress;
  const text = $('status-text');
  const bar = $('scan-progress');
  const pause = $('pause');
  $('open-folders').textContent = S.foldersButton(state.roots.length);
  bar.hidden = true;
  pause.hidden = true;
  if (api.missing) { text.textContent = ''; return; }
  if (!state.rootsLoaded) { text.textContent = S.statusConnecting; return; }
  if (!state.roots.length) { text.textContent = S.statusNoRoots; return; }
  text.textContent = statusText(p, state.roots);
  if (p?.state === 'scanning') {
    bar.hidden = false;
    pause.hidden = false;
    pause.textContent = S.pause;
  } else if (p?.state === 'paused') {
    pause.hidden = false;
    pause.textContent = S.resume;
  }
  $('banner-scan').hidden = !(firstScanRunning() && hasQuery());
}

$('pause').addEventListener('click', async () => {
  const paused = state.progress?.state !== 'paused';
  try {
    await api.setPaused(paused);
    if (state.progress) state.progress.state = paused ? 'paused' : 'scanning';
    renderStatus();
  } catch (err) { showError(errorText(err)); }
});

/* ---------------------------------------------------------------- folder panel */

const foldersDialog = $('folders');
$('open-folders').addEventListener('click', () => {
  $('folders-msg').hidden = true;
  renderRoots();
  foldersDialog.showModal();
});

function folderMessage(text, kind = 'error') {
  $('folders-msg-text').textContent = text;
  $('folders-msg').className = `banner ${kind === 'error' ? 'banner-error' : kind === 'warn' ? 'banner-warn' : 'banner-info'}`;
  $('folders-msg').querySelector('use').setAttribute('href', kind === 'info' ? '#i-info' : '#i-warn');
  $('folders-msg').hidden = false;
}

/* Rows are updated in place (keyed by root id) rather than rebuilt on every progress tick, so
   a button never disappears under the pointer or loses keyboard focus. */
const rootItems = new Map(); // id -> {li, state, files, checked, rescan, remove}

function buildRootItem(r) {
  const li = el('li', 'root');
  const path = el('div', 'root-path');
  path.append(icon('folder'), el('span', null, r.path));
  const meta = el('div', 'root-meta');
  const st = el('span', 'root-state');
  const files = el('span');
  const checked = el('span');
  meta.append(st, files, checked);
  const actions = el('div', 'root-actions');
  const name = F.baseName(r.path);
  const rescan = button(S.rescan, 'btn', () => rescanRoot(r.id));
  rescan.setAttribute('aria-label', `${S.rescan}: ${name}`);
  const remove = button(S.remove, 'btn btn-quiet', () => {
    const current = state.roots.find((x) => x.id === r.id);
    if (current) confirmRemove(current);
  });
  remove.setAttribute('aria-label', `${S.remove}: ${name}`);
  actions.append(rescan, remove);
  li.append(path, meta, actions);
  return { li, st, files, checked, rescan, remove };
}

function renderRoots() {
  const ul = $('roots');
  if (!state.roots.length) {
    rootItems.clear();
    ul.replaceChildren(el('li', 'none', S.foldersEmpty));
    return;
  }
  ul.querySelector('.none')?.remove();
  const progressById = new Map((state.progress?.roots || []).map((r) => [r.rootId, r]));
  const ids = new Set(state.roots.map((r) => r.id));
  for (const [id, item] of rootItems) {
    if (ids.has(id)) continue;
    // The removed folder's own "Quitar" button may hold focus: keep focus inside the panel.
    const hadFocus = item.li.contains(document.activeElement);
    item.li.remove();
    rootItems.delete(id);
    if (hadFocus) $('add-folder').focus();
  }
  for (const r of state.roots) {
    let item = rootItems.get(r.id);
    if (!item) { item = buildRootItem(r); rootItems.set(r.id, item); }
    if (!item.li.isConnected) ul.append(item.li);
    const status = r.status === 'removing' ? r.status : progressById.get(r.id)?.status ?? r.status;
    item.st.textContent = S.rootStatus[status] || status;
    item.st.dataset.status = status;
    item.files.textContent = S.rootFiles(F.count(progressById.get(r.id)?.filesIndexed ?? r.files));
    item.checked.textContent = r.lastFullScan ? S.rootChecked(F.date(r.lastFullScan)) : S.rootNeverChecked;
    item.rescan.disabled = status !== 'ready' && status !== 'unavailable';
    item.remove.disabled = status === 'removing';
  }
}

async function rescanRoot(id) {
  try {
    await api.rescan(id);
    if (foldersDialog.open) folderMessage(S.rescanStarted, 'info'); else toast(S.rescanStarted);
    await refreshRoots();
  } catch (err) { showError(errorText(err)); }
}
$('rescan-all').addEventListener('click', () => rescanRoot(null));
$('add-folder').addEventListener('click', () => addFolder());

/** Explains an overlap precisely (VFS-1 U4) from the folders the UI already knows. */
function overlapText(path) {
  const inside = state.roots.find((r) => F.isUnder(path, r.path));
  if (inside) return S.overlapInside(inside.path);
  const contains = state.roots.find((r) => F.isUnder(r.path, path));
  if (contains) return S.overlapParent(contains.path);
  return S.overlapGeneric;
}

/** The core reports the kind of drive a folder is on; network and removable ones get a
 * warning (their files read as unavailable while disconnected). */
const REMOTE_DRIVES = new Set(['network', 'removable']);

async function addFolder() {
  let path;
  try {
    path = await api.pickFolder();
  } catch (err) { showError(errorText(err)); return; }
  if (!path) return;
  try {
    const root = await api.addRoot(path);
    await refreshRoots();
    const msg = S.rootAdded(path);
    const remote = REMOTE_DRIVES.has(root.drive);
    if (foldersDialog.open) {
      folderMessage(remote ? `${msg} ${S.remoteWarning}` : msg, remote ? 'warn' : 'info');
    } else {
      toast(msg);
      if (remote) showError(S.remoteWarning);
    }
  } catch (err) {
    showError(err?.code === 'overlapping_root' ? overlapText(path) : errorText(err, 'addRoot'));
  }
}

const confirmDialog = $('confirm-remove');
let removing = null;
function confirmRemove(root) {
  removing = root;
  $('confirm-path').textContent = root.path;
  confirmDialog.returnValue = '';
  confirmDialog.showModal();
}
confirmDialog.addEventListener('close', async () => {
  const choice = confirmDialog.returnValue;
  confirmDialog.returnValue = '';
  const root = removing;
  removing = null;
  if (choice !== 'remove' || !root) return;
  try {
    await api.removeRoot(root.id);
    await refreshRoots();
    runSearch({ quiet: true });
  } catch (err) { showError(errorText(err)); }
});

/* ---------------------------------------------------------------- live data */

async function refreshRoots() {
  try {
    state.roots = await api.listRoots();
    state.rootsLoaded = true;
  } catch (err) {
    if (!api.missing) showError(errorText(err));
    return;
  }
  renderFolderSelect();
  renderClearButton();
  renderUnavailableBanner();
  renderStatus();
  if (foldersDialog.open) renderRoots();
  if (state.searchState === 'idle' || !state.total) renderEmpty();
  list.refresh(); // unavailable badges depend on the roots
}

function statusKey(roots) {
  return roots.map((r) => `${r.id}:${r.status}`).join(',');
}

function onProgress(p) {
  const before = state.progress ? statusKey(state.progress.roots) : '';
  state.progress = p;
  renderStatus();
  if (statusKey(p.roots) !== before || p.roots.length !== state.roots.length) refreshRoots();
  else {
    if (foldersDialog.open) renderRoots();
    if (state.searchState === 'idle') renderEmpty();
  }
}

function onIndexChanged() {
  // New files arrived. Refresh the list only when it is scrolled to the top, so rows never
  // jump under the user's eyes.
  if (hasQuery() && state.searchState === 'done' && list.atTop()) runSearch({ quiet: true });
}

async function start() {
  renderClearButton();
  renderFolderSelect();
  renderStatus();
  q.focus();
  if (api.missing) { renderEmpty(); return; }
  await api.onProgress(onProgress);
  await api.onIndexChanged(onIndexChanged);
  await refreshRoots();
  try { onProgress(await api.getProgress()); } catch { /* the event stream will catch up */ }
  renderEmpty();
}

start();
