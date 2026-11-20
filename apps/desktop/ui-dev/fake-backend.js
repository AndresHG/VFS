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
// opened_parent. Each "Añadir carpeta…" walks a fixed list of picks: a new folder, a folder
// inside an existing one, a parent of an existing one, a network share, then cancel.

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

const fold = (s) => s.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase().replace(/\\/g, '/');

const people = ['Carmen', 'Lucía', 'Andrés', 'Concepción', 'José Luis', 'Mª Ángeles', 'Begoña', 'Iñaki', 'Pilar', 'Ramón'];
const places = ['Galicia', 'Cuenca', 'Benidorm', 'Peñíscola', 'Salamanca', 'Albacete', 'Cádiz', 'Logroño', 'Ávila', 'Málaga'];
const events = ['Boda de Lucía y Andrés', 'Navidad en casa de la abuela', 'Viaje a', 'Comunión de Iñaki', 'Cumpleaños de Pilar', 'Verano en', 'Fiestas del pueblo', 'Excursión a', 'Bautizo de Martín'];
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
  const firstScan = scenario === 'first-scan';
  return [
    { id: 1, path: 'C:\\Users\\Carmen\\Documents', status: firstScan ? 'scanning' : 'ready', files: 0, dirs: 0, lastFullScan: firstScan ? null : NOW - 3 * DAY, weight: 0.3 },
    { id: 2, path: 'D:\\Archivo familiar', status: 'scanning', files: 0, dirs: 0, lastFullScan: null, weight: 0.6 },
    { id: 3, path: 'E:\\Copias antiguas', status: firstScan ? 'scanning' : 'unavailable', files: 0, dirs: 0, lastFullScan: firstScan ? null : NOW - 400 * DAY, weight: 0.1, drive: 'removable' },
  ];
}

function makeEntry(root, R) {
  const y = R.int(1998, 2026);
  const t = Math.min(NOW - R.int(0, 3) * DAY, Math.floor(Date.UTC(y, R.int(0, 11), R.int(1, 28)) / 1000) + R.int(0, DAY));
  if (root.id === 1) {
    const sub = R.pick(['Banco', 'Casa', `Hacienda\\${y}`, 'Médico', 'Seguros', 'Recetas', 'Coche', `Facturas\\${y}\\${R.pick(months)}`, 'Varios']);
    const base = R.pick(docs);
    const name = `${base}${R.rand() < 0.5 ? ` ${R.pick(months)} ${y}` : ` ${y}`}.${R.pick(docExt)}`;
    return { dir: `${root.path}\\${sub}`, name, kind: 'document', size: R.int(8_000, 4_000_000), modified: t };
  }
  const r = R.rand();
  if (r < 0.62) {
    const ev = R.pick(events);
    const evName = ev.endsWith(' a') || ev.endsWith(' en') ? `${ev} ${R.pick(places)}` : ev;
    const deep = R.rand() < 0.15
      ? `\\Fotos escaneadas del álbum de la abuela Concepción\\Caja ${R.int(1, 9)} - Años sesenta y setenta\\${evName} con los primos de ${R.pick(places)}\\Sobre ${R.int(1, 40)} sin clasificar todavía\\Revisar con mamá antes de tirar nada de lo que hay aquí dentro, por favor`
      : `\\Fotos\\${y}\\${evName}`;
    const ext = R.pick(['JPG', 'JPG', 'jpg', 'jpeg', 'png', 'HEIC']);
    const name = R.rand() < 0.6 ? `IMG_${String(R.int(1, 9999)).padStart(4, '0')}.${ext}` : `${evName} ${R.int(1, 300)}.${ext}`;
    return { dir: root.path + deep, name, kind: 'photo', size: R.int(400_000, 9_000_000), modified: t };
  }
  if (r < 0.75) {
    const ev = R.pick(events);
    return { dir: `${root.path}\\Vídeos\\${y}`, name: `${ev} ${R.pick(places)}.${R.pick(['mp4', 'MOV', 'avi'])}`, kind: 'video', size: R.int(20_000_000, 4_500_000_000), modified: t };
  }
  if (r < 0.9) {
    const singer = R.pick(singers);
    return { dir: `${root.path}\\Música\\${singer}`, name: `${String(R.int(1, 14)).padStart(2, '0')} - ${R.pick(songs)}.mp3`, kind: 'audio', size: R.int(2_000_000, 12_000_000), modified: t };
  }
  if (r < 0.97) {
    return { dir: `${root.path}\\Documentos de ${R.pick(people)}`, name: `${R.pick(docs)} ${y}.${R.pick(docExt)}`, kind: 'document', size: R.int(8_000, 3_000_000), modified: t };
  }
  return { dir: `${root.path}\\Varios`, name: `${R.pick(['copia', 'backup', 'programa', 'datos'])}_${R.int(1, 999)}.${R.pick(['zip', 'exe', 'dat', 'iso'])}`, kind: 'other', size: R.int(1_000, 900_000_000), modified: t };
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
    // Register every folder between the root and the file as a folder entry (decision D1).
    let d = e.dir;
    while (d.length > root.path.length && !dirs.has(d)) {
      const cut = d.lastIndexOf('\\');
      const parent = d.slice(0, cut);
      const name = d.slice(cut + 1);
      dirs.set(d, { id: dirs.size + 1, rootId: root.id, seq, isDir: true, dir: parent, name, kind: 'folder', size: 0, modified: e.modified, fName: fold(name), fDir: fold(parent) });
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
  const scenario = params.get('scenario') || '';
  const slow = params.has('slow');
  let roots = scenario === 'empty' ? [] : makeRoots(scenario);
  const R = makeRandom(20261120);
  const corpus = buildCorpus(makeRoots(''), R);
  const all = [...corpus.files, ...corpus.dirs];
  const totalOf = (id) => corpus.counts.get(id) || 0;

  // Scan simulation: how many entries of each root are visible so far.
  const done = new Map();
  const pending = new Map();
  for (const r of roots) {
    const total = totalOf(r.id);
    done.set(r.id, r.status === 'scanning' ? Math.floor(total * (scenario === 'first-scan' ? 0.05 : 0.35)) : total);
    pending.set(r.id, r.status === 'scanning' ? 900 : 0);
  }
  let paused = false;
  const progressCbs = new Set();
  const changedCbs = new Set();
  let lastChanged = 0;
  let picks = 0;
  let generation = 0;
  let cache = null;

  const visible = (e) => {
    const r = roots.find((x) => x.id === e.rootId);
    return r && r.status !== 'removing' && e.seq < done.get(r.id);
  };

  function rootView(r) {
    const n = done.get(r.id) || 0;
    return { id: r.id, path: r.path, status: r.status, files: n, dirs: Math.round(n / 22), lastFullScan: r.lastFullScan, drive: r.drive ?? 'local' };
  }

  function progress() {
    const scanning = roots.some((r) => r.status === 'scanning' || r.status === 'pending');
    return {
      state: paused && scanning ? 'paused' : scanning ? 'scanning' : 'idle',
      filesIndexed: roots.reduce((s, r) => s + (done.get(r.id) || 0), 0),
      roots: roots.map((r) => ({
        rootId: r.id, path: r.path, status: r.status,
        dirsDone: Math.round((done.get(r.id) || 0) / 22), dirsPending: pending.get(r.id) || 0,
        filesIndexed: done.get(r.id) || 0,
        currentDir: r.status === 'scanning' ? `${r.path}\\Fotos\\2004` : null,
      })),
    };
  }

  setInterval(() => {
    let changed = false;
    if (!paused) {
      for (const r of roots) {
        if (r.status === 'pending') { r.status = 'scanning'; changed = true; }
        if (r.status !== 'scanning') continue;
        const total = totalOf(r.id) || 4000;
        // Root 2 crawls slowly so the default page keeps showing a scan in progress.
        const step = r.id === 2 && scenario !== 'first-scan' ? 40 : 160;
        done.set(r.id, Math.min(total, (done.get(r.id) || 0) + step));
        pending.set(r.id, Math.max(0, (pending.get(r.id) || 0) - (step > 100 ? 3 : 1)));
        if (done.get(r.id) >= total) {
          r.status = 'ready'; r.lastFullScan = Math.floor(Date.now() / 1000); pending.set(r.id, 0);
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
      if (!ts.length) { out.push({ e, m: 'name', score: 0 }); continue; }
      let inName = 0; let ok = true;
      for (const t of ts) {
        if (e.fName.includes(t)) inName++;
        else if (!e.fDir.includes(t)) { ok = false; break; }
      }
      if (ok) out.push({ e, m: inName === ts.length ? 'name' : 'path', score: inName === ts.length ? 0 : 1 });
    }
    if (!out.length && ts.length) {
      for (const e of all) {
        if (!pass(e)) continue;
        const words = e.fName.split(/[^\p{L}\p{N}]+/u);
        if (ts.every((t) => t.length >= 4 && words.some((w) => withinOneEdit(t, w)))) out.push({ e, m: 'fuzzy', score: 2 });
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
      if (clash) throw { code: 'overlapping_root', message: `overlaps an indexed folder: ${clash.path}` };
      const drive = path.startsWith('\\\\') ? 'network' : 'local';
      const r = { id: 100 + roots.length, path, status: 'pending', files: 0, dirs: 0, lastFullScan: null, drive };
      roots.push(r); done.set(r.id, 0); pending.set(r.id, 40);
      return rootView(r);
    },

    async removeRoot(id) {
      const r = roots.find((x) => x.id === id);
      if (!r) throw { code: 'not_found', message: `not found: ${id}` };
      r.status = 'removing';
      generation++;
      setTimeout(() => { roots = roots.filter((x) => x !== r); generation++; for (const cb of changedCbs) cb(); }, 1500);
    },

    async search(q) {
      const text = q.text || '';
      const f = q.filters || {};
      await sleep(slow ? R.int(400, 1200) : R.int(30, 220));
      if (fold(text).includes('fallo')) throw { code: 'db', message: 'database is locked' };
      if (fold(text).includes('tardar')) {
        await sleep(900);
        return { hits: [], total: 0, truncated: true, truncatedBy: 'deadline', elapsedMs: 1000 };
      }
      const key = JSON.stringify([text, f, generation]);
      if (!cache || cache.key !== key) cache = { key, list: rank(text, f) };
      const list = cache.list;
      const capped = list.length > 2000;
      const total = Math.min(list.length, 2000);
      const offset = q.offset || 0;
      const limit = Math.max(1, Math.min(500, q.limit || 100));
      const hits = list.slice(offset, Math.min(total, offset + limit)).map(({ e, m }) => ({
        id: e.id, name: e.name, dir: e.dir, kind: e.kind, size: e.size, modified: e.modified, matched: m, isDir: e.isDir,
      }));
      return { hits, total, truncated: capped, truncatedBy: capped ? 'cap' : null, elapsedMs: R.int(8, 90) };
    },

    async reveal(id, isDir) {
      await sleep(80);
      const e = (isDir ? corpus.dirs : corpus.files).find((x) => x.id === id);
      if (!e) throw { code: 'not_found', message: `not found: ${id}` };
      const r = roots.find((x) => x.id === e.rootId);
      if (r?.status === 'unavailable') throw { code: 'unavailable', message: `folder unavailable: ${r.path}` };
      if (e.name.includes('borrado')) throw { code: 'not_found', message: `not found: ${e.name}` };
      return (e.dir.length + e.name.length + 1 > 259) ? 'opened_parent' : 'revealed';
    },

    async getProgress() { await sleep(20); return progress(); },
    async rescan(id) {
      for (const r of roots) if ((id == null || r.id === id) && r.status === 'ready') { r.status = 'scanning'; pending.set(r.id, 30); }
    },
    async setPaused(p) { paused = p; },
    async onProgress(cb) { progressCbs.add(cb); return () => progressCbs.delete(cb); },
    async onIndexChanged(cb) { changedCbs.add(cb); return () => changedCbs.delete(cb); },
  };
}
