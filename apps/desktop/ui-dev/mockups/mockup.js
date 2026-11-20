// DEVELOPMENT ONLY. Fills a static mockup with sample rows so the three directions show the
// same data. `?state=idle|results|scan` picks the screen state. No backend, no behaviour.

const ROWS = [
  { kind: 'folder', name: 'Boda de Lucía y Andrés', dir: 'D:\\Archivo familiar\\Fotos\\2025', date: '8 nov 2025', size: '' },
  { kind: 'photo', name: 'Boda de Lucía y Andrés 112.JPG', dir: 'D:\\Archivo familiar\\Fotos\\2025\\Boda de Lucía y Andrés', date: '8 nov 2025', size: '4,1 MB' },
  { kind: 'video', name: 'Boda de Lucía y Andrés Cádiz.mp4', dir: 'D:\\Archivo familiar\\Vídeos\\2025', date: '9 nov 2025', size: '1,8 GB' },
  { kind: 'folder', name: 'Boda de Lucía y Andrés', dir: 'E:\\Copias antiguas\\Fotos\\2009', date: '22 may 2009', size: '', unavailable: true },
  { kind: 'document', name: 'Presupuesto boda restaurante.pdf', dir: 'C:\\Users\\Carmen\\Documents\\Casa', date: '3 feb 2025', size: '312 KB' },
  { kind: 'photo', name: 'Boda de Lucía y Andrés 7.HEIC', dir: 'D:\\Archivo familiar\\Fotos\\2025\\Boda de Lucía y Andrés', date: '8 nov 2025', size: '2,6 MB' },
  { kind: 'audio', name: '03 - Un año de amor (boda).mp3', dir: 'D:\\Archivo familiar\\Música\\Mocedades', date: '14 jun 2019', size: '7,2 MB' },
  { kind: 'photo', name: 'Boda de Lucía y Andrés 41.jpg', dir: 'D:\\Archivo familiar\\Fotos\\2025\\Boda de Lucía y Andrés', date: '8 nov 2025', size: '3,9 MB' },
  { kind: 'document', name: 'Lista de invitados boda.xlsx', dir: 'C:\\Users\\Carmen\\Documents\\Varios', date: '12 ene 2025', size: '48 KB' },
  { kind: 'photo', name: 'Boda de Lucía y Andrés 230.JPG', dir: 'D:\\Archivo familiar\\Fotos\\2025\\Boda de Lucía y Andrés', date: '8 nov 2025', size: '5,0 MB' },
  { kind: 'folder', name: 'Boda de la prima Begoña', dir: 'D:\\Archivo familiar\\Fotos\\1998', date: '30 ago 1998', size: '' },
  { kind: 'photo', name: 'IMG_0042.JPG', dir: 'D:\\Archivo familiar\\Fotos\\2012\\Boda de Iñaki', date: '6 oct 2012', size: '2,2 MB' },
];
const KIND = { document: 'Documento', photo: 'Foto', video: 'Vídeo', audio: 'Audio', folder: 'Carpeta', other: 'Otro' };
const QUERY = 'boda';

function highlight(el, text) {
  const i = text.toLowerCase().indexOf(QUERY);
  if (i < 0) { el.textContent = text; return; }
  const m = document.createElement('mark');
  m.textContent = text.slice(i, i + QUERY.length);
  el.replaceChildren(text.slice(0, i), m, text.slice(i + QUERY.length));
}

const state = new URLSearchParams(location.search).get('state') || 'results';
document.documentElement.dataset.state = state;

const tpl = document.getElementById('row');
const list = document.getElementById('rows');
if (tpl && list) {
  ROWS.forEach((r, i) => {
    const row = tpl.content.firstElementChild.cloneNode(true);
    row.id = `r${i}`;
    if (i === 1) row.setAttribute('aria-selected', 'true');
    row.querySelector('.ico').dataset.kind = r.kind;
    row.querySelector('.ico use').setAttribute('href', `icons.svg#i-${r.kind}`);
    highlight(row.querySelector('[data-f="name"]'), r.name);
    row.querySelector('[data-f="dir"]').textContent = r.dir;
    for (const f of ['date', 'size', 'kind']) {
      const el = row.querySelector(`[data-f="${f}"]`);
      if (el) el.textContent = f === 'kind' ? KIND[r.kind] : r[f];
    }
    const badge = row.querySelector('.badge');
    if (badge) badge.hidden = !r.unavailable;
    if (r.unavailable) row.classList.add('unavailable');
    list.append(row);
  });
}

if (state === 'idle') for (const el of document.querySelectorAll('input[type="search"]')) el.value = '';
