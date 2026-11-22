//! End-to-end tests of the VFS-2 content pipeline: name index from `vfs-core`, extraction in
//! the real worker binary, the content store, and content search.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use rusqlite::Connection;
use vfs_content::indexer::{OutcomeKind, StopReason};
use vfs_content::protocol::{FailReason, SkipReason};
use vfs_content::{store, IndexOptions};
use vfs_core::content::ContentState;
use vfs_core::db::ingest::FLAG_CLOUD_ONLY;
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
                sort: Default::default(),
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
        (SKIPPED, Some(SKIPPED), reason(SkipReason::Scanned))
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
        sort: Default::default(),
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

// ---- PR 2a: cloud-only files, run-level stops, worker lifetime, store version, budget ----

impl Setup {
    /// Runs SQL on `index.db` directly, as the name indexer or the settings would.
    /// `?1` is the file name, `?2` the index's cloud-only flag bit.
    fn index_sql(&self, sql: &str, name: &str) {
        let conn = Connection::open(&self.db).unwrap();
        conn.busy_timeout(Duration::from_secs(10)).unwrap();
        conn.execute(sql, rusqlite::params![name, FLAG_CLOUD_ONLY])
            .unwrap();
    }

    fn content_states(&self) -> Vec<(String, i64)> {
        let conn = Connection::open(&self.db).unwrap();
        let mut st = conn
            .prepare("SELECT name, content_state FROM files WHERE kind = ?1 ORDER BY name")
            .unwrap();
        st.query_map([FileKind::Document.code()], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// Path of an open-log file for the `openlog` worker hook.
    fn log_path(&self) -> PathBuf {
        self._tmp.path().join("opens.log")
    }

    fn opened(&self) -> Vec<String> {
        std::fs::read_to_string(self.log_path())
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

const QUEUED: i64 = ContentState::Queued as i64;

/// VFS-2 AC-13: a document flagged cloud-only in the index is never opened, ends skipped
/// "in the cloud", and is still found by name; once the flag clears and its mtime changes,
/// the next run extracts it.
#[test]
fn ac13_cloud_only_documents_are_never_opened() {
    let s = setup();
    s.copy_fixture("muestra_docx.docx", "a/nube.docx");
    s.copy_fixture("muestra_docx.docx", "a/local.docx");
    s.scan();
    s.index_sql(
        "UPDATE files SET flags = flags | ?2 WHERE name = ?1",
        "nube.docx",
    );
    let mut o = s.opts();
    o.worker.args = vec![format!("--test-hook=openlog:{}", s.log_path().display())];
    let sum = s.run(&o);
    assert_eq!(
        s.state("nube.docx"),
        (SKIPPED, Some(SKIPPED), reason(SkipReason::Cloud))
    );
    assert_eq!(s.state("local.docx").0, INDEXED);
    assert!(
        !s.opened().iter().any(|l| l.ends_with("nube.docx")),
        "{:?}",
        s.opened()
    );
    assert_eq!(sum.files_opened, 1, "{sum:?}");
    let by_name = s
        .vfs
        .search(&SearchQuery {
            text: "nube".into(),
            filters: SearchFilters::default(),
            limit: 10,
            offset: 0,
            sort: Default::default(),
        })
        .unwrap();
    assert_eq!(by_name.hits.len(), 1);

    // The sync client downloads it: the name indexer clears the flag and records the new
    // mtime.
    s.index_sql(
        "UPDATE files SET flags = flags & ~?2, mtime = mtime + 10 WHERE name = ?1",
        "nube.docx",
    );
    s.run(&o);
    assert_eq!(s.state("nube.docx").0, INDEXED);
    assert!(s.opened().iter().any(|l| l == "open nube.docx"));
}

/// VFS-2 AC-13 (Windows): a file given the offline attribute after the walk is checked just
/// before the open and not opened.
#[cfg(windows)]
#[test]
fn ac13_offline_attribute_after_walk_is_not_opened() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        SetFileAttributesW, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_OFFLINE,
    };
    let s = setup();
    let p = s.copy_fixture("muestra_docx.docx", "a/nube.docx");
    s.scan();
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: NUL-terminated path that outlives the call.
    let ok = unsafe {
        SetFileAttributesW(
            wide.as_ptr(),
            FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_ARCHIVE,
        )
    };
    assert_ne!(ok, 0);
    let mut o = s.opts();
    o.worker.args = vec![format!("--test-hook=openlog:{}", s.log_path().display())];
    let sum = s.run(&o);
    assert_eq!(
        s.state("nube.docx"),
        (SKIPPED, Some(SKIPPED), reason(SkipReason::Cloud))
    );
    assert_eq!(sum.files_opened, 0);
    assert!(s.opened().is_empty(), "{:?}", s.opened());
}

/// VFS-2 AC-20: with the worker binary missing the run stops with "worker cannot start"
/// and no file changes state.
#[test]
fn ac20_missing_worker_stops_the_run_without_changes() {
    let s = setup();
    s.copy_fixture("muestra_docx.docx", "a/contrato.docx");
    s.write("a/notas.txt", "una nota");
    s.scan();
    let before = s.content_states();
    let mut o = s.opts();
    o.worker.program = s._tmp.path().join("no-such-worker");
    let sum = s.run(&o);
    assert_eq!(sum.stopped, Some(StopReason::WorkerCannotStart), "{sum:?}");
    assert_eq!(s.content_states(), before);
    assert!(before.iter().all(|(_, st)| *st == NONE));
}

/// VFS-2 AC-20, VFS-27 AC-1: a worker reporting another version stops the run with
/// "version mismatch", and no file changes state.
#[test]
fn ac20_version_mismatch_stops_the_run() {
    let s = setup();
    s.write("a/notas.txt", "una nota");
    s.scan();
    let mut o = s.opts();
    o.worker.args = vec!["--test-hook=version:0.0.0+p0".into()];
    let sum = s.run(&o);
    assert_eq!(sum.stopped, Some(StopReason::VersionMismatch), "{sum:?}");
    assert_eq!(s.state("notas.txt").0, NONE);
}

/// Text of about `words` pseudo-random Spanish-looking words: compresses poorly, so it
/// needs many store pages.
fn noisy_text(words: usize) -> String {
    let syl = [
        "ma", "ra", "lo", "pe", "ci", "ta", "no", "che", "vi", "su", "ñu", "go",
    ];
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    let mut out = String::new();
    for _ in 0..words {
        for _ in 0..3 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            out.push_str(syl[(x % syl.len() as u64) as usize]);
        }
        out.push(' ');
    }
    out
}

/// VFS-2 AC-20: when a store write fails with a full disk the run stops and the document
/// being written stays queued.
#[test]
fn ac20_full_store_stops_and_keeps_the_document_queued() {
    let s = setup();
    s.write("a/largo.txt", noisy_text(150_000));
    s.scan();
    let mut o = s.opts();
    o.max_store_pages = Some(40);
    let sum = s.run(&o);
    assert_eq!(sum.stopped, Some(StopReason::StoreWrite), "{sum:?}");
    assert_eq!(s.state("largo.txt"), (QUEUED, None, None));
    let conn = store::open_index_ro(&s.db).unwrap();
    let queued: i64 = conn
        .query_row("SELECT count(*) FROM c.queue", [], |r| r.get(0))
        .unwrap();
    assert_eq!(queued, 1);
}

/// VFS-2 AC-20: five consecutive files that crash the worker stop the run with "worker
/// unstable"; the file after them is not processed.
#[test]
fn ac20_five_crashes_in_a_row_stop_the_run() {
    let s = setup();
    let mut args = Vec::new();
    for i in 1..=5 {
        s.write(&format!("a/c{i}.txt"), "texto");
        args.push(format!("--test-hook=crash:c{i}.txt"));
    }
    s.write("a/d.txt", "después");
    s.scan();
    let mut o = s.opts();
    o.worker.args = args;
    let sum = s.run(&o);
    assert_eq!(sum.stopped, Some(StopReason::WorkerUnstable), "{sum:?}");
    assert_eq!(
        sum.reasons.get(&OutcomeKind::Failed(FailReason::Crash)),
        Some(&5)
    );
    assert_eq!(s.state("d.txt").0, QUEUED);
}

/// VFS-2 AC-20: five consecutive timeouts are the documents' own failures and do not stop
/// the run.
#[test]
fn ac20_five_timeouts_do_not_stop_the_run() {
    let s = setup();
    let mut args = Vec::new();
    for i in 1..=5 {
        s.write(&format!("a/t{i}.txt"), "texto");
        args.push(format!("--test-hook=hang:t{i}.txt"));
    }
    s.write("a/u.txt", "la cigüeña");
    s.scan();
    let mut o = s.opts();
    o.worker.timeout = Duration::from_secs(2);
    o.worker.args = args;
    let sum = s.run(&o);
    assert_eq!(sum.stopped, None, "{sum:?}");
    assert_eq!(
        sum.reasons.get(&OutcomeKind::Failed(FailReason::Timeout)),
        Some(&5)
    );
    assert_eq!(s.state("u.txt").0, INDEXED);
}

/// VFS-2 AC-20: with one of two roots offline, the other root's documents are read and the
/// offline root's keep their state; once it is back, they are read.
#[test]
fn ac20_offline_root_is_passed_over() {
    let s = setup();
    s.write("a/presente.txt", "la cigüeña");
    let other = s._tmp.path().join("Disco");
    std::fs::create_dir_all(other.join("b")).unwrap();
    std::fs::write(other.join("b/ausente.txt"), "el señor Muñoz").unwrap();
    s.scan();
    s.vfs.add_root(&other).unwrap();
    s.scan();
    let away = s._tmp.path().join("Disco-desconectado");
    std::fs::rename(&other, &away).unwrap();
    let sum = s.run(&s.opts());
    assert_eq!(sum.stopped, None, "{sum:?}");
    assert_eq!(s.state("presente.txt").0, INDEXED);
    assert_eq!(s.state("ausente.txt"), (NONE, None, None));

    std::fs::rename(&away, &other).unwrap();
    s.run(&s.opts());
    assert_eq!(s.state("ausente.txt").0, INDEXED);
}

/// VFS-2 AC-20, R18: a worker whose parent is gone (its stdin pipe closed) exits within 2 s,
/// even in the middle of a file.
#[test]
fn ac20_worker_exits_when_the_parent_pipe_closes() {
    use std::io::{BufRead, Write};
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lento.txt");
    std::fs::write(&file, "texto").unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_vfs-extract"))
        .arg("--test-hook=hang:lento.txt")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut hello = String::new();
    out.read_line(&mut hello).unwrap();
    assert!(hello.contains("version"), "{hello}");
    let mut stdin = child.stdin.take().unwrap();
    let req = vfs_content::protocol::Request {
        path: file,
        max_input_bytes: 1 << 20,
        max_text_bytes: 1 << 20,
    };
    writeln!(stdin, "{}", serde_json::to_string(&req).unwrap()).unwrap();
    stdin.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    drop(stdin);
    let t = std::time::Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if t.elapsed() > Duration::from_secs(2) {
            let _ = child.kill();
            panic!("worker still running 2 s after its pipe closed");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// VFS-2 AC-21: a store with an older `user_version` is rebuilt, never read: its documents
/// are extracted again.
#[test]
fn ac21_older_store_is_rebuilt() {
    let s = setup();
    s.write("a/notas.txt", "la cigüeña");
    s.scan();
    assert_eq!(s.run(&s.opts()).indexed, 1);
    {
        let c = Connection::open(store::store_path(&s.db)).unwrap();
        c.pragma_update(None, "user_version", 0).unwrap();
    }
    let again = s.run(&s.opts());
    assert_eq!((again.queued, again.files_opened), (1, 1), "{again:?}");
    let conn = store::open_index_ro(&s.db).unwrap();
    let attempts: i64 = conn
        .query_row("SELECT attempts FROM c.docs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(attempts, 1);
}

/// Every column of `files` except `content_state`, by id.
fn files_without_content_state(db: &Path) -> Vec<Vec<String>> {
    let conn = Connection::open(db).unwrap();
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('files') WHERE name <> 'content_state'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let sql = format!(
        "SELECT {} FROM files ORDER BY id",
        cols.iter()
            .map(|c| format!("quote({c})"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut st = conn.prepare(&sql).unwrap();
    let n = cols.len();
    st.query_map([], |r| (0..n).map(|i| r.get::<_, String>(i)).collect())
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// VFS-2 AC-21, R20: the content indexer changes no column of `files` but `content_state`,
/// and a content run alongside a name pass fails neither with "database is locked".
#[test]
fn ac21_writes_only_content_state_and_runs_beside_a_name_pass() {
    let s = setup();
    for i in 0..30 {
        s.write(
            &format!("a/nota{i:02}.txt"),
            format!("nota número {i} del señor Muñoz"),
        );
    }
    s.copy_fixture("muestra_docx.docx", "a/contrato.docx");
    s.scan();
    let before = files_without_content_state(&s.db);
    s.run(&s.opts());
    assert_eq!(files_without_content_state(&s.db), before);

    // A second batch of documents, content-indexed while name passes run.
    for i in 0..30 {
        s.write(
            &format!("b/carta{i:02}.txt"),
            format!("carta número {i} de la cigüeña"),
        );
    }
    let root = PathBuf::from(&s.vfs.list_roots().unwrap()[0].path);
    s.vfs.mark_dirty(std::slice::from_ref(&root)).unwrap();
    s.scan();
    let db = s.db.clone();
    let namer = std::thread::spawn(move || {
        let vfs = Vfs::open(&db).unwrap();
        for i in 0..10 {
            std::fs::write(root.join(format!("b/extra{i}.txt")), "extra").unwrap();
            vfs.mark_dirty(&[root.join("b")]).unwrap();
            vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
        }
    });
    let sum = vfs_content::index(&s.db, &s.opts(), &AtomicBool::new(false)).unwrap();
    namer.join().unwrap();
    assert_eq!(sum.stopped, None, "{sum:?}");
    assert_eq!(s.state("carta29.txt").0, INDEXED);
}

/// VFS-2 AC-23: the worker runs at the OS's background priority: `PRIO_DARWIN_BG` on macOS,
/// the idle priority class on Windows.
#[test]
fn ac23_worker_runs_at_background_priority() {
    use std::io::BufRead;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_vfs-extract"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // The hello comes after the priority change.
    let mut hello = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut hello)
        .unwrap();
    let pid = child.id();
    // The worker reads its own priority back from the OS (macOS answers only the process
    // itself) and reports it in its hello.
    let hello: vfs_content::protocol::Hello = serde_json::from_str(&hello).unwrap();
    assert!(
        hello.background,
        "worker is not at background priority (OS reports {})",
        hello.priority
    );
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            GetPriorityClass, OpenProcess, IDLE_PRIORITY_CLASS, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY: the handle is opened, queried and closed here.
        let class = unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            assert!(!h.is_null());
            let c = GetPriorityClass(h);
            CloseHandle(h);
            c
        };
        assert_eq!(class, IDLE_PRIORITY_CLASS);
    }
    let _ = pid;
    drop(child.stdin.take());
    let _ = child.wait();
}

/// VFS-2 AC-23: at most one document is open at a time: the open log never shows a second
/// `open` before the previous `close`.
#[test]
fn ac23_one_document_open_at_a_time() {
    let s = setup();
    for n in ["a/uno.txt", "a/dos.txt", "b/tres.txt"] {
        s.write(n, "texto de prueba");
    }
    s.copy_fixture("muestra_pdf.pdf", "b/cuatro.pdf");
    s.scan();
    let mut o = s.opts();
    o.worker.args = vec![format!("--test-hook=openlog:{}", s.log_path().display())];
    s.run(&o);
    let log = s.opened();
    assert_eq!(log.len(), 8, "{log:?}");
    for pair in log.chunks(2) {
        let name = pair[0].strip_prefix("open ").expect("an open first");
        assert_eq!(pair[1], format!("close {name}"), "{log:?}");
    }
}

/// VFS-2 AC-24: a document removed from the index by the name indexer leaves the store at
/// the next run and is never returned again.
#[test]
fn ac24_removed_documents_leave_the_store() {
    let s = setup();
    let gone = s.write("a/borrada.txt", "la cigüeña de la torre");
    s.write("a/queda.txt", "la cigüeña del campanario");
    s.scan();
    s.run(&s.opts());
    assert_eq!(s.names("cigüeña"), vec!["borrada.txt", "queda.txt"]);
    std::fs::remove_file(&gone).unwrap();
    let root = PathBuf::from(&s.vfs.list_roots().unwrap()[0].path);
    s.vfs.mark_dirty(&[root.join("a")]).unwrap();
    s.scan();
    s.run(&s.opts());
    assert_eq!(s.names("cigüeña"), vec!["queda.txt"]);
    let conn = Connection::open(store::store_path(&s.db)).unwrap();
    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM docs WHERE name = 'borrada.txt'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
}

/// VFS-2 AC-24, R3, R21: the input budget comes from `index.db` `content_max_mb`; a document
/// skipped as too large is extracted once the budget is raised above its size.
#[test]
fn ac24_raised_size_budget_requeues_too_large() {
    let s = setup();
    // About 1.5 MB of text.
    s.write(
        "a/grande.txt",
        "la cigüeña vuela sobre la torre. ".repeat(48_000),
    );
    s.scan();
    let set_mb = |mb: &str| {
        let conn = Connection::open(&s.db).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
            [vfs_content::indexer::CONTENT_MAX_MB_KEY, mb],
        )
        .unwrap();
    };
    set_mb("1");
    let first = s.run(&s.opts());
    assert_eq!(first.files_opened, 0, "{first:?}");
    // Unread skips are counted under their own reason, and read as strings in JSON.
    assert_eq!(
        first
            .reasons
            .get(&OutcomeKind::Skipped(SkipReason::TooLarge)),
        Some(&1)
    );
    let json = serde_json::to_value(&first).unwrap();
    assert_eq!(json["reasons"]["skipped:too_large"], 1, "{json}");
    assert_eq!(
        s.state("grande.txt"),
        (SKIPPED, Some(SKIPPED), reason(SkipReason::TooLarge))
    );
    set_mb("2");
    let second = s.run(&s.opts());
    assert_eq!(second.files_opened, 1, "{second:?}");
    assert_eq!(s.state("grande.txt").0, INDEXED);
}

// ---- PR 2b: query semantics and the engine calls (R13–R16) ----

use vfs_content::{Query, Searcher, StructuredQuery, UnreadReason};

impl Setup {
    fn searcher(&self) -> Searcher {
        Searcher::open(&self.db).unwrap()
    }

    fn retrieve(&self, q: Query, k: u32, offset: u32) -> vfs_content::ContentResponse {
        self.searcher()
            .retrieve(&q, &SearchFilters::default(), k, offset)
            .unwrap()
    }

    fn file_id(&self, name: &str) -> i64 {
        Connection::open(&self.db)
            .unwrap()
            .query_row("SELECT id FROM files WHERE name = ?1", [name], |r| r.get(0))
            .unwrap()
    }
}

fn text(q: &str) -> Query {
    Query::Text(q.into())
}

fn structured(all: &[&str], any: &[&str]) -> Query {
    Query::Structured(StructuredQuery {
        all: all.iter().map(|s| s.to_string()).collect(),
        any: any.iter().map(|s| s.to_string()).collect(),
        phrases: Vec::new(),
    })
}

fn hit_names(r: &vfs_content::ContentResponse) -> Vec<String> {
    r.hits.iter().map(|h| h.name.clone()).collect()
}

/// VFS-2 AC-14: two words find only documents with both; when none has both, documents
/// with any of them come back, marked partial.
#[test]
fn ac14_and_by_default_with_partial_or_fallback() {
    let s = setup();
    s.write(
        "a/a.txt",
        "El contrato de alquiler exige una fianza de dos meses.",
    );
    s.write(
        "a/b.txt",
        "El alquiler del piso de la playa sube en verano.",
    );
    s.scan();
    s.run(&s.opts());
    let both = s.retrieve(text("alquiler fianza"), 10, 0);
    assert_eq!(hit_names(&both), vec!["a.txt"]);
    assert!(!both.partial);
    let either = s.retrieve(text("alquiler zapatilla"), 10, 0);
    assert!(
        hit_names(&either).contains(&"b.txt".to_string()),
        "{either:?}"
    );
    assert!(either.partial);
}

/// VFS-2 AC-15: a word in every document filters but does not rank, so `documento
/// Peñalara` ranks by `Peñalara`; `documento` alone returns at most `limit` hits within the
/// query budget.
#[test]
fn ac15_common_words_filter_but_do_not_rank() {
    let s = setup();
    for i in 0..2000 {
        let extra = match i {
            100 => " Peñalara.",
            700 => " Peñalara Peñalara Peñalara.",
            1500 => " Peñalara Peñalara.",
            _ => "",
        };
        s.write(
            &format!("d{:02}/doc{i:04}.txt", i / 100),
            format!("Este documento número {i} trata de cosas varias.{extra}"),
        );
    }
    s.scan();
    let sum = s.run(&s.opts());
    assert_eq!(sum.indexed, 2000, "{sum:?}");
    let r = s.retrieve(text("documento Peñalara"), 10, 0);
    assert_eq!(
        hit_names(&r),
        vec!["doc0700.txt", "doc1500.txt", "doc0100.txt"],
        "{r:?}"
    );
    assert!(!r.partial);

    let searcher = s.searcher();
    let mut ms = Vec::new();
    for _ in 0..20 {
        let t = std::time::Instant::now();
        let r = searcher
            .retrieve(&text("documento"), &SearchFilters::default(), 20, 0)
            .unwrap();
        ms.push(t.elapsed().as_millis());
        assert_eq!(r.hits.len(), 20);
        assert_eq!(r.total, 1000);
        assert!(r.total_capped);
    }
    ms.sort();
    // The VFS-2 query budget: p95 < 150 ms.
    assert!(ms[18] < 150, "p95 {} ms: {ms:?}", ms[18]);

    // A common word inside a folder whose documents come after the first 1,000 matches:
    // the filter, not the candidate cap, bounds the result (R7).
    let root = PathBuf::from(&s.vfs.list_roots().unwrap()[0].path);
    let filters = SearchFilters {
        folder: Some(root.join("d19").to_string_lossy().into_owned()),
        ..Default::default()
    };
    let r = searcher
        .retrieve(&text("documento"), &filters, 20, 0)
        .unwrap();
    assert_eq!((r.hits.len(), r.total), (20, 100), "{:?}", r.total);
    assert!(r.hits.iter().all(|h| h.name.starts_with("doc19")));
}

/// VFS-2 AC-16: `retrieve` returns what `vfs content-search` shows, each passage's range
/// holds a matched word, and `passage_text` joins neighbouring chunks in order.
#[test]
fn ac16_retrieve_and_passage_text() {
    let s = setup();
    let filler = "La comunidad revisó las cuentas del portal y del jardín. ".repeat(60);
    s.write(
        "a/largo.txt",
        format!("{filler}Al final se firmó el contrato de alquiler del local. {filler}"),
    );
    s.write("a/corto.txt", "Un contrato de alquiler sencillo.");
    s.write(
        "b/otro.txt",
        "El alquiler de la casa subió; el contrato sigue igual.",
    );
    s.scan();
    s.run(&s.opts());
    let searcher = s.searcher();
    let filters = SearchFilters::default();
    let r = searcher
        .retrieve(&text("contrato de alquiler"), &filters, 10, 0)
        .unwrap();
    let cli = s.search("contrato de alquiler");
    assert_eq!(
        r.hits.iter().map(|h| h.id).collect::<Vec<_>>(),
        cli.hits.iter().map(|h| h.id).collect::<Vec<_>>()
    );
    assert_eq!(r.hits.len(), 3);
    for h in &r.hits {
        assert!(!h.passages.is_empty(), "{h:?}");
        for p in &h.passages {
            assert_eq!(p.file_id, h.id);
            let t = searcher
                .passage_text(h.id, p.ordinal, 0, 0)
                .unwrap()
                .unwrap();
            assert_eq!((t.start_char, t.end_char), (p.start_char, p.end_char));
            let folded = vfs_core::fold_key(&t.text);
            assert!(
                ["contrat", "alquil"].iter().any(|w| folded.contains(w)),
                "{t:?}"
            );
        }
    }

    let id = s.file_id("largo.txt");
    let all = searcher.passage_text(id, 0, 0, 100).unwrap().unwrap();
    assert!(all.last_ordinal >= 2, "{all:?}");
    let mid = searcher.passage_text(id, 1, 1, 1).unwrap().unwrap();
    assert_eq!((mid.first_ordinal, mid.last_ordinal), (0, 2));
    let parts: String = (0..=2)
        .map(|o| searcher.passage_text(id, o, 0, 0).unwrap().unwrap().text)
        .collect();
    assert_eq!(mid.text, parts);
    let first = searcher.passage_text(id, 0, 1, 1).unwrap().unwrap();
    assert_eq!((first.first_ordinal, first.last_ordinal), (0, 1));
    assert!(searcher.passage_text(id, 999, 0, 0).unwrap().is_none());
}

/// VFS-2 AC-16: no crate other than `vfs-content` opens `content.db`; the others go through
/// its calls.
#[test]
fn ac16_only_vfs_content_opens_the_store() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut offenders = Vec::new();
    let mut stack = vec![repo.join("crates"), repo.join("apps")];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !["target", "node_modules", "vfs-content"].contains(&name.as_str()) {
                    stack.push(p);
                }
            } else if name.ends_with(".rs") {
                // Code only: a comment may name the store without opening it.
                let src = std::fs::read_to_string(&p).unwrap_or_default();
                let opens = src
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .any(|l| l.contains("content.db") || l.contains("vfs_content::store"));
                if opens {
                    offenders.push(p);
                }
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:?}");
}

/// VFS-2 AC-18: PDF and DOCX hits carry the title, author and creation date of their
/// metadata; a TXT hit carries none.
#[test]
fn ac18_hits_carry_document_metadata() {
    let s = setup();
    s.copy_fixture("meta_pdf.pdf", "a/informe.pdf");
    s.copy_fixture("meta_docx.docx", "a/inventario.docx");
    s.write("a/nota.txt", "Nota sobre el ascensor y la cómoda.");
    s.scan();
    s.run(&s.opts());
    let get = |q: &str, name: &str| {
        s.retrieve(text(q), 10, 0)
            .hits
            .into_iter()
            .find(|h| h.name == name)
            .unwrap_or_else(|| panic!("{name} not found for «{q}»"))
    };
    let pdf = get("ascensor", "informe.pdf");
    assert_eq!(pdf.title.as_deref(), Some("Informe de la comunidad"));
    assert_eq!(pdf.authors, vec!["Luis Pérez"]);
    assert!(pdf
        .created
        .as_deref()
        .unwrap_or("")
        .starts_with("2019-03-03"));
    let docx = get("cómoda", "inventario.docx");
    assert_eq!(docx.title.as_deref(), Some("Inventario de la casa"));
    assert_eq!(docx.authors, vec!["Carmen Muñoz"]);
    assert!(docx
        .created
        .as_deref()
        .unwrap_or("")
        .starts_with("2019-03-03"));
    let txt = get("ascensor", "nota.txt");
    assert_eq!((txt.title, txt.authors.len(), txt.created), (None, 0, None));
}

/// VFS-2 AC-19: `unread` names why a scanned PDF, a password-protected PDF, an empty DOCX,
/// a cloud-only file and a `.pages` file were not read, and `unread_counts` counts one of
/// each.
#[test]
fn ac19_unread_reasons() {
    let s = setup();
    s.copy_fixture("escaneado.pdf", "a/escaneado.pdf");
    s.copy_fixture("con_clave.pdf", "a/con_clave.pdf");
    s.copy_fixture("vacio.docx", "a/vacio.docx");
    s.copy_fixture("muestra_docx.docx", "a/nube.docx");
    s.write("a/presupuesto.pages", b"not really a Pages file");
    s.write("a/leido.txt", "texto leído");
    s.scan();
    s.index_sql(
        "UPDATE files SET flags = flags | ?2 WHERE name = ?1",
        "nube.docx",
    );
    s.run(&s.opts());
    let searcher = s.searcher();
    let names = [
        "escaneado.pdf",
        "con_clave.pdf",
        "vacio.docx",
        "nube.docx",
        "presupuesto.pages",
        "leido.txt",
    ];
    let ids: Vec<i64> = names.iter().map(|n| s.file_id(n)).collect();
    let got: Vec<Option<UnreadReason>> = searcher
        .unread(&ids)
        .unwrap()
        .into_iter()
        .map(|(_, r)| r)
        .collect();
    assert_eq!(
        got,
        vec![
            Some(UnreadReason::Scanned),
            Some(UnreadReason::Password),
            Some(UnreadReason::Failed),
            Some(UnreadReason::Cloud),
            Some(UnreadReason::Unsupported),
            None,
        ]
    );
    let counts = searcher.unread_counts().unwrap();
    assert_eq!(counts.len(), 5, "{counts:?}");
    assert!(counts.values().all(|n| *n == 1), "{counts:?}");
}

/// VFS-2 AC-22: structured queries (`any` ranks and is optional when `all` is given) and
/// stable paging with the total.
#[test]
fn ac22_structured_queries_and_paging() {
    let s = setup();
    s.write("a/a.txt", "Contrato de alquiler con fianza depositada.");
    s.write("a/b.txt", "La fianza se devuelve al final.");
    for i in 0..28 {
        s.write(
            &format!("c/relleno{i:02}.txt"),
            format!("Documento de relleno número {i}."),
        );
    }
    s.scan();
    s.run(&s.opts());
    let any = s.retrieve(structured(&[], &["alquiler", "fianza"]), 10, 0);
    assert_eq!(hit_names(&any), vec!["a.txt", "b.txt"]);
    let all_any = s.retrieve(structured(&["alquiler"], &["fianza"]), 10, 0);
    assert_eq!(hit_names(&all_any), vec!["a.txt"]);

    // 30 documents contain «número» or «fianza»: page through them.
    let q = structured(&[], &["número", "fianza"]);
    let whole = s.retrieve(q.clone(), 30, 0);
    assert_eq!((whole.hits.len(), whole.total), (30, 30));
    let first = s.retrieve(q.clone(), 20, 0);
    let second = s.retrieve(q, 20, 20);
    assert_eq!((second.hits.len(), second.total), (10, 30));
    let paged: Vec<i64> = first
        .hits
        .iter()
        .chain(&second.hits)
        .map(|h| h.id)
        .collect();
    assert_eq!(paged, whole.hits.iter().map(|h| h.id).collect::<Vec<_>>());
}

/// Whether the process `pid` is still running (not merely a reaped id).
fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        // SAFETY: the handle is opened, waited on with no timeout and closed here.
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return false;
            }
            let alive = WaitForSingleObject(h, 0) == WAIT_TIMEOUT;
            CloseHandle(h);
            alive
        }
    }
}

/// VFS-27 AC-3 (R3): a stop during a document gives it the grace period, then the worker is
/// killed and has exited when the run returns; the document stays queued, not failed.
#[test]
fn stop_mid_document_kills_the_worker_within_the_grace() {
    let s = setup();
    s.write("a/lento.txt", "este documento no termina nunca");
    s.scan();
    let pidfile = s._tmp.path().join("worker.pid");
    let mut o = s.opts();
    o.worker.stop_grace = Duration::from_secs(1);
    o.worker.args = vec![
        "--test-hook=hang:lento.txt".into(),
        format!("--test-hook=pidfile:{}", pidfile.display()),
    ];
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let (db, flag) = (s.db.clone(), stop.clone());
    let run = std::thread::spawn(move || vfs_content::index(&db, &o, &flag).unwrap());
    // Wait until the worker has started and the document is in flight.
    let t = std::time::Instant::now();
    while !pidfile.exists() && t.elapsed() < Duration::from_secs(60) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let pid: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let asked = std::time::Instant::now();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let sum = run.join().unwrap();
    // The grace (1 s here, 5 s in the app) plus the kill and reap.
    assert!(
        asked.elapsed() < Duration::from_secs(4),
        "{:?}",
        asked.elapsed()
    );
    assert!(!process_alive(pid), "worker {pid} still running");
    assert_eq!(sum.stopped, None, "{sum:?}");
    assert_eq!((sum.failed, sum.remaining), (0, 1), "{sum:?}");
    assert_eq!(s.state("lento.txt"), (QUEUED, None, None));
}

// ---- VFS-27 PR 27b-1: the engine side of the background indexer ----

/// VFS-27 AC-8 (R8): a folder paused through VFS-32 has none of its documents read and keeps
/// their state; once resumed, they are read.
#[test]
fn paused_folder_is_not_read() {
    let s = setup();
    s.write("a/leido.txt", "la cigüeña");
    let other = s._tmp.path().join("Pausada");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("en_pausa.txt"), "el señor Muñoz").unwrap();
    s.scan();
    let paused = s.vfs.add_root(&other).unwrap();
    s.scan();
    s.vfs.set_root_paused(paused.id, true).unwrap();
    s.run(&s.opts());
    assert_eq!(s.state("leido.txt").0, INDEXED);
    assert_eq!(s.state("en_pausa.txt"), (NONE, None, None));
    s.vfs.set_root_paused(paused.id, false).unwrap();
    s.run(&s.opts());
    assert_eq!(s.state("en_pausa.txt").0, INDEXED);
}

/// A gate that admits `budget` documents, then pauses the run, recording each outcome.
struct Budget {
    budget: usize,
    seen: Vec<vfs_content::indexer::OutcomeKind>,
}

impl vfs_content::Gate for Budget {
    fn before_document(&mut self, _path: &Path) -> vfs_content::Admit {
        if self.seen.len() < self.budget {
            vfs_content::Admit::Read
        } else {
            vfs_content::Admit::Stop
        }
    }
    fn after_document(&mut self, outcome: vfs_content::indexer::OutcomeKind, _worker_ms: u64) {
        self.seen.push(outcome);
    }
}

/// VFS-27 AC-8 (R8, R9): a run paused between documents after 3 of 10 leaves 7 queued and
/// reports each document it read; resumed, it reads only the 7 left.
#[test]
fn paused_run_resumes_with_the_rest() {
    let s = setup();
    for i in 0..10 {
        s.write(&format!("a/doc{i}.txt"), format!("documento número {i}"));
    }
    s.scan();
    let mut gate = Budget {
        budget: 3,
        seen: Vec::new(),
    };
    let first =
        vfs_content::index_with(&s.db, &s.opts(), &AtomicBool::new(false), &mut gate).unwrap();
    assert_eq!((first.indexed, first.remaining), (3, 7), "{first:?}");
    assert_eq!(gate.seen, vec![OutcomeKind::Indexed; 3]);
    let rest = s.run(&s.opts());
    assert_eq!(
        (rest.queued, rest.indexed, rest.files_opened),
        (7, 7, 7),
        "{rest:?}"
    );
}

/// VFS-2 AC-15 (R14): a long-lived `Searcher` opened before any document was read does not
/// keep its empty count, which would make every word "common": once documents are indexed,
/// a rare word still ranks.
#[test]
fn searcher_opened_on_an_empty_store_ranks_new_documents() {
    let s = setup();
    s.scan();
    s.run(&s.opts()); // creates the empty store
    let searcher = s.searcher();
    let filters = SearchFilters::default();
    assert!(searcher
        .retrieve(&text("documento"), &filters, 10, 0)
        .unwrap()
        .hits
        .is_empty());
    for i in 0..40 {
        let extra = if i == 7 { " Peñalara Peñalara" } else { "" };
        s.write(
            &format!("a/doc{i:02}.txt"),
            format!("Este documento número {i}.{extra}"),
        );
    }
    let root = PathBuf::from(&s.vfs.list_roots().unwrap()[0].path);
    s.vfs.mark_dirty(std::slice::from_ref(&root)).unwrap();
    s.scan();
    s.run(&s.opts());
    let r = searcher
        .retrieve(&text("documento Peñalara"), &filters, 10, 0)
        .unwrap();
    assert_eq!(hit_names(&r), vec!["doc07.txt"], "{r:?}");
}

/// VFS-2 R20, VFS-27 R8 (#178): the content states are written in windows of file ids; with
/// enough documents to span several windows and a folder offline, every reachable document is
/// marked queued and the offline folder's documents keep their state.
#[test]
fn mirror_spans_windows_and_respects_offline_roots() {
    let s = setup();
    s.write("a/real.txt", "texto");
    let other = s._tmp.path().join("Disco");
    std::fs::create_dir_all(other.join("b")).unwrap();
    std::fs::write(other.join("b/ausente.txt"), "texto").unwrap();
    s.scan();
    s.vfs.add_root(&other).unwrap();
    s.scan();
    {
        let conn = Connection::open(&s.db).unwrap();
        let dir_of = |name: &str| -> i64 {
            conn.query_row("SELECT dir_id FROM files WHERE name = ?1", [name], |r| {
                r.get(0)
            })
            .unwrap()
        };
        let (online, offline) = (dir_of("real.txt"), dir_of("ausente.txt"));
        conn.execute_batch("BEGIN").unwrap();
        let mut ins = conn
            .prepare(
                "INSERT INTO files(dir_id, name, name_key, kind, size, mtime)
                 VALUES (?1, ?2, ?2, ?3, 10, 1700000000)",
            )
            .unwrap();
        for i in 0..12_000 {
            let dir = if i % 3 == 0 { offline } else { online };
            ins.execute(rusqlite::params![
                dir,
                format!("sint{i:05}.txt"),
                FileKind::Document.code()
            ])
            .unwrap();
        }
        drop(ins);
        conn.execute_batch("COMMIT").unwrap();
    }
    std::fs::rename(&other, s._tmp.path().join("Disco-desconectado")).unwrap();
    let mut o = s.opts();
    o.max_files = Some(0); // mirror the states, read nothing
    let sum = s.run(&o);
    assert_eq!(sum.files_opened, 0, "{sum:?}");
    let conn = Connection::open(&s.db).unwrap();
    let count = |state: i64, offline: bool| -> i64 {
        conn.query_row(
            "SELECT count(*) FROM files f JOIN dirs d ON d.id = f.dir_id JOIN roots r ON r.id = d.root_id
             WHERE f.kind = ?1 AND f.content_state = ?2 AND (r.path LIKE '%Disco') = ?3",
            rusqlite::params![FileKind::Document.code(), state, offline],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        count(QUEUED, false),
        8_000 + 1,
        "every reachable document is queued"
    );
    assert_eq!(count(QUEUED, true), 0, "the offline folder keeps its state");
    assert_eq!(count(NONE, true), 4_000 + 1);
}
