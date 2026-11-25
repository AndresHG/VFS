// Update notice (VFS-21). Run: node --test apps/desktop/ui-dev/updates.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';

import { aboutUpdateView, CheckResult, createUpdates, updateView, UpdatePhase, CHECK_DELAY_MS } from '../ui/updates.js';
import { CATALOGS, Lang } from '../ui/strings.js';

const S = CATALOGS[Lang.ES];

function harness(api) {
  const views = [];
  const timers = [];
  const u = createUpdates({ api, S, render: (v) => views.push(v), schedule: (fn, ms) => timers.push({ fn, ms }) });
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
  assert.equal(updateView({ phase: UpdatePhase.HIDDEN }, S), null);
});

/** VFS-45 AC-5 (R5): the update line of «Acerca de» for each check result, in both languages:
 *  up to date, an update with «Actualizar», a failure and not run yet with «Buscar
 *  actualizaciones», and nothing to press while the setting is off. */
test('«Acerca de» says what this launch\'s check found', () => {
  for (const lang of Object.values(Lang)) {
    const C = CATALOGS[lang];
    const view = (o) => aboutUpdateView({ checkOn: true, version: null, ...o }, C);
    assert.deepEqual(view({ result: CheckResult.NONE }), { text: C.aboutUpToDate, action: null });
    assert.deepEqual(view({ result: CheckResult.FOUND, version: '0.4.0' }), { text: C.aboutUpdateFound('0.4.0'), action: 'install' });
    assert.match(C.aboutUpdateFound('0.4.0'), /0\.4\.0/);
    assert.deepEqual(view({ result: CheckResult.FAILED }), { text: C.aboutUpdateFailed, action: 'check' });
    assert.deepEqual(view({ result: CheckResult.NOT_RUN }), { text: '', action: 'check' });
    for (const result of Object.values(CheckResult)) {
      assert.deepEqual(view({ result, checkOn: false }), { text: C.aboutUpdatesOff, action: null }, `${lang} off, ${result}`);
    }
  }
});

/** VFS-45 R5: «Buscar actualizaciones» runs the check at once, records its result, and a second
 *  press while it runs shares it; an update found then is also offered in the notice. */
test('checking now records the result', async () => {
  let n = 0;
  let answer = null;
  const { u, views } = harness({ checkForUpdate: async () => { n++; if (answer instanceof Error) throw answer; return answer; } });
  assert.equal(u.result().result, CheckResult.NOT_RUN);
  await Promise.all([u.checkNow(), u.checkNow()]);
  assert.equal(n, 1, 'one check for two presses');
  assert.equal(u.result().result, CheckResult.NONE);
  answer = new Error('offline');
  await u.checkNow();
  assert.equal(u.result().result, CheckResult.FAILED);
  answer = { version: '0.4.0' };
  await u.checkNow();
  assert.deepEqual(u.result(), { result: CheckResult.FOUND, version: '0.4.0' });
  assert.equal(u.phase(), UpdatePhase.OFFERED);
  assert.equal(views.at(-1).action, S.updateInstall);
});

/** VFS-45 AC-5 (R5): a check made while the setting is off (the shell answers null without
 *  checking) is not "nothing newer": the result stays not run, so «Acerca de» offers the check
 *  once the setting is on again. */
test('a check with the setting off records nothing', async () => {
  let on = false;
  const views = [];
  const u = createUpdates({ api: { checkForUpdate: async () => null }, S, render: (v) => views.push(v), schedule: () => {}, checkOn: () => on });
  await u.checkNow();
  assert.equal(u.result().result, CheckResult.NOT_RUN);
  on = true;
  await u.checkNow();
  assert.equal(u.result().result, CheckResult.NONE);
});
