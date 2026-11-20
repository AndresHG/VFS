//! Exact search, filters and folder hits through the `Vfs` facade (VFS-1 AC-8, AC-10,
//! AC-11). The database is populated with `db::ingest`, so it has the production layout.
//!
//! Until task T2b (fuzzy + ranking) lands, `rank::rank` and the `fuzzy` helpers are stubs:
//! tests here keep text queries to at most one hit (ranking runs only for two or more) and
//! to terms shorter than 6 characters (the typo pass only considers longer terms). Tests
//! that need T2b end in `_needs_t2b` and are ignored until it merges.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use tempfile::TempDir;
use vfs_core::db::{self, ingest};
use vfs_core::{
    FileKind, MatchKind, SearchConfig, SearchFilters, SearchQuery, SearchResponse, TruncatedBy, Vfs,
};

struct Fixture {
    _dir: TempDir,
    path: PathBuf,
    vfs: Vfs,
}

fn fixture() -> Fixture {
    fixture_with(SearchConfig::default())
}

fn fixture_with(config: SearchConfig) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    let vfs = Vfs::open_with(&path, config).unwrap();
    Fixture {
        _dir: dir,
        path,
        vfs,
    }
}

/// Writes through its own connection, one transaction per call.
struct Writer {
    conn: Connection,
}

impl Fixture {
    fn writer(&self) -> Writer {
        Writer {
            conn: db::open_rw(&self.path).unwrap(),
        }
    }

    fn search(&self, text: &str, filters: SearchFilters) -> SearchResponse {
        self.vfs
            .search(&SearchQuery {
                text: text.into(),
                filters,
                limit: 500,
                offset: 0,
            })
            .unwrap()
    }

    fn text(&self, text: &str) -> SearchResponse {
        self.search(text, SearchFilters::default())
    }
}

impl Writer {
    /// Root `path` and the id of its root directory.
    fn root(&mut self, path: &str) -> (i64, i64) {
        let tx = self.conn.transaction().unwrap();
        let root = ingest::insert_root(&tx, "/", path).unwrap();
        let dir: i64 = tx
            .query_row(
                "SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ''",
                [root],
                |r| r.get(0),
            )
            .unwrap();
        tx.commit().unwrap();
        (root, dir)
    }

    fn dir(&mut self, root: i64, parent: i64, rel: &str, mtime: i64) -> i64 {
        let tx = self.conn.transaction().unwrap();
        let id = ingest::insert_dir(&tx, root, parent, rel, mtime).unwrap();
        tx.commit().unwrap();
        id
    }

    fn file(&mut self, dir: i64, name: &str, size: u64, mtime: i64) -> i64 {
        let tx = self.conn.transaction().unwrap();
        let id = ingest::insert_file(
            &tx,
            dir,
            &ingest::NewFile {
                name,
                size,
                mtime,
                lossy: false,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        id
    }
}

/// (is_dir, id) of every hit, sorted, for order-independent comparisons.
fn ids(r: &SearchResponse) -> Vec<(bool, i64)> {
    let mut v: Vec<(bool, i64)> = r.hits.iter().map(|h| (h.is_dir, h.id)).collect();
    v.sort();
    v
}

fn files(ids: &[i64]) -> Vec<(bool, i64)> {
    let mut v: Vec<(bool, i64)> = ids.iter().map(|i| (false, *i)).collect();
    v.sort();
    v
}

fn only(r: &SearchResponse) -> &vfs_core::SearchHit {
    assert_eq!(r.hits.len(), 1, "expected one hit, got {:?}", r.hits);
    assert_eq!(r.total, 1);
    &r.hits[0]
}

/// The archive used by most tests (root `/datos`):
/// `Gestoría/Año 2019/Banco/extracto.pdf`, `Gestoría2/copia.txt`, empty `Facturas vacías`,
/// `NOTARÍA Pérez.pdf`, `zq.txt` and `otro.txt` at the top.
struct Archive {
    fx: Fixture,
    top: i64,
    gestoria: i64,
    ano: i64,
    banco: i64,
    facturas: i64,
    extracto: i64,
    copia: i64,
    notaria: i64,
    zq: i64,
    otro: i64,
}

fn archive() -> Archive {
    let fx = fixture();
    let mut w = fx.writer();
    let (root, top) = w.root("/datos");
    let gestoria = w.dir(root, top, "Gestoría", 100);
    let ano = w.dir(root, gestoria, "Gestoría/Año 2019", 200);
    let banco = w.dir(root, ano, "Gestoría/Año 2019/Banco", 300);
    let gestoria2 = w.dir(root, top, "Gestoría2", 400);
    let facturas = w.dir(root, top, "Facturas vacías", 500);
    let extracto = w.file(banco, "extracto.pdf", 1_000, 1_000);
    let copia = w.file(gestoria2, "copia.txt", 2_000, 2_000);
    let notaria = w.file(top, "NOTARÍA Pérez.pdf", 3_000, 3_000);
    let zq = w.file(top, "zq.txt", 4_000, 4_000);
    let otro = w.file(top, "otro.txt", 5_000, 5_000);
    drop(w);
    Archive {
        fx,
        top,
        gestoria,
        ano,
        banco,
        facturas,
        extracto,
        copia,
        notaria,
        zq,
        otro,
    }
}

/// VFS-1 AC-10: case and accents are ignored in both the query and the names.
#[test]
fn ac10_accent_and_case_folding() {
    let a = archive();
    for q in ["perez", "PÉREZ", "Pérez", "notar", "NOTAR PEREZ", "ÑOTAR"] {
        let r = a.fx.text(q);
        assert_eq!(only(&r).id, a.notaria, "query {q:?}");
        assert_eq!(r.hits[0].matched, MatchKind::Name);
        assert_eq!(
            r.hits[0].name, "NOTARÍA Pérez.pdf",
            "original name is returned"
        );
        assert_eq!(r.hits[0].dir, "/datos");
        assert!(r.truncated_by.is_none());
    }
    assert!(a.fx.text("perez zzzz").hits.is_empty());
}

/// VFS-1 AC-8, AC-10: names stored in NFC and in NFD are both found, by a query typed in
/// either form.
#[test]
fn ac8_nfc_and_nfd_names() {
    let fx = fixture();
    let mut w = fx.writer();
    let (_, top) = w.root("/datos");
    let nfc = w.file(top, "Le\u{00F1}a nfc.txt", 1, 1);
    let nfd = w.file(top, "Len\u{0303}a nfd.txt", 1, 1);
    drop(w);
    assert_eq!(only(&fx.text("lena nfc")).id, nfc);
    assert_eq!(only(&fx.text("LEÑA nfd")).id, nfd);
    assert_eq!(only(&fx.text("len\u{0303}a nfc")).id, nfc);
    assert_eq!(only(&fx.text("le\u{00F1}a nfd")).id, nfd);
    assert_eq!(fx.text("nfd").hits[0].name, "Len\u{0303}a nfd.txt");
}

/// VFS-1 AC-10: a term may match any segment of the directory path, including a middle one,
/// and terms may be split between the path and the name; `\` in the query is `/`.
#[test]
fn ac10_matches_a_middle_path_segment() {
    let a = archive();
    let docs = SearchFilters {
        kinds: vec![FileKind::Document],
        ..Default::default()
    };
    let r = a.fx.search("2019", docs.clone());
    let hit = only(&r);
    assert_eq!(hit.id, a.extracto);
    assert_eq!(hit.matched, MatchKind::Path);
    let expected = Path::new("/datos")
        .join("Gestoría")
        .join("Año 2019")
        .join("Banco");
    assert_eq!(Path::new(&hit.dir), expected);
    assert_eq!(
        a.fx.vfs.file_path(hit.id).unwrap(),
        expected.join("extracto.pdf")
    );

    let r = a.fx.text("año extr");
    assert_eq!(only(&r).id, a.extracto);
    assert_eq!(r.hits[0].matched, MatchKind::Path);
    assert_eq!(only(&a.fx.search("9\\ban", docs.clone())).id, a.extracto);
    assert_eq!(only(&a.fx.search("9/ban", docs)).id, a.extracto);
}

/// VFS-1 AC-10 (S5): one- and two-character terms, alone (a scan) or next to a longer term
/// (verified on the longer term's candidates).
#[test]
fn ac10_one_and_two_character_terms() {
    let a = archive();
    assert_eq!(only(&a.fx.text("zq")).id, a.zq);
    assert_eq!(only(&a.fx.text("q")).id, a.zq);
    assert_eq!(only(&a.fx.text("Q z")).id, a.zq);
    assert_eq!(only(&a.fx.text("zq txt")).id, a.zq);
    assert_eq!(only(&a.fx.text("txt zq")).id, a.zq);
    assert_eq!(only(&a.fx.text("ot txt")).id, a.otro);
    assert!(a.fx.text("qz").hits.is_empty());
}

/// VFS-1 decision D1: folders are results, empty ones included, marked as folders.
#[test]
fn d1_folders_are_results() {
    let a = archive();
    let r = a.fx.text("factu");
    let hit = only(&r);
    assert!(hit.is_dir);
    assert_eq!(hit.id, a.facturas);
    assert_eq!(hit.kind, FileKind::Folder);
    assert_eq!(hit.name, "Facturas vacías");
    assert_eq!(hit.dir, "/datos");
    assert_eq!(hit.size, 0);
    assert_eq!(hit.modified, 500);
    assert_eq!(hit.matched, MatchKind::Name);
    assert_eq!(
        a.fx.vfs.dir_path(hit.id).unwrap(),
        Path::new("/datos").join("Facturas vacías")
    );

    // Subfolders of a matching folder are not hits unless a term is in their own name:
    // "2019" finds the folder "Año 2019" but not "Banco" below it.
    let folders = SearchFilters {
        kinds: vec![FileKind::Folder],
        ..Default::default()
    };
    let r = a.fx.search("2019", folders);
    let hit = only(&r);
    assert_eq!((hit.is_dir, hit.id), (true, a.ano));
    assert_eq!(Path::new(&hit.dir), Path::new("/datos").join("Gestoría"));
}

/// VFS-1 AC-10 (S1): a term already satisfied by a root's own path is ignored for that root;
/// a query satisfied only by root paths does not return whole roots; the root folder itself
/// is a folder hit by its name.
#[test]
fn s1_root_path_satisfies_a_term() {
    let fx = fixture();
    let mut w = fx.writer();
    let (_, top) = w.root("/datos/Caja");
    let notas = w.file(top, "notas.txt", 1, 1);
    w.file(top, "otro.txt", 1, 1);
    let (_, other_top) = w.root("/otra");
    w.file(other_top, "notas viejas.doc", 1, 1);
    drop(w);
    assert_eq!(only(&fx.text("caja notas")).id, notas);
    assert!(fx.text("datos").hits.is_empty());
    let r = fx.text("caja");
    let hit = only(&r);
    assert!(hit.is_dir);
    assert_eq!(hit.id, top);
    assert_eq!(hit.name, "Caja");
    assert_eq!(hit.dir, "/datos");
}

/// VFS-1 AC-11: the type filter alone, folders included as their own type.
#[test]
fn ac11_kind_filter_alone() {
    let a = archive();
    let kinds = |k: Vec<FileKind>| SearchFilters {
        kinds: k,
        ..Default::default()
    };
    let r = a.fx.search("", kinds(vec![FileKind::Document]));
    assert_eq!(
        ids(&r),
        files(&[a.extracto, a.copia, a.notaria, a.zq, a.otro])
    );
    let r = a.fx.search("", kinds(vec![FileKind::Folder]));
    assert!(r
        .hits
        .iter()
        .all(|h| h.is_dir && h.kind == FileKind::Folder));
    assert_eq!(
        r.total, 6,
        "root, Gestoría, Año 2019, Banco, Gestoría2, Facturas vacías"
    );
    assert!(a
        .fx
        .search("", kinds(vec![FileKind::Photo]))
        .hits
        .is_empty());
    // Filter-only lists are newest first.
    let r = a.fx.search("", kinds(vec![FileKind::Document]));
    let times: Vec<i64> = r.hits.iter().map(|h| h.modified).collect();
    assert_eq!(times, vec![5_000, 4_000, 3_000, 2_000, 1_000]);
    // With text: the folder "Banco" is dropped by a document-only filter.
    let r = a.fx.search("banco", kinds(vec![FileKind::Document]));
    assert_eq!(only(&r).id, a.extracto);
    let r = a.fx.search("banco", kinds(vec![FileKind::Folder]));
    assert_eq!(only(&r).id, a.banco);
}

/// VFS-1 AC-11: the date filter alone (from inclusive, to exclusive); folders by their own
/// modification time.
#[test]
fn ac11_date_filter_alone() {
    let a = archive();
    let r = a.fx.search(
        "",
        SearchFilters {
            modified_from: Some(2_000),
            modified_to: Some(4_000),
            ..Default::default()
        },
    );
    assert_eq!(ids(&r), files(&[a.copia, a.notaria]));
    let r = a.fx.search(
        "",
        SearchFilters {
            modified_to: Some(250),
            ..Default::default()
        },
    );
    // The root directory's own time is 0 until its parent is listed.
    assert_eq!(
        ids(&r),
        vec![(true, a.top), (true, a.gestoria), (true, a.ano)]
    );
}

/// VFS-1 AC-11: the size filter alone (both bounds inclusive); folders have no size and are
/// excluded by it.
#[test]
fn ac11_size_filter_alone() {
    let a = archive();
    let r = a.fx.search(
        "",
        SearchFilters {
            size_min: Some(2_000),
            size_max: Some(4_000),
            ..Default::default()
        },
    );
    assert_eq!(ids(&r), files(&[a.copia, a.notaria, a.zq]));
    let r = a.fx.search(
        "",
        SearchFilters {
            size_min: Some(0),
            ..Default::default()
        },
    );
    assert!(r.hits.iter().all(|h| !h.is_dir));
    assert_eq!(r.total, 5);
}

/// VFS-1 AC-11: the folder filter alone: everything inside at any depth, not the folder
/// itself, not a sibling sharing its name prefix; case, accents and separators ignored.
#[test]
fn ac11_folder_filter_alone() {
    let a = archive();
    let inside = |folder: &str| {
        a.fx.search(
            "",
            SearchFilters {
                folder: Some(folder.into()),
                ..Default::default()
            },
        )
    };
    let expected = vec![(false, a.extracto), (true, a.ano), (true, a.banco)];
    let native = Path::new("/datos").join("Gestoría");
    assert_eq!(ids(&inside(native.to_str().unwrap())), expected);
    assert_eq!(ids(&inside("/datos/Gestoría/")), expected);
    assert_eq!(ids(&inside("/DATOS/gestoria")), expected);
    assert_eq!(ids(&inside("\\datos\\Gestori\u{0301}a")), expected);
    // The root itself: everything but the root folder.
    let all = inside("/datos");
    assert_eq!(all.total, 10);
    assert!(!all.hits.iter().any(|h| h.is_dir && h.id == a.top));
    // A folder containing the root: the root folder too.
    assert_eq!(inside("/").total, 11);
    // Unknown folders give nothing.
    assert!(inside("/datos/nada").hits.is_empty());
    assert!(inside("/otro").hits.is_empty());
}

/// VFS-1 AC-11: all filters combined, with and without text: only hits matching every one.
#[test]
fn ac11_filters_combined() {
    let fx = fixture();
    let mut w = fx.writer();
    let (root, top) = w.root("/datos");
    let fotos = w.dir(root, top, "Fotos", 10);
    let viaje = w.dir(root, fotos, "Fotos/Viaje", 10);
    let want = w.file(viaje, "playa.jpg", 5_000, 2_000);
    w.file(viaje, "playa.pdf", 5_000, 2_000); // wrong type
    w.file(viaje, "playa grande.jpg", 50_000, 2_000); // too big
    w.file(viaje, "playa vieja.jpg", 5_000, 100); // too old
    w.file(top, "playa fuera.jpg", 5_000, 2_000); // outside the folder
    w.dir(root, viaje, "Fotos/Viaje/playa", 2_000); // a folder: excluded by kind and size
    drop(w);
    let filters = SearchFilters {
        kinds: vec![FileKind::Photo, FileKind::Video],
        modified_from: Some(1_000),
        modified_to: Some(3_000),
        size_min: Some(1_000),
        size_max: Some(10_000),
        folder: Some("/datos/Fotos".into()),
    };
    assert_eq!(only(&fx.search("playa", filters.clone())).id, want);
    assert_eq!(only(&fx.search("", filters.clone())).id, want);
    assert_eq!(only(&fx.search("ay", filters.clone())).id, want);
    assert!(fx.search("mar", filters).hits.is_empty());
}

/// VFS-1 AC-9, AC-10: a hit cap cuts a broad query at 2000 hits and says so
/// (`truncatedBy = cap`); pages slice one consistent list.
#[test]
fn ac9_truncation_by_cap_is_reported() {
    let fx = fixture();
    let mut w = fx.writer();
    let (_, top) = w.root("/datos");
    let tx = w.conn.transaction().unwrap();
    for i in 0..2_001 {
        let name = format!("doc {i}.pdf");
        let f = ingest::NewFile {
            name: &name,
            size: 1,
            mtime: i,
            lossy: false,
        };
        ingest::insert_file(&tx, top, &f).unwrap();
    }
    tx.commit().unwrap();
    drop(w);
    let docs = SearchFilters {
        kinds: vec![FileKind::Document],
        ..Default::default()
    };
    let page = |offset: u32, limit: u32| {
        fx.vfs
            .search(&SearchQuery {
                text: String::new(),
                filters: docs.clone(),
                limit,
                offset,
            })
            .unwrap()
    };
    let r = page(0, 100);
    assert_eq!(r.total, 2000);
    assert!(r.truncated);
    assert_eq!(r.truncated_by, Some(TruncatedBy::Cap));
    assert_eq!(r.hits.len(), 100);
    let last = page(1_990, 100);
    assert_eq!(last.hits.len(), 10);
    assert_eq!(page(0, 10_000).hits.len(), 500, "limit is clamped to 500");
    assert_eq!(page(0, 0).hits.len(), 1, "limit is clamped to 1");
    let again = page(0, 100);
    assert_eq!(ids(&again), ids(&r), "paging reuses the same ranked list");
}

/// VFS-1 AC-9 (U1, S3): an expired deadline cuts scans and reports `truncatedBy = deadline`;
/// a generator under its cap still runs to completion.
#[test]
fn ac9_truncation_by_deadline_is_reported() {
    let fx = fixture_with(SearchConfig {
        deadline: Duration::ZERO,
    });
    let mut w = fx.writer();
    let (_, top) = w.root("/datos");
    let zq = w.file(top, "zq.txt", 1, 1);
    w.file(top, "otro.txt", 1, 1);
    drop(w);
    let r = fx.search(
        "",
        SearchFilters {
            kinds: vec![FileKind::Document],
            ..Default::default()
        },
    );
    assert_eq!(r.truncated_by, Some(TruncatedBy::Deadline));
    assert!(r.truncated);
    assert!(r.hits.is_empty());
    let r = fx.text("zq");
    assert_eq!(r.truncated_by, Some(TruncatedBy::Deadline), "a scan");
    let r = fx.text("zq txt");
    assert_eq!(only(&r).id, zq);
    assert!(r.truncated_by.is_none(), "under-cap generator");
}

/// VFS-1 AC-10: an empty query without filters returns nothing, quickly and untruncated;
/// the index is read again after a write (no stale cached list).
#[test]
fn ac10_empty_query_and_fresh_results() {
    let a = archive();
    let r = a.fx.text("   ");
    assert!(r.hits.is_empty());
    assert_eq!(r.total, 0);
    assert!(r.truncated_by.is_none());
    assert!(a.fx.text("nuevo").hits.is_empty());
    let mut w = a.fx.writer();
    let id = w.file(a.top, "nuevo.txt", 1, 1);
    drop(w);
    assert_eq!(only(&a.fx.text("nuevo")).id, id);
}

/// VFS-1 AC-10: the spec's own example, "notaria" finds "Notaría" (goes through the typo
/// pass and ranking).
#[test]
#[ignore = "enable after T2b (fuzzy + rank) merges"]
fn ac10_notaria_finds_notaria_needs_t2b() {
    let a = archive();
    assert_eq!(only(&a.fx.text("notaria")).id, a.notaria);
    assert_eq!(only(&a.fx.text("NOTARIA perez")).id, a.notaria);
}

/// VFS-1 AC-8, AC-10: one query finding both the NFC and the NFD name (ranking).
#[test]
#[ignore = "enable after T2b (fuzzy + rank) merges"]
fn ac8_nfc_and_nfd_in_one_query_needs_t2b() {
    let fx = fixture();
    let mut w = fx.writer();
    let (_, top) = w.root("/datos");
    let nfc = w.file(top, "Le\u{00F1}a nfc.txt", 1, 1);
    let nfd = w.file(top, "Len\u{0303}a nfd.txt", 1, 1);
    drop(w);
    assert_eq!(ids(&fx.text("lena")), files(&[nfc, nfd]));
}

/// VFS-1 AC-9, AC-10: a broad text query is cut at 2000 hits with `truncatedBy = cap`.
#[test]
#[ignore = "enable after T2b (fuzzy + rank) merges"]
fn ac9_text_truncation_by_cap_needs_t2b() {
    let fx = fixture();
    let mut w = fx.writer();
    let (_, top) = w.root("/datos");
    let tx = w.conn.transaction().unwrap();
    for i in 0..2_001 {
        let name = format!("foto {i}.jpg");
        let f = ingest::NewFile {
            name: &name,
            size: 1,
            mtime: i,
            lossy: false,
        };
        ingest::insert_file(&tx, top, &f).unwrap();
    }
    tx.commit().unwrap();
    drop(w);
    let r = fx.text("foto");
    assert_eq!(r.total, 2000);
    assert_eq!(r.truncated_by, Some(TruncatedBy::Cap));
}

/// VFS-1 AC-10: one typo is tolerated ("notaira" finds "Notaría").
#[test]
#[ignore = "enable after T2b (fuzzy + rank) merges"]
fn ac10_one_typo_needs_t2b() {
    let a = archive();
    let r = a.fx.text("notaira");
    assert_eq!(only(&r).id, a.notaria);
    assert_eq!(r.hits[0].matched, MatchKind::Fuzzy);
}
