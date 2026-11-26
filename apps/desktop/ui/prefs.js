// Choices remembered across restarts (VFS-38 R6, D2; VFS-49 R1): the source, the sort order
// and the window's tab, Buscador or Chat, kept in the webview's localStorage as `vfs.language`
// is (VFS-22); the results tab is not remembered
// (VFS-42 R5). A value is used only when it is one of the choices offered at start; a
// missing, unknown or unreadable value gives the default, so damaged or blocked storage never
// stops the app. Tested in ui-dev/prefs.test.mjs.

export const PrefKey = Object.freeze({ SOURCE: 'vfs.source', SORT: 'vfs.sort', VIEW: 'vfs.view' });

/** The webview's storage, or null where even reaching it throws (storage blocked). */
export function webStorage() {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

/** The value kept under `key` when it is in `offered`, otherwise `fallback`. */
export function readPref(storage, key, offered, fallback) {
  try {
    const value = storage.getItem(key);
    return offered.includes(value) ? value : fallback;
  } catch {
    return fallback;
  }
}

/** Keeps `value` under `key`; where storage fails, the choice is simply not remembered. */
export function writePref(storage, key, value) {
  try {
    storage.setItem(key, value);
  } catch {
    // storage blocked or full: the default applies at the next start
  }
}
