//! Golden relevance set in Spanish (VFS-10): a fixed archive modelled on the end user's
//! (photos by event, household papers, the tax adviser's folders, recipes, music) plus noise,
//! and a fixed list of queries, each with the file or folder that must appear among the
//! first results. It guards every ranking change (sort, frecency, query syntax, schema
//! work): a change that moves a right answer off the first screen fails here, with every
//! miss listed at once.

use std::path::Path;
use std::time::Duration;

use rusqlite::Transaction;
use tempfile::TempDir;
use vfs_core::db::{self, ingest};
use vfs_core::{SearchConfig, SearchFilters, SearchHit, SearchQuery, Vfs};

/// The root folder of the golden archive.
const ROOT: &str = "/archivo";

/// Folders (relative to `ROOT`, `/`-separated) and the files directly inside them.
/// Parents are listed before their children.
const TREE: &[(&str, &[&str])] = &[
    ("Fotos", &["Thumbs.db"]),
    (
        "Fotos/2019 Boda Ana y Luis",
        &[
            "IMG_0001.JPG",
            "IMG_0002.JPG",
            "IMG_0003.JPG",
            "IMG_0004.JPG",
            "Ceremonia iglesia.jpg",
            "Banquete.mp4",
            "Thumbs.db",
        ],
    ),
    (
        "Fotos/2021 Vacaciones Asturias",
        &[
            "Playa de Cudillero.jpg",
            "Picos de Europa 1.jpg",
            "Picos de Europa 2.jpg",
        ],
    ),
    (
        "Fotos/Cumpleaños",
        &["Cumpleaños Ñito 2020.jpg", "tarta.jpg"],
    ),
    ("Fotos/Navidad 2018", &["Cena Nochebuena.jpg"]),
    ("Fotos/Móvil 2020", &[]),
    ("Papeles de casa", &[]),
    (
        "Papeles de casa/Escrituras",
        &[
            "Escritura notaría Pérez 2004.pdf",
            "Nota simple registro.pdf",
        ],
    ),
    (
        "Papeles de casa/Hipoteca",
        &["Contrato hipoteca BBVA.pdf", "Recibos hipoteca 2022.xlsx"],
    ),
    (
        "Papeles de casa/Facturas luz",
        &[
            "Factura luz enero 2023.pdf",
            "Factura luz febrero 2023.pdf",
            "Factura luz marzo 2023.pdf",
        ],
    ),
    (
        "Papeles de casa/Facturas agua",
        &["Factura agua 2023 T1.pdf"],
    ),
    (
        "Papeles de casa/Seguros",
        &["Póliza seguro hogar Mapfre.pdf", "Seguro coche 2022.pdf"],
    ),
    (
        "Papeles de casa/Comunidad de vecinos",
        &["Acta junta 2021.docx", "Derrama ascensor.pdf"],
    ),
    ("Gestoría", &[]),
    (
        "Gestoría/Hacienda",
        &[
            "Declaración renta 2021.pdf",
            "Declaración renta 2022.pdf",
            "Borrador renta 2022.pdf",
        ],
    ),
    (
        "Gestoría/Seguridad Social",
        &["Vida laboral.pdf", "Certificado pensión.pdf"],
    ),
    ("Gestoría/IBI", &["Recibo IBI 2023.pdf"]),
    (
        "Documentos personales",
        &[
            "DNI escaneado.pdf",
            "Pasaporte Carmen.pdf",
            "Libro de familia.pdf",
            "Tarjeta sanitaria.jpg",
            "Carné de conducir.pdf",
        ],
    ),
    (
        "Salud",
        &[
            "Análisis de sangre 2022.pdf",
            "Informe cardiólogo.pdf",
            "Receta oftalmólogo gafas.pdf",
        ],
    ),
    (
        "Recetas de cocina",
        &[
            "Croquetas de la abuela.docx",
            "Paella valenciana.txt",
            "Tortilla de patatas.pdf",
            "Leche frita.doc",
        ],
    ),
    ("Música", &[]),
    ("Música/Joaquín Sabina", &["19 días y 500 noches.mp3"]),
    ("Música/Mecano", &["Hijo de la luna.mp3"]),
    // Written decomposed (NFD), as macOS stores names.
    ("Música/Nanas", &["Cancio\u{301}n de cuna.mp3"]),
    ("Vídeos", &["Comunión Lucía 2015.mp4", "Bautizo Martín.avi"]),
    (
        "Cartas",
        &[
            "Carta a la señora Muñoz.docx",
            "Notas sueltas.txt",
            // Written decomposed (NFD).
            "Felicitacio\u{301}n Nin\u{303}o Jesu\u{301}s.docx",
        ],
    ),
    ("Colegio", &["Notas Lucía 3º ESO.pdf"]),
];

/// Noise: a phone roll of this many `IMG_1xxx.jpg` files in `Fotos/Móvil 2020`.
const PHONE_ROLL: usize = 200;

/// One golden query: `expected` (relative to `ROOT`; a trailing `/` names a folder hit) must
/// be among the first `within` results.
struct Case {
    query: &'static str,
    expected: &'static str,
    within: usize,
}

const fn case(query: &'static str, expected: &'static str, within: usize) -> Case {
    Case {
        query,
        expected,
        within,
    }
}

/// Where the first screen ends for a non-exact query.
const TOP: usize = 3;

const CASES: &[Case] = &[
    // Accents and case: the query and the names are folded alike.
    case(
        "notaria",
        "Papeles de casa/Escrituras/Escritura notaría Pérez 2004.pdf",
        TOP,
    ),
    case(
        "declaracion renta 2022",
        "Gestoría/Hacienda/Declaración renta 2022.pdf",
        1,
    ),
    case("analisis sangre", "Salud/Análisis de sangre 2022.pdf", 1),
    case(
        "poliza hogar",
        "Papeles de casa/Seguros/Póliza seguro hogar Mapfre.pdf",
        1,
    ),
    case(
        "PENSION",
        "Gestoría/Seguridad Social/Certificado pensión.pdf",
        1,
    ),
    case("cardiologo", "Salud/Informe cardiólogo.pdf", 1),
    case(
        "LIBRO FAMILIA",
        "Documentos personales/Libro de familia.pdf",
        1,
    ),
    case(
        "carne conducir",
        "Documentos personales/Carné de conducir.pdf",
        1,
    ),
    case("oftalmologo", "Salud/Receta oftalmólogo gafas.pdf", 1),
    case("bautizo martin", "Vídeos/Bautizo Martín.avi", 1),
    case("Lucía notas", "Colegio/Notas Lucía 3º ESO.pdf", TOP),
    case("cancion cuna", "Música/Nanas/Canción de cuna.mp3", 1),
    case(
        "felicitacion niño",
        "Cartas/Felicitación Niño Jesús.docx",
        1,
    ),
    // ñ: typed with or without the tilde.
    case("cumpleaños", "Fotos/Cumpleaños/", TOP),
    case(
        "cumpleanos nito",
        "Fotos/Cumpleaños/Cumpleaños Ñito 2020.jpg",
        1,
    ),
    case("muñoz", "Cartas/Carta a la señora Muñoz.docx", 1),
    case("senora munoz", "Cartas/Carta a la señora Muñoz.docx", 1),
    // One typo in a word of 6 or more letters.
    case(
        "notaira",
        "Papeles de casa/Escrituras/Escritura notaría Pérez 2004.pdf",
        TOP,
    ),
    case("hipotca", "Papeles de casa/Hipoteca/", TOP),
    case(
        "pasaprote",
        "Documentos personales/Pasaporte Carmen.pdf",
        TOP,
    ),
    case(
        "croquteas",
        "Recetas de cocina/Croquetas de la abuela.docx",
        TOP,
    ),
    case(
        "ascensro",
        "Papeles de casa/Comunidad de vecinos/Derrama ascensor.pdf",
        TOP,
    ),
    case("valenciaan", "Recetas de cocina/Paella valenciana.txt", TOP),
    case(
        "certifcado",
        "Gestoría/Seguridad Social/Certificado pensión.pdf",
        TOP,
    ),
    case(
        "declaracon",
        "Gestoría/Hacienda/Declaración renta 2022.pdf",
        TOP,
    ),
    case("nochebeuna", "Fotos/Navidad 2018/Cena Nochebuena.jpg", TOP),
    case("hacineda", "Gestoría/Hacienda/", TOP),
    // The start of a word.
    case(
        "escrit",
        "Papeles de casa/Escrituras/Escritura notaría Pérez 2004.pdf",
        TOP,
    ),
    case("croq", "Recetas de cocina/Croquetas de la abuela.docx", 1),
    case("vacac", "Fotos/2021 Vacaciones Asturias/", TOP),
    case(
        "cudill",
        "Fotos/2021 Vacaciones Asturias/Playa de Cudillero.jpg",
        1,
    ),
    case(
        "tortilla pata",
        "Recetas de cocina/Tortilla de patatas.pdf",
        1,
    ),
    // Words split between the folders and the name.
    case(
        "boda ceremonia",
        "Fotos/2019 Boda Ana y Luis/Ceremonia iglesia.jpg",
        1,
    ),
    case(
        "asturias playa",
        "Fotos/2021 Vacaciones Asturias/Playa de Cudillero.jpg",
        1,
    ),
    case(
        "hacienda 2021",
        "Gestoría/Hacienda/Declaración renta 2021.pdf",
        TOP,
    ),
    case(
        "luz marzo",
        "Papeles de casa/Facturas luz/Factura luz marzo 2023.pdf",
        1,
    ),
    case(
        "escrituras nota simple",
        "Papeles de casa/Escrituras/Nota simple registro.pdf",
        1,
    ),
    case(
        "boda img_0003",
        "Fotos/2019 Boda Ana y Luis/IMG_0003.JPG",
        1,
    ),
    case("mecano luna", "Música/Mecano/Hijo de la luna.mp3", 1),
    case("sabina", "Música/Joaquín Sabina/", TOP),
    case(
        "musica sabina",
        "Música/Joaquín Sabina/19 días y 500 noches.mp3",
        TOP,
    ),
    case(
        "seguros coche",
        "Papeles de casa/Seguros/Seguro coche 2022.pdf",
        1,
    ),
    // Years.
    case("2019", "Fotos/2019 Boda Ana y Luis/", 1),
    case(
        "renta 2021",
        "Gestoría/Hacienda/Declaración renta 2021.pdf",
        1,
    ),
    case(
        "factura luz 2023",
        "Papeles de casa/Facturas luz/Factura luz enero 2023.pdf",
        TOP,
    ),
    case("navidad 2018", "Fotos/Navidad 2018/", 1),
    case("comunion 2015", "Vídeos/Comunión Lucía 2015.mp4", 1),
    // Short words (under 6 letters: no typo tolerance; under 3: no trigram index).
    case("dni", "Documentos personales/DNI escaneado.pdf", 1),
    case("ibi", "Gestoría/IBI/", 1),
    case("ibi", "Gestoría/IBI/Recibo IBI 2023.pdf", TOP),
    case("luz", "Papeles de casa/Facturas luz/", TOP),
    case("ESO", "Colegio/Notas Lucía 3º ESO.pdf", 1),
    case("gafas", "Salud/Receta oftalmólogo gafas.pdf", 1),
    case(
        "acta",
        "Papeles de casa/Comunidad de vecinos/Acta junta 2021.docx",
        1,
    ),
    case("3º", "Colegio/Notas Lucía 3º ESO.pdf", 1),
    case(
        "t1",
        "Papeles de casa/Facturas agua/Factura agua 2023 T1.pdf",
        1,
    ),
    // Whole names and folder names.
    case(
        "vida laboral",
        "Gestoría/Seguridad Social/Vida laboral.pdf",
        1,
    ),
    case("seguridad social", "Gestoría/Seguridad Social/", 1),
    case("recetas", "Recetas de cocina/", 1),
    case(
        "picos europa",
        "Fotos/2021 Vacaciones Asturias/Picos de Europa 1.jpg",
        TOP,
    ),
];

struct Golden {
    _dir: TempDir,
    vfs: Vfs,
}

fn golden() -> Golden {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    // A generous deadline: the set checks the order, not the speed of a busy CI runner.
    let vfs = Vfs::open_with(
        &path,
        SearchConfig {
            deadline: Duration::from_secs(30),
        },
    )
    .unwrap();
    let mut conn = db::open_rw(&path).unwrap();
    let tx = conn.transaction().unwrap();
    populate(&tx);
    tx.commit().unwrap();
    Golden { _dir: dir, vfs }
}

fn populate(tx: &Transaction) {
    let root = ingest::insert_root(tx, "/", ROOT).unwrap();
    let top: i64 = tx
        .query_row(
            "SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ''",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    let mut dirs: Vec<(&str, i64)> = Vec::new();
    // Distinct, increasing times, so ties fall the same way on every run.
    let mut mtime = 1_600_000_000;
    for (rel, names) in TREE {
        let parent = match rel.rsplit_once('/') {
            Some((p, _)) => dirs.iter().find(|(r, _)| *r == p).expect("parent first").1,
            None => top,
        };
        mtime += 1;
        let id = ingest::insert_dir(tx, root, parent, rel, mtime).unwrap();
        dirs.push((rel, id));
        let mut files: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        if *rel == "Fotos/Móvil 2020" {
            files.extend((0..PHONE_ROLL).map(|i| format!("IMG_{}.jpg", 1000 + i)));
        }
        for name in &files {
            mtime += 1;
            let file = ingest::NewFile {
                name,
                size: 1_000,
                mtime,
                lossy: false,
            };
            ingest::insert_file(tx, id, &file).unwrap();
        }
    }
}

/// A hit as the cases name it: relative to `ROOT`, `/`-separated, NFC, with a trailing `/`
/// for a folder.
fn rel(hit: &SearchHit) -> String {
    use unicode_normalization::UnicodeNormalization;
    let dir = hit.dir.replace('\\', "/");
    let root = Path::new(ROOT).to_string_lossy().replace('\\', "/");
    let dir = dir
        .strip_prefix(&root)
        .unwrap_or(&dir)
        .trim_start_matches('/');
    let full = if dir.is_empty() {
        hit.name.clone()
    } else {
        format!("{dir}/{}", hit.name)
    };
    let slash = if hit.is_dir { "/" } else { "" };
    format!("{}{slash}", full.nfc().collect::<String>())
}

/// The ranked results of `text`, as `rel` names them.
fn ranked(g: &Golden, text: &str) -> Vec<String> {
    let r = g
        .vfs
        .search(&SearchQuery {
            text: text.into(),
            filters: SearchFilters::default(),
            limit: 50,
            offset: 0,
        })
        .unwrap();
    r.hits.iter().map(rel).collect()
}

/// VFS-10 AC-1, AC-2, AC-3: every golden query finds its expected file or folder among
/// its first `within` results (accents, ñ, typos, word starts, words split between folder
/// and name, years, short words); all misses are reported together, with what came first.
#[test]
fn golden_queries_find_the_expected_result_first() {
    let g = golden();
    let mut misses = Vec::new();
    let mut reciprocal = 0.0;
    for c in CASES {
        let got = ranked(&g, c.query);
        let pos = got.iter().position(|h| h == c.expected);
        reciprocal += pos.map_or(0.0, |p| 1.0 / (p + 1) as f64);
        if pos.is_none_or(|p| p >= c.within) {
            let first: Vec<&String> = got.iter().take(c.within.max(TOP)).collect();
            misses.push(format!(
                "«{}»: expected «{}» in the first {}, found at {:?}; first: {first:?}",
                c.query,
                c.expected,
                c.within,
                pos.map(|p| p + 1)
            ));
        }
    }
    let mrr = reciprocal / CASES.len() as f64;
    assert!(
        misses.is_empty(),
        "{} of {} golden queries missed (MRR {mrr:.3}):\n{}",
        misses.len(),
        CASES.len(),
        misses.join("\n")
    );
}

/// VFS-10 AC-4: the set is big enough to mean something and every case names an entry of
/// the archive, so a typo in the set cannot pass as "not found".
#[test]
fn golden_set_is_well_formed() {
    assert!(CASES.len() >= 40, "{} cases", CASES.len());
    let mut entries: Vec<String> = Vec::new();
    for (rel, names) in TREE {
        entries.push(format!("{rel}/"));
        for n in *names {
            use unicode_normalization::UnicodeNormalization;
            entries.push(format!("{rel}/{}", n.nfc().collect::<String>()));
        }
    }
    for c in CASES {
        assert!(
            entries.iter().any(|e| e == c.expected),
            "«{}» is not in the golden archive",
            c.expected
        );
        assert!(c.within >= 1 && c.within <= TOP, "«{}»", c.query);
    }
}
