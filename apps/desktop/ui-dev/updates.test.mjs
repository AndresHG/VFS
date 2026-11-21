// Update notice (VFS-21). Run: node --test apps/desktop/ui-dev/updates.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';

import { createUpdates, updateView, UpdatePhase, CHECK_DELAY_MS } from '../ui/updates.js';
import { S } from '../ui/strings.js';

function harness(api) {
  const views = [];
  const timers = [];
  const u = createUpdates({ api, render: (v) => views.push(v), schedule: (fn, ms) => timers.push({ fn, ms }) });
  const runTimers = async () => { for (const t of timers.splice(0)) await t.fn(); };
  return { u, views, timers, runTimers };
}

/** VFS-21 R1, AC-11: the check waits after launch, then offers "Actualizar" and "Ahora no". */
test('a newer version is offered after the start delay', async () => {
  const { u, views, timers, runTimers } = harness({ checkForUpdate: async () => ({ version: '0.2.1' }) });
  u.start();
  assert.equal(views.length, 0);
  assert.equal(timers[0].ms, CHECK_DELAY_MS);
  await runTimers();
  assert.equal(u.phase(), UpdatePhase.OFFERED);
  assert.deepEqual(views.at(-1), { text: S.updateAvailable('0.2.1'), action: S.updateInstall, later: true, progress: null });
  assert.match(views.at(-1).text, /0\.2\.1/);
});

/** VFS-21 AC-4, AC-6: no update, the setting off (null) or a failed check shows nothing. */
test('nothing is shown when there is no update or the check fails', async () => {
  for (const checkForUpdate of [async () => null, async () => { throw { code: 'io' }; }]) {
    const { u, views, runTimers } = harness({ checkForUpdate });
    u.start();
    await runTimers();
    assert.equal(u.phase(), UpdatePhase.HIDDEN);
    assert.equal(views.length, 0);
  }
});

/** VFS-21 AC-13: download progress, then the installing text when the download completes. */
test('installing shows progress and then the installing text', async () => {
  let finish;
  const { u, views, runTimers } = harness({
    checkForUpdate: async () => ({ version: '0.2.1' }),
    installUpdate: () => new Promise((resolve) => { finish = resolve; }),
  });
  u.start();
  await runTimers();
  const done = u.install();
  assert.deepEqual(views.at(-1), { text: S.updateDownloading(null), action: null, later: false, progress: null });
  u.onProgress({ downloaded: 512, total: 2048 });
  assert.equal(views.at(-1).progress, 25);
  assert.equal(views.at(-1).text, S.updateDownloading(25));
  u.onProgress({ downloaded: 2048, total: 2048 });
  assert.equal(u.phase(), UpdatePhase.INSTALLING);
  finish();
  await done;
  assert.equal(views.at(-1).text, S.updateInstalling);
});

/** VFS-21 AC-7: a failed download says so and offers "Reintentar", which tries again. */
test('a failed download offers to retry', async () => {
  let attempts = 0;
  const { u, views, runTimers } = harness({
    checkForUpdate: async () => ({ version: '0.2.1' }),
    installUpdate: async () => { attempts += 1; if (attempts === 1) throw { code: 'io' }; },
  });
  u.start();
  await runTimers();
  await u.install();
  assert.equal(u.phase(), UpdatePhase.FAILED);
  assert.deepEqual(views.at(-1), { text: S.updateFailed, action: S.updateRetry, later: true, progress: null });
  await u.install();
  assert.equal(attempts, 2);
  assert.equal(u.phase(), UpdatePhase.INSTALLING);
});

/** VFS-21 AC-12: a failure after the download completed (signature or installer) says the
 *  install failed, not the download, and still offers "Reintentar". */
test('a failed install after the download says so', async () => {
  const { u, views, runTimers } = harness({
    checkForUpdate: async () => ({ version: '0.2.1' }),
    installUpdate: async () => {
      u.onProgress({ downloaded: 10, total: 10 });
      throw { code: 'io' };
    },
  });
  u.start();
  await runTimers();
  await u.install();
  assert.equal(u.phase(), UpdatePhase.INSTALL_FAILED);
  assert.deepEqual(views.at(-1), { text: S.updateInstallFailed, action: S.updateRetry, later: true, progress: null });
});

/** VFS-21 D8: "Ahora no" hides the notice; late progress events do not bring it back. */
test('dismissing hides the notice', async () => {
  const { u, views, runTimers } = harness({ checkForUpdate: async () => ({ version: '0.2.1' }) });
  u.start();
  await runTimers();
  u.dismiss();
  assert.equal(views.at(-1), null);
  u.onProgress({ downloaded: 1, total: 2 });
  assert.equal(views.at(-1), null);
  assert.equal(updateView({ phase: UpdatePhase.HIDDEN }), null);
});
