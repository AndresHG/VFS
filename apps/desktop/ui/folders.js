// The folder list around adding and removing a folder (VFS-31). Pure, so it is tested
// outside the browser (`apps/desktop/ui-dev/folders.test.mjs`).

import { RootStatus } from './contract.js';
import { isUnder } from './format.js';

/** Paths of the listed folders that a folder added at `path` takes in (VFS-31 R2, R4):
 *  those below it, leaving out folders being removed. */
export function takenInBy(roots, path) {
  return roots
    .filter((r) => r.status !== RootStatus.REMOVING && isUnder(r.path, path) && !isUnder(path, r.path))
    .map((r) => r.path);
}

/** Wraps an async `load` so only the newest call's result counts: an older call that
 *  resolves or fails after a newer one resolves to `{ stale: true }` (VFS-31 R1), so an
 *  old folder list cannot bring back a folder whose removal already finished. */
export function newestOnly(load) {
  let asked = 0;
  return async (...args) => {
    const mine = ++asked;
    try {
      const value = await load(...args);
      return mine === asked ? { value } : { stale: true };
    } catch (error) {
      if (mine !== asked) return { stale: true };
      throw error;
    }
  };
}
