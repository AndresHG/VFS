//! A suggestion for a search with no hits (VFS-13).

use std::time::{Duration, Instant};

use tempfile::TempDir;
use vfs_core::db::{self, ingest};
use vfs_core::{
    FileKind, Scope, SearchConfig, SearchFilters, SearchQuery, SortOrder, Suggestion,
    SuggestionKind, TruncatedBy, Vfs,
};

struct Fx {
    _dir: TempDir,
    vfs: Vfs,
}

/// An index with root `/datos` holding `folders` (relative paths, parents first) and
/// `files` (folder index into `folders` or `None` for the root, name, size, mtime).
fn index_with(
    config: SearchConfig,
    folders: &[&str],
    files: &[(Option<usize>, String, u64, i64)],
) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    let vfs = Vfs::open_with(&path, config).unwrap();
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

fn patient() -> SearchConfig {
    SearchConfig {
        deadline: Duration::from_secs(60),
        ..SearchConfig::default()
    }
}

fn index(folders: &[&str], files: &[(Option<usize>, String, u64, i64)]) -> Fx {
    index_with(patient(), folders, files)
}

fn file(name: &str) -> (Option<usize>, String, u64, i64) {
    (None, name.to_string(), 1, 1)
}

fn query(text: &str, filters: SearchFilters) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        filters,
        limit: 50,
        offset: 0,
        sort: SortOrder::Relevance,
        scope: Default::default(),
    }
}

fn suggest(fx: &Fx, text: &str, filters: SearchFilters) -> Option<Suggestion> {
    let q = query(text, filters);
    assert_eq!(
        fx.vfs.search(&q).unwrap().total,
        0,
        "{text} must find nothing"
    );
    fx.vfs.suggest(&q).unwrap()
}

/// `suggest` for a names-only query (VFS-38).
fn suggest_names(fx: &Fx, text: &str) -> Option<Suggestion> {
    let q = SearchQuery {
        scope: Scope::Names,
        ..query(text, SearchFilters::default())
    };
    assert_eq!(
        fx.vfs.search(&q).unwrap().total,
        0,
        "{text} must find nothing"
    );
    fx.vfs.suggest(&q).unwrap()
}

/// VFS-38 AC-11 (R13), core part: `sierra excursión` under names only, where only the folder
/// path `Sierra/Excursión` holds both words, finds nothing; with no other filter, dropping
/// the filters is offered with the «Todo» count, and that search shows the folder's photo.
#[test]
fn vfs38_ac11_names_only_suggests_dropping_the_scope() {
    let fx = index(
        &["Sierra", "Sierra/Excursión"],
        &[(Some(1), "IMG_0412.jpg".into(), 1, 1)],
    );
    let s = suggest_names(&fx, "sierra excursión").unwrap();
    assert_eq!(s.kind, SuggestionKind::DropFilters);
    assert!(s.clear_filters);
    let all = fx
        .vfs
        .search(&query("sierra excursión", SearchFilters::default()))
        .unwrap();
    assert_eq!((s.count, s.capped), (all.total, false));
    assert!(all.hits.iter().any(|h| h.name == "IMG_0412.jpg"));
}

/// VFS-38 R13: the spelling and drop-a-word suggestions keep the names-only scope: their
/// counts leave out `x.jpg`, found under «Todo» only through its folder `Bodas viejas`.
#[test]
fn vfs38_spelling_and_drop_term_keep_names_only() {
    let fx = index(
        &["Bodas viejas"],
        &[file("boda Ana.mp4"), (Some(0), "x.jpg".into(), 1, 1)],
    );
    let s = suggest_names(&fx, "Bdoa").unwrap();
    assert_eq!(s.kind, SuggestionKind::Spelling);
    assert_eq!((s.text.as_str(), s.count), ("boda", 2));
    let s = suggest_names(&fx, "boda zzzz").unwrap();
    assert_eq!(s.kind, SuggestionKind::DropTerm);
    assert_eq!((s.text.as_str(), s.count), ("boda", 2));
    let all = fx
        .vfs
        .search(&query("boda", SearchFilters::default()))
        .unwrap();
    assert_eq!(all.total, 3);
}

fn photos() -> SearchFilters {
    SearchFilters {
        kinds: vec![FileKind::Photo],
        ..SearchFilters::default()
    }
}

/// VFS-13 AC-1: a filter that hides every match suggests the same text without filters,
/// with the unfiltered count; no filter, or no text, skips that try.
#[test]
fn ac1_filters_hiding_matches_suggest_dropping_them() {
    let fx = index(&[], &[file("factura luz.pdf"), file("factura agua.pdf")]);
    let s = suggest(&fx, "factura", photos()).unwrap();
    assert_eq!(s.kind, SuggestionKind::DropFilters);
    assert!(s.clear_filters);
    assert_eq!((s.text.as_str(), s.count, s.capped), ("factura", 2, false));
    assert_eq!(suggest(&fx, "", photos()), None);
}

/// VFS-13 AC-2: a swapped pair in a short word suggests the right spelling, before any
/// broader deletion; terms of 3 letters, 6 or more, or with a digit or punctuation get none.
#[test]
fn ac2_a_short_misspelt_word_suggests_its_spelling() {
    let fx = index(
        &[],
        &[
            file("boda Ana.mp4"),
            file("boda Luis.mp4"),
            file("Lisboa.jpg"),
            file("Lote12.pdf"),
        ],
    );
    let s = suggest(&fx, "Bdoa", SearchFilters::default()).unwrap();
    assert_eq!(s.kind, SuggestionKind::Spelling);
    assert_eq!(s.term.as_deref(), Some("Bdoa"));
    assert_eq!(s.replacement.as_deref(), Some("boda"));
    assert_eq!((s.text.as_str(), s.count), ("boda", 2));
    // "ltoe1" would match "Lote12" with "t" and "o" swapped, but it holds a digit.
    for none in ["bdo", "qwerty", "ltoe1", "bo.da"] {
        assert_eq!(suggest(&fx, none, SearchFilters::default()), None, "{none}");
    }
}

/// VFS-13 AC-3: of two words, the one no file holds is dropped (last first); a 4-digit word
/// gets no spelling try, so "2019" never becomes "201".
#[test]
fn ac3_a_word_no_file_holds_is_dropped() {
    let fx = index(&[], &[file("boda Ana.mp4"), file("boda 2018.mp4")]);
    let s = suggest(&fx, "boda  2019", SearchFilters::default()).unwrap();
    assert_eq!(s.kind, SuggestionKind::DropTerm);
    assert_eq!(s.term.as_deref(), Some("2019"));
    assert_eq!((s.text.as_str(), s.count), ("boda", 2));
}

/// VFS-13 AC-4: when no try finds anything there is no suggestion.
#[test]
fn ac4_nothing_found_means_no_suggestion() {
    let fx = index(&[], &[file("boda Ana.mp4")]);
    assert_eq!(suggest(&fx, "zzzz qqqq", photos()), None);
}

/// VFS-13 AC-5: a spent budget returns at once with no suggestion.
#[test]
fn ac5_a_spent_budget_returns_at_once() {
    let config = SearchConfig {
        suggest_budget: Duration::ZERO,
        ..patient()
    };
    let fx = index_with(config, &[], &[file("factura luz.pdf")]);
    let t = Instant::now();
    assert_eq!(suggest(&fx, "factura", photos()), None);
    assert!(t.elapsed() < Duration::from_millis(100));
}

/// VFS-13 AC-5: suggestions never interrupt a search: searches running alongside them come
/// back complete, never cut by the deadline.
#[test]
fn ac5_a_suggestion_never_cuts_a_search() {
    let files: Vec<_> = (0..3_000)
        .map(|i| file(&format!("factura {i:04}.pdf")))
        .collect();
    let fx = index(&[], &files);
    let vfs = fx.vfs.clone();
    let suggester = std::thread::spawn(move || {
        for _ in 0..30 {
            vfs.suggest(&query("factura", photos())).unwrap();
        }
    });
    for i in 0..30 {
        let r = fx
            .vfs
            .search(&query(&format!("factura {i:04}"), SearchFilters::default()))
            .unwrap();
        assert_ne!(r.truncated_by, Some(TruncatedBy::Deadline));
        assert!(r.total >= 1);
    }
    suggester.join().unwrap();
}

/// VFS-13 AC-6: a suggestion leaves the cached ranked list of the last search alone, so its
/// next page continues the same list.
#[test]
fn ac6_a_suggestion_leaves_the_last_list_alone() {
    let files: Vec<_> = (0..300)
        .map(|i| file(&format!("factura {i:03}.pdf")))
        .collect();
    let fx = index(&[], &files);
    let mut q = query("factura", SearchFilters::default());
    q.limit = 300;
    let all: Vec<i64> = fx
        .vfs
        .search(&q)
        .unwrap()
        .hits
        .iter()
        .map(|h| h.id)
        .collect();
    q.limit = 100;
    q.offset = 0;
    fx.vfs.search(&q).unwrap();
    fx.vfs.suggest(&query("factura", photos())).unwrap();
    q.offset = 100;
    let page: Vec<i64> = fx
        .vfs
        .search(&q)
        .unwrap()
        .hits
        .iter()
        .map(|h| h.id)
        .collect();
    assert_eq!(page, all[100..200]);
}
