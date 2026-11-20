//! Sort orders applied before the hit cap (VFS-12).

use std::time::Duration;

use tempfile::TempDir;
use vfs_core::db::{self, ingest};
use vfs_core::{
    SearchConfig, SearchFilters, SearchHit, SearchQuery, SearchResponse, SortOrder, Vfs,
};

struct Fx {
    _dir: TempDir,
    vfs: Vfs,
}

/// An index with root `/datos` holding `folders` (relative paths, parents first) and
/// `files` (folder index into `folders` or `None` for the root, name, size, mtime).
fn index(folders: &[&str], files: &[(Option<usize>, String, u64, i64)]) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    let vfs = Vfs::open_with(
        &path,
        SearchConfig {
            deadline: Duration::from_secs(60),
        },
    )
    .unwrap();
    let mut conn = db::open_rw(&path).unwrap();
    let tx = conn.transaction().unwrap();
    let root = ingest::insert_root(&tx, "/", "/datos").unwrap();
    let top: i64 = tx
        .query_row(
            "SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ''",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    let mut ids: Vec<i64> = Vec::new();
    for rel in folders {
        let parent = match rel.rsplit_once('/') {
            Some((p, _)) => ids[folders.iter().position(|f| *f == p).unwrap()],
            None => top,
        };
        ids.push(ingest::insert_dir(&tx, root, parent, rel, 1).unwrap());
    }
    for (folder, name, size, mtime) in files {
        let dir = folder.map_or(top, |i| ids[i]);
        let f = ingest::NewFile {
            name,
            size: *size,
            mtime: *mtime,
            lossy: false,
        };
        ingest::insert_file(&tx, dir, &f).unwrap();
    }
    tx.commit().unwrap();
    Fx { _dir: dir, vfs }
}

fn search(fx: &Fx, text: &str, sort: SortOrder, offset: u32, limit: u32) -> SearchResponse {
    fx.vfs
        .search(&SearchQuery {
            text: text.into(),
            filters: SearchFilters::default(),
            limit,
            offset,
            sort,
        })
        .unwrap()
}

fn names(hits: &[SearchHit]) -> Vec<&str> {
    hits.iter().map(|h| h.name.as_str()).collect()
}

fn file(name: &str, size: u64, mtime: i64) -> (Option<usize>, String, u64, i64) {
    (None, name.to_string(), size, mtime)
}

/// VFS-12 AC-1: newest, oldest, name and size orders; an unknown date (0) is last in both
/// date orders, folders are last by size, and ties keep the relevance order.
#[test]
fn ac1_each_order_sorts_every_result() {
    let fx = index(
        &["Facturas viejas"],
        &[
            file("factura luz.pdf", 300, 30),
            file("Factura agua.pdf", 100, 10),
            file("una factura.pdf", 200, 0),
            file("factura.pdf", 200, 20),
        ],
    );
    let q = |sort| names(&search(&fx, "factura", sort, 0, 100).hits).join(" | ");
    assert_eq!(
        q(SortOrder::Newest),
        "factura luz.pdf | factura.pdf | Factura agua.pdf | Facturas viejas | una factura.pdf"
    );
    assert_eq!(
        q(SortOrder::Oldest),
        "Facturas viejas | Factura agua.pdf | factura.pdf | factura luz.pdf | una factura.pdf"
    );
    assert_eq!(
        q(SortOrder::Name),
        "Factura agua.pdf | factura luz.pdf | factura.pdf | Facturas viejas | una factura.pdf"
    );
    // Equal sizes keep relevance: "factura.pdf" (whole stem) before "una factura.pdf".
    assert_eq!(
        q(SortOrder::Size),
        "factura luz.pdf | factura.pdf | una factura.pdf | Factura agua.pdf | Facturas viejas"
    );
}

/// VFS-12 AC-1: paging a sorted list returns the same sequence as one big page.
#[test]
fn ac1_pages_keep_the_order() {
    let files: Vec<_> = (0..30)
        .map(|i| file(&format!("nota {i}.txt"), i, 1_000 - i as i64 * 7 % 13))
        .collect();
    let fx = index(&[], &files);
    for sort in [SortOrder::Newest, SortOrder::Name, SortOrder::Size] {
        let all = names(&search(&fx, "nota", sort, 0, 100).hits)
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let mut paged = Vec::new();
        for page in 0..3 {
            let r = search(&fx, "nota", sort, page * 10, 10);
            paged.extend(r.hits.iter().map(|h| h.name.clone()));
        }
        assert_eq!(paged, all, "{sort:?}");
    }
}

/// VFS-12 AC-2: with more matches than the 2,000-hit cap, the newest file is first even when
/// more than 2,000 better-ranked files exist, and the order is exact.
#[test]
fn ac2_sorting_happens_before_the_cap() {
    let mut files: Vec<_> = (0..2_500)
        .map(|i| file(&format!("factura {i:04}.pdf"), 1, 1_000 + i))
        .collect();
    // A path-only match (ranked after every name match), newest of all.
    files.push((Some(0), "recibo.pdf".to_string(), 9_999, 9_000));
    let fx = index(&["Factura nueva"], &files);

    let relevance = search(&fx, "factura", SortOrder::Relevance, 0, 500);
    assert!(relevance.truncated);
    assert_ne!(relevance.hits[0].name, "recibo.pdf");

    let newest = search(&fx, "factura", SortOrder::Newest, 0, 500);
    assert_eq!(newest.hits[0].name, "recibo.pdf");
    assert_eq!(newest.hits[1].name, "factura 2499.pdf");
    assert!(newest.truncated, "more than 2,000 matches");
    assert!(!newest.approximate_order);
    let largest = search(&fx, "factura", SortOrder::Size, 0, 1);
    assert_eq!(largest.hits[0].name, "recibo.pdf");
}

/// VFS-12 AC-3: past the sorted read cap the order is flagged approximate; relevance never is.
#[test]
fn ac3_too_many_matches_make_the_order_approximate() {
    let files: Vec<_> = (0..20_100)
        .map(|i| file(&format!("foto {i:05}.jpg"), 1, i))
        .collect();
    let fx = index(&[], &files);
    let newest = search(&fx, "foto", SortOrder::Newest, 0, 10);
    assert!(newest.approximate_order);
    assert!(newest.truncated);
    let relevance = search(&fx, "foto", SortOrder::Relevance, 0, 10);
    assert!(!relevance.approximate_order);
}

/// VFS-12 AC-4: the default order is relevance, unchanged.
#[test]
fn ac4_default_order_is_relevance() {
    let fx = index(
        &[],
        &[
            file("mi factura.pdf", 1, 50),
            file("factura.pdf", 1, 10),
            file("factura 2019.pdf", 1, 30),
        ],
    );
    let q: SearchQuery = serde_json::from_value(serde_json::json!({ "text": "factura" })).unwrap();
    assert_eq!(q.sort, SortOrder::Relevance);
    let r = fx.vfs.search(&q).unwrap();
    assert_eq!(
        names(&r.hits),
        ["factura.pdf", "factura 2019.pdf", "mi factura.pdf"]
    );
    assert!(!r.approximate_order);
}
