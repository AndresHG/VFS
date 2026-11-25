// VFS-45: Ajustes as a centered window with sections, the version and «Acerca de», and the
// quieter results (the selected row and «Mostrar en carpeta» on document hits), on the real page
// in a headless Chrome against the development fake backend (browser.mjs).
// Run: node --test apps/desktop/ui-dev/settings.test.mjs   (needs Google Chrome or Chromium)

import test from 'node:test';
import assert from 'node:assert/strict';

import { startBrowser } from './browser.mjs';
import { CATALOGS, Lang } from '../ui/strings.js';
import { ResultsTab } from '../ui/choices.js';
import { ProjectPage } from '../ui/contract.js';

const S = CATALOGS[Lang.ES];

let page;

test.before(async () => {
  page = await startBrowser();
  await page.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
});

test.after(() => page?.close());

const evaluate = (expression) => page.evaluate(expression);
const until = (expression, what) => page.until(expression, what);
const calls = `(async () => (await import('/ui/api.js')).api.fakeCalls)()`;
const chosen = `document.querySelector('#settings-tabs [aria-selected="true"]').id`;
const panelShown = (id) => `(document.getElementById(${JSON.stringify(id)}).offsetParent !== null)`;

async function key(name, keyCode) {
  for (const type of ['keyDown', 'keyUp']) {
    await page.send('Input.dispatchKeyEvent', { type, key: name, code: name, windowsVirtualKeyCode: keyCode });
  }
}

async function openSettings() {
  await evaluate(`document.getElementById('open-settings').click()`);
  await until(`document.getElementById('settings').open`, 'Ajustes');
  await until(`document.getElementById('settings').getAnimations().every((a) => a.playState === 'finished')`, 'the fade-in');
}

async function closeSettings() {
  await key('Escape', 27);
  await until(`!document.getElementById('settings').open`, 'Ajustes closed');
}

/** VFS-45 AC-1 (R1): Ajustes opens centered in the window, not as a side panel. */
test('Ajustes opens centered', async () => {
  await page.load();
  await openSettings();
  const r = await evaluate(`(() => { const r = document.getElementById('settings').getBoundingClientRect();
    return { left: r.left, right: innerWidth - r.right, top: r.top, bottom: innerHeight - r.bottom }; })()`);
  assert.ok(Math.abs(r.left - r.right) <= 2, `left ${r.left}, right ${r.right}`);
  assert.ok(r.left > 100, 'not full width');
  assert.ok(Math.abs(r.top - r.bottom) <= 2, `top ${r.top}, bottom ${r.bottom}`);
  await closeSettings();
});

/** VFS-45 AC-2 (R2): five sections in order, «General» chosen; Down, End and Home move and
 *  choose; Ajustes opens on «General» again. */
test('the sections are a vertical tablist', async () => {
  await page.load();
  await until(`!document.getElementById('stab-ai').hidden`, 'the AI section');
  await openSettings();
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('#settings-tabs [role="tab"]')].map((t) => [t.textContent, t.tabIndex])`),
    [[S.settingsTabGeneral, 0], [S.settingsTabDocuments, -1], [S.settingsTabAi, -1], [S.advancedTitle, -1], [S.settingsTabAbout, -1]]);
  assert.equal(await evaluate(`document.getElementById('settings-tabs').getAttribute('aria-orientation')`), 'vertical');
  assert.equal(await evaluate(panelShown('language')), true, '«General» holds the language');
  await evaluate(`document.getElementById('stab-general').focus()`);
  await key('ArrowDown', 40);
  assert.equal(await evaluate(chosen), 'stab-documents');
  assert.equal(await evaluate(`document.activeElement.id`), 'stab-documents');
  assert.equal(await evaluate(panelShown('content-on')), true, '«Documentos» holds the content switch');
  assert.equal(await evaluate(panelShown('language')), false);
  await key('End', 35);
  assert.equal(await evaluate(chosen), 'stab-about');
  await key('Home', 36);
  assert.equal(await evaluate(chosen), 'stab-general');
  await key('ArrowUp', 38);
  assert.equal(await evaluate(chosen), 'stab-about', 'the arrows wrap');
  await closeSettings();
  await openSettings();
  assert.equal(await evaluate(chosen), 'stab-general', 'opens on «General»');
  await closeSettings();
});

/** VFS-45 AC-2 (R2): without the `ai` feature the AI section is not listed and the arrows skip
 *  it. */
test('without the ai feature there is no AI section', async () => {
  await page.load('&ai=off');
  await openSettings();
  assert.equal(await evaluate(`document.getElementById('stab-ai').hidden`), true);
  await evaluate(`document.getElementById('stab-documents').focus()`);
  await key('ArrowDown', 40);
  assert.equal(await evaluate(chosen), 'stab-advanced');
  await closeSettings();
});

/** VFS-45 AC-3 (R3): a switch's label and the switch share one row, the switch at its end. */
test('a switch sits at the end of its row', async () => {
  await page.load();
  await openSettings();
  const r = await evaluate(`(() => {
    const box = document.getElementById('updates-on').getBoundingClientRect();
    const text = document.querySelector('#updates-on + span').getBoundingClientRect();
    const card = document.getElementById('updates-on').closest('.setting').getBoundingClientRect();
    return { boxLeft: box.left, boxRight: box.right, textRight: text.right, boxMid: box.top + box.height / 2,
      textTop: text.top, textBottom: text.bottom, cardRight: card.right };
  })()`);
  assert.ok(r.boxLeft > r.textRight, 'the switch after the label');
  assert.ok(r.cardRight - r.boxRight < 40, 'at the row end');
  assert.ok(r.boxMid > r.textTop && r.boxMid < r.textBottom, 'on the same row');
  assert.equal(await evaluate(`document.getElementById('updates-on').type`), 'checkbox', 'still a native checkbox');
  await closeSettings();
});

/** VFS-45 AC-4 (R4): the section list ends with the version from the shell. */
test('the version shows under the sections', async () => {
  await page.load();
  await openSettings();
  await until(`document.getElementById('settings-version').textContent !== ''`, 'the version');
  assert.equal(await evaluate(`document.getElementById('settings-version').textContent`), S.appVersion('0.3.1'));
  assert.equal(await evaluate(`document.querySelector('.settings-nav').lastElementChild.id`), 'settings-version');
  await closeSettings();
});

/** VFS-45 AC-5, AC-6 (R5, R6): «Acerca de» shows the version, the author, the license and
 *  «Código fuente»; «Buscar actualizaciones» says it is up to date; with an update it offers
 *  «Actualizar»; with the setting off it offers nothing. The license name and «Ver en GitHub»
 *  each ask the shell to open their project page once, by name. */
test('«Acerca de» shows the version, the update state and the credits', async () => {
  await page.load();
  await openSettings();
  await evaluate(`document.getElementById('stab-about').click()`);
  const text = await evaluate(`document.getElementById('spanel-about').innerText`);
  for (const s of [S.aboutVersion('0.3.1'), S.aboutAuthor, S.aboutLicenseName, S.aboutSourceOpen]) assert.ok(text.includes(s), s);
  assert.equal(await evaluate(`document.getElementById('about-update-action').textContent`), S.aboutCheck);
  await evaluate(`document.getElementById('about-update-action').click()`);
  await until(`document.getElementById('about-update').textContent === ${JSON.stringify(S.aboutUpToDate)}`, '«Está al día»');
  assert.equal(await evaluate(`document.getElementById('about-update-action').hidden`), true);

  await evaluate(`document.getElementById('about-source').click()`);
  await until(`${calls}.then((c) => c.openProjectPage === 1)`, 'the project page opened');
  assert.equal((await evaluate(calls)).lastProjectPage, ProjectPage.SOURCE);
  await evaluate(`document.getElementById('about-license').click()`);
  await until(`${calls}.then((c) => c.openProjectPage === 2)`, 'the license opened');
  assert.equal((await evaluate(calls)).lastProjectPage, ProjectPage.LICENSE);

  await evaluate(`document.getElementById('stab-general').click()`);
  await evaluate(`document.getElementById('updates-on').click()`);
  await until(`!document.getElementById('updates-on').checked`, 'the setting off');
  await evaluate(`document.getElementById('stab-about').click()`);
  assert.equal(await evaluate(`document.getElementById('about-update').textContent`), S.aboutUpdatesOff);
  assert.equal(await evaluate(`document.getElementById('about-update-action').hidden`), true, 'nothing to press');
  await closeSettings();

  await page.load('&update');
  await openSettings();
  await evaluate(`document.getElementById('stab-about').click()`);
  await evaluate(`document.getElementById('about-update-action').click()`);
  await until(`document.getElementById('about-update').textContent === ${JSON.stringify(S.aboutUpdateFound('0.9.0'))}`, 'the update');
  assert.equal(await evaluate(`document.getElementById('about-update-action').textContent`), S.updateInstall);
  await closeSettings();
});

/** VFS-45 AC-7 (R7): on «Documentos», «Mostrar en carpeta» is faded out until the hit is under
 *  the pointer or holds the focus, stays reachable with Tab, and a double click reveals the hit.
 *  On «Nombres», the selected row's button has no accent fill. */
test('«Mostrar en carpeta» shows on the hit in use', async () => {
  await page.load('&content=on');
  // Headless Chrome reports no hover, where the button always shows; the desktop shells have a
  // mouse. Check that rule, then take it out to test the pointer and focus rules.
  assert.equal(await evaluate(`(() => {
    const sheet = [...document.styleSheets].find((x) => x.href?.endsWith('styles.css'));
    const i = [...sheet.cssRules].findIndex((r) => r.conditionText === '(hover: none)' && r.cssText.includes('.chit .btn'));
    if (i < 0) return false;
    sheet.deleteRule(i);
    return true;
  })()`), true, 'always shown without hover');
  await until(`document.getElementById('content-on').checked`, 'the content status');
  await page.type('foto');
  await until(`document.querySelectorAll('#results .row[aria-selected="true"] .reveal').length > 0`, 'a selected name hit');
  const fill = await evaluate(`(() => {
    const b = getComputedStyle(document.querySelector('#results .row[aria-selected="true"] .reveal')).backgroundColor;
    const accent = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim();
    const probe = document.createElement('i'); probe.style.color = accent; document.body.append(probe);
    const a = getComputedStyle(probe).color; probe.remove();
    return { b, a };
  })()`);
  assert.notEqual(fill.b, fill.a, 'no accent fill on the selected row');

  await page.tab(ResultsTab.DOCUMENTS);
  await until(`document.querySelectorAll('#content-list .chit').length > 1`, 'the content hits');
  const opacity = (i) => `getComputedStyle(document.querySelectorAll('#content-list .chit .btn')[${i}]).opacity`;
  await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1, y: 1 });
  await until(`${opacity(0)} === '0' && ${opacity(1)} === '0'`, 'faded out');
  const at = await evaluate(`(() => { const r = document.querySelectorAll('#content-list .chit')[1].getBoundingClientRect(); return [r.left + 60, r.top + r.height / 2]; })()`);
  await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: at[0], y: at[1] });
  await until(`${opacity(1)} === '1'`, 'shown under the pointer');
  assert.equal(await evaluate(opacity(0)), '0', 'not on the other hits');
  await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1, y: 1 });
  await evaluate(`document.querySelectorAll('#content-list .chit .btn')[0].focus()`);
  await until(`${opacity(0)} === '1'`, 'shown with the focus');
  assert.equal(await evaluate(`document.querySelectorAll('#content-list .chit .btn')[0].tabIndex`), 0, 'in the tab order');

  await evaluate(`(async () => { const c = (await import('/ui/api.js')).api.fakeCalls; c.reveal = 0; })()`);
  for (const [type, clickCount] of [['mousePressed', 1], ['mouseReleased', 1], ['mousePressed', 2], ['mouseReleased', 2]]) {
    await page.send('Input.dispatchMouseEvent', { type, x: at[0], y: at[1], button: 'left', clickCount });
  }
  await until(`${calls}.then((c) => c.reveal === 1)`, 'the double click revealed the hit');
});
