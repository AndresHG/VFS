#!/usr/bin/env python3
"""Generate a Spanish document corpus for the VFS-2 content-search measurements.

Writes documents plus `truth.jsonl` (one line per document: path, format, the exact text
written, and probe words) so extractors can be scored. Deterministic for a given seed.

    make_corpus.py eval   --out DIR             # ~40 files per format + hostile set
    make_corpus.py bulk   --out DIR --docs N    # realistic format mix for scale runs
    make_corpus.py real   --out DIR --src BOE   # append downloaded BOE PDFs (see the doc)
    make_corpus.py fixtures --out crates/vfs-content/tests/fixtures

Needs: python-docx, openpyxl, python-pptx, odfpy, fpdf2, pillow, msoffcrypto-tool;
LibreOffice (`soffice`) for DOC/XLS/PPT and office-produced PDFs; `qpdf` for encrypted PDFs.
Dev-machine tool only (macOS); not used by the app.
"""

import argparse
import io
import json
import math
import os
import random
import shutil
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

# Arial covers every Spanish letter; Arial Unicode (22 MB) makes fpdf2 slow to subset.
FONT = "/System/Library/Fonts/Supplemental/Arial.ttf"

WORDS = """
contrato alquiler vivienda arrendador arrendatario fianza renta mensual cláusula
señor señora muñoz núñez peñalara españa año años niño niña montaña cigüeña pingüino
factura facturas recibo recibos pago pagos importe euros banco cuenta transferencia
notaría escritura compraventa hipoteca préstamo interés cuota amortización tasación
ayuntamiento licencia obra reforma cocina baño calefacción caldera electricidad agua
comunidad propietarios reunión acta junta presidente secretario administrador derrama
seguro póliza siniestro indemnización cobertura vehículo matrícula garaje trastero
médico análisis receta farmacia tratamiento informe diagnóstico consulta urgencias
colegio matrícula curso evaluación calificación profesor alumno excursión comedor
declaración renta hacienda impuesto retención deducción devolución borrador ejercicio
nómina salario empresa trabajador vacaciones despido indemnización convenio categoría
viaje billete hotel reserva vuelo maleta pasaporte visado itinerario excursión playa
receta tortilla patatas cebolla huevos aceite oliva sal pimienta horno sartén cazuela
jardín árbol césped riego piscina terraza ventana puerta pasillo salón dormitorio
herencia testamento heredero legado albacea partición inventario valoración bienes
el la los las un una de del al en con por para sobre entre hasta desde según sin
que como cuando donde porque aunque mientras también además después antes siempre
nunca pronto tarde aquí allí muy más menos bien mal mucho poco todo nada algo
está están estaba fueron será serán tiene tienen había hubo hace hizo dijo según
firmado acordado pagado recibido enviado revisado aprobado pendiente vencido anulado
primero segundo tercero último próximo anterior siguiente actual nuevo antiguo
lunes martes miércoles jueves viernes sábado domingo enero febrero marzo abril mayo
junio julio agosto septiembre octubre noviembre diciembre mañana tarde noche
""".split()

# Long words hyphenated at line ends in some PDFs; the extractor should rejoin them.
HYPHEN_WORDS = ["arrendamiento", "comunicación", "administración", "amortización",
                "indemnización", "declaración", "calefacción", "documentación"]

PSEUDO_SYL = ["za", "ro", "vel", "qui", "tra", "mon", "fel", "dú", "ñal", "bri", "cos", "len"]


def pseudo_word(rng):
    """A unique, searchable nonsense word (with an accent or ñ half the time)."""
    return "".join(rng.choice(PSEUDO_SYL) for _ in range(4))


def sentence(rng, n=None):
    n = n or rng.randint(6, 16)
    w = [rng.choice(WORDS) for _ in range(n)]
    w[0] = w[0].capitalize()
    return " ".join(w) + rng.choice([".", ".", ".", ";", ":"])


def paragraph(rng, words_target):
    out, count = [], 0
    while count < words_target:
        s = sentence(rng)
        out.append(s)
        count += len(s.split())
    return " ".join(out)


def doc_text(rng, n_words, marker):
    """Paragraphs totalling about n_words; the marker sentence goes in a random paragraph."""
    paras = []
    left = n_words
    while left > 0:
        k = min(left, rng.randint(40, 160))
        paras.append(paragraph(rng, k))
        left -= k
    marker_sentence = (f"Expediente {marker}: el señor Muñoz pagó la factura de la "
                       f"calefacción del año pasado en Peñalara.")
    paras.insert(rng.randrange(len(paras) + 1), marker_sentence)
    return paras


# ---------------------------------------------------------------- writers


def write_txt(path, paras, enc="utf-8"):
    path.write_text("\n\n".join(paras) + "\n", encoding=enc, errors="replace")


def write_docx(path, title, paras):
    import docx
    d = docx.Document()
    d.core_properties.title = title
    d.add_heading(title, level=1)
    for p in paras:
        d.add_paragraph(p)
    d.save(path)


def write_odt(path, title, paras):
    from odf.opendocument import OpenDocumentText
    from odf.text import H, P
    d = OpenDocumentText()
    d.text.addElement(H(outlinelevel=1, text=title))
    for p in paras:
        d.text.addElement(P(text=p))
    d.save(str(path))


def rtf_escape(s):
    out = []
    for ch in s:
        o = ord(ch)
        if ch in "\\{}":
            out.append("\\" + ch)
        elif o < 128:
            out.append(ch)
        else:
            # \uN with a '?' fallback, as Word writes it.
            out.append(f"\\u{o if o < 32768 else o - 65536}?")
    return "".join(out)


def write_rtf(path, title, paras):
    body = [r"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Times New Roman;}}",
            r"{\info{\title " + rtf_escape(title) + "}}", r"\f0\fs24"]
    body.append(r"{\b " + rtf_escape(title) + r"}\par")
    for p in paras:
        body.append(rtf_escape(p) + r"\par")
    body.append("}")
    path.write_text("\n".join(body), encoding="ascii")


def write_xlsx(path, title, paras, rng):
    import openpyxl
    wb = openpyxl.Workbook()
    ws = wb.active
    ws.title = "Gastos"
    ws.append(["Concepto", "Descripción", "Importe (€)", "Fecha"])
    for i, p in enumerate(paras):
        ws.append([f"Línea {i + 1}", p, round(rng.uniform(5, 2000), 2), f"2019-0{rng.randint(1, 9)}-1{rng.randint(0, 9)}"])
    wb.properties.title = title
    wb.save(path)


def write_pptx(path, title, paras):
    from pptx import Presentation
    from pptx.util import Inches
    prs = Presentation()
    s = prs.slides.add_slide(prs.slide_layouts[0])
    s.shapes.title.text = title
    for i in range(0, len(paras), 2):
        s = prs.slides.add_slide(prs.slide_layouts[1])
        s.shapes.title.text = f"Diapositiva {i // 2 + 1}"
        tf = s.placeholders[1].text_frame
        tf.text = paras[i]
        if i + 1 < len(paras):
            tf.add_paragraph().text = paras[i + 1]
    prs.save(path)


def wrap_hyphenated(words, width, hyphenate, rng):
    """Greedy line wrap; with `hyphenate`, a long word at a line end is split with '-'."""
    lines, cur = [], ""
    for w in words:
        cand = (cur + " " + w).strip()
        if len(cand) <= width:
            cur = cand
            continue
        if hyphenate and len(w) >= 9 and len(cur) + 6 < width:
            cut = rng.randint(4, len(w) - 4)
            lines.append(cur + " " + w[:cut] + "-")
            cur = w[cut:]
        else:
            lines.append(cur)
            cur = w
    if cur:
        lines.append(cur)
    return lines


def write_pdf(path, title, paras, rng, two_columns=False, hyphenate=False):
    from fpdf import FPDF
    pdf = FPDF(format="A4")
    pdf.set_auto_page_break(False)
    pdf.add_font("AU", "", FONT)
    pdf.set_title(title)
    pdf.add_page()
    pdf.set_font("AU", size=10)
    words = " ".join([title] + paras).split()
    if hyphenate:
        # Guarantee hyphenation probes exist: inject long words.
        for hw in HYPHEN_WORDS:
            words.insert(rng.randrange(len(words)), hw)
    width = 45 if two_columns else 95
    lines = wrap_hyphenated(words, width, hyphenate, rng)
    line_h, top, bottom = 4.6, 15, 282
    cols = [10, 108] if two_columns else [10]
    col, y = 0, top
    for ln in lines:
        if y > bottom:
            col += 1
            y = top
            if col >= len(cols):
                pdf.add_page()
                col = 0
        pdf.text(cols[col], y, ln)
        y += line_h
    pdf.output(str(path))
    return words


def write_scanned_pdf(path, paras):
    """An image-only PDF (no text layer), like a scanner produces."""
    from PIL import Image, ImageDraw, ImageFont
    font = ImageFont.truetype(FONT, 22)
    img = Image.new("L", (1240, 1754), 255)
    d = ImageDraw.Draw(img)
    y = 60
    for p in paras[:3]:
        for i in range(0, len(p), 90):
            d.text((60, y), p[i:i + 90], fill=0, font=font)
            y += 30
    img.save(path, "PDF", resolution=150)


# ---------------------------------------------------------------- legacy via LibreOffice


def soffice_convert(src_files, fmt, outdir):
    """Batch-convert with LibreOffice; returns the paths written."""
    outdir.mkdir(parents=True, exist_ok=True)
    written = []
    profile = Path(tempfile.mkdtemp(prefix="lo-profile-"))
    for i in range(0, len(src_files), 200):
        chunk = [str(p) for p in src_files[i:i + 200]]
        subprocess.run(["soffice", f"-env:UserInstallation=file://{profile}", "--headless",
                        "--convert-to", fmt, "--outdir", str(outdir), *chunk],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    ext = fmt.split(":")[0]
    for p in src_files:
        q = outdir / (Path(p).stem + "." + ext)
        if q.exists():
            written.append(q)
    shutil.rmtree(profile, ignore_errors=True)
    return written


# ---------------------------------------------------------------- corpus builders

MAKERS = {
    "txt": lambda p, t, paras, rng: write_txt(p, [t] + paras),
    "docx": lambda p, t, paras, rng: write_docx(p, t, paras),
    "odt": lambda p, t, paras, rng: write_odt(p, t, paras),
    "rtf": lambda p, t, paras, rng: write_rtf(p, t, paras),
    "xlsx": lambda p, t, paras, rng: write_xlsx(p, t, paras, rng),
    "pptx": lambda p, t, paras, rng: write_pptx(p, t, paras),
}

# Legacy formats are converted from a modern sibling.
LEGACY = {"doc": ("docx", "doc:MS Word 97"), "xls": ("xlsx", "xls:MS Excel 97"),
          "ppt": ("pptx", "ppt:MS PowerPoint 97")}

FOLDERS = ["Contratos", "Facturas/2019", "Facturas/2020", "Banco", "Hacienda/Renta 2018",
           "Médico", "Colegio/Niños", "Casa/Reforma cocina", "Comunidad", "Herencia",
           "Viajes/Peñalara", "Trabajo/Nóminas", "Seguros", "Recetas"]


def n_words(rng):
    # Log-normal around ~1,200 words, clipped: short letters to long reports.
    return int(min(12000, max(60, rng.lognormvariate(math.log(1200), 0.9))))


def make_one(out, rel, fmt, rng, truth, producer="python"):
    path = out / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    marker = pseudo_word(rng)
    title = f"{rng.choice(['Contrato', 'Factura', 'Informe', 'Acta', 'Recibo', 'Carta'])} {marker}"
    paras = doc_text(rng, n_words(rng), marker)
    rec = {"path": str(rel), "format": fmt, "marker": marker, "producer": producer}
    if fmt == "pdf":
        two = rng.random() < 0.15
        hyph = rng.random() < 0.3
        words = write_pdf(path, title, paras, rng, two_columns=two, hyphenate=hyph)
        rec.update(text=" ".join(words), two_columns=two, hyphenated=hyph, producer="fpdf2")
    else:
        MAKERS[fmt](path, title, paras, rng)
        rec["text"] = title + "\n" + "\n".join(paras)
    truth.append(rec)
    return path


def build(out, plan, rng, office_pdf_share=0.5):
    """plan: list of (format, count). Writes files and truth.jsonl; returns the truth list."""
    out.mkdir(parents=True, exist_ok=True)
    truth = []
    staged = {k: [] for k in LEGACY}   # legacy format -> [(modern path, truth record)]
    office_pdf = []                    # docx paths to convert to PDF with LibreOffice
    stage_dir = out / ".stage"
    seq = 0
    for fmt, count in plan:
        for _ in range(count):
            seq += 1
            folder = rng.choice(FOLDERS)
            if fmt in LEGACY:
                modern = LEGACY[fmt][0]
                p = make_one(stage_dir, Path(f"{fmt}_{seq:06d}.{modern}"), modern, rng, truth)
                truth[-1].update(format=fmt, path=str(Path(folder) / f"doc_{seq:06d}.{fmt}"),
                                 producer="libreoffice")
                staged[fmt].append((p, truth[-1]))
            elif fmt == "pdf" and rng.random() < office_pdf_share:
                p = make_one(stage_dir, Path(f"pdf_{seq:06d}.docx"), "docx", rng, truth)
                truth[-1].update(format="pdf", path=str(Path(folder) / f"doc_{seq:06d}.pdf"),
                                 producer="libreoffice")
                office_pdf.append((p, truth[-1]))
            else:
                make_one(out, Path(folder) / f"doc_{seq:06d}.{fmt}", fmt, rng, truth)
    conv = [(k, LEGACY[k][1], v) for k, v in staged.items()] + [("pdf", "pdf", office_pdf)]
    for fmt, filt, items in conv:
        if not items:
            continue
        written = soffice_convert([p for p, _ in items], filt, stage_dir / ("out_" + fmt))
        by_stem = {w.stem: w for w in written}
        for p, rec in items:
            w = by_stem.get(p.stem)
            if w is None:
                truth.remove(rec)
                continue
            dest = out / rec["path"]
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(w), dest)
    shutil.rmtree(stage_dir, ignore_errors=True)
    return truth


def build_hostile(out, rng, truth):
    """Damaged and adversarial inputs. Every file gets a truth line with `expect`."""
    h = out / "hostile"
    h.mkdir(parents=True, exist_ok=True)
    good = [Path(t["path"]) for t in truth if not t["path"].startswith("hostile")]
    by_fmt = {}
    for p in good:
        by_fmt.setdefault(p.suffix[1:], []).append(p)

    def add(name, expect, note):
        truth.append({"path": f"hostile/{name}", "format": Path(name).suffix[1:],
                      "expect": expect, "note": note, "text": ""})

    for fmt, paths in sorted(by_fmt.items()):
        src = (out / paths[0]).read_bytes()
        (h / f"truncated_half.{fmt}").write_bytes(src[: len(src) // 2])
        add(f"truncated_half.{fmt}", "failed_or_partial", "first half of a valid file")
        (h / f"truncated_head.{fmt}").write_bytes(src[:100])
        add(f"truncated_head.{fmt}", "failed", "first 100 bytes")
        b = bytearray(src)
        for _ in range(max(1, len(b) // 100)):
            b[rng.randrange(len(b))] = rng.randrange(256)
        (h / f"corrupt_bytes.{fmt}").write_bytes(bytes(b))
        add(f"corrupt_bytes.{fmt}", "failed_or_partial", "1 % of bytes randomised")
        (h / f"random.{fmt}").write_bytes(os.urandom(50_000))
        add(f"random.{fmt}", "failed", "random bytes")
        (h / f"empty.{fmt}").write_bytes(b"")
        add(f"empty.{fmt}", "skipped_or_failed", "zero bytes")

    # Wrong extension: a JPEG named .pdf; an RTF named .doc; an HTML named .doc (Word saves these).
    from PIL import Image
    buf = io.BytesIO()
    Image.new("RGB", (64, 64), (200, 30, 30)).save(buf, "JPEG")
    (h / "foto_renombrada.pdf").write_bytes(buf.getvalue())
    add("foto_renombrada.pdf", "failed", "JPEG with a .pdf extension")
    paras = ["Carta del señor Núñez sobre la fianza del alquiler en Peñalara."]
    write_rtf(h / "rtf_disfrazado.doc", "Carta", paras)
    add("rtf_disfrazado.doc", "indexed", "RTF saved with a .doc extension")
    truth[-1]["text"] = "Carta\n" + paras[0]
    (h / "html_disfrazado.doc").write_text(
        "<html><body><p>Acta de la reunión de la comunidad de propietarios.</p></body></html>",
        encoding="utf-8")
    add("html_disfrazado.doc", "indexed_or_failed", "HTML saved with a .doc extension")

    # Password-protected DOCX (encrypted OOXML) and PDF (user password).
    docx_src = out / by_fmt["docx"][0]
    try:
        import msoffcrypto
        from msoffcrypto.format.ooxml import OOXMLFile
        with open(docx_src, "rb") as f, open(h / "con_clave.docx", "wb") as g:
            OOXMLFile(f).encrypt("clave1234", g)
        add("con_clave.docx", "skipped", "encrypted OOXML (password)")
    except Exception as e:  # noqa: BLE001 - the corpus still builds without it
        print(f"warning: no encrypted docx: {e}", file=sys.stderr)
    pdf_src = out / by_fmt["pdf"][0]
    subprocess.run(["qpdf", "--encrypt", "clave1234", "dueño", "256", "--", str(pdf_src),
                    str(h / "con_clave.pdf")], check=True)
    add("con_clave.pdf", "skipped", "PDF with a user password")
    subprocess.run(["qpdf", "--encrypt", "", "dueño", "256", "--print=none", "--", str(pdf_src),
                    str(h / "solo_permisos.pdf")], check=True)
    add("solo_permisos.pdf", "indexed", "PDF with an owner password only (opens without one)")
    rec = next(t for t in truth if t["path"] == str(by_fmt["pdf"][0]))
    truth[-1]["text"] = rec["text"]

    # Scanned PDF: no text layer.
    write_scanned_pdf(h / "escaneado.pdf", ["Escritura de compraventa ante la notaría de Peñalara."] * 3)
    add("escaneado.pdf", "skipped_or_empty", "image-only PDF (needs OCR)")

    # Zip bomb disguised as DOCX: a 1 GB document.xml compressing to ~1 MB.
    with zipfile.ZipFile(h / "bomba.docx", "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>')
        with z.open("word/document.xml", "w", force_zip64=True) as f:
            f.write(b'<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>')
            chunk = b"a" * (1 << 20)
            for _ in range(1024):
                f.write(chunk)
            f.write(b"</w:t></w:r></w:p></w:body></w:document>")
    add("bomba.docx", "failed_or_capped", "zip bomb: 1 GB document.xml")

    # Very large valid files.
    big_paras = doc_text(rng, 400_000, "granmarcador")
    write_txt(h / "grande.txt", big_paras)
    add("grande.txt", "indexed_capped", "about 2.8 MB of text, above the output cap")
    words = write_pdf(h / "grande.pdf", "Informe grande", doc_text(rng, 300_000, "pdfgrande"), rng)
    add("grande.pdf", "indexed_capped", "about 1,000 pages")
    write_xlsx(h / "grande.xlsx", "Hoja grande", doc_text(rng, 600_000, "hojagrande"), rng)
    add("grande.xlsx", "indexed_capped", "tens of thousands of rows")
    return truth


def add_real(out, src):
    """Append BOE PDFs (official Spanish gazette; legal texts are not copyrightable, LPI art.
    13) downloaded next to their `txt.php` HTML page, which gives the reference text."""
    import html as htmlmod
    import re
    truth = []
    dest = out / "real"
    dest.mkdir(parents=True, exist_ok=True)
    for pdf in sorted(src.glob("BOE-*.pdf")):
        page = pdf.with_suffix(".html").read_text(encoding="utf-8", errors="replace")
        m = re.search(r'<div id="textoxslt">(.*?)</div>\s*</div>', page, re.S)
        body = m.group(1) if m else page
        text = htmlmod.unescape(re.sub(r"<[^>]+>", " ", body))
        shutil.copy(pdf, dest / pdf.name)
        truth.append({"path": f"real/{pdf.name}", "format": "pdf", "producer": "boe",
                      "text": " ".join(text.split())})
    with open(out / "truth.jsonl", "a", encoding="utf-8") as f:
        for t in truth:
            f.write(json.dumps(t, ensure_ascii=False) + "\n")
    print(f"added {len(truth)} real PDFs")


FIXTURE_TEXT = ("El contrato de alquiler de la vivienda de Peñalara incluye la calefacción y la "
                "cigüeña del tejado. Las facturas de la acción judicial siguen pendientes.")


def make_fixtures(out):
    """One small file per format for the vfs-content tests (VFS-2 AC-2, AC-4), each holding
    FIXTURE_TEXT plus a per-format marker word `fixtura<format>`."""
    out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(1)
    stage = out / ".stage"
    stage.mkdir(exist_ok=True)
    modern = {"docx": "docx", "xlsx": "xlsx", "pptx": "pptx", "doc": "docx", "xls": "xlsx",
              "ppt": "pptx", "odt": "odt", "rtf": "rtf", "txt": "txt"}
    for fmt, base in modern.items():
        paras = [FIXTURE_TEXT, f"Marcador fixtura{fmt}."]
        target = (stage if fmt in LEGACY else out) / f"muestra_{fmt}.{base}"
        MAKERS[base](target, f"Muestra {fmt}", paras, rng)
        if fmt in LEGACY:
            w = soffice_convert([target], LEGACY[fmt][1], stage / "conv")
            shutil.move(str(w[0]), out / f"muestra_{fmt}.{fmt}")
    docx_for_pdf = stage / "muestra_pdf.docx"
    write_docx(docx_for_pdf, "Muestra pdf", [FIXTURE_TEXT, "Marcador fixturapdf."])
    w = soffice_convert([docx_for_pdf], "pdf", stage / "conv")
    shutil.move(str(w[0]), out / "muestra_pdf.pdf")
    import msoffcrypto
    from msoffcrypto.format.ooxml import OOXMLFile
    with open(out / "muestra_docx.docx", "rb") as f, open(out / "con_clave.docx", "wb") as g:
        OOXMLFile(f).encrypt("clave1234", g)
    subprocess.run(["qpdf", "--encrypt", "clave1234", "dueño", "256", "--",
                    str(out / "muestra_pdf.pdf"), str(out / "con_clave.pdf")], check=True)
    write_scanned_pdf(out / "escaneado.pdf", ["Escritura ante la notaría de Peñalara."])
    shutil.rmtree(stage, ignore_errors=True)
    print(f"fixtures in {out}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["eval", "bulk", "real", "fixtures"])
    ap.add_argument("--src", type=Path, help="real: folder with BOE-*.pdf and BOE-*.html")
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--docs", type=int, default=20000)
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args()
    rng = random.Random(a.seed)
    if a.mode == "real":
        return add_real(a.out, a.src)
    if a.mode == "fixtures":
        return make_fixtures(a.out)
    if a.mode == "eval":
        fmts = ["pdf", "docx", "doc", "xlsx", "xls", "pptx", "ppt", "odt", "rtf", "txt"]
        plan = [(f, 80 if f == "pdf" else 40) for f in fmts]
        truth = build(a.out, plan, rng)
        truth = build_hostile(a.out, rng, truth)
    else:
        mix = {"pdf": .40, "docx": .20, "doc": .08, "xlsx": .10, "xls": .04, "pptx": .04,
               "ppt": .02, "odt": .03, "rtf": .03, "txt": .06}
        plan = [(f, round(a.docs * s)) for f, s in mix.items()]
        truth = build(a.out, plan, rng)
    with open(a.out / "truth.jsonl", "w", encoding="utf-8") as f:
        for t in truth:
            f.write(json.dumps(t, ensure_ascii=False) + "\n")
    total = sum((a.out / t["path"]).stat().st_size for t in truth if (a.out / t["path"]).exists())
    print(f"{len(truth)} files, {total / 1e6:.1f} MB in {a.out}")


if __name__ == "__main__":
    main()
