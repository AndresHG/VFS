// The folder list around adding and removing a folder (VFS-31). Run:
// node --test apps/desktop/ui-dev/folders.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';

import { takenInBy, newestOnly } from '../ui/folders.js';
import { RootStatus } from '../ui/contract.js';

/** VFS-31 AC-4: the notice names the listed folders below the added one, not the added
 *  folder itself, not unrelated or sibling folders, and not folders being removed. */
test('a folder takes in the listed folders below it, except those being removed', () => {
  const roots = [
    { path: 'C:\\Datos\\Fotos', status: RootStatus.READY },
    { path: 'C:\\Datos\\Docs\\Banco', status: RootStatus.SCANNING },
    { path: 'C:\\Datos\\Viejo', status: RootStatus.REMOVING },
    { path: 'C:\\Datos2', status: RootStatus.READY },
    { path: 'D:\\Otra', status: RootStatus.READY },
  ];
  assert.deepEqual(takenInBy(roots, 'c:\\datos'), ['C:\\Datos\\Fotos', 'C:\\Datos\\Docs\\Banco']);
  assert.deepEqual(takenInBy(roots, 'C:\\Datos\\Fotos'), []);
  assert.deepEqual(takenInBy([{ path: '/Users/ana/Fotos', status: RootStatus.READY }], '/Users/ana'), ['/Users/ana/Fotos']);
});

/** VFS-31 R1: only the newest folder list counts; an older reply that arrives last, or
 *  fails last, is stale, so a finished removal is never undone on screen. */
test('an older folder list arriving last is dropped', async () => {
  const pending = [];
  const load = newestOnly(() => new Promise((resolve, reject) => pending.push({ resolve, reject })));
  const older = load();
  const newer = load();
  pending[1].resolve(['nueva']);
  pending[0].resolve(['vieja', 'quitada']);
  assert.deepEqual(await newer, { value: ['nueva'] });
  assert.deepEqual(await older, { stale: true });

  const failing = load();
  const latest = load();
  pending[3].resolve([]);
  pending[2].reject(new Error('tarde'));
  assert.deepEqual(await failing, { stale: true });
  assert.deepEqual(await latest, { value: [] });
  const alone = load();
  pending[4].reject(new Error('fallo'));
  await assert.rejects(alone, /fallo/);
});
