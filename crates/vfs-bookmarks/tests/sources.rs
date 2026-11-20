//! Searching files and bookmarks together (VFS-6 AC-6, AC-7, AC-8, AC-12).

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use rusqlite::Connection;
use vfs_bookmarks::sources::search_sources;
use vfs_bookmarks::testing::{FakeFirefox, Places, MENU, UNFILED};
use vfs_bookmarks::{Bookmarks, ReadState, SortOrder, Source, SourceHit, SourceOptions};
use vfs_core::{Error, FileKind, MatchKind, SearchFilters, SearchQuery, Vfs};

struct World {
    _tmp: tempfile::TempDir,
    vfs: Vfs,
    bm: Bookmarks,
}

/// Files `Recetas de cocina.pdf`, `recetas.txt` and `Banco/extracto.pdf`; bookmarks
/// "Recetas" (exact title), "Flan" in folder Recetas, "Banco online" saved in 2020, a
/// `file:` bookmark, and 30 numbered "Receta n" bookmarks.
fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("datos");
    std::fs::create_dir_all(data.join("Banco")).unwrap();
    for f in ["Recetas de cocina.pdf", "recetas.txt", "Banco/extracto.pdf"] {
        std::fs::write(data.join(f), "x").unwrap();
    }
    let vfs = Vfs::open(&tmp.path().join("app/index.db")).unwrap();
    vfs.add_root(&data).unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();

    let mut ff = FakeFirefox::new(&tmp.path().join("Firefox"));
    let dir = ff.add_profile("default", "p.default");
    {
        let conn = Connection::open(dir.join("places.sqlite")).unwrap();
        let p = Places { conn: &conn };
        p.bookmark(MENU, Some("Recetas"), "https://recetas.es/", 1_700_000_000);
        let folder = p.folder(MENU, "Recetas");
        p.bookmark(
            folder,
            Some("Flan"),
            "https://cocina.es/flan",
            1_650_000_000,
        );
        p.bookmark(
            UNFILED,
            Some("Banco online"),
            "https://banco.es/",
            1_590_000_000,
        );
        p.bookmark(
            UNFILED,
            Some("Documento local"),
            "file:///C:/recetas.pdf",
            10,
        );
        p.bookmark(UNFILED, Some("Dato incrustado"), "data:text/html,hola", 10);
        for i in 0..30 {
            p.bookmark(
                UNFILED,
                Some(&format!("Receta {i}")),
                &format!("https://r.es/{i}"),
                i,
            );
        }
    }
    let bm = Bookmarks::open(
        &vfs_bookmarks::bookmarks_db_path(&tmp.path().join("app/index.db")),
        ff.locator(),
    )
    .unwrap();
    bm.set_enabled(true).unwrap();
    assert_eq!(bm.refresh().unwrap().state, ReadState::Ok);
    World { _tmp: tmp, vfs, bm }
}

fn only(sources: &[Source]) -> SourceOptions {
    SourceOptions {
        sources: sources.to_vec(),
        ..Default::default()
    }
}

fn q(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        filters: SearchFilters::default(),
        limit: 100,
        offset: 0,
    }
}

fn names(hits: &[SourceHit]) -> Vec<String> {
    hits.iter()
        .map(|h| match h {
            SourceHit::Files(f) => format!("F:{}", f.name),
            SourceHit::Bookmarks(b) if b.is_folder => format!("BF:{}", b.title),
            SourceHit::Bookmarks(b) => format!("B:{}", b.title),
        })
        .collect()
}

/// VFS-6 AC-6: "Todo" returns both sources ranked by the shared tiers (an exact title before
/// a longer file name, files first on a tie); "Archivos" only files, "Marcadores" only
/// bookmarks; accents, case and one typo follow the file rules; folder names match.
#[test]
fn ac6_both_sources_merge_by_shared_tiers() {
    let w = world();
    let all = search_sources(&w.vfs, Some(&w.bm), &q("recetas"), &only(&[])).unwrap();
    let n = names(&all.hits);
    // Tier 0 (the whole query) for all three; shorter name first, a file on a tie.
    assert_eq!(
        &n[..3],
        ["B:Recetas", "BF:Recetas", "F:recetas.txt"],
        "{n:?}"
    );
    assert!(n.contains(&"F:Recetas de cocina.pdf".to_string()));
    assert!(
        n.contains(&"B:Flan".to_string()),
        "folder name matches: {n:?}"
    );
    assert_eq!(all.total as usize, all.hits.len());

    let files =
        search_sources(&w.vfs, Some(&w.bm), &q("recetas"), &only(&[Source::Files])).unwrap();
    assert!(files.hits.iter().all(|h| h.source() == Source::Files));
    assert_eq!(files.hits.len(), 2);
    let marks = search_sources(
        &w.vfs,
        Some(&w.bm),
        &q("recetas"),
        &only(&[Source::Bookmarks]),
    )
    .unwrap();
    assert!(marks.hits.iter().all(|h| h.source() == Source::Bookmarks));

    for text in ["RECETAS", "récetas"] {
        let r = search_sources(&w.vfs, Some(&w.bm), &q(text), &only(&[Source::Bookmarks])).unwrap();
        assert_eq!(names(&r.hits)[0], "B:Recetas", "{text}");
    }
    // The spec's example: one transposition in a 7-letter word.
    let swapped = search_sources(
        &w.vfs,
        Some(&w.bm),
        &q("recteas"),
        &only(&[Source::Bookmarks]),
    )
    .unwrap();
    assert!(names(&swapped.hits).contains(&"B:Recetas".to_string()));
    let typo = search_sources(
        &w.vfs,
        Some(&w.bm),
        &q("bancoo onlin"),
        &only(&[Source::Bookmarks]),
    )
    .unwrap();
    assert_eq!(names(&typo.hits), ["B:Banco online"]);
    match &typo.hits[0] {
        SourceHit::Bookmarks(b) => assert_eq!(b.matched, MatchKind::Fuzzy),
        other => panic!("{other:?}"),
    }
    let by_site = search_sources(
        &w.vfs,
        Some(&w.bm),
        &q("cocina.es"),
        &only(&[Source::Bookmarks]),
    )
    .unwrap();
    assert_eq!(names(&by_site.hits), ["B:Flan"]);
}

/// VFS-6 AC-6: paging the merged list returns every hit exactly once, in the same order as
/// one big page.
#[test]
fn ac6_paging_the_merged_list_returns_each_hit_once() {
    let w = world();
    let mut whole = q("receta");
    whole.limit = 500;
    let whole = search_sources(&w.vfs, Some(&w.bm), &whole, &only(&[])).unwrap();
    assert!(whole.hits.len() > 30);
    let mut paged = Vec::new();
    let mut page = q("receta");
    page.limit = 7;
    loop {
        let r = search_sources(&w.vfs, Some(&w.bm), &page, &only(&[])).unwrap();
        if r.hits.is_empty() {
            break;
        }
        assert_eq!(r.total, whole.total);
        paged.extend(names(&r.hits));
        page.offset += 7;
    }
    assert_eq!(paged, names(&whole.hits));
    assert_eq!(paged.iter().collect::<HashSet<_>>().len(), paged.len());
}

/// VFS-6 AC-7 (R7): a type, size or folder filter shows no bookmark; a date filter keeps the
/// bookmarks saved in its range, and a date filter alone lists them newest first.
#[test]
fn ac7_filters_on_bookmarks() {
    let w = world();
    for filters in [
        SearchFilters {
            kinds: vec![FileKind::Document],
            ..Default::default()
        },
        SearchFilters {
            size_max: Some(10),
            ..Default::default()
        },
        SearchFilters {
            folder: Some(w.vfs.list_roots().unwrap()[0].path.clone()),
            ..Default::default()
        },
    ] {
        let mut query = q("recetas");
        query.filters = filters.clone();
        let r = search_sources(&w.vfs, Some(&w.bm), &query, &only(&[])).unwrap();
        assert!(
            r.hits.iter().all(|h| h.source() == Source::Files),
            "{filters:?}"
        );
    }
    let mut query = q("");
    query.filters.modified_from = Some(1_600_000_000);
    query.filters.modified_to = Some(1_680_000_000);
    let r = search_sources(&w.vfs, Some(&w.bm), &query, &only(&[Source::Bookmarks])).unwrap();
    assert_eq!(names(&r.hits), ["B:Flan"]);
    query.filters.modified_to = None;
    let r = search_sources(&w.vfs, Some(&w.bm), &query, &only(&[Source::Bookmarks])).unwrap();
    assert_eq!(names(&r.hits), ["B:Recetas", "B:Flan"]);
}

/// VFS-6 AC-8 (R1, R8): while the setting is off, nothing is read and only files are
/// searched; turning it off after a read deletes every stored bookmark.
#[test]
fn ac8_setting_off_searches_files_and_deletes_bookmarks() {
    let w = world();
    assert!(w.bm.stored_count().unwrap() > 0);
    let st = w.bm.set_enabled(false).unwrap();
    assert_eq!((st.enabled, st.count), (false, 0));
    assert_eq!(w.bm.stored_count().unwrap(), 0);
    let r = search_sources(&w.vfs, Some(&w.bm), &q("recetas"), &only(&[])).unwrap();
    assert!(r.hits.iter().all(|h| h.source() == Source::Files));
    let r = search_sources(
        &w.vfs,
        Some(&w.bm),
        &q("recetas"),
        &only(&[Source::Bookmarks]),
    )
    .unwrap();
    assert!(r.hits.is_empty());
    let st = w.bm.refresh().unwrap();
    assert_eq!(
        (st.state, st.count),
        (ReadState::Never, 0),
        "read while off"
    );
    assert_eq!(w.bm.stored_count().unwrap(), 0);
}

/// VFS-6 AC-12 (R10): a web bookmark's address is given to open; a `file:` bookmark and an
/// unknown id are refused, and only web bookmarks are marked openable.
#[test]
fn ac12_only_web_addresses_are_opened() {
    let w = world();
    let r = search_sources(
        &w.vfs,
        Some(&w.bm),
        &q("banco online"),
        &only(&[Source::Bookmarks]),
    )
    .unwrap();
    let SourceHit::Bookmarks(banco) = &r.hits[0] else {
        panic!("{:?}", r.hits)
    };
    assert!(banco.openable);
    assert_eq!(w.bm.url_to_open(banco.id).unwrap(), "https://banco.es/");
    let r = search_sources(
        &w.vfs,
        Some(&w.bm),
        &q("documento local"),
        &only(&[Source::Bookmarks]),
    )
    .unwrap();
    let SourceHit::Bookmarks(local) = &r.hits[0] else {
        panic!("{:?}", r.hits)
    };
    assert!(!local.openable);
    assert!(matches!(
        w.bm.url_to_open(local.id),
        Err(Error::Unavailable(_))
    ));
    let r = search_sources(
        &w.vfs,
        Some(&w.bm),
        &q("dato incrustado"),
        &only(&[Source::Bookmarks]),
    )
    .unwrap();
    let SourceHit::Bookmarks(data) = &r.hits[0] else {
        panic!("{:?}", r.hits)
    };
    assert!(!data.openable);
    assert!(matches!(
        w.bm.url_to_open(data.id),
        Err(Error::Unavailable(_))
    ));
    assert!(matches!(w.bm.url_to_open(999_999), Err(Error::NotFound(_))));
    assert!(vfs_bookmarks::is_openable("HTTPS://x.es") && !vfs_bookmarks::is_openable("ftp://x"));
    for odd in [
        "https://a.es/ b",
        "https://a.es/\"x",
        "https://a.es/\tb",
        " https://a.es",
    ] {
        assert!(!vfs_bookmarks::is_openable(odd), "{odd:?}");
    }
}

/// VFS-6 AC-7 (R7, R11): the type "Carpetas" keeps bookmark folders and drops bookmarks.
#[test]
fn ac7_folder_type_keeps_bookmark_folders() {
    let w = world();
    let mut query = q("recetas");
    query.filters.kinds = vec![FileKind::Folder];
    let r = search_sources(&w.vfs, Some(&w.bm), &query, &only(&[Source::Bookmarks])).unwrap();
    assert_eq!(names(&r.hits), ["BF:Recetas"]);
}

/// VFS-6 AC-14 (R13): newest first across both sources; by source, every file before any
/// bookmark; relevance is the AC-6 order.
#[test]
fn ac14_sort_orders() {
    let w = world();
    let opts = |sort| SourceOptions {
        sort,
        ..Default::default()
    };
    let date = |h: &SourceHit| match h {
        SourceHit::Files(f) => f.modified,
        SourceHit::Bookmarks(b) => b.added,
    };
    let newest =
        search_sources(&w.vfs, Some(&w.bm), &q("receta"), &opts(SortOrder::Newest)).unwrap();
    let dates: Vec<i64> = newest.hits.iter().map(date).collect();
    assert!(dates.windows(2).all(|d| d[0] >= d[1]), "{dates:?}");
    assert!(newest.hits.iter().any(|h| h.source() == Source::Files));
    assert!(newest.hits.iter().any(|h| h.source() == Source::Bookmarks));
    // Newest first pages like the others: page 2 continues page 1.
    let mut page = q("receta");
    page.limit = 5;
    page.offset = 5;
    let second = search_sources(&w.vfs, Some(&w.bm), &page, &opts(SortOrder::Newest)).unwrap();
    assert_eq!(names(&second.hits), names(&newest.hits[5..10]));

    let by_source =
        search_sources(&w.vfs, Some(&w.bm), &q("receta"), &opts(SortOrder::Source)).unwrap();
    let kinds: Vec<Source> = by_source.hits.iter().map(SourceHit::source).collect();
    let first_mark = kinds.iter().position(|s| *s == Source::Bookmarks).unwrap();
    assert!(first_mark > 0);
    assert!(kinds[..first_mark].iter().all(|s| *s == Source::Files));
    assert!(kinds[first_mark..].iter().all(|s| *s == Source::Bookmarks));

    let relevance = search_sources(
        &w.vfs,
        Some(&w.bm),
        &q("receta"),
        &opts(SortOrder::Relevance),
    )
    .unwrap();
    let default = search_sources(&w.vfs, Some(&w.bm), &q("receta"), &only(&[])).unwrap();
    assert_eq!(names(&relevance.hits), names(&default.hits));
    assert_eq!(newest.total, relevance.total);
}
