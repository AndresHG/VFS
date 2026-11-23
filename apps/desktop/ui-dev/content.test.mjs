// Content search on the screen (VFS-27 27c). Run: node --test apps/desktop/ui-dev/content.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  CONTENT_DEBOUNCE_MS, ContentAction, TimeUnit, contentQueryDue, unreadLabel, contentStatusLine, createContentSearch, hitPage,
  offerDue, offerText, roughTime, sectionNotes, snippetRuns, unreadCandidates,
} from '../ui/content.js';
import { CATALOGS, Lang, localeFor } from '../ui/strings.js';
import { ContentPhase, ContentStop, ContentWait, FileKind, RootStatus, Source, UnreadReason } from '../ui/contract.js';

const L = localeFor(Lang.ES, 'es-ES');
const { S } = L;
const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');
const mainJs = readFileSync(new URL('../ui/main.js', import.meta.url), 'utf8');

const status = (kind, { phase, ...extra } = {}) => ({
  settings: { enabled: true, paused: false, offered: true },
  done: 40, total: 100, perHour: null, secondsLeft: null,
  ...extra,
  phase: { kind, ...phase },
});
const root = (lastFullScan, status = RootStatus.READY) => ({ id: 1, path: '/a', status, lastFullScan });

/** VFS-27 AC-4 (R4): offered once every folder has finished its first scan and it was never
 *  offered; after «Ahora no» (offered) or with it on, never again. Upgraded installs have no
 *  settings file, so they read as never offered. */
test('the offer is due once, after every first scan', () => {
  const never = { settings: { enabled: false, paused: false, offered: false } };
  assert.equal(offerDue(never, [root(10), root(20)]), true);
  assert.equal(offerDue(never, [root(10), root(null, RootStatus.SCANNING)]), false, 'a folder still on its first scan');
  assert.equal(offerDue(never, []), false);
  assert.equal(offerDue(never, [root(10), root(null, RootStatus.REMOVING)]), true, 'a folder being removed does not count');
  assert.equal(offerDue({ settings: { ...never.settings, offered: true } }, [root(10)]), false, '«Ahora no»');
  assert.equal(offerDue({ settings: { ...never.settings, enabled: true } }, [root(10)]), false);
  assert.equal(offerDue(null, [root(10)]), false, 'status not known yet');
});

/** VFS-27 AC-4 (R4, R6): the count, a time range and a disk range; a warning when low on space. */
test('the offer shows the count, a time range and the disk use', () => {
  const offer = { documents: 12345, seconds: [12345 * 0.17, 12345 * 0.55], bytes: [160e6, 197e6], freeBytes: 1.5e9, lowSpace: false };
  const { text, warn } = offerText(L, offer);
  assert.match(text, /12\.345 documentos/);
  assert.match(text, /entre 1 y 2 horas/);
  assert.match(text, /entre 153 MB y 188 MB/);
  assert.equal(warn, '');
  assert.match(offerText(L, { ...offer, lowSpace: true }).warn, /1,4 GB/);
  assert.equal(offerText(L, { ...offer, seconds: [1, 1] }).text.includes('un minuto'), true);
  for (const id of ['banner-content-offer', 'content-offer-yes', 'content-offer-no']) assert.match(html, new RegExp(`id="${id}"`));
  assert.equal(S.contentOfferYes, 'Activar');
  assert.equal(S.contentOfferNo, 'Ahora no');
});

test('times are rounded to one unit', () => {
  for (const unit of Object.values(TimeUnit)) {
    assert.equal(typeof S.timeRange[unit], 'function', unit);
    assert.equal(typeof S.timeLeft[unit], 'function', unit);
  }
  assert.deepEqual(roughTime(30), { unit: TimeUnit.MINUTES, lo: 1, hi: 1 });
  assert.deepEqual(roughTime(600, 3000), { unit: TimeUnit.MINUTES, lo: 10, hi: 50 });
  assert.deepEqual(roughTime(2000, 7200), { unit: TimeUnit.HOURS, lo: 1, hi: 2 });
  assert.deepEqual(roughTime(86_400, 400_000), { unit: TimeUnit.DAYS, lo: 1, hi: 5 });
});

/** VFS-27 R8: documents read out of all, the rate, the time left, and «Pausar la lectura». */
test('the status area shows progress with pause', () => {
  const line = contentStatusLine(L, status(ContentPhase.READING, { done: 300, total: 1000, perHour: 1234.4, secondsLeft: 3 * 3600 }));
  assert.equal(line.action, ContentAction.PAUSE);
  assert.match(line.text, /Leyendo documentos: 300 de 1\.000/);
  assert.match(line.text, /1\.234 por hora/);
  assert.match(line.text, /quedan unas 3 horas con el equipo sin usar/);
  // A pause between documents while the PC is in use reads the same: no flicker per document.
  const inUse = contentStatusLine(L, status(ContentPhase.WAITING, { phase: { reason: ContentWait.IN_USE }, done: 300, total: 1000, perHour: 1234.4, secondsLeft: 3 * 3600 }));
  assert.equal(inUse.text, line.text);
  const paused = contentStatusLine(L, status(ContentPhase.PAUSED, { done: 300, total: 1000 }));
  assert.deepEqual(paused, { text: 'Lectura de documentos en pausa: 300 de 1.000', action: ContentAction.RESUME });
  assert.match(mainJs, /contentSetPaused\(action === ContentAction\.PAUSE\)/);
});

/** VFS-27 R6, R7, AC-7: waits say why; the low-space pause ends with «Reanudar la lectura». */
test('waits say why', () => {
  const name = contentStatusLine(L, status(ContentPhase.WAITING, { phase: { reason: ContentWait.NAME_INDEXING } }));
  assert.match(name.text, /revisión de carpetas/);
  assert.equal(name.action, ContentAction.PAUSE);
  // R7: a busy disk or processor is named, never left to guess; every wait but the in-use gap
  // has its words in both catalogs.
  const disk = contentStatusLine(L, status(ContentPhase.WAITING, { phase: { reason: ContentWait.DISK_BUSY } }));
  assert.match(disk.text, /esperando: el disco está ocupado/);
  const cpu = contentStatusLine(L, status(ContentPhase.WAITING, { phase: { reason: ContentWait.CPU_BUSY } }));
  assert.match(cpu.text, /esperando: el ordenador está ocupado con otros programas/);
  assert.equal(cpu.action, ContentAction.PAUSE);
  for (const C of Object.values(CATALOGS)) {
    for (const w of Object.values(ContentWait)) if (w !== ContentWait.IN_USE) assert.ok(C.contentWaiting[w], w);
  }
  const low = contentStatusLine(L, status(ContentPhase.WAITING, { phase: { reason: ContentWait.LOW_SPACE } }));
  assert.match(low.text, /poco espacio/);
  assert.equal(low.action, ContentAction.RESUME);
});

/** VFS-27 AC-14 (R15): each run-level reason in plain words, with «Reintentar». */
test('a run-level stop shows its reason with retry', () => {
  for (const reason of Object.values(ContentStop)) {
    const line = contentStatusLine(L, status(ContentPhase.STOPPED, { phase: { reason, detail: 'x' } }));
    assert.equal(line.action, ContentAction.RETRY);
    assert.equal(line.text, S.contentStopped[reason]);
  }
  assert.match(S.contentStopped[ContentStop.VERSION_MISMATCH], /el lector de documentos no es de esta versión/);
  assert.equal(S.contentRetry, 'Reintentar');
});

/** VFS-27 R8, R9: a finished run says nothing; one short of the total says the rest is in
 *  offline or paused folders. Off and damaged say nothing here (damage has its notice). */
test('a finished run is quiet unless documents are left', () => {
  assert.equal(contentStatusLine(L, status(ContentPhase.FINISHED, { done: 100, total: 100 })), null);
  const part = contentStatusLine(L, status(ContentPhase.FINISHED, { done: 90, total: 100 }));
  assert.match(part.text, /90 de 100.*sin conexión o en pausa/);
  assert.equal(part.action, null);
  assert.equal(contentStatusLine(L, status(ContentPhase.OFF)), null);
  assert.equal(contentStatusLine(L, status(ContentPhase.DAMAGED)), null);
  assert.equal(contentStatusLine(L, null), null);
  assert.equal(contentStatusLine(L, status(ContentPhase.CHECKING)).text, S.contentChecking);
});

/** VFS-27 AC-10 (R11): the damage notice offers both choices, and nothing deletes without them. */
test('damage offers to read again or turn off', () => {
  assert.match(html, /id="banner-content-damaged"[\s\S]*?data-s="contentDamaged"[\s\S]*?id="content-rebuild"[\s\S]*?id="content-disable"/);
  assert.equal(S.contentRebuild, 'Volver a leer los documentos');
  assert.equal(S.contentDisable, 'Desactivar');
  assert.match(mainJs, /\$\('content-rebuild'\)\.addEventListener\('click', \(\) => contentCall\(\(\) => api\.contentRebuild\(\)\)\)/);
  assert.match(mainJs, /\$\('content-disable'\)\.addEventListener\('click', \(\) => contentCall\(\(\) => api\.contentSetEnabled\(false\)\)\)/);
});

/** VFS-27 AC-12 (R13): reading under way says so; a partial result says so. */
test('the section says when reading is not finished or the result is partial', () => {
  assert.deepEqual(sectionNotes(L, status(ContentPhase.READING), false), ['Todavía leyendo documentos: 40 de 100']);
  assert.deepEqual(sectionNotes(L, status(ContentPhase.FINISHED, { done: 100, total: 100 }), true),
    ['Ningún documento tiene todas las palabras; estos tienen alguna']);
  assert.match(sectionNotes(L, status(ContentPhase.FINISHED, { done: 90 }), false)[0], /Se han leído 90 de 100/);
});

/** VFS-27 AC-11 (R12): the fragment's highlights are UTF-8 byte offsets; accents count twice. */
test('the fragment is highlighted from byte offsets', () => {
  const snippet = '… la cumbre de Peñalara y el puerto …';
  const enc = new TextEncoder();
  const a = enc.encode('… la cumbre de ').length;
  const b = a + enc.encode('Peñalara').length;
  assert.deepEqual(snippetRuns(snippet, [[a, b]]), [
    { text: '… la cumbre de ', hit: false },
    { text: 'Peñalara', hit: true },
    { text: ' y el puerto …', hit: false },
  ]);
  assert.deepEqual(snippetRuns('abc', [[2, 9], [1, 0]]), [{ text: 'abc', hit: false }], 'bad ranges are ignored');
  assert.equal(hitPage({ passages: [{ page: 3 }, { page: 9 }] }), 3);
  assert.equal(hitPage({ passages: [{ page: null }] }), null);
  assert.match(html, /id="content-hits"[\s\S]*?data-s="contentTitle"/);
  assert.equal(S.contentTitle, 'En el contenido de los documentos');
});

/** VFS-27 R12: three characters at least, spaces at the ends not counted. */
test('a content query needs three characters', () => {
  assert.equal(contentQueryDue('ab'), false);
  assert.equal(contentQueryDue('  ab  '), false);
  assert.equal(contentQueryDue('peñ'), true);
});

function harness(api) {
  const views = [];
  const timers = [];
  const cs = createContentSearch({
    api, render: (v) => views.push(v),
    schedule: (fn, ms) => { timers.push({ fn, ms, live: true }); return timers.length; },
    cancel: (id) => { if (timers[id - 1]) timers[id - 1].live = false; },
  });
  const fire = () => Promise.all(timers.splice(0).filter((t) => t.live).map((t) => t.fn()));
  return { cs, views, timers, fire };
}

const page = (n, from = 0) => Array.from({ length: n }, (_, i) => ({ id: from + i, name: `d${from + i}.pdf`, passages: [] }));

/** VFS-27 AC-11 (R12): the query waits 300 ms, carries the filters, and pages with offset. */
test('the content query waits, filters and pages', async () => {
  const calls = [];
  const { cs, views, timers, fire } = harness({
    contentSearch: async (q, f, offset) => { calls.push([q, f, offset]); return { hits: page(20, offset), partial: false, total: 45 }; },
  });
  const filters = { kinds: [FileKind.DOCUMENT], folder: '/a' };
  cs.query(' peñalara ', filters, true);
  assert.equal(calls.length, 0, 'nothing before the debounce');
  assert.equal(timers[0].ms, CONTENT_DEBOUNCE_MS);
  assert.equal(views.at(-1).loading, true);
  await fire();
  assert.deepEqual(calls, [['peñalara', filters, 0]]);
  assert.equal(views.at(-1).hits.length, 20);
  assert.equal(views.at(-1).more, true);
  cs.more();
  await new Promise((r) => setImmediate(r));
  assert.deepEqual(calls[1], ['peñalara', filters, 20]);
  assert.equal(views.at(-1).hits.length, 40);
  cs.query('pe', filters, true);
  assert.equal(views.at(-1), null, 'too short: no section');
  cs.query('peñalara', filters, false);
  assert.equal(views.at(-1), null, 'content search off: no section (R5)');
});

/** VFS-27 AC-11 (R12, D5): an older query's answer is never shown once a newer one is typed. */
test('a stale answer is dropped', async () => {
  const pending = [];
  const { cs, views, fire } = harness({
    contentSearch: (q) => new Promise((resolve) => pending.push({ q, resolve })),
  });
  // The answers are held, so the timers' work is started but not awaited.
  cs.query('primero', {}, true);
  fire();
  cs.query('segundo', {}, true);
  fire();
  pending[1].resolve({ hits: page(1, 2), partial: false, total: 1 });
  await new Promise((r) => setImmediate(r));
  pending[0].resolve({ hits: page(3), partial: false, total: 3 });
  await new Promise((r) => setImmediate(r));
  const last = views.at(-1);
  assert.deepEqual(last.hits.map((h) => h.id), [2]);
  assert.ok(!views.some((v) => v?.hits.length === 3), 'the first answer never reached the screen');
});

/** VFS-27 AC-13 (R14): documents among the name hits are looked up; the labels and the
 *  Ajustes counts have their words. */
test('unread labels and counts', () => {
  const hits = [
    { id: 1, kind: FileKind.DOCUMENT, isDir: false, source: Source.FILES },
    { id: 2, kind: FileKind.PHOTO, isDir: false, source: Source.FILES },
    { id: 3, kind: FileKind.DOCUMENT, isDir: true, source: Source.FILES },
    { id: 4, kind: FileKind.DOCUMENT, isDir: false, source: Source.BOOKMARKS },
    undefined,
  ];
  assert.deepEqual(unreadCandidates(hits), [1]);
  assert.deepEqual(Object.values(UnreadReason).map((r) => S.contentUnread[r]), [
    'en la nube', 'sin texto (escaneado)', 'protegido con contraseña', 'demasiado grande', 'formato no compatible', 'no se pudo leer',
  ]);
  assert.equal(S.contentPending('3'), 'pendientes: 3');
  // A name hit carries its reason; a cloud-only file keeps only its cloud badge.
  const doc = { id: 1, kind: FileKind.DOCUMENT, isDir: false, source: Source.FILES };
  assert.equal(unreadLabel(S, doc, UnreadReason.SCANNED), 'sin texto (escaneado)');
  assert.equal(unreadLabel(S, doc, UnreadReason.CLOUD), 'en la nube');
  assert.equal(unreadLabel(S, { ...doc, cloudOnly: true }, UnreadReason.CLOUD), '');
  assert.equal(unreadLabel(S, { ...doc, cloudOnly: true }, UnreadReason.PASSWORD), 'protegido con contraseña');
  assert.equal(unreadLabel(S, doc, null), '');
  assert.match(mainJs, /const unreadText = unreadLabel\(S, hit, unread\);[\s\S]*?badge badge-unread/);
});

/** VFS-27 AC-5 (R5): the switch, the store size and «Borrar lo leído de los documentos» with
 *  its confirmation, in Ajustes. */
test('Ajustes has the switch, the size and the delete with confirmation', () => {
  const settings = html.slice(html.indexOf('<dialog id="settings"'), html.indexOf('</dialog>', html.indexOf('<dialog id="settings"')));
  for (const id of ['content-on', 'content-store', 'content-unread-list', 'content-delete']) assert.match(settings, new RegExp(`id="${id}"`));
  assert.equal(S.contentToggle, 'Buscar también dentro de los documentos');
  assert.equal(S.contentDelete, 'Borrar lo leído de los documentos');
  assert.match(html, /<dialog id="confirm-content-delete"[\s\S]*?value="delete" data-s="contentDeleteConfirm"/);
  assert.match(mainJs, /contentDeleteDialog\.returnValue !== 'delete'\) return;\s*try \{\s*setContent\(await api\.contentDeleteStore\(\)\)/);
  assert.equal(CATALOGS[Lang.EN].contentToggle, 'Also search inside documents');
  // VFS-34 R6: its content size field follows the switch; it is not repeated here.
  assert.match(mainJs, /showContentLimit\(\$\('content-max-row'\), contentOn\(st\)\)/);
  assert.equal([...html.matchAll(/type="number"[^>]*max="100000"/g)].length, 1, 'one content size field');
});
