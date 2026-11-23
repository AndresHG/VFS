// Index commands wait for the index to open (VFS-39 R2). Run: node --test apps/desktop/ui-dev/coregate.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { gateOnCore, NO_CORE } from '../ui/coregate.js';

/** A fake `invoke` whose `wait_core` resolves or rejects when the test says so. */
function backend() {
  const calls = [];
  let open, fail;
  const opened = new Promise((res, rej) => { open = res; fail = rej; });
  const invoke = (cmd, args) => {
    calls.push(cmd);
    if (cmd === 'wait_core') return opened;
    return Promise.resolve({ cmd, args });
  };
  return { invoke, calls, open: () => open(), fail: (e) => fail(e) };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

/** VFS-39 AC-4: «Añadir carpeta» while the index opens waits for it, never fails as UNAVAILABLE. */
test('an index command sent while the index opens runs once it is open', async () => {
  const b = backend();
  const invoke = gateOnCore(b.invoke);
  let answer = null;
  invoke('add_root', { path: '/x' }).then((r) => { answer = r; });
  await tick();
  assert.deepEqual(b.calls, ['wait_core']);
  assert.equal(answer, null);
  b.open();
  await tick();
  assert.deepEqual(b.calls, ['wait_core', 'add_root']);
  assert.deepEqual(answer, { cmd: 'add_root', args: { path: '/x' } });
});

/** VFS-39 AC-4: one wait serves every command; later ones go straight through. */
test('the index wait is asked once', async () => {
  const b = backend();
  const invoke = gateOnCore(b.invoke);
  b.open();
  await invoke('wait_core');
  await invoke('list_roots');
  await invoke('content_status');
  assert.deepEqual(b.calls, ['wait_core', 'list_roots', 'content_status']);
});

/** VFS-39 AC-4: the folder dialog, language and update commands never wait for the index. */
test('commands that need no index run at once', async () => {
  const b = backend();
  const invoke = gateOnCore(b.invoke);
  for (const cmd of NO_CORE) {
    if (cmd !== 'wait_core') await invoke(cmd);
  }
  assert.deepEqual(b.calls, [...NO_CORE].filter((c) => c !== 'wait_core'));
});

/** VFS-39 AC-4: when the index cannot open, index commands reject with that error. */
test('a failed open rejects the waiting commands', async () => {
  const b = backend();
  const invoke = gateOnCore(b.invoke);
  const err = { code: 'unavailable', message: 'disk full' };
  const pending = invoke('remove_root', { id: 1 });
  b.fail(err);
  await assert.rejects(pending, (e) => e === err);
  assert.deepEqual(b.calls, ['wait_core']);
});

/** VFS-39 AC-4: every Tauri command of the app goes through the gate, so a new one cannot skip it. */
test('the backend bridge calls Tauri only through the gate', () => {
  const src = readFileSync(new URL('../ui/api.js', import.meta.url), 'utf8');
  assert.equal(src.match(/t\.core\.invoke/g)?.length, 1);
  assert.match(src, /gateOnCore\(t\.core\.invoke\)/);
  assert.doesNotMatch(src, /\{\s*invoke\s*\}\s*=\s*t\.core/);
});
