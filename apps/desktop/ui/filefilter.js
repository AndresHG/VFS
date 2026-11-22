// «Filtro de archivos» (VFS-34): adding a pattern and wording the backend's answers. Pure,
// so it is tested outside the browser (`apps/desktop/ui-dev/filefilter.test.mjs`).

import { ErrorCode } from './contract.js';

/** Above this many matching files, adding a pattern asks first (VFS-34 R2). */
export const CONFIRM_ABOVE = 1000;

/** Adds `pattern`: counts the indexed files it matches, asks `confirm(count)` when there
 *  are more than {@link CONFIRM_ABOVE}, then adds it (VFS-34 R2). Resolves to
 *  `{ list, count }`, or `{ cancelled: true, count }`; a refused pattern rejects with the
 *  backend's `invalid_pattern` error before anything is asked. */
export async function addPattern(api, pattern, confirm) {
  const count = await api.countFileExclusion(pattern);
  if (count > CONFIRM_ABOVE && !(await confirm(count))) return { cancelled: true, count };
  const list = await api.addFileExclusion(pattern);
  return { list, count };
}

/** The message for a refused pattern (a `PatternReason`), or null for any other error. */
export function patternErrorText(err, S) {
  if (err?.code !== ErrorCode.INVALID_PATTERN) return null;
  return S.fileFilterErrors[err.message] ?? null;
}

/** Shows the content size field only while content search is on (VFS-34 R1, R6). VFS-27's
 *  switch calls it; until then the field stays hidden. */
export function showContentLimit(row, on) {
  row.hidden = !on;
}

/** The size limit typed in the field, in whole MB (0 = no limit), or null when it is not
 *  a whole number from 0 to 100,000 (VFS-34 R6). */
export function parseLimit(text) {
  const t = String(text).trim();
  if (!/^\d{1,6}$/.test(t)) return null;
  const mb = Number(t);
  return mb <= 100_000 ? mb : null;
}
