// The one Spanish screen (VFS-1 AC-12), checked without a browser: the copy in strings.js
// and the markup of index.html. Run: node --test apps/desktop/ui-dev/screen.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { CATALOGS, Lang } from '../ui/strings.js';
import { FileKind, LinkKind, Source, UnreadableReason } from '../ui/contract.js';
import { DateChoice, SizeChoice } from '../ui/choices.js';
import { placesOf, showsCloudBadge } from '../ui/copies.js';
import { SecondAction, secondActionOf } from '../ui/links.js';

const S = CATALOGS[Lang.ES];

const html = readFileSync(new URL('../ui/index.html', import.meta.url), 'utf8');

/** Every user-visible string in `S`: nested tables walked, functions called with samples. */
function allStrings(value, out = []) {
  if (typeof value === 'string') out.push(value);
  else if (typeof value === 'function') allStrings(value('1', '1', '/a', '1 mar', '1 KB', true), out);
  else if (value && typeof value === 'object') for (const v of Object.values(value)) allStrings(v, out);
  return out;
}

/** VFS-1 AC-12, AC-11: every type chip and every filter has a Spanish label. */
test('every type chip and filter option has a label', () => {
  for (const kind of Object.values(FileKind)) {
    assert.ok(S.kindsPlural[kind], `chip label for ${kind}`);
    assert.ok(S.kindSingular[kind], `row label for ${kind}`);
  }
  assert.deepEqual(Object.keys(S.dateOptions).sort(), Object.values(DateChoice).sort());
  assert.deepEqual(Object.keys(S.sizeOptions).sort(), Object.values(SizeChoice).sort());
  for (const key of ['fType', 'fDate', 'fSize', 'fFolder', 'fFrom', 'fTo', 'folderAll', 'folderPick']) {
    assert.ok(S[key], key);
  }
});

/** VFS-1 AC-12, decision 16: no user-facing text says "indexar", "índice" or "reindexar". */
test('plain words only', () => {
  const texts = allStrings(S);
  assert.ok(texts.length > 50);
  for (const s of texts) assert.doesNotMatch(s, /index|índice|indice/i, s);
});

/** VFS-1 AC-12: the page is Spanish, every text slot it names exists in strings.js, and it is
 *  one screen with one search box. */
test('one Spanish screen with one search box', () => {
  assert.match(html, /<html lang="es">/);
  const slots = [...html.matchAll(/data-s(?:-label)?="([A-Za-z]+)"/g)].map((m) => m[1]);
  assert.ok(slots.length > 10);
  for (const key of slots) assert.equal(typeof S[key], 'string', `strings.js has ${key}`);
  assert.equal([...html.matchAll(/type="search"/g)].length, 1);
  assert.match(html, /<input id="q" type="search"[^>]*maxlength="\d+"/s);
  for (const id of ['kind-chips', 'f-date', 'f-size', 'f-folder']) assert.match(html, new RegExp(`id="${id}"`));
});

/** VFS-1 AC-11: the type filter names what it narrows to, so a chosen filter is visible
 *  without opening it. */
test('type button names the chosen types', () => {
  assert.equal(S.typeButton(), 'Todos los tipos');
  assert.equal(S.typeButton('Fotos'), 'Fotos');
  assert.equal(S.typeButton('Fotos', 'Vídeos'), 'Fotos, Vídeos');
  assert.equal(S.typeButton('Documentos', 'Fotos', 'Vídeos'), 'Documentos y 2 más');
});

/** VFS-1 AC-11, AC-12: the type checklist is a popover opened by its own button (keyboard and
 *  light dismiss come from the platform), and the chips live inside it. */
test('type checklist is a popover owned by the type button', () => {
  assert.match(html, /<button id="f-type"[^>]*popovertarget="f-type-pop"/);
  assert.match(html, /<div id="f-type-pop"[^>]*popover>\s*<fieldset[^>]*>[\s\S]*?id="kind-chips"[\s\S]*?<\/fieldset>\s*<\/div>/);
});

/** VFS-19 AC-5: a result with copies stands for every place, itself first; folders,
 *  bookmarks and results without copies stand for one place or none. */
test('a result with copies lists every place', () => {
  const hit = { id: 1, dir: '/a', modified: 3, isDir: false, copies: [{ id: 2, dir: '/b', modified: 2 }] };
  assert.deepEqual(placesOf(hit), [{ id: 1, dir: '/a', modified: 3 }, { id: 2, dir: '/b', modified: 2 }]);
  assert.equal(placesOf({ id: 1, dir: '/a', modified: 3, isDir: false }).length, 1);
  assert.deepEqual(placesOf({ id: 1, dir: '/a', isDir: true, copies: [] }), []);
  assert.deepEqual(placesOf({ id: 1, source: Source.BOOKMARKS }), []);
  assert.deepEqual(placesOf(undefined), []);
});

/** VFS-19 AC-5: the "en N sitios" mark, the row label and the list of places are in Spanish,
 *  and the list is a labelled dialog. */
test('copies have their Spanish copy and dialog', () => {
  assert.equal(S.copiesBadge(3), 'en 3 sitios');
  assert.match(S.copiesTitle('Boda.mp4', 3), /Boda\.mp4.*3 sitios/);
  assert.match(S.rowLabel('a', 'Vídeo', '/d', '1 mar', '1 KB', false, 3), /está en 3 sitios/);
  assert.doesNotMatch(S.rowLabel('a', 'Vídeo', '/d', '1 mar', '1 KB', false, 1), /sitios/);
  assert.ok(S.copiesIntro && S.copiesBadgeLabel(2) && S.copiesModified('1 mar'));
  assert.match(html, /<dialog id="copies"[^>]*aria-labelledby="copies-title"/);
  assert.match(html, /<ul class="roots" id="copies-list"/);
});

/** VFS-18 D2, AC-6: every link kind has its badge and its details label, and a missing
 *  target names the target and says why it may be missing. */
test('link rows have their wording and the missing-target message', () => {
  for (const kind of Object.values(LinkKind)) {
    assert.ok(S.linkBadge[kind], `badge for ${kind}`);
    assert.ok(S.detailsLinkAt[kind], `details label for ${kind}`);
  }
  const t = 'D:\\Boda 2003';
  const msg = S.targetMissing(t);
  assert.ok(msg.includes(`«${t}»`));
  assert.match(msg, /movido o borrado/);
  assert.match(msg, /disco no esté conectado/);
  assert.equal(S.targetButton, 'Mostrar el destino');
  assert.equal(S.secondKey, 'Mayús+Intro');
});

/** VFS-18 AC-6, AC-7, D3, D6: "Mostrar el destino" only for a shortcut or link with a
 *  target; "Abrir en Firefox" only for an Internet Shortcut that can be opened (http/https);
 *  nothing for a file: address, a link with no target or an ordinary file. */
test('the second action of a row depends on the link', () => {
  const hit = (link) => ({ id: 1, name: 'x', dir: '/a', isDir: false, ...(link ? { link } : {}) });
  assert.equal(secondActionOf(hit({ kind: LinkKind.SHORTCUT, target: 'D:\\Boda', openable: false })), SecondAction.TARGET);
  assert.equal(secondActionOf(hit({ kind: LinkKind.SYMLINK, target: '/Volumes/Fotos', openable: false })), SecondAction.TARGET);
  assert.equal(secondActionOf(hit({ kind: LinkKind.INTERNET, target: 'https://mibanco.es/', openable: true })), SecondAction.OPEN);
  assert.equal(secondActionOf(hit({ kind: LinkKind.INTERNET, target: 'file:///C:/x.txt', openable: false })), null);
  assert.equal(secondActionOf(hit({ kind: LinkKind.SHORTCUT, target: null, openable: false })), null);
  assert.equal(secondActionOf(hit(null)), null);
  assert.equal(secondActionOf(undefined), null);
});

const css = readFileSync(new URL('../ui/styles.css', import.meta.url), 'utf8');

/** CSS blocks `@media <query> { … }`, balanced braces included. */
function mediaBlocks(query) {
  const out = [];
  let at = css.indexOf(`@media ${query}`);
  while (at >= 0) {
    let i = css.indexOf('{', at) + 1;
    const start = i;
    for (let depth = 1; depth > 0; i++) depth += css[i] === '{' ? 1 : css[i] === '}' ? -1 : 0;
    out.push(css.slice(start, i - 1));
    at = css.indexOf(`@media ${query}`, i);
  }
  return out.join('\n');
}

/** VFS-24 AC-3: every control has a name a screen reader can read — its text, a
 *  visually hidden text, a text span main.js fills, or a label set from the copy table. */
test('every button and field has an accessible name', () => {
  // Buttons whose whole text main.js sets from the copy table (`$('id').textContent = S.…`,
  // or the pattern listed here).
  const mainJs = readFileSync(new URL('../ui/main.js', import.meta.url), 'utf8');
  const scripted = {
    pause: /pause\.textContent = actions\.paused \? S\.resume : S\.pause/,
    // VFS-21: shown only with an action, whose text it carries.
    'update-action': /action\.hidden = !view\.action;\s*action\.textContent = view\.action/,
  };
  for (const m of html.matchAll(/<button\b([^>]*)>([\s\S]*?)<\/button>/g)) {
    const [, attrs, inner] = m;
    const id = /\bid="([^"]+)"/.exec(attrs)?.[1];
    if (scripted[id] || mainJs.includes(`$('${id}').textContent = S.`)) {
      if (scripted[id]) assert.match(mainJs, scripted[id], `#${id} gets its text from main.js`);
      continue;
    }
    const named = /aria-label|data-s(-label)?="/.test(attrs) || /data-s="|<span id="[\w-]+-text">|>[^<\s][^<]*</.test(inner);
    assert.ok(named, `unnamed button: ${m[0].slice(0, 80)}`);
  }
  for (const m of html.matchAll(/<(input|select)\b[^>]*\bid="([^"]+)"[^>]*>/g)) {
    const [tag, , id] = m;
    const labelled = /aria-label/.test(tag) || html.includes(`for="${id}"`)
      || new RegExp(`<label[^>]*>(?:(?!</label>)[\\s\\S])*id="${id}"`).test(html);
    assert.ok(labelled, `unlabelled field #${id}`);
  }
});

/** VFS-24 AC-3: movement runs only without "reduce motion", except the progress
 *  sweep, which slows down instead; Windows high contrast keeps the selected row, the state
 *  dots and the progress bars visible. */
test('reduced motion and high contrast are honoured', () => {
  const motion = mediaBlocks('(prefers-reduced-motion: no-preference)');
  for (const m of css.matchAll(/animation:\s*([\w-]+)/g)) {
    if (m[1] === 'sweep') continue;
    assert.ok(motion.includes(m[0]), `${m[0]} runs under reduced motion`);
  }
  assert.match(mediaBlocks('(prefers-reduced-motion: reduce)'), /progress:not\(\[value\]\)/);
  const forced = mediaBlocks('(forced-colors: active)');
  for (const sel of ['.row[aria-selected="true"]', '.root-state::before', 'progress::-webkit-progress-value']) {
    const at = forced.indexOf(`${sel} {`);
    assert.ok(at >= 0, `${sel} under forced colours`);
    const body = forced.slice(at, forced.indexOf('}', at));
    assert.match(body, /Highlight|CanvasText|outline/, `${sel} drawn with system colours or an outline`);
  }
});

/** VFS-10 AC-4: the unreadable-folders line has a singular and a plural form and no text for
 *  none (the panel hides it), every reason
 *  has its plain text, and the "y N más" line names the rest; in both catalogs. */
test('folders that could not be read have their wording', () => {
  const { es, en } = { es: CATALOGS[Lang.ES], en: CATALOGS[Lang.EN] };
  assert.equal(es.unreadableLine(1, '1'), '1 carpeta no se pudo revisar');
  assert.equal(es.unreadableLine(1234, '1.234'), '1.234 carpetas no se pudieron revisar');
  assert.equal(en.unreadableLine(1, '1'), '1 folder could not be checked');
  assert.equal(en.unreadableLine(3, '3'), '3 folders could not be checked');
  // None: no text, so the line stays hidden.
  assert.equal(es.unreadableLine(0, '0'), '');
  assert.equal(en.unreadableLine(0, '0'), '');
  for (const cat of [es, en]) {
    for (const reason of Object.values(UnreadableReason)) assert.ok(cat.unreadableReason[reason], reason);
    assert.ok(cat.unreadableMore('7').includes('7'));
    assert.ok(cat.unreadableSee && cat.unreadableTitle && cat.unreadableHint);
  }
  assert.deepEqual(Object.values(es.unreadableReason).sort(), ['Error de disco', 'No disponible', 'Sin permiso', 'Solo en la nube']);
});

/** VFS-25 AC-4: the cloud-only badge has its text in both catalogs; `placesOf` carries each
 *  place's `cloudOnly`; the badge shows only for a file marked cloud-only. */
test('cloud-only files and places get the badge', () => {
  assert.equal(CATALOGS[Lang.ES].cloudOnlyBadge, 'Solo en la nube');
  assert.equal(CATALOGS[Lang.EN].cloudOnlyBadge, 'Cloud only');
  const hit = {
    id: 1, name: 'boda.mp4', dir: 'C:\\Fotos', isDir: false, modified: 1, cloudOnly: true,
    copies: [{ id: 2, dir: 'D:\\Copia', modified: 1 }, { id: 3, dir: 'E:\\Nube', modified: 1, cloudOnly: true }],
  };
  assert.deepEqual(placesOf(hit).map((p) => showsCloudBadge(p)), [true, false, true]);
  assert.equal(showsCloudBadge(hit), true);
  assert.equal(showsCloudBadge({ ...hit, cloudOnly: false }), false);
  assert.equal(showsCloudBadge({ ...hit, isDir: true }), false);
  assert.equal(showsCloudBadge({ ...hit, source: Source.BOOKMARKS }), false);
  assert.equal(showsCloudBadge(undefined), false);
});

/** VFS-33 AC-10: the switch for the excluded folders sits, unchecked, in a folded «Avanzado»
 *  section of Ajustes, with its copy in both catalogs. */
test('advanced switch for system and program folders', () => {
  const settings = html.slice(html.indexOf('<dialog id="settings"'), html.indexOf('</dialog>', html.indexOf('<dialog id="settings"')));
  const details = settings.slice(settings.indexOf('<details class="setting setting-advanced"'));
  assert.ok(details.startsWith('<details class="setting setting-advanced">'), 'folded by default (no open attribute)');
  assert.match(details, /<input type="checkbox" id="include-all">/, 'unchecked by default');
  for (const lang of Object.values(Lang)) {
    for (const key of ['advancedTitle', 'includeAllToggle', 'includeAllIntro']) {
      assert.ok(CATALOGS[lang][key], `${lang}.${key}`);
    }
  }
  assert.match(S.includeAllIntro, /Library/);
  assert.match(S.includeAllIntro, /AppData/);
});
