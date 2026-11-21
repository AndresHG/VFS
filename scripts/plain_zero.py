#!/usr/bin/env python3
"""Rebuilds the bundled Atkinson Hyperlegible with a plain zero (issue 111).

Atkinson's zero carries a slash ("2Ø19") that an older reader can take for a letter, and the
face has no alternate zero. This replaces the two half-counters the slash splits with one oval
counter, the outer ring scaled into the same bounds and wound the other way, so stroke and width
are unchanged; the tabular zero is a composite of it and follows. Everything else is the
original font, released under the SIL OFL 1.1 (no Reserved Font Name), now as TrueType so tests
can read the glyph table with no decompressor.

Run by hand, with fontTools, on the original files (the .woff2 pair in git history before
issue 111): `python3 scripts/plain_zero.py <regular> <bold>`; it writes
apps/desktop/ui/fonts/atkinson-{400,700}.ttf.
"""

import os
import sys

from fontTools.ttLib import TTFont
from fontTools.ttLib.tables import ttProgram
from fontTools.ttLib.tables._g_l_y_f import GlyphCoordinates

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "apps", "desktop", "ui", "fonts")
NOTE = "; plain zero for VFS (issue 111)"


def contours(glyph):
    start = 0
    for end in glyph.endPtsOfContours:
        yield start, end + 1
        start = end + 1


def plain_zero(font):
    glyf = font["glyf"]
    zero = glyf["zero"]
    if zero.numberOfContours != 3:
        raise SystemExit(f"expected the slashed zero (3 contours), found {zero.numberOfContours}")
    coords, flags = zero.coordinates, zero.flags
    (o0, o1), *counters = list(contours(zero))
    inner = [coords[i] for a, b in counters for i in range(a, b)]
    ix0, iy0 = min(p[0] for p in inner), min(p[1] for p in inner)
    ix1, iy1 = max(p[0] for p in inner), max(p[1] for p in inner)
    outer = [coords[i] for i in range(o0, o1)]
    ox0, oy0 = min(p[0] for p in outer), min(p[1] for p in outer)
    ox1, oy1 = max(p[0] for p in outer), max(p[1] for p in outer)
    sx, sy = (ix1 - ix0) / (ox1 - ox0), (iy1 - iy0) / (oy1 - oy0)
    counter = [(round(ix0 + (x - ox0) * sx), round(iy0 + (y - oy0) * sy)) for x, y in reversed(outer)]
    counter_flags = [flags[i] for i in reversed(range(o0, o1))]
    zero.coordinates = GlyphCoordinates(outer + counter)
    zero.flags = bytearray(flags[o0:o1]) + bytearray(counter_flags)
    zero.endPtsOfContours = [len(outer) - 1, 2 * len(outer) - 1]
    zero.numberOfContours = 2
    # The autohinter's program addressed the old points.
    zero.program = ttProgram.Program()
    zero.program.fromBytecode(b"")
    zero.recalcBounds(glyf)
    name = font["name"]
    for rec in name.names:
        if rec.nameID == 5 and NOTE not in rec.toUnicode():
            rec.string = rec.toUnicode() + NOTE


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    for src, weight in zip(sys.argv[1:], ("400", "700")):
        font = TTFont(src)
        font.flavor = None
        plain_zero(font)
        font.save(os.path.join(OUT, f"atkinson-{weight}.ttf"))


if __name__ == "__main__":
    main()
