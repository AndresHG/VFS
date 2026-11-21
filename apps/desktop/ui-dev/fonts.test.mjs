// The bundled typeface's digits (issue 111). Run: node --test apps/desktop/ui-dev/fonts.test.mjs
// Reads the TrueType tables directly: table directory, cmap, head, hhea, hmtx, loca, glyf.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const FONTS = ['atkinson-400.ttf', 'atkinson-700.ttf'];

function parse(file) {
  const b = readFileSync(new URL(`../ui/fonts/${file}`, import.meta.url));
  const tables = {};
  for (let i = 0, n = b.readUInt16BE(4); i < n; i++) {
    const r = 12 + 16 * i;
    tables[b.toString('latin1', r, r + 4)] = b.readUInt32BE(r + 8);
  }
  const glyphOf = (cp) => {
    const cmap = tables.cmap;
    for (let i = 0, n = b.readUInt16BE(cmap + 2); i < n; i++) {
      const sub = cmap + b.readUInt32BE(cmap + 4 + 8 * i + 4);
      if (b.readUInt16BE(sub) !== 4) continue;
      const segs = b.readUInt16BE(sub + 6) / 2;
      const ends = sub + 14, starts = ends + 2 * segs + 2, deltas = starts + 2 * segs, ranges = deltas + 2 * segs;
      for (let s = 0; s < segs; s++) {
        const start = b.readUInt16BE(starts + 2 * s);
        if (cp < start || cp > b.readUInt16BE(ends + 2 * s)) continue;
        const delta = b.readInt16BE(deltas + 2 * s), range = b.readUInt16BE(ranges + 2 * s);
        if (range === 0) return (cp + delta) & 0xffff;
        const g = b.readUInt16BE(ranges + 2 * s + range + 2 * (cp - start));
        return g === 0 ? 0 : (g + delta) & 0xffff;
      }
    }
    throw new Error(`U+${cp.toString(16)} not in ${file}`);
  };
  const longLoca = b.readInt16BE(tables.head + 50) === 1;
  const glyphOffset = (g) => tables.glyf + (longLoca ? b.readUInt32BE(tables.loca + 4 * g) : 2 * b.readUInt16BE(tables.loca + 2 * g));
  const metrics = b.readUInt16BE(tables.hhea + 34);
  return {
    contours: (ch) => b.readInt16BE(glyphOffset(glyphOf(ch.codePointAt(0)))),
    advance: (ch) => b.readUInt16BE(tables.hmtx + 4 * Math.min(glyphOf(ch.codePointAt(0)), metrics - 1)),
  };
}

/** Issue 111: the zero is a plain oval (one ring, one counter), not slashed like "Ø". */
test('the zero has no slash in either weight', () => {
  for (const file of FONTS) {
    assert.equal(parse(file).contours('0'), 2, `${file}: the slash splits the counter in two`);
  }
});

/** Issue 111: without the slash, the zero stays narrower than the letter O, so the two differ. */
test('the zero is narrower than the letter O', () => {
  for (const file of FONTS) {
    const f = parse(file);
    assert.ok(f.advance('0') < f.advance('O'), `${file}: 0 ${f.advance('0')} vs O ${f.advance('O')}`);
  }
});
