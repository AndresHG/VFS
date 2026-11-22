// The one screen (VFS-1 R9): search box, filters, virtualised results, folder panel, status.

import { api } from './api.js';
import { choiceFor, languageFor, localeFor } from './strings.js';
import * as F from './format.js';
import { createVirtualList } from './vlist.js';
import { placesOf, showsCloudBadge } from './copies.js';
import { SecondAction, secondActionOf } from './links.js';
import { createUpdates } from './updates.js';
import { wantsSuggestion, suggestionLabel, folderAfterClear } from './suggest.js';
import { setupFeedback } from './feedback.js';
import { statusText, firstScanRunning as firstScanOf, viewFor, footerActions, rootAction, RootAction } from './status.js';
import { takenInBy, newestOnly } from './folders.js';
import {
  DriveKind, ErrorCode, FileKind, IndexerState, LinkKind, MatchKind, ReadState, RevealOutcome, RootStatus, SortOrder,
  Source, TruncatedBy, UiErrorCode,
} from './contract.js';
import {
  DateChoice, LanguageChoice, SearchState, SizeChoice, SourceChoice, TypeChecklist, View, typeChecklistMode,
} from './choices.js';
import {
  PATH_SEPARATOR, bookmarkFolderOf, bookmarkFolderValue, bookmarkPathParts, site, sizeFilterApplies, sortChoices, sortFor,
  sourcesFor,
} from './sources.js';

const $ = (id) => document.getElementById(id);
const PAGE = 100;
const DEBOUNCE_MS = 60;
/** The type chips, in display order. */
const KIND_CHIPS = [
  FileKind.DOCUMENT, FileKind.PHOTO, FileKind.VIDEO, FileKind.AUDIO, FileKind.FOLDER, FileKind.OTHER,
];
const MiB = 1024 * 1024;
const SIZE_RANGES = {
  [SizeChoice.ANY]: [null, null],
  [SizeChoice.SMALL]: [null, MiB - 1],
  [SizeChoice.MEDIUM]: [MiB, 100 * MiB],
  [SizeChoice.LARGE]: [100 * MiB, 1024 * MiB],
  [SizeChoice.HUGE]: [1024 * MiB + 1, null],
};
const DATE_DAYS = { [DateChoice.WEEK]: 7, [DateChoice.MONTH]: 30, [DateChoice.YEAR]: 365 };
/** Folder-select value of the "Elegir otra carpeta…" entry. */
const PICK_FOLDER = '__pick';
/** Class of a filter that narrows the list. */
const ON = 'is-on';
/** Look of a message in the folder panel. */
const Notice = Object.freeze({ ERROR: 'error', WARN: 'warn', INFO: 'info' });
/** Icons of a bookmark row and of the row actions (VFS-6). */
const BOOKMARK_ICON = 'bookmark';
const OPEN_ICON = 'open';
const REVEAL_ICON = 'reveal';
/** "Mostrar el destino" (VFS-18): the shortcut arrow. */
const TARGET_ICON = 'target';

/** Where the user's language choice is kept (VFS-22 R3); absent means the system language. */
const LANGUAGE_KEY = 'vfs.language';

function savedLanguage() {
  try {
    return localStorage.getItem(LANGUAGE_KEY);
  } catch {
    return null; // storage blocked: the system language applies
  }
}

/** Keeps `choice` (a LanguageChoice); SYSTEM forgets any saved language. */
function saveLanguage(choice) {
  try {
    if (choice === LanguageChoice.SYSTEM) localStorage.removeItem(LANGUAGE_KEY);
    else localStorage.setItem(LANGUAGE_KEY, choice);
  } catch {
    // storage blocked: the choice is lost and the system language applies
  }
}

/** VFS-22 R2, R3: the screen's language, known before anything is drawn. */
async function loadLocale() {
  let system = '';
  try {
    system = await api.systemLocale();
  } catch {
    system = navigator.language;
  }
  return localeFor(languageFor(system, savedLanguage()), system);
}

// Everything that changes while the screen is open lives in this call's scope, owned by the
// screen; the module itself holds only constants, never mutable state.
function main(L) {
  const { S, N } = L;
  document.documentElement.lang = L.lang;
  const state = {
    text: '',
    kinds: new Set(),
    date: DateChoice.ANY,
    size: SizeChoice.ANY,
    folder: '', // '' = all; otherwise an absolute path
    pickedFolder: null,
    roots: [],
    progress: null,
    seq: 0,
    /** The core's suggestion for the current search with no hits (VFS-13), or null. */
    suggestion: null,
    total: 0,
    truncatedBy: null,
    approximateOrder: false, // VFS-12: the order covers only the matches read
    pages: new Map(),
    pagesPending: new Set(),
    active: -1,
    terms: [],
    searchState: SearchState.IDLE,
    unavailableDismissed: '',
    source: SourceChoice.BOTH, // VFS-6 D7: both sources at every start
    sort: SortOrder.RELEVANCE,
    bookmarks: null, // BookmarkStatus, null until known
    pickedBookmarkFolder: null, // {id, title} chosen with "Ver su contenido"
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
  for (const k of KIND_CHIPS) {
    const label = el('label', 'chip');
    const input = el('input');
    input.type = 'checkbox';
    input.value = k;
    const span = el('span');
    span.append(icon(k), S.kindsPlural[k]);
    label.append(input, span);
    chips.append(label);
  }

  function renderTypeButton() {
    const names = KIND_CHIPS.filter((k) => state.kinds.has(k)).map((k) => S.kindsPlural[k]);
    $('f-type-text').textContent = S.typeButton(...names);
    $('f-type').classList.toggle(ON, names.length > 0);
  }

  // The checklist opens under its button. Placed from the button's box so it also works where
  // CSS anchor positioning is missing (older WKWebView). Without the Popover API at all, the
  // chips are shown inline instead.
  const typePop = $('f-type-pop');
  $('filters').dataset.types = typeChecklistMode(HTMLElement.prototype);
  if ($('filters').dataset.types === TypeChecklist.INLINE) $('f-type').hidden = true;
  typePop.addEventListener('beforetoggle', (e) => {
    if (e.newState !== 'open') return;
    const r = $('f-type').getBoundingClientRect();
    typePop.style.top = `${r.bottom + 6}px`;
    typePop.style.left = `${Math.max(8, Math.min(r.left, innerWidth - typePop.offsetWidth - 8))}px`;
  });
  typePop.addEventListener('toggle', (e) => {
    if (e.newState === 'open') chips.querySelector('input')?.focus();
  });

  function fillSelect(select, options) {
    select.replaceChildren(...Object.entries(options).map(([v, t]) => new Option(t, v)));
  }
  fillSelect($('f-date'), S.dateOptions);
  fillSelect($('f-size'), S.sizeOptions);
  fillSelect($('f-source'), S.sourceOptions);
  $('f-source').value = state.source;
  $('f-source').addEventListener('change', (e) => { state.source = e.target.value; renderSources(); filtersChanged(); });
  $('f-sort').addEventListener('change', (e) => { state.sort = e.target.value; filtersChanged(); });
  // The "Orden" options before the bookmark setting is known (VFS-12).
  renderSources();

  function bookmarksOn() {
    return !!state.bookmarks?.enabled;
  }

  /** "Buscar en" shows only while bookmarks are included, and "Orden" offers "by source" only
   *  then (VFS-12); the size filter does not apply to bookmarks alone (VFS-6 R7). */
  function renderSources() {
    const on = bookmarksOn();
    $('f-source-pill').hidden = !on;
    state.sort = sortFor(state.sort, on);
    const sorts = $('f-sort');
    sorts.replaceChildren(...sortChoices(on).map((o) => new Option(S.sortOptions[o], o)));
    sorts.value = state.sort;
    $('f-source').value = state.source;
    $('f-size').disabled = !sizeFilterApplies(state.source, on);
    $('f-source-pill').classList.toggle(ON, on && state.source !== SourceChoice.BOTH);
    $('f-sort-pill').classList.toggle(ON, state.sort !== SortOrder.RELEVANCE);
  }

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
    opts.push(new Option(S.folderPick, PICK_FOLDER));
    // VFS-6 R12: bookmark roots, and a folder chosen with "Ver su contenido".
    if (bookmarksOn()) {
      const group = document.createElement('optgroup');
      group.label = S.folderGroupBookmarks;
      for (const r of state.bookmarks.roots || []) {
        const label = bookmarkPathParts({ root: r.root, folder: [], profile: r.profile }, S.bookmarkRoots).join(PATH_SEPARATOR);
        group.append(new Option(label, bookmarkFolderValue(r.id)));
      }
      const b = state.pickedBookmarkFolder;
      if (b && !(state.bookmarks.roots || []).some((r) => r.id === b.id)) {
        group.append(new Option(S.folderInside(b.title), bookmarkFolderValue(b.id)));
      }
      if (group.children.length) opts.push(group);
    }
    sel.replaceChildren(...opts);
    if (state.folder && ![...sel.options].some((o) => o.value === state.folder)) state.folder = '';
    sel.value = state.folder;
  }

  function sizeApplies() {
    return sizeFilterApplies(state.source, bookmarksOn());
  }

  function activeFilterCount() {
    const size = sizeApplies() && state.size !== SizeChoice.ANY;
    return state.kinds.size + (state.date !== DateChoice.ANY) + size + (state.folder !== '');
  }

  function renderClearButton() {
    const n = activeFilterCount();
    const b = $('f-clear');
    b.hidden = n === 0; // nothing to clear
    b.textContent = n ? S.clearFiltersN(n) : S.clearFilters;
    renderTypeButton();
    // A filter that narrows the list looks chosen.
    $('f-date').parentElement.classList.toggle(ON, state.date !== DateChoice.ANY);
    $('f-size').parentElement.classList.toggle(ON, state.size !== SizeChoice.ANY);
    $('f-folder').parentElement.classList.toggle(ON, state.folder !== '');
  }

  /** Local midnight of a yyyy-mm-dd value, in unix seconds. */
  function dayStart(v) {
    const [y, m, d] = v.split('-').map(Number);
    return Math.floor(new Date(y, m - 1, d).getTime() / 1000);
  }

  function dateRange() {
    if (DATE_DAYS[state.date]) return [Math.floor(Date.now() / 1000) - DATE_DAYS[state.date] * 86400, null];
    if (state.date === DateChoice.CUSTOM) {
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
    const [sizeMin, sizeMax] = sizeApplies() ? SIZE_RANGES[state.size] : [null, null];
    return {
      kinds: [...state.kinds],
      modifiedFrom, modifiedTo, sizeMin, sizeMax,
      // A bookmark folder is not a path: it goes in the options instead (VFS-6 R12).
      folder: bookmarkFolderOf(state.folder) == null ? state.folder || null : null,
    };
  }

  /** One page of the current query over the chosen sources (VFS-6). */
  function fetchPage(offset) {
    const query = { text: state.text.trim(), filters: filters(), limit: PAGE, offset };
    return api.searchSources(query, {
      sources: sourcesFor(state.source, bookmarksOn()),
      bookmarkFolder: bookmarkFolderOf(state.folder),
      sort: sortFor(state.sort, bookmarksOn()),
    });
  }

  chips.addEventListener('change', (e) => {
    const k = e.target.value;
    if (e.target.checked) state.kinds.add(k); else state.kinds.delete(k);
    filtersChanged();
  });
  $('f-date').addEventListener('change', (e) => {
    state.date = e.target.value;
    $('date-range').hidden = state.date !== DateChoice.CUSTOM;
    if (state.date === DateChoice.CUSTOM) { $('f-from').focus(); return; }
    $('range-error').textContent = '';
    filtersChanged();
  });
  $('f-from').addEventListener('change', filtersChanged);
  $('f-to').addEventListener('change', filtersChanged);
  $('f-size').addEventListener('change', (e) => { state.size = e.target.value; filtersChanged(); });
  $('f-folder').addEventListener('change', async (e) => {
    if (e.target.value !== PICK_FOLDER) { state.folder = e.target.value; filtersChanged(); return; }
    e.target.value = state.folder;
    try {
      const p = await api.pickFolder();
      if (p) { state.pickedFolder = p; state.folder = p; renderFolderSelect(); filtersChanged(); }
    } catch (err) { showError(errorText(err)); }
  });
  /** Clears type, date, size and folder; a suggestion (VFS-13) keeps a bookmark folder. */
  function clearFilters({ keepBookmarkFolder = false } = {}) {
    state.kinds.clear();
    for (const c of chips.querySelectorAll('input')) c.checked = false;
    state.date = DateChoice.ANY; $('f-date').value = DateChoice.ANY;
    $('date-range').hidden = true; $('f-from').value = ''; $('f-to').value = '';
    $('range-error').textContent = '';
    state.size = SizeChoice.ANY; $('f-size').value = SizeChoice.ANY;
    state.folder = folderAfterClear(state.folder, keepBookmarkFolder);
    $('f-folder').value = state.folder;
    filtersChanged();
    $('q').focus();
  }
  $('f-clear').addEventListener('click', () => clearFilters());
  $('filters').addEventListener('submit', (e) => e.preventDefault());

  function filtersChanged() {
    renderClearButton();
    if (state.date === DateChoice.CUSTOM && !rangeValid()) return;
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
    // A live refresh keeps the suggestion on screen until its own answer replaces it.
    if (!quiet) state.suggestion = null;
    clearTimeout(loadingTimer);
    if (!hasQuery()) {
      state.searchState = SearchState.IDLE;
      setResults(0, null, new Map());
      return;
    }
    state.terms = F.terms(state.text);
    if (!quiet) {
      // Only say "Buscando…" if the answer is slow; fast answers should not flicker.
      loadingTimer = setTimeout(() => { if (seq === state.seq) { state.searchState = SearchState.LOADING; renderSummary(); } }, 350);
    }
    try {
      const res = await fetchPage(0);
      if (seq !== state.seq) return; // a newer query is on its way: drop this stale answer
      clearTimeout(loadingTimer);
      state.searchState = SearchState.DONE;
      // A live refresh keeps the same file selected, so Enter never reveals a different one.
      let active = 0;
      if (quiet && state.active >= 0) {
        const prev = hitAt(state.active);
        const same = prev
          ? res.hits.findIndex((h) => h.source === prev.source && h.id === prev.id && h.isDir === prev.isDir)
          : -1;
        active = same >= 0 ? same : state.active;
      }
      setResults(res.total, res.truncatedBy, new Map([[0, res.hits]]), active, res.approximateOrder);
      if (wantsSuggestion(res.total, res.truncatedBy, sourcesFor(state.source, bookmarksOn()))) {
        askSuggestion(seq);
      } else {
        state.suggestion = null;
      }
    } catch (err) {
      if (seq !== state.seq) return;
      clearTimeout(loadingTimer);
      state.searchState = SearchState.ERROR;
      state.lastError = err;
      setResults(0, null, new Map());
    }
  }

  /** VFS-13 R1, R5: asks the core for one suggestion; shown only while no newer search
   *  started. A failure leaves the empty state as it is. */
  async function askSuggestion(seq) {
    const query = { text: state.text.trim(), filters: filters(), limit: 1, offset: 0 };
    try {
      const s = await api.suggest(query);
      if (seq !== state.seq) return;
      state.suggestion = s;
      renderEmpty();
    } catch { /* no suggestion */ }
  }

  /** Runs the suggested search: the box gets its text; filters go when it says so. */
  function applySuggestion(s) {
    q.value = s.text;
    state.text = s.text;
    $('q-clear').hidden = !s.text;
    if (s.clearFilters) clearFilters({ keepBookmarkFolder: true }); else runSearch();
    q.focus();
  }

  function setResults(total, truncatedBy, pages, active = 0, approximateOrder = false) {
    state.total = total;
    state.truncatedBy = truncatedBy;
    state.approximateOrder = approximateOrder;
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
      const res = await fetchPage(p * PAGE);
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
    return state.roots.find((r) => r.status === RootStatus.UNAVAILABLE && F.isUnder(dir, r.path));
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
    reveal.dataset.icon = REVEAL_ICON;
    reveal.append(icon(REVEAL_ICON), el('span', 'reveal-text', S.revealButton), el('span', 'kbd', S.revealKey));
    // The second action of a shortcut or link row (VFS-18 D3, D6); hidden on other rows.
    const second = el('button', 'reveal reveal-second');
    second.type = 'button';
    second.tabIndex = -1;
    second.setAttribute('aria-hidden', 'true');
    second.hidden = true;
    row.append(ico, main, el('span', 'kind', 'Documento'), el('span', 'date', '12 sept 2026'), el('span', 'size', '999,9 MB'), reveal, second);
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
    if (hit.source === Source.BOOKMARKS) { setSecond(row, null); fillBookmarkRow(row, hit); return; }
    setAction(row.children[5], REVEAL_ICON, S.revealButton, true);
    setSecond(row, secondAction(hit));
    const kindKey = hit.isDir ? FileKind.FOLDER : hit.kind;
    ico.dataset.kind = kindKey;
    ico.replaceChildren(icon(kindKey));

    const [nameEl, pathEl] = main.children;
    const nameText = el('span', 'name-text');
    F.appendHighlighted(nameText, hit.name, F.matchRanges(hit.name, state.terms));
    const nameParts = [nameText];
    const unavailable = unavailableRootFor(hit.dir);
    if (unavailable) nameParts.push(el('span', 'badge badge-unavailable', S.unavailableBadge));
    if (showsCloudBadge(hit)) nameParts.push(el('span', 'badge badge-cloud', S.cloudOnlyBadge));
    const places = placesOf(hit).length;
    if (places > 1) {
      const copies = el('button', 'badge badge-copies', S.copiesBadge(places));
      copies.type = 'button';
      copies.tabIndex = -1; // the keyboard uses Mayús+Intro on the selected row
      copies.setAttribute('aria-label', S.copiesBadgeLabel(places));
      nameParts.push(copies);
    }
    const link = hit.link;
    if (link) nameParts.push(el('span', 'badge badge-link', S.linkBadge[link.kind]));
    nameEl.replaceChildren(...nameParts);

    // A link's second line is where it leads (VFS-18 D2, D9): text only, nothing on the
    // target is touched; its own folder is in the details line and "Mostrar en carpeta".
    const line = link?.target ?? hit.dir;
    const ranges = F.matchRanges(line, state.terms);
    const cut = F.lastSegmentStart(line);
    const head = el('span', 'p-head');
    const tail = el('span', 'p-tail');
    F.appendHighlighted(head, line, ranges, 0, cut);
    F.appendHighlighted(tail, line, ranges, cut, line.length);
    const parts = link?.target ? [el('span', 'p-lead', S.linkTargetPrefix), head, tail] : [head, tail];
    pathEl.replaceChildren(...parts);
    pathEl.title = link?.target ? S.linkTargetPrefix + link.target : hit.dir;

    const kindLabel = S.kindSingular[kindKey];
    const dateText = N.date(hit.modified);
    const sizeText = hit.isDir ? '' : N.size(hit.size);
    kind.textContent = kindLabel;
    date.textContent = dateText;
    size.textContent = sizeText;
    const label = S.rowLabel(hit.name, kindLabel, hit.dir, dateText, sizeText, !!unavailable, places);
    row.setAttribute('aria-label', link ? `${label}, ${S.linkRowLabel(S.linkBadge[link.kind], link.target)}` : label);
  }

  /** The second action of a file hit, or null: "Mostrar el destino" for a shortcut or link
   *  with a target, "Abrir en Firefox" for an Internet Shortcut that can be opened. */
  function secondAction(hit) {
    switch (secondActionOf(hit)) {
      case SecondAction.OPEN: return { icon: OPEN_ICON, text: S.openButton };
      case SecondAction.TARGET: return { icon: TARGET_ICON, text: S.targetButton };
      default: return null;
    }
  }

  function setSecond(row, action) {
    const btn = row.children[6];
    row.classList.toggle('has-second', !!action);
    btn.hidden = !action;
    if (!action || btn.querySelector('.reveal-text')?.textContent === action.text) return;
    btn.dataset.icon = action.icon;
    btn.title = `${action.text} (${S.secondKey})`;
    btn.replaceChildren(icon(action.icon), el('span', 'reveal-text', action.text), el('span', 'kbd', S.secondKey));
  }

  /** The row's one action: "Mostrar en carpeta", "Abrir en Firefox" or "Ver su contenido". */
  function setAction(btn, iconName, text, visible) {
    btn.hidden = !visible;
    if (btn.dataset.icon === iconName && btn.querySelector('.reveal-text')?.textContent === text) return;
    btn.dataset.icon = iconName;
    btn.replaceChildren(icon(iconName), el('span', 'reveal-text', text), el('span', 'kbd', S.revealKey));
  }

  /** A bookmark or bookmark-folder row (VFS-6): title, the folder path it is saved in with the
   *  same middle truncation as a file's, its kind and the date it was saved. */
  function fillBookmarkRow(row, hit) {
    const [ico, main, kind, date, size, action] = row.children;
    const iconName = hit.isFolder ? FileKind.FOLDER : BOOKMARK_ICON;
    ico.dataset.kind = iconName;
    ico.replaceChildren(icon(iconName));
    const [nameEl, pathEl] = main.children;
    const nameText = el('span', 'name-text');
    F.appendHighlighted(nameText, hit.title, F.matchRanges(hit.title, state.terms));
    nameEl.replaceChildren(nameText);
    const parts = bookmarkPathParts(hit, S.bookmarkRoots);
    const whole = parts.join(PATH_SEPARATOR);
    const cut = whole.length - parts[parts.length - 1].length;
    const ranges = F.matchRanges(whole, state.terms);
    const head = el('span', 'p-head');
    const tail = el('span', 'p-tail');
    F.appendHighlighted(head, whole, ranges, 0, cut);
    F.appendHighlighted(tail, whole, ranges, cut, whole.length);
    pathEl.replaceChildren(head, tail);
    pathEl.title = whole;
    const kindLabel = hit.isFolder ? S.bookmarkFolderKind : S.bookmarkKind;
    const dateText = N.date(hit.added);
    // A bookmark has no size; its slot shows the site (VFS-6 AC-11).
    const host = hit.isFolder ? '' : site(hit.url);
    kind.textContent = kindLabel;
    date.textContent = dateText;
    size.textContent = host;
    size.title = host;
    if (hit.isFolder) setAction(action, FileKind.FOLDER, S.folderContentsButton, true);
    else setAction(action, OPEN_ICON, S.openButton, hit.openable);
    row.setAttribute('aria-label', S.bookmarkRowLabel(hit.title, kindLabel, whole, host, dateText));
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
      case 'Enter':
        e.preventDefault();
        if (state.active < 0) break;
        // Mayús+Intro: a link's own second action first, else the places of a file's copies.
        if (!e.shiftKey) act(state.active);
        else if (secondActionOf(hitAt(state.active))) actSecond(state.active);
        else if (placesOf(hitAt(state.active)).length > 1) openPlaces(state.active);
        break;
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
    const copies = e.target.closest('.badge-copies');
    if (copies) { openPlaces(Number(copies.closest('.row').dataset.index)); return; }
    const btn = e.target.closest('.reveal');
    if (!btn) return;
    const i = Number(btn.closest('.row').dataset.index);
    if (btn.classList.contains('reveal-second')) actSecond(i);
    else act(i);
  });
  listEl.addEventListener('dblclick', (e) => {
    const row = e.target.closest('.row');
    if (row && !e.target.closest('.reveal, .badge-copies')) act(Number(row.dataset.index));
  });

  /** The selected result's one action (Intro, double click or its button). */
  function act(i) {
    const hit = hitAt(i);
    if (hit?.source !== Source.BOOKMARKS) reveal(i);
    else if (hit.isFolder) showBookmarkFolder(hit);
    else openBookmark(hit);
  }

  /** The selected link's second action (Mayús+Intro or its button, VFS-18 R6, R7). */
  async function actSecond(i) {
    const hit = hitAt(i);
    if (hit?.source === Source.BOOKMARKS || !secondAction(hit)) return;
    try {
      if (hit.link.kind === LinkKind.INTERNET) {
        await api.openLinkUrl(hit.id);
        return;
      }
      const outcome = await api.revealTarget(hit.id);
      if (outcome === RevealOutcome.OPENED_PARENT) toast(S.revealedParent);
    } catch (err) {
      if (err?.code === ErrorCode.NOT_FOUND && hit.link.kind !== LinkKind.INTERNET) {
        showError(S.targetMissing(hit.link.target));
      } else {
        showError(errorText(err, hit.link.kind === LinkKind.INTERNET ? S.errors.openLinkUrl : S.errors.revealTarget));
      }
    }
  }

  /** "Abrir en Firefox" (VFS-6 R10). */
  async function openBookmark(hit) {
    if (!hit.openable) return;
    try {
      await api.openBookmark(hit.id);
    } catch (err) {
      showError(errorText(err, S.errors.openBookmark));
    }
  }

  /** "Ver su contenido": the folder becomes the "Carpeta" filter and its contents are listed
   *  in Firefox's order (VFS-6 R12). */
  function showBookmarkFolder(hit) {
    state.pickedBookmarkFolder = { id: hit.id, title: hit.title };
    state.folder = bookmarkFolderValue(hit.id);
    renderFolderSelect();
    q.value = '';
    state.text = '';
    $('q-clear').hidden = true;
    filtersChanged();
  }

  async function reveal(i) {
    const hit = hitAt(i);
    if (!hit) return;
    await revealFile(hit.id, hit.isDir);
  }

  async function revealFile(id, isDir) {
    try {
      const outcome = await api.reveal(id, isDir);
      if (outcome === RevealOutcome.OPENED_PARENT) toast(S.revealedParent);
    } catch (err) {
      showError(errorText(err, S.errors.reveal));
    }
  }

  /* ---------------------------------------------------------------- copies (VFS-19) */

  const placesDialog = $('copies');
  function openPlaces(i) {
    const hit = hitAt(i);
    const places = placesOf(hit);
    if (places.length < 2) return;
    $('copies-title').textContent = S.copiesTitle(hit.name, places.length);
    $('copies-list').replaceChildren(...places.map((p) => {
      const li = el('li', 'root');
      const path = el('div', 'root-path');
      path.append(icon(FileKind.FOLDER), el('span', null, p.dir));
      const meta = el('div', 'root-meta');
      meta.append(el('span', null, S.copiesModified(N.date(p.modified))));
      if (showsCloudBadge(p)) meta.append(el('span', 'badge badge-cloud', S.cloudOnlyBadge));
      const actions = el('div', 'root-actions');
      const show = el('button', 'btn', S.revealButton);
      show.type = 'button';
      show.addEventListener('click', () => revealFile(p.id, false));
      actions.append(show);
      li.append(path, meta, actions);
      return li;
    }));
    placesDialog.showModal();
    $('copies-list').querySelector('button')?.focus();
  }
  placesDialog.addEventListener('close', () => q.focus({ preventScroll: true }));

  /* ---------------------------------------------------------------- summary, empty, details */

  function renderSummary() {
    const s = $('summary');
    const fuzzy = $('fuzzy-note');
    fuzzy.hidden = true;
    if (state.searchState === SearchState.LOADING) { s.textContent = S.searching; return; }
    if (state.searchState !== SearchState.DONE || !state.total) { s.textContent = ''; return; }
    const n = N.count(state.total);
    if (state.approximateOrder) s.textContent = S.resultsApproximate(n);
    else if (state.truncatedBy === TruncatedBy.CAP) s.textContent = S.resultsCapped(n);
    else if (state.truncatedBy === TruncatedBy.DEADLINE) s.textContent = S.resultsPartial(n);
    else s.textContent = S.results(n, state.total === 1);
    fuzzy.hidden = hitAt(0)?.matched !== MatchKind.FUZZY;
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
    } else if (state.searchState === SearchState.ERROR) {
      const code = state.lastError?.code;
      mode = `error:${code}`;
      parts.push(icon('warn'), el('h3', null, S.emptyErrorTitle),
        el('p', null, S.errors.any[code] || S.emptyErrorBody));
      actions = [button(S.retry, 'btn btn-primary', () => runSearch())];
    } else if (!state.roots.length && state.rootsLoaded) {
      mode = 'no-roots';
      parts.push(icon(FileKind.FOLDER), el('h3', null, S.emptyNoRootsTitle), el('p', null, S.emptyNoRootsBody));
      actions = [button(S.addFolderEllipsis, 'btn btn-primary', () => addFolder())];
    } else if (state.searchState === SearchState.IDLE) {
      if (!state.rootsLoaded) { box.hidden = true; setView(); return; }
      if (firstScanRunning()) {
        mode = 'first-scan';
        const bar = document.createElement('progress');
        parts.push(icon('search'), el('h3', null, S.emptyFirstScanTitle), bar,
          el('p', null, S.emptyFirstScanBody(N.count(filesIndexed))));
      } else {
        mode = SearchState.IDLE;
        parts.push(icon('search'), el('h3', null, S.emptyIdleTitle(N.count(filesIndexed))), el('p', null, S.emptyIdleBody));
      }
    } else if (state.searchState === SearchState.DONE && !state.total) {
      if (state.truncatedBy === TruncatedBy.DEADLINE) {
        mode = TruncatedBy.DEADLINE;
        parts.push(icon('info'), el('h3', null, S.emptyDeadlineTitle), el('p', null, S.emptyDeadlineBody));
        actions = [button(S.retry, 'btn btn-primary', () => runSearch())];
      } else {
        const sg = state.suggestion;
        mode = `no-hits:${activeFilterCount() > 0}:${sg ? sg.kind + sg.text : ''}`;
        parts.push(icon('search'), el('h3', null, S.emptyNoHitsTitle(state.text.trim())));
        if (activeFilterCount()) {
          parts.push(el('p', null, S.emptyNoHitsFilters));
          // A "drop filters" suggestion is this same action with its count.
          if (!sg?.clearFilters) actions = [button(S.clearFilters, sg ? 'btn' : 'btn btn-primary', () => $('f-clear').click())];
        } else {
          parts.push(el('p', null, S.emptyNoHitsBody));
        }
        // VFS-13: the suggestion goes first, as the one primary action.
        if (sg) actions.unshift(button(suggestionLabel(S, N, sg), 'btn btn-primary', () => applySuggestion(sg)));
      }
    } else {
      box.hidden = true;
      setView();
      return;
    }
    setView();
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

  /** Before any search the box sits in the middle of an empty page; results move it up. */
  function setView() {
    $('app').dataset.view = viewFor({
      backendMissing: !!api.missing,
      rootsLoaded: !!state.rootsLoaded,
      rootCount: state.roots.length,
      searchState: state.searchState,
    });
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
    if (hit.source === Source.BOOKMARKS) {
      const where = bookmarkPathParts(hit, S.bookmarkRoots).join(PATH_SEPARATOR);
      const parts = [el('strong', null, `${S.detailsBookmarkFolder} `), where];
      if (!hit.isFolder) parts.push(' · ', el('strong', null, `${S.detailsBookmark} `), hit.url);
      d.replaceChildren(...parts);
      return;
    }
    const full = F.joinPath(hit.dir, hit.name);
    if (hit.link) {
      const parts = [el('strong', null, `${S.detailsLinkAt[hit.link.kind]} `), full];
      if (hit.link.target) {
        const label = hit.link.kind === LinkKind.INTERNET ? S.detailsBookmark : S.detailsLinkTarget;
        parts.push(' · ', el('strong', null, `${label} `), hit.link.target);
      }
      d.replaceChildren(...parts);
      return;
    }
    d.replaceChildren(el('strong', null, `${S.detailsLabel} `), full);
  }

  /* Screen readers hear the result count once typing pauses, not on every keystroke. */
  let announceTimer = 0;
  function announceSoon() {
    clearTimeout(announceTimer);
    announceTimer = setTimeout(() => {
      const a = $('announcer');
      if (state.searchState === SearchState.DONE) {
        a.textContent = state.total ? $('summary').textContent : ($('empty').querySelector('h3')?.textContent || '');
      } else if (state.searchState === SearchState.ERROR) {
        a.textContent = S.emptyErrorTitle;
      }
    }, 900);
  }

  /* ---------------------------------------------------------------- notices */

  /** Text for a rejected call, in the active language; `byCode` holds the texts of one action, if any. */
  function errorText(err, byCode) {
    const code = err?.code;
    if (code === UiErrorCode.BACKEND_MISSING) return S.backendMissing;
    return byCode?.[code] || S.errors.any[code] || S.errors.generic;
  }

  function showError(text) {
    // While a modal is open the page is inert, so the message goes inside the folder panel.
    if ($('folders').open) { folderMessage(text); return; }
    $('banner-error-text').textContent = text;
    $('banner-error').hidden = false;
  }
  $('banner-error-close').addEventListener('click', () => { $('banner-error').hidden = true; q.focus(); });

  function renderUnavailableBanner() {
    const missing = state.roots.filter((r) => r.status === RootStatus.UNAVAILABLE).map((r) => r.path);
    const key = missing.join('|');
    const banner = $('banner-unavailable');
    if (!missing.length || key === state.unavailableDismissed) { banner.hidden = true; return; }
    $('banner-unavailable-text').textContent = missing.length === 1
      ? S.unavailableOne(missing[0])
      : S.unavailableMany(missing.map(S.quoted).join(', '));
    banner.hidden = false;
  }
  $('banner-unavailable-close').addEventListener('click', () => {
    state.unavailableDismissed = state.roots.filter((r) => r.status === RootStatus.UNAVAILABLE).map((r) => r.path).join('|');
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
    $('rescan-now').hidden = true;
    if (api.missing) { text.textContent = ''; return; }
    if (!state.rootsLoaded) { text.textContent = S.statusConnecting; return; }
    if (!state.roots.length) { text.textContent = S.statusNoRoots; return; }
    text.textContent = statusText(L, p, state.roots);
    const actions = footerActions(p);
    bar.hidden = p?.state !== IndexerState.SCANNING;
    pause.hidden = !actions.pause;
    pause.textContent = actions.paused ? S.resume : S.pause;
    $('rescan-now').hidden = !actions.rescan;
    $('banner-scan').hidden = !(firstScanRunning() && hasQuery());
  }

  $('pause').addEventListener('click', async () => {
    const paused = state.progress?.state !== IndexerState.PAUSED;
    try {
      await api.setPaused(paused);
      if (state.progress) state.progress.state = paused ? IndexerState.PAUSED : IndexerState.SCANNING;
      renderStatus();
    } catch (err) { showError(errorText(err)); }
  });

  /* ---------------------------------------------------------------- settings panel */

  // VFS-22 R3, R5: the shell (window title, macOS menu, folder picker) follows the UI.
  api.setLanguage(L.lang).catch(() => {});

  const settingsDialog = $('settings');
  fillSelect($('language'), S.languageOptions);
  $('open-settings').addEventListener('click', () => {
    $('language').value = choiceFor(savedLanguage());
    settingsDialog.showModal();
  });
  // A new language redraws the whole screen in it: the page reloads with the new choice.
  $('language').addEventListener('change', (e) => {
    saveLanguage(e.target.value);
    location.reload();
  });

  /* ---------------------------------------------------------------- feedback (VFS-20) */

  setupFeedback({ api, S, $, errorText });

  /* ---------------------------------------------------------------- folder panel */

  const foldersDialog = $('folders');
  $('open-folders').addEventListener('click', () => {
    $('folders-msg').hidden = true;
    renderRoots();
    foldersDialog.showModal();
  });

  function folderMessage(text, kind = Notice.ERROR) {
    $('folders-msg-text').textContent = text;
    $('folders-msg').className = `banner ${kind === Notice.ERROR ? 'banner-error' : kind === Notice.WARN ? 'banner-warn' : 'banner-info'}`;
    $('folders-msg').querySelector('use').setAttribute('href', kind === Notice.INFO ? '#i-info' : '#i-warn');
    $('folders-msg').hidden = false;
  }

  /* Rows are updated in place (keyed by root id) rather than rebuilt on every progress tick, so
     a button never disappears under the pointer or loses keyboard focus. */
  const rootItems = new Map(); // id -> {li, state, files, checked, rescan, remove}

  function buildRootItem(r) {
    const li = el('li', 'root');
    const path = el('div', 'root-path');
    path.append(icon(FileKind.FOLDER), el('span', null, r.path));
    const meta = el('div', 'root-meta');
    const st = el('span', 'root-state');
    const files = el('span');
    const checked = el('span');
    meta.append(st, files, checked);
    // VFS-10 R5: "N carpetas no se pudieron revisar · Ver", shown while there are any.
    const unreadable = el('div', 'root-unreadable');
    const unreadableText = el('span');
    const see = button(S.unreadableSee, 'link-btn', () => openUnreadable(r.id));
    unreadable.append(unreadableText, ' · ', see);
    unreadable.hidden = true;
    const actions = el('div', 'root-actions');
    const name = F.baseName(r.path);
    // VFS-32 R8: "Pausar" / "Reanudar" / "Volver a revisar", set by renderRoots.
    const rescan = button(S.rescan, 'btn', () => rootActionClicked(r.id, rescan.dataset.action));
    const remove = button(S.remove, 'btn btn-quiet', () => {
      const current = state.roots.find((x) => x.id === r.id);
      if (current) confirmRemove(current);
    });
    remove.setAttribute('aria-label', `${S.remove}: ${name}`);
    actions.append(rescan, remove);
    li.append(path, meta, unreadable, actions);
    return { li, st, files, checked, rescan, remove, unreadable, unreadableText, name };
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
      const status = r.status === RootStatus.REMOVING ? r.status : progressById.get(r.id)?.status ?? r.status;
      const paused = r.paused && status !== RootStatus.REMOVING;
      item.st.textContent = paused ? S.rootPaused : S.rootStatus[status] || status;
      item.st.dataset.status = paused ? 'paused' : status;
      item.files.textContent = S.rootFiles(N.count(progressById.get(r.id)?.filesIndexed ?? r.files));
      item.checked.textContent = r.lastFullScan ? S.rootChecked(N.date(r.lastFullScan)) : S.rootNeverChecked;
      const unreadable = S.unreadableLine(r.unreadable || 0, N.count(r.unreadable || 0));
      item.unreadable.hidden = !unreadable;
      item.unreadableText.textContent = unreadable;
      const action = rootAction(r, status);
      const label = { [RootAction.PAUSE]: S.rootPause, [RootAction.RESUME]: S.rootResume, [RootAction.RESCAN]: S.rescan }[action];
      item.rescan.dataset.action = action;
      item.rescan.textContent = label;
      item.rescan.setAttribute('aria-label', `${label}: ${item.name}`);
      item.rescan.disabled = action === RootAction.RESCAN && status !== RootStatus.READY && status !== RootStatus.UNAVAILABLE;
      item.remove.disabled = status === RootStatus.REMOVING;
    }
  }

  const unreadableDialog = $('unreadable');

  /** The folders of root `id` that could not be read, each with its reason (VFS-10 R5). */
  async function openUnreadable(id) {
    const root = state.roots.find((r) => r.id === id);
    let dirs;
    try { dirs = await api.listUnreadable(id); } catch (err) { showError(errorText(err)); return; }
    $('unreadable-list').replaceChildren(...dirs.map((d) => {
      const li = el('li', 'root');
      const path = el('div', 'root-path');
      path.append(icon(FileKind.FOLDER), el('span', null, d.path));
      const meta = el('div', 'root-meta');
      meta.append(el('span', null, S.unreadableReason[d.reason] || d.reason));
      const actions = el('div', 'root-actions');
      actions.append(button(S.revealButton, 'btn', () => revealFile(d.id, true)));
      li.append(path, meta, actions);
      return li;
    }));
    const more = Math.max(0, (root?.unreadable || 0) - dirs.length);
    $('unreadable-more').hidden = !more;
    $('unreadable-more').textContent = more ? S.unreadableMore(N.count(more)) : '';
    unreadableDialog.showModal();
    $('unreadable-list').querySelector('button')?.focus();
  }

  async function rootActionClicked(id, action) {
    if (action === RootAction.RESCAN) { rescanRoot(id); return; }
    try {
      await api.setRootPaused(id, action === RootAction.PAUSE);
      await refreshRoots();
    } catch (err) { showError(errorText(err)); }
  }

  async function rescanRoot(id) {
    try {
      await api.rescan(id);
      if (foldersDialog.open) folderMessage(S.rescanStarted, Notice.INFO); else toast(S.rescanStarted);
      await refreshRoots();
    } catch (err) { showError(errorText(err)); }
  }
  $('rescan-all').addEventListener('click', () => rescanRoot(null));
  $('rescan-now').addEventListener('click', () => rescanRoot(null));
  $('add-folder').addEventListener('click', () => addFolder());

  /** Explains an overlap precisely (VFS-1 U4) from the folders the UI already knows: the
   * core refuses only the same folder or one inside a listed folder (VFS-31 R3). */
  function overlapText(path) {
    const inside = state.roots.find((r) => F.isUnder(path, r.path));
    if (inside) return S.overlapInside(inside.path);
    return S.overlapGeneric;
  }

  /** The core reports the kind of drive a folder is on; network and removable ones get a
   * warning (their files read as unavailable while disconnected). */
  const REMOTE_DRIVES = new Set([DriveKind.NETWORK, DriveKind.REMOVABLE]);

  async function addFolder() {
    let path;
    try {
      path = await api.pickFolder();
    } catch (err) { showError(errorText(err)); return; }
    if (!path) return;
    const takenIn = takenInBy(state.roots, path);
    try {
      const root = await api.addRoot(path);
      await refreshRoots();
      const msg = takenIn.length ? S.rootTakesIn(path, takenIn) : S.rootAdded(path);
      const remote = REMOTE_DRIVES.has(root.drive);
      if (foldersDialog.open) {
        folderMessage(remote ? `${msg} ${S.remoteWarning}` : msg, remote ? Notice.WARN : Notice.INFO);
      } else {
        toast(msg);
        if (remote) showError(S.remoteWarning);
      }
    } catch (err) {
      showError(err?.code === ErrorCode.OVERLAPPING_ROOT ? overlapText(path) : errorText(err, S.errors.addRoot));
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

  /* ---------------------------------------------------------------- bookmarks setting (VFS-6) */

  const bookmarksToggle = $('bookmarks-on');

  function setBookmarks(status) {
    state.bookmarks = status;
    if (!bookmarksOn()) {
      state.source = SourceChoice.BOTH;
      state.pickedBookmarkFolder = null;
      if (bookmarkFolderOf(state.folder) != null) state.folder = '';
    }
    renderBookmarks();
    renderSources();
    renderFolderSelect();
    renderClearButton();
  }

  function renderBookmarks() {
    const st = state.bookmarks;
    bookmarksToggle.checked = !!st?.enabled;
    const parts = [];
    if (st?.enabled) {
      parts.push(S.bookmarksState[st.state] || '');
      if (st.count && st.state !== ReadState.READING) parts.push(S.bookmarksCount(N.count(st.count), N.date(st.lastRead)));
    }
    $('bookmarks-status').textContent = parts.filter(Boolean).join(' ');
  }

  bookmarksToggle.addEventListener('change', async () => {
    try {
      const status = await api.setBookmarksEnabled(bookmarksToggle.checked);
      // Turning it on starts a read in the background; say so until it reports.
      if (status.enabled && status.state === ReadState.NEVER) status.state = ReadState.READING;
      setBookmarks(status);
      runSearch({ quiet: true });
    } catch (err) {
      bookmarksToggle.checked = bookmarksOn();
      showError(errorText(err));
    }
  });

  function onBookmarksChanged(status) {
    setBookmarks(status);
    if (hasQuery() && state.searchState === SearchState.DONE && list.atTop()) runSearch({ quiet: true });
  }

  /* ---------------------------------------------------------------- updates (VFS-21) */

  const updatesToggle = $('updates-on');
  const updates = createUpdates({ api, S, render: renderUpdate });

  function renderUpdate(view) {
    $('banner-update').hidden = !view;
    if (!view) return;
    $('banner-update-text').textContent = view.text;
    const bar = $('update-progress');
    bar.hidden = view.progress == null;
    if (view.progress != null) bar.value = view.progress;
    const action = $('update-action');
    action.hidden = !view.action;
    action.textContent = view.action || '';
    $('update-later').hidden = !view.later;
  }

  $('update-action').addEventListener('click', () => updates.install());
  $('update-later').addEventListener('click', () => updates.dismiss());

  updatesToggle.addEventListener('change', async () => {
    try {
      updatesToggle.checked = (await api.setUpdateCheck(updatesToggle.checked)).checkOnOpen;
    } catch (err) {
      updatesToggle.checked = !updatesToggle.checked;
      showError(errorText(err));
    }
  });

  // VFS-33: «Incluir carpetas del sistema y de programas»; a change re-walks every folder.
  const includeAll = $('include-all');
  includeAll.addEventListener('change', async () => {
    try {
      includeAll.checked = await api.setIncludeAllFolders(includeAll.checked);
    } catch (err) {
      includeAll.checked = !includeAll.checked;
      showError(errorText(err));
    }
  });
  api.getIncludeAllFolders().then((on) => { includeAll.checked = on; }, () => {});

  async function startUpdates() {
    try {
      updatesToggle.checked = (await api.getUpdateSettings()).checkOnOpen;
    } catch { /* the toggle stays as rendered */ }
    try { await api.onUpdateProgress((p) => updates.onProgress(p)); } catch { /* no progress bar */ }
    updates.start();
  }

  /* ---------------------------------------------------------------- live data */

  const listRoots = newestOnly(() => api.listRoots());
  async function refreshRoots() {
    let reply;
    try {
      reply = await listRoots();
    } catch (err) {
      if (!api.missing) showError(errorText(err));
      return;
    }
    if (reply.stale) return;
    state.roots = reply.value;
    state.rootsLoaded = true;
    renderFolderSelect();
    renderClearButton();
    renderUnavailableBanner();
    renderStatus();
    if (foldersDialog.open) renderRoots();
    if (state.searchState === SearchState.IDLE || !state.total) renderEmpty();
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
      if (state.searchState === SearchState.IDLE) renderEmpty();
    }
  }

  function onIndexChanged() {
    // New files arrived. Refresh the list only when it is scrolled to the top, so rows never
    // jump under the user's eyes.
    if (hasQuery() && state.searchState === SearchState.DONE && list.atTop()) runSearch({ quiet: true });
  }

  async function start() {
    renderClearButton();
    renderFolderSelect();
    renderStatus();
    q.focus();
    if (api.missing) { renderEmpty(); return; }
    await api.onProgress(onProgress);
    await api.onIndexChanged(onIndexChanged);
    await api.onBookmarksChanged(onBookmarksChanged);
    try { setBookmarks(await api.getBookmarksStatus()); } catch { /* files only */ }
    await refreshRoots();
    try { onProgress(await api.getProgress()); } catch { /* the event stream will catch up */ }
    renderEmpty();
    startUpdates();
  }

  start();
}

main(await loadLocale());
