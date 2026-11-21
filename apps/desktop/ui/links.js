// The second action of a link row (VFS-18 D3, D6, R7). Pure, tested in
// ui-dev/screen.test.mjs.

import { LinkKind } from './contract.js';

export const SecondAction = Object.freeze({ TARGET: 'target', OPEN: 'open' });

/** "Mostrar el destino" (TARGET) for a shortcut or link with a target, "Abrir en Firefox"
 *  (OPEN) for an Internet Shortcut whose address can be opened, else null. */
export function secondActionOf(hit) {
  const link = hit?.link;
  if (!link) return null;
  if (link.kind === LinkKind.INTERNET) return link.openable ? SecondAction.OPEN : null;
  return link.target ? SecondAction.TARGET : null;
}
