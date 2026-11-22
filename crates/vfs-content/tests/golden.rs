//! Content golden set in Spanish (VFS-2 AC-17, D17): a fixed corpus of household documents,
//! BOE gazette PDFs and Wikipedia excerpts (`tests/golden`, sources in its NOTICE.md), and a
//! fixed list of queries, each with the document that must appear among the first results.
//! It guards every change to analysis, query semantics or ranking: the share of cases found
//! may only go up, and every miss is listed at once.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use vfs_content::{IndexOptions, Query, Searcher};
use vfs_core::{SearchFilters, Vfs};

/// The expected document must be among this many first hits.
const TOP: usize = 5;

/// Share of cases whose expected document is in the top `TOP`, fixed by the first run. It may
/// only go up (VFS-2 AC-17).
const MIN_SHARE: f64 = 1.0;

struct Case {
    query: &'static str,
    expected: &'static str,
}

const fn case(query: &'static str, expected: &'static str) -> Case {
    Case { query, expected }
}

const CASES: &[Case] = &[
    // Household documents (synthetic).
    case("fianza dos mensualidades", "escaneo0034.docx"),
    case("arrendataria Muñoz", "escaneo0034.docx"),
    case("peñalara calle", "escaneo0034.docx"),
    case("potencia contratada", "doc_2020_07.txt"),
    case("factura electricidad junio", "doc_2020_07.txt"),
    case("caldera presión", "nota_rapida.txt"),
    case("técnico Junkers", "nota_rapida.txt"),
    case("cocido garbanzos", "receta_abuela.docx"),
    case("garbanzo remojado", "receta_abuela.docx"),
    case("morcilla chorizo", "receta_abuela.docx"),
    case("derrama ascensor", "acta.pdf"),
    case("junta propietarios Alcalá", "acta.pdf"),
    case("renovación póliza hogar", "carta_seguro.docx"),
    case("seguro robo responsabilidad civil", "carta_seguro.docx"),
    case("declaración renta devolver", "renta2022.pdf"),
    case("deducción vivienda habitual", "renta2022.pdf"),
    case("euríbor amortización anticipada", "hipoteca_bbva.pdf"),
    case("notario prestatarios", "hipoteca_bbva.pdf"),
    case("metformina glucosa", "medico.txt"),
    case("endocrina analitica", "medico.txt"),
    case("Covadonga Cares", "viaje_asturias.docx"),
    case("\"bajada del Sella\"", "viaje_asturias.docx"),
    case("garantía lavadora Bosch", "garantia_lavadora.pdf"),
    case("excursión Segovia autorización", "colegio.txt"),
    case("impuesto bienes inmuebles", "ibi.pdf"),
    case("referencia catastral", "ibi.pdf"),
    case("vacunas perro", "perro.docx"),
    case("veterinario desparasitación", "perro.docx"),
    case("pensión jubilación cotizados", "jubilacion.txt"),
    // BOE gazette PDFs.
    case(
        "Universidad de Córdoba profesores titulares",
        "BOE-A-2022-10000.pdf",
    ),
    case("Auxilio Judicial Tortosa", "BOE-A-2021-8000.pdf"),
    case(
        "libre designación Trabajo Economía Social",
        "BOE-A-2024-1.pdf",
    ),
    case("Monzón Jardín de Infancia", "BOE-A-2024-100.pdf"),
    case(
        "Castilla-La Mancha prácticas Francia",
        "BOE-A-2024-12000.pdf",
    ),
    case(
        "Sabiñánigo operario matarife albañil",
        "BOE-A-2024-5000.pdf",
    ),
    // Wikipedia excerpts.
    case("huevo batido sartén patatas", "wiki_tortilla.docx"),
    case("cigüeña blanca envergadura", "wiki_ciguena.pdf"),
    case("sierra de Guadarrama montaña", "wiki_penalara.pdf"),
    case("bienestar térmico climatización", "wiki_calefaccion.docx"),
    case(
        "préstamo hipotecario entidad financiera",
        "wiki_hipoteca.pdf",
    ),
    case(
        "herederos legatarios última voluntad",
        "wiki_testamento.docx",
    ),
    case("locación renta precio cierto", "wiki_arrendamiento.docx"),
    case("IRPF progresivo personas físicas", "wiki_irpf.pdf"),
    case(
        "copropietarios elementos privativos",
        "wiki_propiedad_horizontal.docx",
    ),
    case("arroz Comunidad Valenciana", "wiki_paella.pdf"),
];

/// VFS-2 AC-17: the expected document is in the top 5 for at least `MIN_SHARE` of the
/// golden queries; every miss is listed.
#[test]
fn golden_content_queries_find_the_expected_document() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Archivo");
    std::fs::create_dir(&root).unwrap();
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    for e in std::fs::read_dir(&corpus).unwrap().flatten() {
        let name = e.file_name();
        if name != "NOTICE.md" {
            std::fs::copy(e.path(), root.join(&name)).unwrap();
        }
    }
    let db = tmp.path().join("index.db");
    let vfs = Vfs::open(&db).unwrap();
    vfs.add_root(&root).unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    let opts = IndexOptions::new(env!("CARGO_BIN_EXE_vfs-extract"), Vec::new());
    let sum = vfs_content::index(&db, &opts, &AtomicBool::new(false)).unwrap();
    assert_eq!(sum.failed, 0, "{sum:?}");

    let searcher = Searcher::open(&db).unwrap();
    let mut misses = Vec::new();
    for c in CASES {
        let r = searcher
            .retrieve(
                &Query::Text(c.query.into()),
                &SearchFilters::default(),
                TOP as u32,
                0,
            )
            .unwrap();
        if !r.hits.iter().any(|h| h.name == c.expected) {
            let got: Vec<&str> = r.hits.iter().map(|h| h.name.as_str()).collect();
            misses.push(format!("«{}» → want {}, got {got:?}", c.query, c.expected));
        }
    }
    let share = (CASES.len() - misses.len()) as f64 / CASES.len() as f64;
    assert!(
        share >= MIN_SHARE,
        "share {share:.3} < {MIN_SHARE}; misses:\n{}",
        misses.join("\n")
    );
}

/// VFS-2 AC-17: the golden set has about 40 cases, each naming a document of the corpus.
#[test]
fn golden_content_set_is_well_formed() {
    assert!(CASES.len() >= 40, "{} cases", CASES.len());
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    for c in CASES {
        assert!(
            corpus.join(c.expected).is_file(),
            "«{}»: {}",
            c.query,
            c.expected
        );
    }
    assert!(corpus.join("NOTICE.md").is_file());
}
