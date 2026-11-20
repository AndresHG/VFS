#!/usr/bin/env python3
"""VFS-5: writes a deterministic tree of empty-ish files shaped like a Spanish personal archive,
for the end-to-end journey and the slow-disk runs, plus `manifest.json` naming the planted
targets the journey searches for.

Shape: about `--files` files, about 13 per folder, folders up to 12 levels deep; Spanish names
with accents and ñ, 5 % of them in NFD; dates 2000-2025; camera counters and "Copia de"
duplicates; a few paths over 260 characters; planted targets at known paths, each with a kind,
size and date chosen so one search or one filter finds it. File bodies are empty except the
size-filter targets, which are extended with `truncate` (no data is written).

Usage: gen_tree.py <root> [--files N] [--seed S]
"""

import argparse
import json
import os
import random
import sys
import time
import unicodedata
from datetime import datetime, timedelta, timezone

FILES_PER_DIR = 13
MAX_DEPTH = 12
MB = 1024 * 1024
DAY = 86400

TOP = ["Documentos", "Fotos", "Vídeos", "Música", "Gestoría", "Descargas", "Escritorio",
       "Copia de seguridad portátil", "Fotos del móvil", "Trabajo", "Casa", "Colegio niños"]
WORDS = ["año", "niño", "señora", "España", "Peñalara", "Córdoba", "Málaga", "Cádiz", "Ávila",
         "León", "Logroño", "Begoña", "Íñigo", "Ramón", "José", "María", "Concepción",
         "Jesús", "Inés", "Óscar", "Raúl", "Sofía", "Lucía", "Martín", "Andrés", "Pérez",
         "Muñoz", "Ibáñez", "Núñez", "Gómez", "Fernández", "García", "factura", "recibo",
         "contrato", "escritura", "notaría", "hipoteca", "seguro", "declaración", "renta",
         "nómina", "certificado", "médico", "análisis", "vacaciones", "cumpleaños",
         "comunión", "bautizo", "navidad", "verano", "excursión", "reunión", "presupuesto",
         "reforma", "cocina", "baño", "jardín", "coche", "matrícula", "boletín", "examen",
         "apuntes", "lección", "música", "canción", "película", "vídeo", "foto", "álbum",
         "viaje", "playa", "montaña", "campo", "pueblo", "abuela", "abuelo", "tía", "primo",
         "comunidad", "vecinos", "acta", "junta", "electricidad", "agua", "teléfono"]
EXT = {"document": [".pdf", ".docx", ".doc", ".xlsx", ".odt", ".txt", ".pptx"],
       "photo": [".jpg", ".jpeg", ".png", ".heic"],
       "video": [".mp4", ".mov", ".avi"],
       "audio": [".mp3", ".m4a", ".wav"],
       "other": [".zip", ".dat", ".bak"]}
KIND_WEIGHTS = [("document", 40), ("photo", 40), ("video", 5), ("audio", 7), ("other", 8)]

# One segment of the over-260-character chain (an old backup copied inside a backup).
LONG_SEGMENT = "Copia de seguridad del portátil de la oficina antigua"
LONG_LEVELS = 6
LONG_EXTRA = 4

# Planted targets: key -> (directory relative to the root, name or None for a folder target,
# kind, size in bytes, age in days or None for a fixed old date). The journey reads them from
# manifest.json; keep the keys stable.
TARGETS = {
    "name": ("Gestoría/Año 2019", "Notaría Valdepeñas escritura 2019.pdf", "document", 0, None),
    "middle": ("Casa/Herencia tía Remedios/Papeles banco", "extracto.pdf", "document", 0,
               None),
    "folder": ("Vídeos/2014/Boda Íñigo y Ainhoa", None, "folder", 0, None),
    "week": ("Fotos/2025/Vacaciones Peñíscola", "Peñíscola playa atardecer.jpg", "photo", 0, 2),
    "medium": ("Trabajo/Jubilación", "Presentación jubilación Ramón.pptx", "document", 2 * MB,
               None),
    "large": ("Vídeos/2019", "Excursión Covadonga.mp4", "video", 150 * MB, None),
    "huge": ("Vídeos/Antiguos", "Película familiar Navidad 1998.mp4", "video", 1100 * MB,
             None),
    "audio": ("Música/Familia", "Canción de cuna María.mp3", "audio", 0, None),
    "long": ("/".join([LONG_SEGMENT] * LONG_LEVELS), "Informe pericial Ibáñez definitivo.docx",
             "document", 0, None),
}
# Files the journey creates, renames and deletes while the app is open (live changes).
LIVE_NAME = "Carta urgente Muñoz.docx"
LIVE_RENAMED = "Carta respondida Muñoz.docx"


def fs_path(p):
    """`p` as the OS accepts it: on Windows an absolute path gets the `\\\\?\\` prefix, so
    paths over 260 characters work whatever the long-path setting."""
    if os.name == "nt":
        p = os.path.abspath(p)
        if not p.startswith("\\\\?\\"):
            p = "\\\\?\\" + p
    return p


def random_name(rng, kind):
    n = rng.choice([2, 3, 3, 4, 5])
    words = [rng.choice(WORDS) for _ in range(n)]
    roll = rng.random()
    if kind == "photo" and roll < 0.5:
        stem = rng.choice(["IMG_%04d" % rng.randrange(10000), "DSC%05d" % rng.randrange(100000),
                           "IMG-%d%02d%02d-WA%04d" % (rng.randrange(2010, 2026),
                                                       rng.randrange(1, 13),
                                                       rng.randrange(1, 29),
                                                       rng.randrange(10000))])
    else:
        words[0] = words[0].capitalize()
        stem = " ".join(words)
        if rng.random() < 0.3:
            stem += " %d" % rng.randrange(2000, 2026)
    if roll > 0.97:
        stem = "Copia de " + stem
    elif roll > 0.94:
        stem += " (2)"
    name = stem + rng.choice(EXT[kind])
    if rng.random() < 0.05:
        name = unicodedata.normalize("NFD", name)
    return name


def random_dirname(rng):
    words = [rng.choice(WORDS) for _ in range(rng.choice([1, 2, 2, 3]))]
    words[0] = words[0].capitalize()
    name = " ".join(words)
    if rng.random() < 0.3:
        name = "%d %s" % (rng.randrange(2000, 2026), name)
    return name


def pick_kind(rng):
    total = sum(w for _, w in KIND_WEIGHTS)
    r = rng.randrange(total)
    for kind, w in KIND_WEIGHTS:
        if r < w:
            return kind
        r -= w
    return "other"


def random_mtime(rng):
    start = datetime(2000, 1, 1, tzinfo=timezone.utc).timestamp()
    end = datetime(2025, 12, 31, tzinfo=timezone.utc).timestamp()
    return rng.uniform(start, end)


def make_dirs(rng, files):
    """Relative directory paths: the TOP folders, then random children up to MAX_DEPTH."""
    want = max(len(TOP), files // FILES_PER_DIR)
    dirs = list(TOP)
    seen = set(dirs)
    while len(dirs) < want:
        parent = rng.choice(dirs)
        if parent.count("/") + 1 >= MAX_DEPTH:
            continue
        child = parent + "/" + random_dirname(rng)
        if child not in seen:
            seen.add(child)
            dirs.append(child)
    return dirs


def write_file(path, size, mtime):
    with open(fs_path(path), "wb") as f:
        if size:
            f.truncate(size)
    os.utime(fs_path(path), (mtime, mtime))


def generate(root, files=100_000, seed=42):
    """Writes the tree under `root` (must not exist or be empty) and returns the manifest."""
    rng = random.Random(seed)
    root = os.path.abspath(root)
    if os.path.isdir(root) and os.listdir(root):
        raise SystemExit(f"{root} is not empty; refusing to write into it")
    os.makedirs(fs_path(root), exist_ok=True)
    dirs = make_dirs(rng, files)
    for d in dirs:
        os.makedirs(fs_path(os.path.join(root, *d.split("/"))), exist_ok=True)
    now = time.time()
    targets = {}
    for key, (rel, name, kind, size, age) in TARGETS.items():
        dpath = os.path.join(root, *rel.split("/"))
        os.makedirs(fs_path(dpath), exist_ok=True)
        mtime = now - age * DAY if age is not None else datetime(
            2018, 6, 15, tzinfo=timezone.utc).timestamp()
        if name is None:
            path = dpath
            for i in range(3):  # a folder target holds a few videos of its own
                write_file(os.path.join(dpath, "Ceremonia parte %d.mp4" % (i + 1)), 0, mtime)
        else:
            path = os.path.join(dpath, name)
            write_file(path, size, mtime)
        targets[key] = {"path": path, "dir": dpath, "name": name or os.path.basename(dpath),
                        "kind": kind, "size": size, "mtime": int(mtime)}
    written = sum(3 if t[1] is None else 1 for t in TARGETS.values())
    long_dir = targets["long"]["dir"]
    for i in range(LONG_EXTRA):  # more paths over 260 characters, beside the long target
        write_file(os.path.join(long_dir, "Anexo %d escaneado.pdf" % (i + 1)), 0,
                   random_mtime(rng))
    written += LONG_EXTRA
    names_in_dir = {}
    while written < files:
        d = rng.choice(dirs)
        kind = pick_kind(rng)
        name = random_name(rng, kind)
        used = names_in_dir.setdefault(d, set())
        key = unicodedata.normalize("NFC", name).lower()
        if key in used:
            continue
        used.add(key)
        write_file(os.path.join(root, *d.split("/"), name), 0, random_mtime(rng))
        written += 1
    over_260 = sum(1 for t in targets.values() if len(t["path"]) > 260) + LONG_EXTRA
    manifest = {"root": os.path.abspath(root), "files": written, "dirs": len(dirs),
                "seed": seed, "targets": targets, "paths_over_260": over_260,
                "live_name": LIVE_NAME, "live_renamed": LIVE_RENAMED}
    return manifest


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("root")
    ap.add_argument("--files", type=int, default=100_000)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--manifest", help="where to write manifest.json (default: next to root)")
    args = ap.parse_args(argv)
    t = time.time()
    manifest = generate(args.root, args.files, args.seed)
    manifest["generate_seconds"] = round(time.time() - t, 1)
    out = args.manifest or os.path.join(os.path.dirname(os.path.abspath(args.root)),
                                        "manifest.json")
    with open(out, "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)
    print(f"{manifest['files']} files in {manifest['dirs']} folders, "
          f"{manifest['paths_over_260']} paths over 260 chars, "
          f"{manifest['generate_seconds']} s -> {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
