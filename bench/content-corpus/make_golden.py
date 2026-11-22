#!/usr/bin/env python3
"""Build the VFS-2 content golden corpus (AC-17, D17) and the metadata fixtures (AC-18).

    make_golden.py --wiki DIR --boe DIR --out crates/vfs-content/tests/golden
    make_golden.py meta --out crates/vfs-content/tests/fixtures   # metadata and empty-DOCX fixtures

The golden corpus has three parts:
- short excerpts of Spanish Wikipedia articles (CC BY-SA 4.0), rendered to DOCX and PDF;
  `--wiki` holds one JSON per article ({title, url, revid, timestamp, text}) fetched from the
  MediaWiki API (prop=extracts, explaintext). Each excerpt is attributed in NOTICE.md;
- BOE PDFs (Spanish official gazette, public domain under art. 13 of the Spanish copyright
  act), copied as published; `--boe` holds them;
- synthetic household documents written here.

Deterministic for the same inputs. Dev-machine tool (needs python-docx, fpdf2); the output
is committed, so CI never runs this.
"""

import argparse
import datetime
import json
import random
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from make_corpus import write_docx, write_pdf, write_txt  # noqa: E402

# Characters kept from each Wikipedia article: a short excerpt, not the article.
EXCERPT_CHARS = 2500

# Wikipedia articles and the (ASCII) file each excerpt is rendered to.
WIKI = [
    ("Tortilla de patatas", "wiki_tortilla.docx"),
    ("Ciconia ciconia", "wiki_ciguena.pdf"),
    ("Peñalara", "wiki_penalara.pdf"),
    ("Calefacción", "wiki_calefaccion.docx"),
    ("Crédito hipotecario", "wiki_hipoteca.pdf"),
    ("Testamento", "wiki_testamento.docx"),
    ("Contrato de arrendamiento", "wiki_arrendamiento.docx"),
    ("Impuesto sobre la renta de las personas físicas (España)", "wiki_irpf.pdf"),
    ("Propiedad horizontal", "wiki_propiedad_horizontal.docx"),
    ("Paella", "wiki_paella.pdf"),
]

BOE = [
    "BOE-A-2021-8000.pdf",
    "BOE-A-2022-10000.pdf",
    "BOE-A-2024-1.pdf",
    "BOE-A-2024-100.pdf",
    "BOE-A-2024-12000.pdf",
    "BOE-A-2024-5000.pdf",
]

# Synthetic household documents: (file name, title, paragraphs).
SYNTHETIC = [
    ("escaneo0034.docx", "Contrato de alquiler", [
        "En Madrid, a 3 de marzo de 2019. El arrendador, don Luis Pérez Gil, y la arrendataria, "
        "doña Carmen Muñoz Ruiz, acuerdan el alquiler de la vivienda de la calle Peñalara 12.",
        "La renta mensual es de 750 euros. La arrendataria entrega una fianza de dos mensualidades. "
        "El contrato dura cinco años y se prorroga por anualidades.",
    ]),
    ("doc_2020_07.txt", None, [
        "Factura de electricidad. Periodo de facturación: junio de 2020. Potencia contratada 4,6 kW.",
        "Consumo de 312 kWh. Importe total 68,40 euros, domiciliado en la cuenta del Banco Sabadell.",
    ]),
    ("nota_rapida.txt", None, [
        "Llamar al fontanero por la caldera: pierde presión y la calefacción no arranca por las mañanas.",
        "El técnico de Junkers viene el jueves entre las nueve y las doce.",
    ]),
    ("receta_abuela.docx", "Cocido madrileño de la abuela", [
        "Garbanzos remojados la víspera, morcillo, tocino, chorizo, morcilla y un hueso de jamón.",
        "Primero la sopa de fideos con el caldo; después los garbanzos con la verdura y por último las carnes.",
    ]),
    ("acta.pdf", "Acta de la junta de propietarios", [
        "Junta ordinaria de la comunidad de propietarios de la calle Alcalá 140, celebrada el 12 de abril de 2021.",
        "Se aprueba por mayoría la derrama para cambiar el ascensor, de 1.200 euros por vivienda en tres plazos.",
        "El administrador de fincas presenta las cuentas del ejercicio y el presupuesto del año siguiente.",
    ]),
    ("carta_seguro.docx", "Mapfre: renovación de la póliza", [
        "Estimada clienta: le informamos de la renovación de su seguro de hogar con efecto 1 de enero.",
        "La prima anual asciende a 312 euros e incluye daños por agua, robo y responsabilidad civil.",
    ]),
    ("renta2022.pdf", "Declaración de la renta 2022", [
        "Borrador de la declaración del IRPF del ejercicio 2022. Resultado: a devolver 412 euros.",
        "Deducción por inversión en vivienda habitual y por donativos a una ONG.",
    ]),
    ("hipoteca_bbva.pdf", "Escritura de préstamo hipotecario", [
        "Ante el notario don Tomás Ortega comparecen los prestatarios y el BBVA.",
        "Capital prestado de 180.000 euros a 25 años, con un tipo de interés variable de euríbor más 0,99.",
        "La amortización anticipada parcial tiene una comisión del 0,5 por ciento.",
    ]),
    ("medico.txt", None, [
        "Cita con la endocrina el 14 de octubre. Llevar la analítica con la hemoglobina glicosilada.",
        "Revisar la dosis de metformina y el control de la glucosa en ayunas.",
    ]),
    ("viaje_asturias.docx", "Vacaciones en Asturias", [
        "Del 2 al 12 de agosto en Cangas de Onís. Excursión a los lagos de Covadonga y a la ruta del Cares.",
        "Reservar sidrería en Arriondas y la bajada del Sella en canoa.",
    ]),
    ("garantia_lavadora.pdf", "Garantía de la lavadora", [
        "Lavadora Bosch Serie 6, número de serie 4401-2283. Fecha de compra: 18 de mayo de 2021.",
        "La garantía cubre tres años de piezas y mano de obra en el servicio técnico oficial.",
    ]),
    ("colegio.txt", None, [
        "Reunión de padres del colegio Nuestra Señora del Pilar: excursión de quinto curso a Segovia.",
        "Hay que firmar la autorización y pagar 25 euros antes del viernes.",
    ]),
    ("ibi.pdf", "Recibo del IBI", [
        "Ayuntamiento de Madrid. Impuesto sobre bienes inmuebles, ejercicio 2023. Referencia catastral 9872023VK4797A.",
        "Importe 498,32 euros, fraccionado en dos plazos: junio y noviembre.",
    ]),
    ("perro.docx", "Cartilla del perro", [
        "Toby, mestizo de dos años. Vacuna de la rabia puesta el 3 de febrero; desparasitación cada tres meses.",
        "Veterinario: clínica San Antón, doctora Elena Ríos.",
    ]),
    ("jubilacion.txt", None, [
        "Simulación de la pensión de jubilación de la Seguridad Social: base reguladora de 1.850 euros.",
        "Con 38 años cotizados, la jubilación ordinaria es a los 66 años y cuatro meses.",
    ]),
]


def excerpt(text):
    """The first paragraphs of an article, cut at a paragraph end near EXCERPT_CHARS."""
    out, n = [], 0
    for para in (p.strip() for p in text.split("\n")):
        if not para:
            continue
        if n and n + len(para) > EXCERPT_CHARS:
            break
        out.append(para)
        n += len(para)
    return out


def build(args):
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(17)
    notice = [
        "# Content golden corpus: sources and licences",
        "",
        "Fixtures for the VFS-2 content golden set (`crates/vfs-content/tests/golden.rs`,",
        "AC-17). Built by `bench/content-corpus/make_golden.py`.",
        "",
        "## Spanish Wikipedia excerpts (CC BY-SA 4.0)",
        "",
        "Short excerpts (the opening paragraphs, at most about 2,500 characters) of these",
        "articles, rendered to DOCX or PDF, unchanged except for layout. Licence:",
        "[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). Authors: the",
        "contributors listed in each article's history.",
        "",
        "| File | Article | Revision |",
        "|---|---|---|",
    ]
    for title, name in WIKI:
        j = json.load(open(Path(args.wiki) / f"{title.replace('/', '-')}.json", encoding="utf-8"))
        paras = excerpt(j["text"])
        if name.endswith(".docx"):
            write_docx(out / name, j["title"], paras)
        else:
            write_pdf(out / name, j["title"], paras, rng)
        rev = f"https://es.wikipedia.org/w/index.php?oldid={j['revid']}"
        notice.append(f"| `{name}` | [{j['title']}]({j['url']}) | [{j['revid']}]({rev}) |")
    notice += [
        "",
        "## Boletín Oficial del Estado (public domain)",
        "",
        "Copied as published by the Agencia Estatal Boletín Oficial del Estado (boe.es). Legal",
        "and official texts are not subject to copyright in Spain (art. 13, Real Decreto",
        "Legislativo 1/1996, Ley de Propiedad Intelectual).",
        "",
    ]
    for name in BOE:
        shutil.copy(Path(args.boe) / name, out / name)
        notice.append(f"- `{name}`: https://www.boe.es/diario_boe/txt.php?id={name[:-4]}")
    notice += [
        "",
        "## Synthetic documents",
        "",
        "Every other file was written for this corpus; the names, people and figures are invented.",
        "",
    ]
    for name, title, paras in SYNTHETIC:
        p = out / name
        if name.endswith(".txt"):
            write_txt(p, paras)
        elif name.endswith(".docx"):
            write_docx(p, title, paras)
        else:
            write_pdf(p, title, paras, rng)
    (out / "NOTICE.md").write_text("\n".join(notice) + "\n", encoding="utf-8")


def meta(args):
    """AC-18: a DOCX and a PDF whose metadata hold a title, an author and a creation date.
    AC-19: a DOCX with no text."""
    import docx
    from fpdf import FPDF

    out = Path(args.out)
    created = datetime.datetime(2019, 3, 3, 10, 30, 0)
    d = docx.Document()
    d.add_paragraph("Inventario de la casa del pueblo: muebles, cuadros y la cómoda de la abuela.")
    cp = d.core_properties
    cp.title = "Inventario de la casa"
    cp.author = "Carmen Muñoz"
    cp.created = created
    d.save(out / "meta_docx.docx")

    pdf = FPDF()
    pdf.set_title("Informe de la comunidad")
    pdf.set_author("Luis Pérez")
    pdf.set_creation_date(created.replace(tzinfo=datetime.timezone.utc))
    pdf.add_page()
    pdf.set_font("helvetica", size=12)
    pdf.multi_cell(0, 8, "Informe anual de la comunidad: ascensor, portal y jardines.")
    pdf.output(str(out / "meta_pdf.pdf"))

    docx.Document().save(out / "vacio.docx")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", nargs="?", default="build", choices=["build", "meta"])
    ap.add_argument("--wiki")
    ap.add_argument("--boe")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    (meta if a.mode == "meta" else build)(a)


if __name__ == "__main__":
    main()
