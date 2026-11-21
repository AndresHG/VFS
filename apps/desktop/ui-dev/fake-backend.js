// DEVELOPMENT ONLY. A fake of the Tauri commands and events so the UI can be opened in a
// normal browser with realistic data. It lives outside `ui/` (the bundled frontendDist), and
// `ui/api.js` imports it only without a Tauri runtime, on http://localhost|127.0.0.1, with
// `?fake` in the URL (VFS-1 U5).
//
// Run:  python3 -m http.server 8765 --directory apps/desktop
// Open: http://localhost:8765/ui/?fake
//
// URL options:
//   scenario=empty       no folders yet
//   scenario=first-scan  every folder still being scanned for the first time
//   (default)            one ready folder, one scanning, one unavailable; 50.000 files
//   slow                 add 0.4–1.2 s of latency to every search
// Query words that trigger states: "fallo" (search error), "tardar" (deadline, no hits),
// any name containing "borrado" reveals as not_found; paths over 259 chars reveal as
// opened_parent. Firefox bookmarks (VFS-6) start off; turning them on in the folder panel
// gives a few fake ones after 1 s. Each "Añadir carpeta…" walks a fixed list of picks: a new folder, a folder
// inside an existing one, a parent of an existing one, a network share, then cancel.

import {
  BookmarkRoot, DriveKind, ErrorCode, FileKind, IndexerState, MatchKind, ReadState, RevealOutcome, RootStatus,
  Source, TruncatedBy,
} from '../ui/contract.js';

/** `?scenario=` values (see the header). */
const Scenario = Object.freeze({ DEFAULT: '', EMPTY: 'empty', FIRST_SCAN: 'first-scan' });

const TOTAL_FILES = 50_000;
const NOW = Math.floor(Date.now() / 1000);
const DAY = 86_400;

function rng(seed) {
  return () => {
    seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
// The seeded generator is created per backend and passed down, so no module state mutates.
function makeRandom(seed) {
  const rand = rng(seed);
  return {
    rand,
    pick: (a) => a[Math.floor(rand() * a.length)],
    int: (lo, hi) => lo + Math.floor(rand() * (hi - lo + 1)),
  };
}

/** A few bookmarks for the VFS-6 source (title, url, root, folder, days ago). */
const BOOKMARKS = [
  ['Recetas', '', BookmarkRoot.MENU, [], 800],
  ['Receta de croquetas caseras', 'https://www.cocina-casera.es/croquetas', BookmarkRoot.MENU, ['Recetas', 'Entrantes'], 40],
  ['Flan de huevo de la abuela', 'https://recetasdelaabuela.es/flan', BookmarkRoot.MENU, ['Recetas', 'Postres'], 400],
  ['Banco - acceso clientes', 'https://www.banco.es/clientes', BookmarkRoot.TOOLBAR, [], 1200],
  ['Cómo podar los rosales', 'https://jardineria.es/rosales-poda', BookmarkRoot.UNFILED, [], 15],
  ['Ruta por Galicia en coche', 'https://viajes.es/galicia', BookmarkRoot.MENU, ['Viajes'], 700],
  ['Documento guardado en el equipo', 'file:///C:/Users/Carmen/receta.pdf', BookmarkRoot.UNFILED, [], 90],
].map(([title, url, root, folder, days], i) => ({
  id: i + 1, isFolder: !url, title, url, root, folder, profile: null, added: NOW - days * DAY,
  openable: url.startsWith('http'),
  // Folder ids: 1 is the "Recetas" folder (the first entry), 1001 the menu, 1003 "Otros marcadores".
  parent: folder[0] === 'Recetas' ? 1 : root === BookmarkRoot.UNFILED ? 1003 : 1001,
}));

const fold = (s) => s.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase().replace(/\\/g, '/');

const people = ['Carmen', 'Lucía', 'Andrés', 'Concepción', 'José Luis', 'Mª Ángeles', 'Begoña', 'Iñaki', 'Pilar', 'Ramón'];
const places = ['Galicia', 'Cuenca', 'Benidorm', 'Peñíscola', 'Salamanca', 'Albacete', 'Cádiz', 'Logroño', 'Ávila', 'Málaga'];
const events = ['Boda de Lucía y Marcos', 'Navidad en casa de la abuela', 'Viaje a', 'Comunión de Iñaki', 'Cumpleaños de Pilar', 'Verano en', 'Fiestas del pueblo', 'Excursión a', 'Bautizo de Martín'];
const docs = [
  'Escritura notaría piso Calle Mayor', 'Declaración de la renta', 'Factura Iberdrola', 'Contrato de alquiler',
  'Receta de croquetas de la abuela', 'Póliza seguro hogar', 'Carta al ayuntamiento', 'Presupuesto reforma cocina',
  'Recibo comunidad de vecinos', 'Informe médico revisión', 'Certificado de empadronamiento', 'Nómina',
  'Extracto bancario', 'Garantía lavadora', 'Libro de familia escaneado', 'Testamento copia simple',
  'Solicitud pensión Seguridad Social', 'ITV coche', 'Multa de tráfico recurso', 'Lista de la compra',
];
const docExt = ['pdf', 'pdf', 'pdf', 'docx', 'doc', 'odt', 'xlsx', 'txt'];
const months = ['enero', 'febrero', 'marzo', 'abril', 'mayo', 'junio', 'julio', 'agosto', 'septiembre', 'octubre', 'noviembre', 'diciembre'];
const songs = ['Españoleta', 'Entre dos aguas', 'Cántame', 'La bien pagá', 'Mediterráneo', 'Libertad sin ira', 'Sevillanas del adiós', 'Un año de amor'];
const singers = ['Joan Manuel Serrat', 'Paco de Lucía', 'Rocío Jurado', 'Raphael', 'Mocedades', 'Cecilia', 'Jarcha'];

function makeRoots(scenario) {
  const firstScan = scenario === Scenario.FIRST_SCAN;
  return [
    { id: 1, path: 'C:\\Users\\Carmen\\Documents', status: firstScan ? RootStatus.SCANNING : RootStatus.READY, files: 0, dirs: 0, lastFullScan: firstScan ? null : NOW - 3 * DAY, weight: 0.3 },
    { id: 2, path: 'D:\\Archivo familiar', status: RootStatus.SCANNING, files: 0, dirs: 0, lastFullScan: null, weight: 0.6 },
    { id: 3, path: 'E:\\Copias antiguas', status: firstScan ? RootStatus.SCANNING : RootStatus.UNAVAILABLE, files: 0, dirs: 0, lastFullScan: firstScan ? null : NOW - 400 * DAY, weight: 0.1, drive: DriveKind.REMOVABLE },
  ];
}

function makeEntry(root, R) {
  const y = R.int(1998, 2026);
  const t = Math.min(NOW - R.int(0, 3) * DAY, Math.floor(Date.UTC(y, R.int(0, 11), R.int(1, 28)) / 1000) + R.int(0, DAY));
  if (root.id === 1) {
    const sub = R.pick(['Banco', 'Casa', `Hacienda\\${y}`, 'Médico', 'Seguros', 'Recetas', 'Coche', `Facturas\\${y}\\${R.pick(months)}`, 'Varios']);
    const base = R.pick(docs);
    const name = `${base}${R.rand() < 0.5 ? ` ${R.pick(months)} ${y}` : ` ${y}`}.${R.pick(docExt)}`;
    return { dir: `${root.path}\\${sub}`, name, kind: FileKind.DOCUMENT, size: R.int(8_000, 4_000_000), modified: t };
  }
  const r = R.rand();
  if (r < 0.62) {
    const ev = R.pick(events);
    const evName = ev.endsWith(' a') || ev.endsWith(' en') ? `${ev} ${R.pick(places)}` : ev;
    const deep = R.rand() < 0.15
      ? `\\Fotos escaneadas del álbum de la abuela Concepción\\Caja ${R.int(1, 9)} - Años sesenta y setenta\\${evName} con los primos de ${R.pick(places)}\\Sobre ${R.int(1, 40)} sin clasificar todavía\\Revisar con calma antes de tirar nada de lo que hay aquí dentro, por favor`
      : `\\Fotos\\${y}\\${evName}`;
    const ext = R.pick(['JPG', 'JPG', 'jpg', 'jpeg', 'png', 'HEIC']);
    const name = R.rand() < 0.6 ? `IMG_${String(R.int(1, 9999)).padStart(4, '0')}.${ext}` : `${evName} ${R.int(1, 300)}.${ext}`;
    return { dir: root.path + deep, name, kind: FileKind.PHOTO, size: R.int(400_000, 9_000_000), modified: t };
  }
  if (r < 0.75) {
    const ev = R.pick(events);
    return { dir: `${root.path}\\Vídeos\\${y}`, name: `${ev} ${R.pick(places)}.${R.pick(['mp4', 'MOV', 'avi'])}`, kind: FileKind.VIDEO, size: R.int(20_000_000, 4_500_000_000), modified: t };
  }
  if (r < 0.9) {
    const singer = R.pick(singers);
    return { dir: `${root.path}\\Música\\${singer}`, name: `${String(R.int(1, 14)).padStart(2, '0')} - ${R.pick(songs)}.mp3`, kind: FileKind.AUDIO, size: R.int(2_000_000, 12_000_000), modified: t };
  }
  if (r < 0.97) {
    return { dir: `${root.path}\\Documentos de ${R.pick(people)}`, name: `${R.pick(docs)} ${y}.${R.pick(docExt)}`, kind: FileKind.DOCUMENT, size: R.int(8_000, 3_000_000), modified: t };
  }
  return { dir: `${root.path}\\Varios`, name: `${R.pick(['copia', 'backup', 'programa', 'datos'])}_${R.int(1, 999)}.${R.pick(['zip', 'exe', 'dat', 'iso'])}`, kind: FileKind.OTHER, size: R.int(1_000, 900_000_000), modified: t };
}

/** VFS-19, as the core: files with the same folded name and size follow the first. */
function groupCopies(list) {
  const out = [];
  const first = new Map();
  for (const item of list) {
    const k = item.e.isDir ? null : `${item.e.fName}\u0000${item.e.size}`;
    if (k !== null && first.has(k)) { first.get(k).copies.push(item.e); continue; }
    const row = { ...item, copies: [] };
    if (k !== null) first.set(k, row);
    out.push(row);
  }
  return out;
}

function buildCorpus(roots, R) {
  const files = [];
  const dirs = new Map(); // path -> entry
  const weights = roots.map((r) => r.weight);
  let seqPerRoot = new Map(roots.map((r) => [r.id, 0]));
  for (let i = 0; i < TOTAL_FILES; i++) {
    let x = R.rand();
    let root = roots[roots.length - 1];
    for (let k = 0; k < roots.length; k++) { if ((x -= weights[k]) < 0) { root = roots[k]; break; } }
    const e = makeEntry(root, R);
    if (R.rand() < 0.002) e.name = e.name.replace(/(\.[^.]+)$/, ' borrado$1');
    const seq = seqPerRoot.get(root.id);
    seqPerRoot.set(root.id, seq + 1);
    files.push({ id: i + 1, rootId: root.id, seq, isDir: false, matched: null, ...e, fName: fold(e.name), fDir: fold(e.dir) });
    // VFS-19: some files also kept in a backup folder, same name and size (copies).
    if (i % 25 === 0) {
      const dir = `${root.path}\\Copia de seguridad\\${e.dir.slice(e.dir.lastIndexOf('\\') + 1)}`;
      files.push({ id: TOTAL_FILES + i + 1, rootId: root.id, seq, isDir: false, matched: null, ...e, dir, modified: e.modified - 86_400 * 30, fName: fold(e.name), fDir: fold(dir) });
    }
    // Register every folder between the root and the file as a folder entry (decision D1).
    let d = e.dir;
    while (d.length > root.path.length && !dirs.has(d)) {
      const cut = d.lastIndexOf('\\');
      const parent = d.slice(0, cut);
      const name = d.slice(cut + 1);
      dirs.set(d, { id: dirs.size + 1, rootId: root.id, seq, isDir: true, dir: parent, name, kind: FileKind.FOLDER, size: 0, modified: e.modified, fName: fold(name), fDir: fold(parent) });
      d = parent;
    }
  }
  return { files, dirs: [...dirs.values()], counts: seqPerRoot };
}

function withinOneEdit(a, b) {
  if (Math.abs(a.length - b.length) > 1) return false;
  let i = 0; let j = 0; let edits = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) { i++; j++; continue; }
    if (++edits > 1) return false;
    if (a.length > b.length) i++;
    else if (a.length < b.length) j++;
    else if (a[i + 1] === b[j] && a[i] === b[j + 1]) { i += 2; j += 2; }
    else { i++; j++; }
  }
  return edits + (a.length - i) + (b.length - j) <= 1;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const clone = (o) => JSON.parse(JSON.stringify(o));

export function createFakeBackend(params) {
  const scenario = params.get('scenario') || Scenario.DEFAULT;
  const slow = params.has('slow');
  let roots = scenario === Scenario.EMPTY ? [] : makeRoots(scenario);
  const R = makeRandom(20261120);
  const corpus = buildCorpus(makeRoots(Scenario.DEFAULT), R);
  const all = [...corpus.files, ...corpus.dirs];
  const totalOf = (id) => corpus.counts.get(id) || 0;

  // Scan simulation: how many entries of each root are visible so far.
  const done = new Map();
  const pending = new Map();
  for (const r of roots) {
    const total = totalOf(r.id);
    done.set(r.id, r.status === RootStatus.SCANNING ? Math.floor(total * (scenario === Scenario.FIRST_SCAN ? 0.05 : 0.35)) : total);
    pending.set(r.id, r.status === RootStatus.SCANNING ? 900 : 0);
  }
  let paused = false;
  const progressCbs = new Set();
  const changedCbs = new Set();
  const updateSettings = { checkOnOpen: true };
  const updateCbs = new Set();
  const bookmarkCbs = new Set();
  const bookmarks = { enabled: false, state: ReadState.NEVER, count: 0, profiles: 0, lastRead: null, roots: [] };
  let lastChanged = 0;
  let picks = 0;
  let generation = 0;
  let cache = null;

  const visible = (e) => {
    const r = roots.find((x) => x.id === e.rootId);
    return r && r.status !== RootStatus.REMOVING && e.seq < done.get(r.id);
  };

  function rootView(r) {
    const n = done.get(r.id) || 0;
    return { id: r.id, path: r.path, status: r.status, files: n, dirs: Math.round(n / 22), lastFullScan: r.lastFullScan, drive: r.drive ?? DriveKind.LOCAL };
  }

  function progress() {
    const scanning = roots.some((r) => r.status === RootStatus.SCANNING || r.status === RootStatus.PENDING);
    return {
      state: paused && scanning ? IndexerState.PAUSED : scanning ? IndexerState.SCANNING : IndexerState.IDLE,
      filesIndexed: roots.reduce((s, r) => s + (done.get(r.id) || 0), 0),
      roots: roots.map((r) => ({
        rootId: r.id, path: r.path, status: r.status,
        dirsDone: Math.round((done.get(r.id) || 0) / 22), dirsPending: pending.get(r.id) || 0,
        filesIndexed: done.get(r.id) || 0,
        currentDir: r.status === RootStatus.SCANNING ? `${r.path}\\Fotos\\2004` : null,
      })),
    };
  }

  setInterval(() => {
    let changed = false;
    if (!paused) {
      for (const r of roots) {
        if (r.status === RootStatus.PENDING) { r.status = RootStatus.SCANNING; changed = true; }
        if (r.status !== RootStatus.SCANNING) continue;
        const total = totalOf(r.id) || 4000;
        // Root 2 crawls slowly so the default page keeps showing a scan in progress.
        const step = r.id === 2 && scenario !== Scenario.FIRST_SCAN ? 40 : 160;
        done.set(r.id, Math.min(total, (done.get(r.id) || 0) + step));
        pending.set(r.id, Math.max(0, (pending.get(r.id) || 0) - (step > 100 ? 3 : 1)));
        if (done.get(r.id) >= total) {
          r.status = RootStatus.READY; r.lastFullScan = Math.floor(Date.now() / 1000); pending.set(r.id, 0);
        }
        changed = true;
      }
    }
    const p = progress();
    for (const cb of progressCbs) cb(clone(p));
    if (changed && Date.now() - lastChanged > 2000) {
      lastChanged = Date.now();
      generation++;
      for (const cb of changedCbs) cb();
    }
  }, 500);

  function rank(text, f) {
    const ts = fold(text).split(/\s+/).filter(Boolean);
    const kinds = f.kinds || [];
    const pass = (e) => visible(e)
      && (!kinds.length || kinds.includes(e.kind))
      && (f.modifiedFrom == null || e.modified >= f.modifiedFrom)
      && (f.modifiedTo == null || e.modified < f.modifiedTo)
      && (f.sizeMin == null || (!e.isDir && e.size >= f.sizeMin))
      && (f.sizeMax == null || (!e.isDir && e.size <= f.sizeMax))
      && (!f.folder || fold(e.dir + '\\').startsWith(fold(f.folder.replace(/[\\/]$/, '') + '\\')));
    const out = [];
    for (const e of all) {
      if (!pass(e)) continue;
      if (!ts.length) { out.push({ e, m: MatchKind.NAME, score: 0 }); continue; }
      let inName = 0; let ok = true;
      for (const t of ts) {
        if (e.fName.includes(t)) inName++;
        else if (!e.fDir.includes(t)) { ok = false; break; }
      }
      if (ok) out.push({ e, m: inName === ts.length ? MatchKind.NAME : MatchKind.PATH, score: inName === ts.length ? 0 : 1 });
    }
    if (!out.length && ts.length) {
      for (const e of all) {
        if (!pass(e)) continue;
        const words = e.fName.split(/[^\p{L}\p{N}]+/u);
        if (ts.every((t) => t.length >= 4 && words.some((w) => withinOneEdit(t, w)))) out.push({ e, m: MatchKind.FUZZY, score: 2 });
      }
    }
    if (ts.length) out.sort((a, b) => a.score - b.score || (b.e.isDir - a.e.isDir) || a.e.name.length - b.e.name.length);
    else out.sort((a, b) => b.e.modified - a.e.modified);
    return out;
  }

  return {
    isFake: true,

    async listRoots() { await sleep(30); return roots.map(rootView); },

    async pickFolder() {
      await sleep(250);
      const seq = ['C:\\Users\\Carmen\\Pictures', 'C:\\Users\\Carmen\\Documents\\Banco', 'C:\\Users', '\\\\NAS-CASA\\Fotos', null];
      return seq[picks++ % seq.length];
    },

    async addRoot(path) {
      await sleep(120);
      const norm = (p) => fold(p).replace(/\/+$/, '');
      const clash = roots.find((r) => norm(path) === norm(r.path) || norm(path).startsWith(norm(r.path) + '/') || norm(r.path).startsWith(norm(path) + '/'));
      if (clash) throw { code: ErrorCode.OVERLAPPING_ROOT, message: `overlaps an indexed folder: ${clash.path}` };
      const drive = path.startsWith('\\\\') ? DriveKind.NETWORK : DriveKind.LOCAL;
      const r = { id: 100 + roots.length, path, status: RootStatus.PENDING, files: 0, dirs: 0, lastFullScan: null, drive };
      roots.push(r); done.set(r.id, 0); pending.set(r.id, 40);
      return rootView(r);
    },

    async removeRoot(id) {
      const r = roots.find((x) => x.id === id);
      if (!r) throw { code: ErrorCode.NOT_FOUND, message: `not found: ${id}` };
      r.status = RootStatus.REMOVING;
      generation++;
      setTimeout(() => { roots = roots.filter((x) => x !== r); generation++; for (const cb of changedCbs) cb(); }, 1500);
    },

    async search(q) {
      const text = q.text || '';
      const f = q.filters || {};
      await sleep(slow ? R.int(400, 1200) : R.int(30, 220));
      if (fold(text).includes('fallo')) throw { code: ErrorCode.DB, message: 'database is locked' };
      if (fold(text).includes('tardar')) {
        await sleep(900);
        return { hits: [], total: 0, truncated: true, truncatedBy: TruncatedBy.DEADLINE, elapsedMs: 1000 };
      }
      const key = JSON.stringify([text, f, generation]);
      if (!cache || cache.key !== key) cache = { key, list: groupCopies(rank(text, f)) };
      const list = cache.list;
      const capped = list.length > 2000;
      const total = Math.min(list.length, 2000);
      const offset = q.offset || 0;
      const limit = Math.max(1, Math.min(500, q.limit || 100));
      const hits = list.slice(offset, Math.min(total, offset + limit)).map(({ e, m, copies }) => ({
        id: e.id, name: e.name, dir: e.dir, kind: e.kind, size: e.size, modified: e.modified, matched: m, isDir: e.isDir,
        ...(copies.length ? { copies: copies.map((c) => ({ id: c.id, dir: c.dir, modified: c.modified })) } : {}),
      }));
      return { hits, total, truncated: capped, truncatedBy: capped ? TruncatedBy.CAP : null, elapsedMs: R.int(8, 90) };
    },

    async reveal(id, isDir) {
      await sleep(80);
      const e = (isDir ? corpus.dirs : corpus.files).find((x) => x.id === id);
      if (!e) throw { code: ErrorCode.NOT_FOUND, message: `not found: ${id}` };
      const r = roots.find((x) => x.id === e.rootId);
      if (r?.status === RootStatus.UNAVAILABLE) throw { code: ErrorCode.UNAVAILABLE, message: `folder unavailable: ${r.path}` };
      if (e.name.includes('borrado')) throw { code: ErrorCode.NOT_FOUND, message: `not found: ${e.name}` };
      return (e.dir.length + e.name.length + 1 > 259) ? RevealOutcome.OPENED_PARENT : RevealOutcome.REVEALED;
    },

    async searchSources(q, options = {}) {
      const sources = options.sources || [];
      const inside = options.bookmarkFolder ?? null;
      const files = inside == null && (sources.length === 0 || sources.includes(Source.FILES));
      const marks = bookmarks.enabled && (sources.length === 0 || sources.includes(Source.BOOKMARKS));
      const f = q.filters || {};
      const blocked = (f.kinds || []).length || f.sizeMin != null || f.sizeMax != null || f.folder;
      const ts = fold(q.text || '').split(/\s+/).filter(Boolean);
      const pool = BOOKMARKS.filter((b) => inside == null || b.parent === inside);
      const found = marks && !blocked && (ts.length || inside != null)
        ? pool.filter((b) => ts.every((t) => fold(`${b.title} ${b.url} ${b.folder.join(' ')}`).includes(t)))
          .map(({ parent, ...b }) => ({ ...b, source: Source.BOOKMARKS, matched: MatchKind.NAME }))
        : [];
      const offset = q.offset || 0;
      const limit = q.limit || 100;
      if (!files) {
        await sleep(30);
        return { hits: found.slice(offset, offset + limit), total: found.length, truncated: false, truncatedBy: null, elapsedMs: 1 };
      }
      const r = await this.search({ ...q, offset: Math.max(0, offset - found.length) });
      const fileHits = r.hits.map((h) => ({ ...h, source: Source.FILES }));
      // Bookmarks first, then files: enough for looking at the screen.
      const head = offset < found.length ? found.slice(offset) : [];
      return { ...r, hits: [...head, ...fileHits].slice(0, limit), total: r.total + found.length };
    },
    async getBookmarksStatus() { await sleep(20); return { ...bookmarks }; },
    async setBookmarksEnabled(on) {
      bookmarks.enabled = on;
      bookmarks.state = ReadState.NEVER;
      bookmarks.count = 0;
      if (on) {
        setTimeout(() => {
          Object.assign(bookmarks, {
            state: ReadState.OK, count: BOOKMARKS.length - 1, profiles: 1, lastRead: Math.floor(Date.now() / 1000),
            roots: [{ id: 1001, root: BookmarkRoot.MENU, profile: null }, { id: 1003, root: BookmarkRoot.UNFILED, profile: null }],
          });
          for (const cb of bookmarkCbs) cb({ ...bookmarks });
        }, 1000);
      }
      return { ...bookmarks };
    },
    async openBookmark(id) {
      const b = BOOKMARKS.find((x) => x.id === id);
      if (!b) throw { code: ErrorCode.NOT_FOUND, message: `bookmark ${id}` };
      if (!b.openable) throw { code: ErrorCode.UNAVAILABLE, message: 'not a web address' };
    },
    async onBookmarksChanged(cb) { bookmarkCbs.add(cb); return () => bookmarkCbs.delete(cb); },

    // VFS-21: `?update` offers 0.9.0; `?update=fail` makes the download fail halfway.
    async getUpdateSettings() { return { ...updateSettings }; },
    async setUpdateCheck(enabled) { updateSettings.checkOnOpen = enabled; return { ...updateSettings }; },
    async checkForUpdate() {
      await sleep(50);
      return updateSettings.checkOnOpen && params.has('update') ? { version: '0.9.0' } : null;
    },
    async installUpdate() {
      const total = 8_000_000;
      for (let downloaded = 0; downloaded <= total; downloaded += 400_000) {
        await sleep(60);
        if (params.get('update') === 'fail' && downloaded > total / 2) throw { code: ErrorCode.IO, message: 'connection reset' };
        for (const cb of updateCbs) cb({ downloaded, total });
      }
      await new Promise(() => {}); // the real app relaunches here
    },
    async onUpdateProgress(cb) { updateCbs.add(cb); return () => updateCbs.delete(cb); },

    async getProgress() { await sleep(20); return progress(); },
    async rescan(id) {
      for (const r of roots) if ((id == null || r.id === id) && r.status === RootStatus.READY) { r.status = RootStatus.SCANNING; pending.set(r.id, 30); }
    },
    async setPaused(p) { paused = p; },
    async onProgress(cb) { progressCbs.add(cb); return () => progressCbs.delete(cb); },
    async onIndexChanged(cb) { changedCbs.add(cb); return () => changedCbs.delete(cb); },
  };
}
