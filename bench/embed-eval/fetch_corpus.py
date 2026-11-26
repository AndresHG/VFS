#!/usr/bin/env python3
"""Fetch the distractor corpus for the VFS-29b embedding benchmark.

Short excerpts (the lead section, at most EXCERPT_CHARS characters) of Spanish and English
Wikipedia articles, CC BY-SA 4.0. Most are topical (what a search for a family-archive theme
returns: health, taxes, insurance, utilities, school, appliances, housing...), so the
meaning-only targets compete with documents on the same subjects; the rest are random.

    fetch_corpus.py --out corpus/distractors.jsonl.gz

Each output row: {lang, title, revid, url, text}. The rows are the attribution: every
excerpt names its article and the revision it was taken from (see corpus/NOTICE.md).
Dev-machine tool; needs network access to *.wikipedia.org.
"""

import argparse
import gzip
import json
import random
import time
import urllib.parse
import urllib.request

EXCERPT_CHARS = 3000
MIN_CHARS = 400
ES, EN = "es", "en"  # Wikipedia language editions
UA = "vfs-embed-eval/0.1 (offline search research benchmark)"

TOPICS_ES = """
factura electricidad|recibo del agua|gas natural suministro|tarifa telefonía móvil|fibra óptica
seguro de hogar|seguro de automóvil|seguro de vida|seguro médico privado|póliza de seguros
impuesto sobre la renta|declaración tributaria|hacienda pública|impuesto de sucesiones|IVA España
hipoteca préstamo|cuenta bancaria|tarjeta de crédito|pensión de jubilación|seguridad social España
contrato de trabajo|nómina salario|despido laboral|baja por enfermedad|vacaciones laborales
colegio educación primaria|educación secundaria España|universidad matrícula|beca de estudios
boletín de notas|enfermedad diabetes|hipertensión arterial|vacunación infantil|odontología
oftalmología|pediatría|medicamento antibiótico|alergia|fisioterapia|análisis de sangre
hospital urgencias|cirugía|lavadora electrodoméstico|frigorífico|lavavajillas|horno microondas
aire acondicionado|caldera de gas|televisor|ordenador portátil|teléfono inteligente|impresora
router|vivienda alquiler|comunidad de propietarios|reforma de vivienda|fontanería|electricista
tejado|testamento herencia|notario|registro de la propiedad|catastro|escritura pública
automóvil mantenimiento|inspección técnica de vehículos|neumático|multa de tráfico
carné de conducir|viaje vacaciones|hotel|aeropuerto|tren de alta velocidad|camping|crucero
cocina española|receta|repostería|vino|aceite de oliva|mercado|perro mascota|gato doméstico
veterinario|acuario|caballo|jardinería|huerto|piscina|bricolaje|herramienta|boda|bautismo
funeral|cumpleaños|Navidad|fotografía familiar|ayuntamiento|empadronamiento
documento nacional de identidad|pasaporte|correos|gimnasio|fútbol|ciclismo|natación
senderismo|música|cine|teatro|literatura española|biblioteca
"""

TOPICS_EN = """
electricity bill|water supply utility|natural gas heating|mobile phone plan|broadband internet
home insurance|car insurance|life insurance|health insurance|insurance policy|income tax
tax return|pension|bank account|credit card|mortgage loan|employment contract|salary
sick leave|primary school|secondary education|university tuition|school report card
diabetes|hypertension|vaccination|dentistry|optometry|pediatrics|antibiotic|allergy
physical therapy|blood test|washing machine|refrigerator|dishwasher|microwave oven
air conditioning|boiler|apartment rental|homeowners association|plumbing|roof repair
will and testament|notary public|car maintenance|vehicle inspection|traffic ticket
driving licence|travel holiday|hotel|airport|recipe|baking|dog|cat|veterinarian|gardening
wedding|birthday
"""


def topics(block):
    return [t.strip() for t in block.replace("\n", "|").split("|") if t.strip()]


def api(lang, params):
    params = {"action": "query", "format": "json", "formatversion": "2", **params}
    url = f"https://{lang}.wikipedia.org/w/api.php?" + urllib.parse.urlencode(params)
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                return json.load(r)
        except Exception as e:
            err = e
            time.sleep(5 * 2 ** attempt)
    print(f"skipped after retries ({err}): {url}", flush=True)
    return {}


def excerpt(text):
    if len(text) <= EXCERPT_CHARS:
        return text
    cut = text.rfind("\n", 0, EXCERPT_CHARS)
    if cut < EXCERPT_CHARS // 2:
        cut = text.rfind(". ", 0, EXCERPT_CHARS) + 1
    return text[:cut].strip()


def pages(lang, generator):
    base = {"prop": "extracts|info", "exintro": "1", "explaintext": "1", "exlimit": "20"}
    d = api(lang, {**base, **generator})
    for p in d.get("query", {}).get("pages", []):
        text = (p.get("extract") or "").strip()
        if len(text) < MIN_CHARS or p.get("ns", 0) != 0 or not p.get("lastrevid"):
            continue
        yield {
            "lang": lang,
            "title": p["title"],
            "revid": p["lastrevid"],
            "url": f"https://{lang}.wikipedia.org/w/index.php?oldid={p['lastrevid']}",
            "text": excerpt(text),
        }


def collect(lang, topic_list, n_random, want):
    seen, rows = set(), []

    def add(gen):
        for row in pages(lang, gen):
            if row["title"] not in seen:
                seen.add(row["title"])
                rows.append(row)
        time.sleep(1.0)

    for t in topic_list:
        add({"generator": "search", "gsrsearch": t, "gsrlimit": "20", "gsrnamespace": "0"})
    topical = len(rows)
    while len(rows) < topical + n_random or len(rows) < want:
        add({"generator": "random", "grnnamespace": "0", "grnlimit": "20"})
    print(f"{lang}: {topical} topical + {len(rows) - topical} random", flush=True)
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--es", type=int, default=2100, help="minimum Spanish excerpts")
    ap.add_argument("--en", type=int, default=400, help="minimum English excerpts")
    a = ap.parse_args()
    rows = (collect(ES, topics(TOPICS_ES), 500, a.es)
            + collect(EN, topics(TOPICS_EN), 100, a.en))
    random.Random(29).shuffle(rows)
    with gzip.open(a.out, "wt", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"{len(rows)} excerpts -> {a.out}")


if __name__ == "__main__":
    main()
