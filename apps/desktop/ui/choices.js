// The screen's own closed sets (VFS-1 R8, AC-11, AC-12): the values of the date and size
// selects, where a search is, and how the screen is laid out. Their meaning is in main.js and
// status.js, their labels in the catalogs (strings.es.js, strings.en.js).

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

/** The "Buscar en" choice (VFS-6), shown while Firefox bookmarks are included. */
export const SourceChoice = Object.freeze({
  BOTH: 'both',
  FILES: 'files',
  BOOKMARKS: 'bookmarks',
});
