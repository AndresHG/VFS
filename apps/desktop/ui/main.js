// The one screen (VFS-1 R9): search box, filters, virtualised results, folder panel, status.

import { api } from './api.js';
import { choiceFor, languageFor, localeFor } from './strings.js';
import * as F from './format.js';
import { createVirtualList } from './vlist.js';
import { placesOf, showsCloudBadge } from './copies.js';
import { ROW_KINDS, rowKindOf } from './kinds.js';
import { SecondAction, secondActionOf } from './links.js';
import { createUpdates } from './updates.js';
import { wantsSuggestion, suggestionLabel, folderAfterClear } from './suggest.js';
import { setupFeedback } from './feedback.js';
import { setupAiSettings } from './ai.js';
import { statusText, firstScanRunning as firstScanOf, viewFor, footerActions, rootAction, RootAction } from './status.js';
import { takenInBy, newestOnly } from './folders.js';
import { addPattern, patternErrorText, parseLimit, showContentLimit } from './filefilter.js';
import {
  ContentAction, contentOn, contentQueryDue, contentStatusLine, createContentSearch, hitPage, offerDue, offerText, sectionNotes, snippetRuns,
  unreadCandidates, unreadLabel,
} from './content.js';
import {
  ContentPhase, DriveKind, ErrorCode, FileKind, IndexerState, LinkKind, MatchKind, ReadState, RevealOutcome, RootStatus,
  SortOrder, Source, TruncatedBy, UiErrorCode,
} from './contract.js';
import {
  DateChoice, LanguageChoice, ScopeChoice, SearchState, SizeChoice, SourceChoice, TypeChecklist, View,
  scopeFor, scopesFor, typeChecklistMode,
} from './choices.js';
import { PrefKey, readPref, webStorage, writePref } from './prefs.js';
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
/** Icons of the row actions (VFS-6); the row's own icon comes from kinds.js (VFS-36). */
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
  const store = webStorage();
  const state = {
    coreReady: false, // VFS-39 R2: false until wait_core resolves; searches wait for it
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
    // VFS-38 R6: remembered; source and sort are read once the bookmark setting is known.
    // «Solo documentos» is checked against content search once its status is known (R5).
    scope: readPref(store, PrefKey.SCOPE, scopesFor(true), ScopeChoice.ALL),
    source: SourceChoice.BOTH,
    sort: SortOrder.RELEVANCE,
    bookmarks: null, // BookmarkStatus, null until known
    pickedBookmarkFolder: null, // {id, title} chosen with "Ver su contenido"
    content: null, // VFS-27: ContentStatus, null until known
    unread: new Map(), // VFS-27 R14: file id -> UnreadReason|null, for the pages on screen
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

  // A popover opens under its button. Placed from the button's box so it also works where
  // CSS anchor positioning is missing (older WKWebView).
  function placeUnder(pop, button) {
    pop.addEventListener('beforetoggle', (e) => {
      if (e.newState !== 'open') return;
      const r = button.getBoundingClientRect();
      pop.style.top = `${r.bottom + 6}px`;
      pop.style.left = `${Math.max(8, Math.min(r.left, innerWidth - pop.offsetWidth - 8))}px`;
    });
  }

  // Without the Popover API at all, the type chips are shown inline instead.
  const typePop = $('f-type-pop');
  const popoverMode = typeChecklistMode(HTMLElement.prototype);
  $('filters').dataset.types = popoverMode;
  if (popoverMode === TypeChecklist.INLINE) $('f-type').hidden = true;
  placeUnder(typePop, $('f-type'));
  typePop.addEventListener('toggle', (e) => {
    if (e.newState === 'open') chips.querySelector('input')?.focus();
  });

  // VFS-38 R7: «Más filtros» opens a popover, or without the Popover API unfolds its fields as a
  // row below the line (same test as the type checklist). Changes there apply at once.
  const morePop = $('f-more-pop');
  $('filters').dataset.more = popoverMode;
  const firstMoreField = () => [...morePop.querySelectorAll('select')].find((s) => !s.disabled && s.offsetParent);
  if (popoverMode === TypeChecklist.INLINE) {
    morePop.removeAttribute('popover');
    $('f-more').removeAttribute('popovertarget');
    morePop.hidden = true;
    $('f-more').setAttribute('aria-expanded', 'false');
    $('f-more').addEventListener('click', () => {
      morePop.hidden = !morePop.hidden;
      $('f-more').setAttribute('aria-expanded', String(!morePop.hidden));
      if (!morePop.hidden) firstMoreField()?.focus();
    });
  }
  placeUnder(morePop, $('f-more'));
  morePop.addEventListener('toggle', (e) => {
    if (e.newState === 'open') firstMoreField()?.focus();
  });

  /* VFS-38 R1: the scope group, a radio group with one tab stop; the arrow keys move the choice
     and every change runs the search again (the newer query drops the older, VFS-1 S6). */
  const scopeGroup = $('f-scope');
  const scopeRadios = () => [...scopeGroup.querySelectorAll('[role="radio"]')];

  /** The offered scopes: «Solo documentos» while content search is on, and while its status
   *  is not known yet when it is the remembered choice (R5 decides once it is). */
  function offeredScopes() {
    return scopesFor(state.content ? contentOn(state.content) : state.scope === ScopeChoice.DOCUMENTS);
  }

  function renderScope() {
    const offered = offeredScopes();
    if (scopeRadios().map((b) => b.dataset.scope).join() !== offered.join()) {
      const focused = scopeGroup.contains(document.activeElement);
      for (const b of scopeRadios()) b.remove();
      for (const choice of offered) {
        const b = el('button', 'scope-btn', S.scopeOptions[choice]);
        b.type = 'button';
        b.setAttribute('role', 'radio');
        b.dataset.scope = choice;
        scopeGroup.append(b);
      }
      if (focused) scopeGroup.querySelector(`[data-scope="${state.scope}"]`)?.focus();
    }
    for (const b of scopeRadios()) {
      const chosen = b.dataset.scope === state.scope;
      b.setAttribute('aria-checked', String(chosen));
      b.tabIndex = chosen ? 0 : -1;
    }
    scopeGroup.classList.toggle(ON, state.scope !== ScopeChoice.ALL);
    $('main').dataset.scope = state.scope; // VFS-38 R4: the layout without a name list
  }

  function setScope(choice) {
    if (choice === state.scope) return;
    state.scope = choice;
    writePref(store, PrefKey.SCOPE, choice);
    renderScope();
    renderSources();
    filtersChanged();
  }

  scopeGroup.addEventListener('click', (e) => {
    const b = e.target.closest('[role="radio"]');
    if (b) setScope(b.dataset.scope);
  });
  /** Arrow keys step through the radios (wrapping), Home and End jump to the ends. */
  const SCOPE_STEPS = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };
  scopeGroup.addEventListener('keydown', (e) => {
    const radios = scopeRadios();
    const at = radios.indexOf(e.target);
    if (at < 0) return;
    let next;
    if (e.key in SCOPE_STEPS) next = (at + SCOPE_STEPS[e.key] + radios.length) % radios.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = radios.length - 1;
    else return;
    e.preventDefault();
    radios[next].focus();
    setScope(radios[next].dataset.scope);
  });
  renderScope();

  /** VFS-38 R10: «Buscar en» shows while the filter line fits on one row with it; otherwise
   *  it is kept for screen readers only, and no other control shrinks. */
  function fitScopeLabel() {
    const label = $('f-scope-label');
    const form = $('filters');
    label.classList.remove('visually-hidden');
    const first = scopeGroup.getBoundingClientRect();
    const last = $('f-more').getBoundingClientRect();
    if (Math.round(last.top) !== Math.round(first.top) || form.scrollWidth > form.clientWidth) label.classList.add('visually-hidden');
  }
  new ResizeObserver(fitScopeLabel).observe($('filters'));
  document.fonts?.ready.then(fitScopeLabel);

  function fillSelect(select, options) {
    select.replaceChildren(...Object.entries(options).map(([v, t]) => new Option(t, v)));
  }
  fillSelect($('f-date'), S.dateOptions);
  fillSelect($('f-size'), S.sizeOptions);
  fillSelect($('f-source'), S.sourceOptions);
  $('f-source').value = state.source;
  $('f-source').addEventListener('change', (e) => { setSource(e.target.value); filtersChanged(); });
  $('f-sort').addEventListener('change', (e) => {
    state.sort = e.target.value;
    writePref(store, PrefKey.SORT, state.sort);
    renderSources();
    filtersChanged();
  });
  // The "Orden" options before the bookmark setting is known (VFS-12).
  renderSources();

  function bookmarksOn() {
    return !!state.bookmarks?.enabled;
  }

  /** Whether the source choice applies: bookmarks on, and not under «Solo documentos», which
   *  searches no bookmarks (VFS-38 R4). */
  function sourceApplies() {
    return bookmarksOn() && state.scope !== ScopeChoice.DOCUMENTS;
  }

  /** The source choice (VFS-6), remembered (VFS-38 R6). */
  function setSource(choice) {
    state.source = choice;
    writePref(store, PrefKey.SOURCE, choice);
    renderSources();
  }

  /** VFS-38 R6: the remembered source and sort, once the bookmark setting is known; a choice
   *  not offered now gives the default. */
  function restoreChoices() {
    const on = bookmarksOn();
    state.source = on ? readPref(store, PrefKey.SOURCE, Object.values(SourceChoice), SourceChoice.BOTH) : SourceChoice.BOTH;
    state.sort = readPref(store, PrefKey.SORT, sortChoices(on), SortOrder.RELEVANCE);
    renderSources();
    renderClearButton();
  }

  /** «Origen» shows only while bookmarks are included, and "Orden" offers "by source" only
   *  then (VFS-12); the size filter does not apply to bookmarks alone (VFS-6 R7). */
  function renderSources() {
    const on = bookmarksOn();
    $('f-source-field').hidden = !sourceApplies();
    state.sort = sortFor(state.sort, on);
    const sorts = $('f-sort');
    sorts.replaceChildren(...sortChoices(on).map((o) => new Option(S.sortOptions[o], o)));
    sorts.value = state.sort;
    $('f-source').value = state.source;
    $('f-size').disabled = !sizeApplies();
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
    return sizeFilterApplies(state.source, sourceApplies());
  }

  /** The filters that narrow the files: type, date, size and folder. Any of them makes a
   *  search without words list files. */
  function narrowingCount() {
    const size = sizeApplies() && state.size !== SizeChoice.ANY;
    return state.kinds.size + (state.date !== DateChoice.ANY) + size + (state.folder !== '');
  }

  /** The name a «Carpeta» chip shows: a disk folder's last segment, or a bookmark folder's. */
  function folderName() {
    const id = bookmarkFolderOf(state.folder);
    if (id == null) return F.baseName(state.folder);
    if (state.pickedBookmarkFolder?.id === id) return state.pickedBookmarkFolder.title;
    const root = state.bookmarks?.roots?.find((r) => r.id === id);
    return root ? bookmarkPathParts({ root: root.root, folder: [], profile: root.profile }, S.bookmarkRoots).join(PATH_SEPARATOR) : '';
  }

  /** VFS-38 R7, R8: the active filters behind «Más filtros», each with its chip text and how
   *  to clear it alone. */
  function moreFilters() {
    const out = [];
    if (sizeApplies() && state.size !== SizeChoice.ANY) {
      out.push({ text: S.chipSize(S.sizeOptions[state.size]), clear: () => { state.size = SizeChoice.ANY; $('f-size').value = state.size; } });
    }
    if (state.folder !== '') {
      out.push({ text: S.chipFolder(folderName()), clear: () => { state.folder = ''; $('f-folder').value = ''; } });
    }
    if (sourceApplies() && state.source !== SourceChoice.BOTH) {
      out.push({ text: S.sourceChips[state.source], clear: () => setSource(SourceChoice.BOTH) });
    }
    return out;
  }

  /** VFS-38 R8: every active filter, the source and the scope included. */
  function activeFilterCount() {
    return state.kinds.size + (state.date !== DateChoice.ANY) + moreFilters().length + (state.scope !== ScopeChoice.ALL);
  }

  function renderClearButton() {
    const more = moreFilters();
    const n = activeFilterCount();
    $('active-filters').hidden = n === 0; // nothing to clear
    $('f-clear').textContent = n ? S.clearFiltersN(n) : S.clearFilters;
    $('filter-tags').replaceChildren(...more.map(({ text, clear }) => {
      const b = button(text, 'filter-tag', () => { clear(); filtersChanged(); q.focus(); });
      b.setAttribute('aria-label', S.removeFilter(text));
      b.append(icon('close'));
      return b;
    }));
    $('f-more-text').textContent = more.length ? S.moreFiltersN(more.length) : S.moreFilters;
    renderTypeButton();
    // A filter that narrows the list looks chosen.
    $('f-date').parentElement.classList.toggle(ON, state.date !== DateChoice.ANY);
    $('f-more').classList.toggle(ON, more.length > 0);
    fitScopeLabel();
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
    const query = { text: state.text.trim(), filters: filters(), limit: PAGE, offset, scope: scopeFor(state.scope) };
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
  /** «Quitar filtros» (VFS-38 R8): clears type, date, size, folder, source and scope and keeps
   *  the sort; a suggestion (VFS-13) keeps a bookmark folder. */
  function clearFilters({ keepBookmarkFolder = false } = {}) {
    state.kinds.clear();
    for (const c of chips.querySelectorAll('input')) c.checked = false;
    state.date = DateChoice.ANY; $('f-date').value = DateChoice.ANY;
    $('date-range').hidden = true; $('f-from').value = ''; $('f-to').value = '';
    $('range-error').textContent = '';
    state.size = SizeChoice.ANY; $('f-size').value = SizeChoice.ANY;
    state.folder = folderAfterClear(state.folder, keepBookmarkFolder);
    $('f-folder').value = state.folder;
    if (state.source !== SourceChoice.BOTH) setSource(SourceChoice.BOTH);
    if (state.scope !== ScopeChoice.ALL) {
      state.scope = ScopeChoice.ALL;
      writePref(store, PrefKey.SCOPE, state.scope);
      renderScope();
      renderSources();
    }
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

  /** Words, or a filter that narrows the files; the scope and source alone list nothing. */
  function hasQuery() {
    return state.text.trim() !== '' || narrowingCount() > 0;
  }

  let loadingTimer = 0;

  /** Runs page 0 of the current query. `quiet` keeps the list and selection (live refresh). */
  async function runSearch({ quiet = false } = {}) {
    const seq = ++state.seq;
    // A live refresh keeps the suggestion on screen until its own answer replaces it.
    if (!quiet) state.suggestion = null;
    clearTimeout(loadingTimer);
    if (!quiet) runContentQuery();
    if (!hasQuery()) {
      state.searchState = SearchState.IDLE;
      setResults(0, null, new Map());
      return;
    }
    if (state.scope === ScopeChoice.DOCUMENTS) {
      // VFS-38 R4: only the content section; no name or path search, no suggestion.
      state.searchState = SearchState.DONE;
      setResults(0, null, new Map());
      return;
    }
    // A query typed while the search is getting ready runs once it is (start()).
    if (!state.coreReady && !api.missing) return;
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
    const query = { text: state.text.trim(), filters: filters(), limit: 1, offset: 0, scope: scopeFor(state.scope) };
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
    state.unread = new Map();
    loadUnread(pages.get(0) || []);
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
      loadUnread(res.hits);
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
    const rowKind = rowKindOf(hit);
    ico.dataset.kind = rowKind;
    ico.replaceChildren(icon(ROW_KINDS[rowKind].icon));

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
    // VFS-27 R14: why its text was not read; a cloud-only file already says so.
    const unread = state.unread.get(hit.id);
    const unreadText = unreadLabel(S, hit, unread);
    if (unreadText) nameParts.push(el('span', 'badge badge-unread', unreadText));
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

    const kindLabel = ROW_KINDS[rowKind].label(S);
    const dateText = N.date(hit.modified);
    const sizeText = hit.isDir ? '' : N.size(hit.size);
    kind.textContent = kindLabel;
    date.textContent = dateText;
    size.textContent = sizeText;
    const base = S.rowLabel(hit.name, kindLabel, hit.dir, dateText, sizeText, !!unavailable, places);
    const label = unreadText ? `${base}, ${unreadText}` : base;
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
    const rowKind = rowKindOf(hit);
    ico.dataset.kind = rowKind;
    ico.replaceChildren(icon(ROW_KINDS[rowKind].icon));
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
    const kindLabel = ROW_KINDS[rowKind].label(S);
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

  /** The results header (VFS-38 R9): the count, and «Orden» while there are results. */
  function renderSummary() {
    const s = $('summary');
    const fuzzy = $('fuzzy-note');
    fuzzy.hidden = true;
    renderSummaryText(s);
    s.title = s.textContent;
    // «Orden» stays while a newer search is loading, so the header does not jump.
    $('f-sort-pill').hidden = !state.total;
    $('results-head').hidden = !s.textContent && !state.total;
    if (state.searchState === SearchState.DONE && state.total) fuzzy.hidden = hitAt(0)?.matched !== MatchKind.FUZZY;
  }

  function renderSummaryText(s) {
    if (state.searchState === SearchState.LOADING) { s.textContent = S.searching; return; }
    if (state.searchState !== SearchState.DONE || !state.total) { s.textContent = ''; return; }
    const n = N.count(state.total);
    if (state.approximateOrder) s.textContent = S.resultsApproximate(n);
    else if (state.truncatedBy === TruncatedBy.CAP) s.textContent = S.resultsCapped(n);
    else if (state.truncatedBy === TruncatedBy.DEADLINE) s.textContent = S.resultsPartial(n);
    else s.textContent = S.results(n, state.total === 1);
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
    } else if (state.searchState === SearchState.DONE && state.scope === ScopeChoice.DOCUMENTS) {
      // VFS-38 R4: content results only. A short query asks for 3 letters; no match offers «Todo».
      const view = contentSearch.view();
      if (!contentQueryDue(state.text)) {
        mode = 'documents-short';
        parts.push(icon('search'), el('h3', null, S.emptyDocumentsShort));
      } else if (!view || (!view.loading && !view.failed && !view.hits.length)) {
        mode = `documents-none:${state.text.trim()}`;
        parts.push(icon('search'), el('h3', null, S.emptyNoHitsTitle(state.text.trim())), el('p', null, S.contentNone));
        actions = [button(S.searchEverything, 'btn btn-primary', () => { setScope(ScopeChoice.ALL); q.focus(); })];
      } else {
        box.hidden = true;
        setView();
        return;
      }
    } else if (state.searchState === SearchState.DONE && !state.total && contentSearch.view()?.hits.length) {
      // No name hits, but hits inside documents below: no "nothing found" over them (R12).
      box.hidden = true;
      setView();
      return;
    } else if (state.searchState === SearchState.DONE && !state.total) {
      if (state.truncatedBy === TruncatedBy.DEADLINE) {
        mode = TruncatedBy.DEADLINE;
        parts.push(icon('info'), el('h3', null, S.emptyDeadlineTitle), el('p', null, S.emptyDeadlineBody));
        actions = [button(S.retry, 'btn btn-primary', () => runSearch())];
      } else {
        const sg = state.suggestion;
        const namesOnly = state.scope === ScopeChoice.NAMES;
        const filtered = activeFilterCount() - namesOnly;
        mode = `no-hits:${filtered > 0}:${namesOnly}:${sg ? sg.kind + sg.text : ''}`;
        parts.push(icon('search'), el('h3', null, S.emptyNoHitsTitle(state.text.trim())));
        // VFS-38 R13: a names-only search says so; «Quitar filtros» also brings back «Todo».
        if (namesOnly) parts.push(el('p', null, S.emptyNamesOnly));
        if (activeFilterCount()) {
          if (filtered) parts.push(el('p', null, S.emptyNoHitsFilters));
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
    text.title = '';
    if (api.missing) { text.textContent = ''; return; }
    if (!state.rootsLoaded) { text.textContent = S.statusConnecting; return; }
    if (!state.roots.length) { text.textContent = S.statusNoRoots; return; }
    text.textContent = statusText(L, p, state.roots);
    text.title = text.textContent; // clamped to one line on short windows with results
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
    refreshContentStore();
    ai.refresh();
    settingsDialog.showModal();
  });
  // A new language redraws the whole screen in it: the page reloads with the new choice.
  $('language').addEventListener('change', (e) => {
    saveLanguage(e.target.value);
    location.reload();
  });

  /* ---------------------------------------------------------------- feedback (VFS-20) */

  setupFeedback({ api, S, $, errorText });

  /* ---------------------------------------------------------------- AI (VFS-28) */

  const ai = setupAiSettings({ api, S, lang: L.lang, $, el, toast });

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

  // VFS-34: «Filtro de archivos». Adding a pattern hides its files at once (no walk);
  // removing one or restoring the list brings files back after a full check.
  const filterList = $('file-filter-list');
  const filterInput = $('file-filter-pattern');
  filterInput.placeholder = S.fileFilterPlaceholder;
  function filterMessage(text, error = false) {
    const msg = $('file-filter-msg');
    msg.textContent = text;
    msg.toggleAttribute('data-error', error);
  }
  function renderFilter(list) {
    filterList.replaceChildren(...list.map((p) => {
      const remove = el('button', 'icon-btn');
      remove.type = 'button';
      remove.append(icon('close'), el('span', 'visually-hidden', S.fileFilterRemove(p)));
      remove.addEventListener('click', async () => {
        try {
          renderFilter(await api.removeFileExclusion(p));
          filterMessage(S.fileFilterComeBack);
        } catch (err) { filterMessage(errorText(err), true); }
      });
      const item = el('li');
      item.append(el('code', null, p), remove);
      return item;
    }));
  }
  const confirmFilter = $('confirm-filter');
  function askFilter(pattern, count) {
    $('confirm-filter-desc').textContent = S.fileFilterConfirmBody(pattern, N.count(count));
    confirmFilter.returnValue = '';
    confirmFilter.showModal();
    return new Promise((resolve) => {
      confirmFilter.addEventListener('close', () => resolve(confirmFilter.returnValue === 'add'), { once: true });
    });
  }
  $('file-filter-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const pattern = filterInput.value.trim();
    try {
      const r = await addPattern(api, pattern, (n) => askFilter(pattern, n));
      if (r.cancelled) return;
      renderFilter(r.list);
      filterInput.value = '';
      filterMessage(S.fileFilterAdded(pattern, N.count(r.count), r.count === 0));
    } catch (err) {
      filterMessage(patternErrorText(err, S) ?? errorText(err), true);
    }
    filterInput.focus();
  });
  $('file-filter-reset').addEventListener('click', async () => {
    try {
      renderFilter(await api.resetFileExclusions());
      filterMessage(S.fileFilterComeBack);
    } catch (err) { filterMessage(errorText(err), true); }
  });
  api.getFileExclusions().then(renderFilter, () => {});

  // VFS-34 R6: the content size limit, shown only while content search is on (setContent
  // follows the switch).
  const contentMax = $('content-max-mb');
  showContentLimit($('content-max-row'), false);
  api.getContentMaxMb().then((mb) => { contentMax.value = String(mb); }, () => {});
  contentMax.addEventListener('change', async () => {
    const mb = parseLimit(contentMax.value);
    if (mb == null) { filterMessage(S.contentMaxInvalid, true); return; }
    try {
      contentMax.value = String(await api.setContentMaxMb(mb));
      filterMessage('');
    } catch (err) { filterMessage(errorText(err), true); }
  });

  async function startUpdates() {
    try {
      updatesToggle.checked = (await api.getUpdateSettings()).checkOnOpen;
    } catch { /* the toggle stays as rendered */ }
    try { await api.onUpdateProgress((p) => updates.onProgress(p)); } catch { /* no progress bar */ }
    updates.start();
  }

  /* ---------------------------------------------------------------- content search (VFS-27) */

  const contentSearch = createContentSearch({ api, render: renderContentHits });
  let shownContentHits = null;

  /** The content section applies to file searches with content search on (R12), never under
   *  «Solo nombres» (VFS-38 R3); under «Solo documentos» the source choice does not apply (R4). */
  function contentApplies() {
    return contentOn(state.content) && sourcesFor(sourceApplies() ? state.source : SourceChoice.FILES, bookmarksOn()).includes(Source.FILES)
      && bookmarkFolderOf(state.folder) == null && state.scope !== ScopeChoice.NAMES;
  }

  function runContentQuery() {
    contentSearch.query(state.text, filters(), contentApplies());
  }

  function renderContentHits(view) {
    const sec = $('content-hits');
    sec.hidden = !view;
    if (!view) { shownContentHits = null; renderEmpty(); return; }
    const notes = [];
    if (view.loading && !view.hits.length) notes.push(S.contentSearching);
    if (view.failed) notes.push(S.contentFailed);
    notes.push(...sectionNotes(L, state.content, view.partial));
    // Under «Solo documentos» the empty view says it, with «Buscar en todo» (VFS-38 R4).
    if (!view.loading && !view.failed && !view.hits.length && state.scope !== ScopeChoice.DOCUMENTS) notes.push(S.contentNone);
    $('content-notes').replaceChildren(...notes.map((t) => el('p', null, t)));
    $('content-more').hidden = !view.more;
    if (view.hits === shownContentHits) return;
    shownContentHits = view.hits;
    $('content-list').replaceChildren(...view.hits.map(contentHitItem));
    renderEmpty();
  }

  /** One hit inside a document: name, folder, the highlighted fragment, its page, and
   *  «Mostrar en carpeta» (R12). */
  function contentHitItem(hit) {
    const li = el('li', 'chit');
    const ico = el('span', 'ico');
    ico.setAttribute('aria-hidden', 'true');
    ico.append(icon(FileKind.DOCUMENT));
    const main = el('div', 'main');
    const name = el('span', 'name');
    F.appendHighlighted(name, hit.name, F.matchRanges(hit.name, state.terms));
    const path = el('span', 'path', hit.dir);
    path.title = hit.dir;
    const fragment = el('p', 'fragment');
    for (const run of snippetRuns(hit.snippet, hit.highlights)) {
      fragment.append(run.hit ? el('mark', null, run.text) : run.text);
    }
    const page = hitPage(hit);
    if (page != null) fragment.append(' ', el('span', 'page', S.contentPage(N.count(page))));
    main.append(name, path, fragment);
    const show = button(S.revealButton, 'btn', () => revealFile(hit.id, false));
    show.setAttribute('aria-label', `${S.revealButton}: ${S.contentHitLabel(hit.name, hit.dir, page && N.count(page))}`);
    li.append(ico, main, show);
    return li;
  }
  $('content-more').addEventListener('click', () => contentSearch.more());

  /** R14: the unread reasons of the document hits on a page just shown. */
  async function loadUnread(hits) {
    if (!contentOn(state.content)) return;
    const ids = unreadCandidates(hits).filter((id) => !state.unread.has(id));
    if (!ids.length) return;
    const unread = state.unread;
    try {
      const pairs = await api.contentUnread(ids);
      if (unread !== state.unread) return; // a newer search replaced the list
      for (const [id, reason] of pairs) unread.set(id, reason);
      if (pairs.some(([, reason]) => reason)) list.refresh();
    } catch { /* no labels */ }
  }

  function setContent(st) {
    const before = state.content;
    state.content = st;
    // VFS-38 R5: at start, or when content search goes off, «Solo documentos» becomes «Todo».
    if (state.scope === ScopeChoice.DOCUMENTS && !contentOn(st)) {
      state.scope = ScopeChoice.ALL;
      writePref(store, PrefKey.SCOPE, state.scope);
      renderScope();
      renderSources();
      filtersChanged();
    } else {
      renderScope();
    }
    if (contentOn(before) !== contentOn(st)) {
      runContentQuery();
      if (contentOn(st)) loadUnread([...state.pages.values()].flat());
      else { state.unread = new Map(); list.refresh(); }
    } else {
      const view = contentSearch.view();
      if (view) renderContentHits(view); // the "still reading" line follows the run
    }
    renderContentStatus();
    $('banner-content-damaged').hidden = st?.phase.kind !== ContentPhase.DAMAGED;
    $('content-on').checked = contentOn(st);
    showContentLimit($('content-max-row'), contentOn(st)); // VFS-34 R6
    if (settingsDialog.open && before?.phase.kind !== st?.phase.kind) refreshContentStore();
    maybeOffer();
  }

  async function contentCall(call) {
    try { setContent(await call()); } catch (err) { showError(errorText(err)); }
  }

  /** R8, R15: progress, «Pausar la lectura» / «Reanudar la lectura», or the reason with
   *  «Reintentar». */
  function renderContentStatus() {
    const line = api.missing ? null : contentStatusLine(L, state.content);
    $('content-line').hidden = !line;
    $('content-line').textContent = line?.text || '';
    const action = $('content-action');
    action.hidden = !line?.action;
    action.dataset.action = line?.action || '';
    if (line?.action === ContentAction.PAUSE) action.textContent = S.contentPause;
    else if (line?.action === ContentAction.RESUME) action.textContent = S.contentResume;
    else if (line?.action === ContentAction.RETRY) action.textContent = S.contentRetry;
  }

  $('content-action').addEventListener('click', (e) => {
    const action = e.currentTarget.dataset.action;
    if (action === ContentAction.RETRY) contentCall(() => api.contentRetry());
    else contentCall(() => api.contentSetPaused(action === ContentAction.PAUSE));
  });
  $('content-rebuild').addEventListener('click', () => contentCall(() => api.contentRebuild()));
  $('content-disable').addEventListener('click', () => contentCall(() => api.contentSetEnabled(false)));

  /* The offer (R4): asked once the folders have finished their first scan, and again only when
     the folders change before it was answered. */
  let offerKey = '';
  async function maybeOffer() {
    if (!offerDue(state.content, state.roots)) { $('banner-content-offer').hidden = true; return; }
    const key = state.roots.map((r) => `${r.id}:${r.lastFullScan}`).join(',');
    if (key === offerKey) return;
    offerKey = key;
    let offer;
    try { offer = await api.contentOffer(); } catch { offerKey = ''; return; } // asked again next time
    // Nothing to read yet: no offer, and not marked as offered.
    if (!offer.documents || !offerDue(state.content, state.roots)) return;
    const { text, warn } = offerText(L, offer);
    $('content-offer-text').textContent = text;
    $('content-offer-warn').textContent = warn;
    $('banner-content-offer').hidden = false;
  }
  $('content-offer-yes').addEventListener('click', () => {
    $('banner-content-offer').hidden = true;
    contentCall(() => api.contentSetEnabled(true));
  });
  $('content-offer-no').addEventListener('click', () => {
    $('banner-content-offer').hidden = true;
    contentCall(() => api.contentMarkOffered());
    q.focus();
  });

  /* Ajustes (R5, R14): the switch, the store's size, the unread counts and the delete. */
  const contentToggle = $('content-on');
  contentToggle.addEventListener('change', async () => {
    try {
      setContent(await api.contentSetEnabled(contentToggle.checked));
      refreshContentStore();
    } catch (err) {
      contentToggle.checked = contentOn(state.content);
      showError(errorText(err));
    }
  });

  async function refreshContentStore() {
    let counts = null;
    try { counts = await api.contentUnreadCounts(); } catch { /* no store */ }
    $('content-store').hidden = !counts;
    $('content-delete').hidden = !counts;
    $('content-store').textContent = counts ? S.contentStore(N.size(counts.storeBytes)) : '';
    const lines = (counts?.byReason || []).filter(([, n]) => n > 0)
      .map(([reason, n]) => S.contentUnreadLine(S.contentUnread[reason] || reason, N.count(n)));
    if (counts?.pending) lines.push(S.contentPending(N.count(counts.pending)));
    $('content-unread').hidden = !lines.length;
    $('content-unread-list').replaceChildren(...lines.map((t) => el('li', null, t)));
  }

  const contentDeleteDialog = $('confirm-content-delete');
  $('content-delete').addEventListener('click', () => {
    contentDeleteDialog.returnValue = '';
    contentDeleteDialog.showModal();
  });
  contentDeleteDialog.addEventListener('close', async () => {
    if (contentDeleteDialog.returnValue !== 'delete') return;
    try {
      setContent(await api.contentDeleteStore());
      toast(S.contentDeleted);
    } catch (err) { showError(errorText(err)); }
    refreshContentStore();
  });

  async function startContent() {
    try { await api.onContentStatus(setContent); } catch { /* status on request only */ }
    try { setContent(await api.contentStatus()); } catch { /* not started yet; asked again with the folders */ }
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
    // VFS-27: the service starts with the index; the offer waits for the first scans (R4).
    if (!state.content) api.contentStatus().then(setContent, () => {});
    else maybeOffer();
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
    startUpdates();
    // VFS-39 R2: the index opens off the main thread, which can take minutes after an unclean
    // exit; the status says «Preparando la búsqueda…» until it is open.
    try { await api.waitCore(); } catch (err) { showError(errorText(err)); return; }
    state.coreReady = true;
    api.getIncludeAllFolders().then((on) => { includeAll.checked = on; }, () => {});
    await api.onProgress(onProgress);
    await api.onIndexChanged(onIndexChanged);
    await api.onBookmarksChanged(onBookmarksChanged);
    try { setBookmarks(await api.getBookmarksStatus()); } catch { /* files only */ }
    restoreChoices();
    await refreshRoots();
    try { onProgress(await api.getProgress()); } catch { /* the event stream will catch up */ }
    renderEmpty();
    startContent();
    if (hasQuery()) runSearch();
  }

  start();
}

main(await loadLocale());
