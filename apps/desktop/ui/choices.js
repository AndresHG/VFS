// The screen's own closed sets (VFS-1 R8, AC-11, AC-12): the values of the date and size
// selects, where a search is, and how the screen is laid out. Their meaning is in main.js and
// status.js, their labels in the catalogs (strings.es.js, strings.en.js).

import { Scope } from './contract.js';

/** The UI languages (VFS-22). */
export const Lang = Object.freeze({ ES: 'es', EN: 'en' });

/** The Ajustes language select: a language, or follow the system (VFS-22 R3). */
export const LanguageChoice = Object.freeze({ SYSTEM: 'system', ...Lang });

export const DateChoice = Object.freeze({
  ANY: 'any',
  WEEK: 'week',
  MONTH: 'month',
  YEAR: 'year',
  CUSTOM: 'custom',
});

export const SizeChoice = Object.freeze({
  ANY: 'any',
  SMALL: 'small',
  MEDIUM: 'medium',
  LARGE: 'large',
  HUGE: 'huge',
});

/** Where the current search is. */
export const SearchState = Object.freeze({ IDLE: 'idle', LOADING: 'loading', DONE: 'done', ERROR: 'error' });

/** Layout of the screen: the box alone in the middle (no search made yet), or above results. */
export const View = Object.freeze({ START: 'start', LIST: 'list' });

/** How the type checklist is shown: in a popover, or inline where the Popover API is missing
 *  (WKWebView before Safari 17, i.e. macOS 11 and 12). */
export const TypeChecklist = Object.freeze({ POPOVER: 'popover', INLINE: 'inline' });

/** The checklist mode for an element prototype (`HTMLElement.prototype` in the app). */
export function typeChecklistMode(proto) {
  return Object.prototype.hasOwnProperty.call(proto, 'popover') ? TypeChecklist.POPOVER : TypeChecklist.INLINE;
}

/** The source choice (VFS-6), «Origen» in «Más filtros» (VFS-38 R7), shown while Firefox bookmarks are included. */
export const SourceChoice = Object.freeze({
  BOTH: 'both',
  FILES: 'files',
  BOOKMARKS: 'bookmarks',
});

/** Units of the rough times in the content offer and status (VFS-27): keys of `S.timeRange`
 *  and `S.timeLeft`. */
export const TimeUnit = Object.freeze({ MINUTES: 'minutes', HOURS: 'hours', DAYS: 'days' });

/** The «Buscar en» scope group (VFS-38 R1, R14): the screen's choices. `all` and `names` are
 *  the core's `Scope`; `documents` («Solo documentos») runs content search only and is built
 *  with the content switch (VFS-27 27c). */
export const ScopeChoice = Object.freeze({ ...Scope, DOCUMENTS: 'documents' });

/** The scope buttons on the filter line, in order (VFS-38 R1): «Solo documentos» only while
 *  content search is on (VFS-38 R5). */
export function scopesFor(contentOn) {
  return contentOn ? [ScopeChoice.ALL, ScopeChoice.NAMES, ScopeChoice.DOCUMENTS] : [ScopeChoice.ALL, ScopeChoice.NAMES];
}

/** The `SearchQuery.scope` of a scope choice. */
export function scopeFor(choice) {
  return choice === ScopeChoice.NAMES ? Scope.NAMES : Scope.ALL;
}
