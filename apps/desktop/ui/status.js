// The status-bar sentence (VFS-1 U6): the first scan, a check for changes (the catch-up at
// every start included), or the time of the last completed check. Pure, so it is tested
// outside the browser (`apps/desktop/ui-dev/status.test.mjs`).

import { IndexerState, RootStatus } from './contract.js';
import { SearchState, View } from './choices.js';
import { S } from './strings.js';
import * as F from './format.js';

const timeFmt = new Intl.DateTimeFormat('es-ES', { hour: 'numeric', minute: '2-digit' });
const dayFmt = new Intl.DateTimeFormat('es-ES', { day: 'numeric', month: 'short' });
const DAY_MS = 86_400_000;

function startOfDay(d) {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** Unix seconds -> "hoy 9:14", "ayer 18:02", "12 mar, 9:14", or "12 mar 2025" (local time). */
export function checkedAt(secs, now) {
  const d = new Date(secs * 1000);
  const today = new Date(now * 1000);
  const time = timeFmt.format(d);
  const days = Math.round((startOfDay(today) - startOfDay(d)) / DAY_MS);
  if (days === 0) return S.today(time);
  if (days === 1) return S.yesterday(time);
  if (d.getFullYear() !== today.getFullYear()) return F.date(secs);
  return S.dayAndTime(dayFmt.format(d).replace('.', ''), time);
}

/** True while no root has finished its first scan. */
export function firstScanRunning(roots) {
  return roots.length > 0 && !roots.some((r) => r.lastFullScan != null || r.status === RootStatus.READY);
}

/**
 * The status text for progress `p` (or null) and `roots` (non-empty). The last check shown
 * is the oldest completed check among the reachable roots: every reachable folder has been
 * checked at least since then.
 */
export function statusText(p, roots, now = Date.now() / 1000) {
  const files = F.count(p?.filesIndexed ?? roots.reduce((s, r) => s + r.files, 0));
  if (p?.state === IndexerState.SCANNING) {
    if (!firstScanRunning(roots)) return S.statusChecking(files);
    return S.statusScanning(files, F.count(p.roots.reduce((s, r) => s + r.dirsPending, 0)));
  }
  if (p?.state === IndexerState.PAUSED) return S.statusPaused(files);
  const missing = roots.filter((r) => r.status === RootStatus.UNAVAILABLE).length;
  if (missing) return S.statusReadyUnavailable(files, missing);
  const checks = roots.map((r) => r.lastFullScan).filter((t) => t != null);
  if (!checks.length) return S.statusFiles(files);
  return S.statusChecked(checkedAt(Math.min(...checks), now), files);
}

/**
 * Start view (the box alone in the middle) until a search has been made, also when there is
 * nothing to search yet (no folders, no backend); list view for results, no hits and errors.
 */
export function viewFor({ backendMissing, rootsLoaded, rootCount, searchState }) {
  if (backendMissing) return View.START;
  if (searchState === SearchState.ERROR) return View.LIST;
  if (rootsLoaded && rootCount === 0) return View.START;
  return searchState === SearchState.IDLE ? View.START : View.LIST;
}

/**
 * Status-line buttons for progress `p`: "Pausar"/"Continuar" while folders are being checked,
 * "Buscar cambios ahora" only when nothing is being checked (VFS-1 U6).
 */
export function footerActions(p) {
  return {
    pause: p?.state === IndexerState.SCANNING || p?.state === IndexerState.PAUSED,
    paused: p?.state === IndexerState.PAUSED,
    rescan: p?.state === IndexerState.IDLE,
  };
}
