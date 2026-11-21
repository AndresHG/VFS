// Status-bar wording (VFS-1 U6). Run: node --test apps/desktop/ui-dev/status.test.mjs
// Times are built in local time, as the app shows them.

import test from 'node:test';
import assert from 'node:assert/strict';

import { statusText, checkedAt, viewFor, footerActions } from '../ui/status.js';
import { SearchState, View, TypeChecklist, typeChecklistMode } from '../ui/choices.js';
import { S } from '../ui/strings.js';
import { IndexerState, RootStatus } from '../ui/contract.js';

const secs = (y, mo, d, h, mi) => new Date(y, mo - 1, d, h, mi).getTime() / 1000;
const NOW = secs(2026, 11, 20, 18, 30);
const root = (over) => ({ id: 1, path: '/a', status: RootStatus.READY, files: 10, dirs: 1, lastFullScan: null, ...over });
const progress = (state, over = {}) => ({ state, filesIndexed: 1234, roots: [{ rootId: 1, dirsPending: 7 }], ...over });

/** VFS-1 U6: the last completed check reads as today, yesterday, a date this year or older. */
test('checkedAt names the day of the last check', () => {
  assert.equal(checkedAt(secs(2026, 11, 20, 9, 14), NOW), 'hoy 9:14');
  assert.equal(checkedAt(secs(2026, 11, 19, 23, 5), NOW), 'ayer 23:05');
  assert.equal(checkedAt(secs(2026, 3, 12, 9, 14), NOW), '12 mar, 9:14');
  assert.equal(checkedAt(secs(2025, 3, 12, 9, 14), NOW), '12 mar 2025');
});

/** VFS-1 U6, I4: idle, the status shows the time of the oldest completed check of the
 *  reachable folders, never a blanket "up to date". */
test('idle status shows the last completed check', () => {
  const roots = [
    root({ lastFullScan: secs(2026, 11, 20, 9, 14) }),
    root({ id: 2, lastFullScan: secs(2026, 11, 20, 17, 0) }),
  ];
  assert.equal(statusText(progress(IndexerState.IDLE), roots, NOW), S.statusChecked('hoy 9:14', '1.234'));
  assert.equal(statusText(null, [root({ lastFullScan: null })], NOW), S.statusFiles('10'));
});

/** VFS-1 U6, I4: scanning after a first scan has finished (a catch-up walk at start, a
 *  manual or periodic check) reads as checking for changes; the first scan keeps its
 *  own wording with the folders left. */
test('scanning reads as checking for changes once a first scan has finished', () => {
  const checked = [root({ status: RootStatus.SCANNING, lastFullScan: secs(2026, 11, 19, 9, 0) })];
  assert.equal(statusText(progress(IndexerState.SCANNING), checked, NOW), S.statusChecking('1.234'));
  const first = [root({ status: RootStatus.SCANNING })];
  assert.equal(statusText(progress(IndexerState.SCANNING), first, NOW), S.statusScanning('1.234', '7'));
});

/** VFS-1 U3, U6: paused and unavailable states keep their own wording. */
test('paused and unavailable states', () => {
  const roots = [root({ lastFullScan: NOW }), root({ id: 2, status: RootStatus.UNAVAILABLE, lastFullScan: NOW })];
  assert.equal(statusText(progress(IndexerState.PAUSED), roots, NOW), S.statusPaused('1.234'));
  assert.equal(statusText(progress(IndexerState.IDLE), roots, NOW), S.statusReadyUnavailable('1.234', 1));
});

/** VFS-1 decision 16: no status or button text says "indexar" or "índice". */
test('plain wording', () => {
  const texts = Object.values(S).map((v) => (typeof v === 'function' ? v('1', '1') : v));
  for (const s of texts.filter((t) => typeof t === 'string')) {
    assert.doesNotMatch(s, /index|índice/i);
  }
});

const screen = (over) => ({ backendMissing: false, rootsLoaded: true, rootCount: 2, searchState: SearchState.IDLE, ...over });

/** VFS-1 AC-12: before any search the box sits alone in the middle (also with no folders or no
 *  backend); results, no hits and errors use the list layout. */
test('viewFor picks the start or the list layout', () => {
  assert.equal(viewFor(screen()), View.START);
  assert.equal(viewFor(screen({ rootsLoaded: false, rootCount: 0 })), View.START);
  assert.equal(viewFor(screen({ rootCount: 0 })), View.START);
  assert.equal(viewFor(screen({ backendMissing: true, searchState: SearchState.ERROR })), View.START);
  assert.equal(viewFor(screen({ searchState: SearchState.DONE })), View.LIST);
  assert.equal(viewFor(screen({ searchState: SearchState.LOADING })), View.LIST);
  assert.equal(viewFor(screen({ searchState: SearchState.ERROR })), View.LIST);
  assert.equal(viewFor(screen({ rootCount: 0, searchState: SearchState.ERROR })), View.LIST);
});

/** VFS-1 U6: "Buscar cambios ahora" sits in the status line only when nothing is being
 *  checked; "Pausar"/"Continuar" only while folders are being checked. */
test('footerActions offers pause or a check, never both', () => {
  assert.deepEqual(footerActions(progress(IndexerState.IDLE)), { pause: false, paused: false, rescan: true });
  assert.deepEqual(footerActions(progress(IndexerState.SCANNING)), { pause: true, paused: false, rescan: false });
  assert.deepEqual(footerActions(progress(IndexerState.PAUSED)), { pause: true, paused: true, rescan: false });
  assert.deepEqual(footerActions(null), { pause: false, paused: false, rescan: false });
});

/** VFS-1 AC-11: where the Popover API is missing (macOS 11-12 WKWebView) the type filter
 *  falls back to inline chips instead of a checklist that cannot open. */
test('typeChecklistMode falls back to inline chips without the Popover API', () => {
  assert.equal(typeChecklistMode({ popover: null }), TypeChecklist.POPOVER);
  assert.equal(typeChecklistMode({}), TypeChecklist.INLINE);
});
