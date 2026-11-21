// Dates and numbers in the active language (VFS-22 R4), and the matching helpers the row
// renderer needs. `setFormatLocale` (called by strings.js `setLanguage`) picks the language;
// Spanish until then.

const GROUP = { es: '.', en: ',' };
let group = GROUP.es;
let dec1;
let dateFmt;
let timeFmt;
let dayFmt;

/** Makes `lang` ('es' | 'en') and its `Intl` locale `intl` the formatting language. */
export function setFormatLocale(lang, intl) {
  group = GROUP[lang] ?? GROUP.es;
  dec1 = new Intl.NumberFormat(intl, { maximumFractionDigits: 1 });
  dateFmt = new Intl.DateTimeFormat(intl, { day: 'numeric', month: 'short', year: 'numeric' });
  timeFmt = new Intl.DateTimeFormat(intl, { hour: 'numeric', minute: '2-digit' });
  dayFmt = new Intl.DateTimeFormat(intl, { day: 'numeric', month: 'short' });
}
setFormatLocale('es', 'es-ES');

/** 1234567 -> "1.234.567" (es) or "1,234,567" (en). Manual so 4-digit numbers group too
 *  (es-ES Intl prints "1234"). */
export function count(n) {
  return String(Math.trunc(n || 0)).replace(/\B(?=(\d{3})+(?!\d))/g, group);
}

const UNITS = ['KB', 'MB', 'GB', 'TB'];

/** Bytes -> "1,2 MB" (es) or "1.2 MB" (en) in binary units, the ones Windows Explorer shows.
 *  "byte" is the same word in both languages. */
export function size(bytes) {
  if (bytes < 1024) return bytes === 1 ? '1 byte' : `${count(bytes)} bytes`;
  let v = bytes / 1024;
  let u = 0;
  while (v >= 1024 && u < UNITS.length - 1) { v /= 1024; u++; }
  return `${v < 10 ? dec1.format(v) : count(Math.round(v))} ${UNITS[u]}`;
}

/** Spanish short months carry a dot ("mar."); the UI drops it. */
const noDot = (s) => s.replace('.', '');

/** Unix seconds -> "12 mar 2021" (es) or "Mar 12, 2021" (en-US). */
export function date(secs) {
  return secs ? noDot(dateFmt.format(new Date(secs * 1000))) : '';
}

/** Date -> "9:14" (es) or "9:14 AM" (en-US). */
export function time(d) {
  return timeFmt.format(d);
}

/** Date -> "12 mar" (es) or "Mar 12" (en-US). */
export function dayMonth(d) {
  return noDot(dayFmt.format(d));
}

/**
 * Case- and accent-folds `s` one source character at a time so every folded position maps
 * back to its source index. Mirrors the backend's folding closely enough for highlighting.
 */
function foldWithMap(s) {
  let out = '';
  const map = [];
  for (let i = 0; i < s.length; i++) {
    const f = s[i].normalize('NFD').replace(/\p{M}/gu, '').toLowerCase().replace(/\\/g, '/');
    for (let k = 0; k < f.length; k++) map.push(i);
    out += f;
  }
  map.push(s.length);
  return { out, map };
}

export function fold(s) {
  return foldWithMap(s).out;
}

/** Query text -> folded terms worth highlighting (2+ chars). */
export function terms(text) {
  return fold(text).split(/\s+/).filter((t) => t.length >= 2);
}

/** Merged [start, end) source ranges of `s` where any term occurs. */
export function matchRanges(s, ts) {
  if (!ts.length || !s) return [];
  const { out, map } = foldWithMap(s);
  const ranges = [];
  for (const t of ts) {
    let at = out.indexOf(t);
    while (at !== -1) {
      ranges.push([map[at], map[at + t.length - 1] + 1]);
      at = out.indexOf(t, at + t.length);
    }
  }
  ranges.sort((a, b) => a[0] - b[0]);
  const merged = [];
  for (const r of ranges) {
    const last = merged[merged.length - 1];
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]);
    else merged.push([...r]);
  }
  return merged;
}

/** Appends `s[from, to)` to `el` as text with <mark> around the parts in `ranges`. */
export function appendHighlighted(el, s, ranges, from = 0, to = s.length) {
  let pos = from;
  for (const [a0, b0] of ranges) {
    const a = Math.max(a0, from);
    const b = Math.min(b0, to);
    if (b <= a) continue;
    if (a > pos) el.append(s.slice(pos, a));
    const m = document.createElement('mark');
    m.textContent = s.slice(a, b);
    el.append(m);
    pos = b;
  }
  if (pos < to) el.append(s.slice(pos, to));
}

/** Index where the last path segment of `dir` starts (after the last / or \). */
export function lastSegmentStart(dir) {
  const trimmed = dir.replace(/[\\/]+$/, '');
  const i = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return i < 0 ? 0 : i + 1;
}

export function joinPath(dir, name) {
  const sep = dir.includes('\\') ? '\\' : '/';
  return /[\\/]$/.test(dir) ? dir + name : dir + sep + name;
}

export function baseName(p) {
  const trimmed = p.replace(/[\\/]+$/, '');
  return trimmed.slice(lastSegmentStart(trimmed)) || p;
}

/** Windows-style paths compare case-insensitively; both separators are equivalent. */
function norm(p) {
  const n = p.replace(/\\/g, '/').replace(/\/+$/, '');
  return /^[a-z]:|^\/\//i.test(n) ? n.toLowerCase() : n;
}

/** `child` is `parent` or anywhere below it. */
export function isUnder(child, parent) {
  const c = norm(child);
  const p = norm(parent);
  return c === p || c.startsWith(p + '/');
}
