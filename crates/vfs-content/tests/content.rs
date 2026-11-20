//! End-to-end tests of the VFS-2 content pipeline: name index from `vfs-core`, extraction in
//! the real worker binary, the content store, and content search.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use rusqlite::Connection;
use vfs_content::protocol::{FailReason, SkipReason};
use vfs_content::{store, IndexOptions};
use vfs_core::content::ContentState;
use vfs_core::{FileKind, SearchFilters, SearchQuery, Vfs};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct Setup {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
    vfs: Vfs,
}

fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Archivo");
    std::fs::create_dir(&root).unwrap();
    let db = tmp.path().join("index.db");
    let vfs = Vfs::open(&db).unwrap();
    Setup {
        _tmp: tmp,
        root,
        db,
        vfs,
    }
}

impl Setup {
    fn write(&self, rel: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn copy_fixture(&self, name: &str, rel: &str) -> PathBuf {
        self.write(rel, std::fs::read(Path::new(FIXTURES).join(name)).unwrap())
    }

    /// Adds the root (first call) and brings the name index up to date.
    fn scan(&self) {
        if self.vfs.list_roots().unwrap().is_empty() {
            self.vfs.add_root(&self.root).unwrap();
        }
        self.vfs
            .scan_to_completion(&AtomicBool::new(false))
            .unwrap();
    }

    fn opts(&self) -> IndexOptions {
        // Default limits (30 s per file): debug builds on slow CI runners stay well inside.
        IndexOptions::new(env!("CARGO_BIN_EXE_vfs-extract"), Vec::new())
    }

    fn run(&self, opts: &IndexOptions) -> vfs_content::RunSummary {
        vfs_content::index(&self.db, opts, &AtomicBool::new(false)).unwrap()
    }

    fn search(&self, text: &str) -> vfs_content::ContentResponse {
        self.search_with(text, SearchFilters::default())
    }

    fn search_with(&self, text: &str, filters: SearchFilters) -> vfs_content::ContentResponse {
        vfs_content::search(
            &self.db,
            &SearchQuery {
                text: text.into(),
                filters,
                limit: 100,
                offset: 0,
            },
        )
        .unwrap()
    }

    fn names(&self, text: &str) -> Vec<String> {
        let mut v: Vec<String> = self.search(text).hits.into_iter().map(|h| h.name).collect();
        v.sort();
        v
    }

    /// (content_state in index.db, docs.state, docs.reason) of a file by name.
    fn state(&self, name: &str) -> (i64, Option<i64>, Option<String>) {
        if !store::exists(&self.db) {
            let conn = Connection::open(&self.db).unwrap();
            let st = conn
                .query_row(
                    "SELECT content_state FROM files WHERE name = ?1",
                    [name],
                    |r| r.get(0),
                )
                .unwrap();
            return (st, None, None);
        }
        let conn = store::open_index_ro(&self.db).unwrap();
        conn.query_row(
            "SELECT f.content_state, d.state, d.reason FROM files f
             LEFT JOIN c.docs d ON d.file_id = f.id WHERE f.name = ?1",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }
}

fn reason<T: serde::Serialize>(r: T) -> Option<String> {
    serde_json::to_value(r).unwrap().as_str().map(String::from)
}

const INDEXED: i64 = ContentState::Indexed as i64;
const FAILED: i64 = ContentState::Failed as i64;
const SKIPPED: i64 = ContentState::Skipped as i64;
const NONE: i64 = ContentState::None as i64;

/// VFS-2 AC-1: only documents are opened (photo and audio keep content_state 0 and get no
/// content row) and every user file is byte-identical afterwards.
#[test]
fn ac1_only_documents_and_files_untouched() {
    let s = setup();
    let files = [
        s.copy_fixture("muestra_docx.docx", "a/contrato.docx"),
        s.copy_fixture("muestra_pdf.pdf", "a/contrato.pdf"),
        s.copy_fixture("muestra_xlsx.xlsx", "a/gastos.xlsx"),
        s.write("fotos/IMG_0001.jpg", b"\xFF\xD8\xFF\xE0 not really a jpeg"),
        s.write("musica/tema.mp3", b"ID3 not really audio"),
    ];
    let before: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let mtimes: Vec<SystemTime> = files
        .iter()
        .map(|p| std::fs::metadata(p).unwrap().modified().unwrap())
        .collect();
    s.scan();
    let sum = s.run(&s.opts());
    assert_eq!(sum.files_opened, 3);
    assert_eq!(s.state("IMG_0001.jpg"), (0, None, None));
    assert_eq!(s.state("tema.mp3"), (0, None, None));
    assert_eq!(
        s.state("contrato.docx").0,
        INDEXED,
        "{:?} {:?}",
        sum,
        s.state("contrato.docx")
    );
    for (i, p) in files.iter().enumerate() {
        assert_eq!(std::fs::read(p).unwrap(), before[i], "{p:?} changed");
        assert_eq!(std::fs::metadata(p).unwrap().modified().unwrap(), mtimes[i]);
    }
}

/// VFS-2 AC-2: a word of the known sentence finds every format, with accents intact in the
/// snippet; each file is found by its own marker word too.
#[test]
fn ac2_every_format_is_searchable() {
    let s = setup();
    let formats = [
        "docx", "doc", "pdf", "xlsx", "xls", "pptx", "ppt", "odt", "rtf", "txt",
    ];
    for f in formats {
        s.copy_fixture(
            &format!("muestra_{f}.{f}"),
            &format!("docs/muestra_{f}.{f}"),
        );
    }
    s.scan();
    let sum = s.run(&s.opts());
    assert_eq!(sum.indexed, formats.len() as u64, "{:?}", sum.reasons);
    let r = s.search("cigüeña");
    assert_eq!(r.hits.len(), formats.len());
    for h in &r.hits {
        assert!(h.snippet.contains("cigüeña"), "{}: {}", h.name, h.snippet);
        assert!(h.snippet.contains("Peñalara"), "{}: {}", h.name, h.snippet);
    }
    for f in formats {
        assert_eq!(
            s.names(&format!("fixtura{f}")),
            vec![format!("muestra_{f}.{f}")]
        );
    }
}

/// VFS-2 AC-3: a worker that hangs is killed within the limit plus one second, the file is
/// marked failed (timeout) and the next file is still indexed; the timeout is retried once
/// on a later run and then never again (AC-6).
#[test]
fn ac3_hanging_worker_is_killed() {
    let s = setup();
    s.write("a/colgado.txt", "este archivo cuelga al extractor");
    s.write("b/siguiente.txt", "la cigüeña sigue aquí");
    s.scan();
    let mut o = s.opts();
    o.worker.timeout = Duration::from_secs(4);
    o.worker.args = vec!["--test-hook=hang:colgado.txt".into()];
    let t = std::time::Instant::now();
    let sum = s.run(&o);
    // The limit plus one second for the hung file, plus the normal file (a debug build of
    // the worker extracting a few words, under parallel tests).
    assert!(
        t.elapsed() < Duration::from_secs(4 + 1 + 3),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(
        s.state("colgado.txt"),
        (FAILED, Some(FAILED), reason(FailReason::Timeout))
    );
    assert_eq!(s.state("siguiente.txt").0, INDEXED);
    assert_eq!(sum.worker_starts, 2, "{sum:?}");
    assert_eq!(s.names("cigüeña"), vec!["siguiente.txt"]);

    // A timeout is retried once on a later run, then the failure is final.
    let retry = s.run(&o);
    assert_eq!((retry.queued, retry.files_opened, retry.failed), (1, 1, 1));
    let last = s.run(&o);
    assert_eq!((last.queued, last.files_opened), (0, 0));
    assert_eq!(s.state("colgado.txt").0, FAILED);
}

/// VFS-2 AC-4: truncated, corrupted, password-protected, empty and zip-bomb documents end
/// failed or skipped (passwords and empty text skipped) and the valid files are indexed.
#[test]
fn ac4_hostile_documents_never_stop_the_run() {
    let s = setup();
    let docx = std::fs::read(Path::new(FIXTURES).join("muestra_docx.docx")).unwrap();
    let pdf = std::fs::read(Path::new(FIXTURES).join("muestra_pdf.pdf")).unwrap();
    s.write("h/truncado.docx", &docx[..docx.len() / 2]);
    s.write("h/truncado.pdf", &pdf[..100]);
    let mut noisy = docx.clone();
    for i in (0..noisy.len()).step_by(37) {
        noisy[i] ^= 0x5A;
    }
    s.write("h/corrupto.docx", &noisy);
    s.write("h/vacio.pdf", b"");
    s.write(
        "h/aleatorio.xls",
        (0..20_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect::<Vec<u8>>(),
    );
    s.copy_fixture("con_clave.docx", "h/con_clave.docx");
    s.copy_fixture("con_clave.pdf", "h/con_clave.pdf");
    s.copy_fixture("escaneado.pdf", "h/escaneado.pdf");
    s.write("h/bomba.docx", zip_bomb());
    s.copy_fixture("muestra_docx.docx", "ok/bueno.docx");
    s.copy_fixture("muestra_pdf.pdf", "ok/bueno.pdf");
    s.scan();
    let sum = s.run(&s.opts());
    assert_eq!(sum.remaining, 0);
    for n in [
        "truncado.docx",
        "truncado.pdf",
        "corrupto.docx",
        "aleatorio.xls",
        "bomba.docx",
    ] {
        let st = s.state(n).0;
        assert!(st == FAILED || st == SKIPPED, "{n}: {st}");
    }
    assert_eq!(
        s.state("con_clave.docx"),
        (SKIPPED, Some(SKIPPED), reason(SkipReason::Password))
    );
    assert_eq!(
        s.state("con_clave.pdf"),
        (SKIPPED, Some(SKIPPED), reason(SkipReason::Password))
    );
    assert_eq!(s.state("vacio.pdf").0, SKIPPED);
    assert_eq!(
        s.state("escaneado.pdf"),
        (SKIPPED, Some(SKIPPED), reason(SkipReason::NoText))
    );
    assert_eq!(s.names("cigüeña"), vec!["bueno.docx", "bueno.pdf"]);
}

/// A DOCX whose document.xml inflates to 300 MB from well under 1 MB.
fn zip_bomb() -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    w.start_file("[Content_Types].xml", o).unwrap();
    std::io::Write::write_all(&mut w, br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
    w.start_file("word/document.xml", o).unwrap();
    std::io::Write::write_all(&mut w, br#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>"#).unwrap();
    let chunk = vec![b'a'; 1 << 20];
    for _ in 0..300 {
        std::io::Write::write_all(&mut w, &chunk).unwrap();
    }
    std::io::Write::write_all(&mut w, b"</w:t></w:r></w:p></w:body></w:document>").unwrap();
    w.finish().unwrap().into_inner()
}

/// VFS-2 AC-5: a worker over its memory cap is killed and the file marked failed (memory);
/// text over the output cap is stored only up to the cap.
#[test]
fn ac5_memory_cap_and_output_cap() {
    let s = setup();
    s.write(
        "a/gloton.txt",
        "este archivo hace que el extractor pida memoria sin fin",
    );
    let big: String = "la cigüeña del campanario ".repeat(20_000); // ~540 KB
    s.write("b/grande.txt", &big);
    s.scan();
    let mut o = s.opts();
    o.worker.mem_cap = 256 << 20;
    o.max_text_bytes = 50_000;
    o.worker.args = vec!["--test-hook=alloc:gloton.txt".into()];
    let sum = s.run(&o);
    assert_eq!(
        s.state("gloton.txt"),
        (FAILED, Some(FAILED), reason(FailReason::Memory))
    );
    assert_eq!(s.state("grande.txt").0, INDEXED);
    assert_eq!(sum.truncated, 1);
    let conn = Connection::open(store::store_path(&s.db)).unwrap();
    let blob: Vec<u8> = conn
        .query_row("SELECT text FROM docs WHERE name = 'grande.txt'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let text = store::decompress(&blob);
    assert!(
        text.len() <= 50_000 && text.len() > 49_000,
        "{}",
        text.len()
    );
}

/// VFS-2 AC-6: an unchanged index opens no file; a changed file alone is extracted again and
/// its new text replaces the old; an unchanged failed file is not opened again.
#[test]
fn ac6_only_changed_files_are_read_again() {
    let s = setup();
    let changing = s.write("a/notas.txt", "texto viejo sobre la caldera");
    s.write("a/otra.txt", "otra nota sobre el garaje");
    s.write("a/roto.pdf", b"%PDF-1.4 truncated garbage");
    s.scan();
    let first = s.run(&s.opts());
    assert_eq!(first.files_opened, 3);
    assert_eq!(s.state("roto.pdf").0, FAILED);

    let again = s.run(&s.opts());
    assert_eq!(
        (again.files_opened, again.bytes_read, again.queued),
        (0, 0, 0)
    );

    std::fs::write(&changing, "texto nuevo sobre la piscina comunitaria").unwrap();
    let later = SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&changing)
        .unwrap()
        .set_modified(later)
        .unwrap();
    // The watcher reports paths under the stored (canonical) root path.
    let root = PathBuf::from(&s.vfs.list_roots().unwrap()[0].path);
    s.vfs.mark_dirty(&[root.join("a/notas.txt")]).unwrap();
    s.scan();
    let third = s.run(&s.opts());
    assert_eq!(third.files_opened, 1);
    assert_eq!(s.names("piscina"), vec!["notas.txt"]);
    assert!(s.names("caldera").is_empty());
    assert_eq!(s.names("garaje"), vec!["otra.txt"]);
}

/// VFS-2 AC-7: a run stopped after N files leaves the rest queued; the next run processes
/// only the remaining files.
#[test]
fn ac7_interrupted_run_resumes() {
    let s = setup();
    for i in 0..6 {
        s.write(
            &format!("d/nota{i}.txt"),
            format!("nota número {i} sobre la hipoteca"),
        );
    }
    s.scan();
    let mut o = s.opts();
    o.max_files = Some(2);
    o.batch_size = 1;
    let first = s.run(&o);
    assert_eq!((first.indexed, first.remaining), (2, 4));
    let second = s.run(&s.opts());
    assert_eq!(
        (
            second.queued,
            second.files_opened,
            second.indexed,
            second.remaining
        ),
        (4, 4, 4, 0)
    );
    assert_eq!(s.search("hipoteca").hits.len(), 6);
}

/// VFS-2 AC-8: stemming, accent and case folding, phrases (and not words apart), BM25
/// order, and highlight offsets on the matched words.
#[test]
fn ac8_spanish_queries() {
    let s = setup();
    s.copy_fixture("muestra_docx.docx", "a/muestra.docx");
    s.write(
        "a/separado.txt",
        "El contrato fue firmado ayer. Del alquiler hablaremos el lunes.",
    );
    s.write(
        "a/facturas.txt",
        "Factura. Factura de la luz, factura del agua y factura del gas.",
    );
    s.write(
        "a/una.txt",
        "Una sola factura entre muchas otras palabras sin relación ninguna con nada.",
    );
    s.scan();
    s.run(&s.opts());
    let found = s.names("facturas");
    assert!(found.contains(&"facturas.txt".into()) && found.contains(&"muestra.docx".into()));
    assert_eq!(s.names("accion"), vec!["muestra.docx"]);
    assert_eq!(s.names("PEÑALARA"), vec!["muestra.docx"]);
    assert_eq!(s.names("penalara"), vec!["muestra.docx"]);
    assert_eq!(s.names("\"contrato de alquiler\""), vec!["muestra.docx"]);
    let mut apart = s.names("contrato alquiler");
    apart.sort();
    assert_eq!(apart, vec!["muestra.docx", "separado.txt"]);

    let r = s.search("factura");
    let pos = |n: &str| r.hits.iter().position(|h| h.name == n).unwrap();
    assert!(pos("facturas.txt") < pos("una.txt"));
    assert!(r.hits.windows(2).all(|w| w[0].score >= w[1].score));
    for h in &r.hits {
        assert!(!h.highlights.is_empty());
        for [b, e] in &h.highlights {
            assert!(
                h.snippet[*b..*e].to_lowercase().starts_with("factura"),
                "{}",
                &h.snippet[*b..*e]
            );
        }
    }
}

/// VFS-2 AC-9: kind, date, size and folder filters narrow content hits like name search.
#[test]
fn ac9_filters_combine() {
    let s = setup();
    let old = s.write(
        "Casa/Reforma/presupuesto.txt",
        "presupuesto de la reforma de la cocina",
    );
    s.write(
        "Banco/extracto.txt",
        "la reforma se pagó con la hipoteca y mucho texto más para que pese bastante",
    );
    let t2019 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_560_000_000); // 2019-06
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(t2019)
        .unwrap();
    s.scan();
    s.run(&s.opts());
    let all = |f: SearchFilters| -> Vec<String> {
        let mut v: Vec<String> = s
            .search_with("reforma", f)
            .hits
            .into_iter()
            .map(|h| h.name)
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        all(SearchFilters::default()),
        vec!["extracto.txt", "presupuesto.txt"]
    );
    assert!(all(SearchFilters {
        kinds: vec![FileKind::Photo],
        ..Default::default()
    })
    .is_empty());
    assert_eq!(
        all(SearchFilters {
            kinds: vec![FileKind::Document],
            ..Default::default()
        })
        .len(),
        2
    );
    assert_eq!(
        all(SearchFilters {
            modified_to: Some(1_577_836_800),
            ..Default::default()
        }), // < 2020
        vec!["presupuesto.txt"]
    );
    assert_eq!(
        all(SearchFilters {
            modified_from: Some(1_577_836_800),
            ..Default::default()
        }),
        vec!["extracto.txt"]
    );
    assert_eq!(
        all(SearchFilters {
            size_min: Some(60),
            ..Default::default()
        }),
        vec!["extracto.txt"]
    );
    assert_eq!(
        all(SearchFilters {
            size_max: Some(59),
            ..Default::default()
        }),
        vec!["presupuesto.txt"]
    );
    // The stored root path (canonical: /private/var on macOS), its child in another case.
    let root = PathBuf::from(&s.vfs.list_roots().unwrap()[0].path);
    let casa = root.join("casa").to_string_lossy().into_owned();
    assert_eq!(
        all(SearchFilters {
            folder: Some(casa),
            ..Default::default()
        }),
        vec!["presupuesto.txt"]
    );
    let elsewhere = root.join("Viajes").to_string_lossy().into_owned();
    assert!(all(SearchFilters {
        folder: Some(elsewhere),
        ..Default::default()
    })
    .is_empty());
}

/// VFS-2 AC-10: without content search no store exists, no content state is written and
/// name search works; deleting the store switches content search off and name search goes on.
#[test]
fn ac10_off_by_default_and_deletable() {
    let s = setup();
    s.copy_fixture("muestra_docx.docx", "a/contrato.docx");
    s.scan();
    let q = SearchQuery {
        text: "contrato".into(),
        filters: SearchFilters::default(),
        limit: 10,
        offset: 0,
    };
    let before = s.vfs.search(&q).unwrap();
    assert_eq!(before.hits.len(), 1);
    assert!(!store::exists(&s.db));
    assert!(matches!(
        vfs_content::search(&s.db, &q),
        Err(vfs_content::Error::Off)
    ));
    assert_eq!(s.state("contrato.docx").0, NONE);

    s.run(&s.opts());
    assert_eq!(s.search("cigüeña").hits.len(), 1);
    let during: Vec<i64> = s
        .vfs
        .search(&q)
        .unwrap()
        .hits
        .iter()
        .map(|h| h.id)
        .collect();
    assert_eq!(during, before.hits.iter().map(|h| h.id).collect::<Vec<_>>());

    vfs_content::switch_off(&s.db).unwrap();
    assert!(!store::exists(&s.db));
    assert_eq!(s.state("contrato.docx").0, NONE);
    assert!(matches!(
        vfs_content::search(&s.db, &q),
        Err(vfs_content::Error::Off)
    ));
    assert_eq!(s.vfs.search(&q).unwrap().hits.len(), 1);
}

/// VFS-2 AC-12: hits carry passages addressed by chunk ordinal, character offsets into the
/// stored text and page; the offsets select text that contains the matched word.
#[test]
fn ac12_passages_are_addressable() {
    let s = setup();
    let filler = "Párrafo de relleno sobre la comunidad de propietarios y la derrama. ".repeat(80);
    s.write(
        "a/largo.txt",
        format!("{filler}\n\nAl final aparece el pingüino de Peñalara.\n"),
    );
    s.copy_fixture("muestra_pdf.pdf", "a/muestra.pdf");
    s.copy_fixture("muestra_docx.docx", "a/muestra.docx");
    s.scan();
    s.run(&s.opts());

    let conn = Connection::open(store::store_path(&s.db)).unwrap();
    let stored = |name: &str| -> String {
        let blob: Vec<u8> = conn
            .query_row("SELECT text FROM docs WHERE name = ?1", [name], |r| {
                r.get(0)
            })
            .unwrap();
        store::decompress(&blob)
    };
    let r = s.search("pingüino");
    let h = &r.hits[0];
    assert_eq!(h.name, "largo.txt");
    let p = &h.passages[0];
    assert!(p.ordinal > 0, "match is near the end of a long text");
    assert_eq!(p.page, None);
    let text = stored("largo.txt");
    let cited: String = text
        .chars()
        .skip(p.start_char)
        .take(p.end_char - p.start_char)
        .collect();
    assert!(cited.contains("pingüino"), "{cited}");
    assert_eq!(h.snippet, p.snippet);

    let r = s.search("cigüeña");
    let pdf = r.hits.iter().find(|h| h.name == "muestra.pdf").unwrap();
    assert_eq!(pdf.passages[0].page, Some(1));
    // Xberg also splits Word documents into pages; plain text has none (checked above).
    let docx = r.hits.iter().find(|h| h.name == "muestra.docx").unwrap();
    assert!(!docx.passages.is_empty());
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM chunks c JOIN docs d USING(file_id) WHERE d.name = 'largo.txt'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(n >= 3, "{n}");
}
