//! `retrieve` with a ranker by meaning (VFS-29 R11–R13, D15, D17): fusion with BM25, the
//! partial-list rule, filters, hits found by meaning alone, and the fallback. The ranker here
//! is a fake that answers fixed chunks; the real one is `vfs-semantic`'s.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use rusqlite::Connection;
use vfs_content::{
    IndexOptions, Meaning, MeaningHit, MeaningUnavailable, MeaningUse, Query, Searcher,
    StructuredQuery,
};
use vfs_core::{SearchFilters, Vfs};

struct Setup {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
}

const DENTIST: &str = "Clínica dental. Factura de la endodoncia y el empaste de la pieza 36.";

fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Archivo");
    std::fs::create_dir_all(root.join("salud")).unwrap();
    std::fs::create_dir_all(root.join("casa")).unwrap();
    let files = [
        ("salud/dentista.txt", DENTIST),
        (
            "casa/agua.txt",
            "Recibo del agua de julio: consumo de 28 metros cúbicos.",
        ),
        (
            "casa/alquiler.txt",
            "Contrato de alquiler con fianza de dos mensualidades.",
        ),
        (
            "salud/gastos.txt",
            "Gastos médicos del año: farmacia y óptica.",
        ),
    ];
    for (rel, text) in files {
        std::fs::write(root.join(rel), text).unwrap();
    }
    let db = tmp.path().join("index.db");
    let vfs = Vfs::open(&db).unwrap();
    vfs.add_root(&root).unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    // The root as the index stores it (canonical, e.g. /private/var on macOS).
    let root = PathBuf::from(&vfs.list_roots().unwrap()[0].path);
    let opts = IndexOptions::new(env!("CARGO_BIN_EXE_vfs-extract"), Vec::new());
    let sum = vfs_content::index(&db, &opts, &AtomicBool::new(false)).unwrap();
    assert_eq!(sum.failed, 0);
    Setup {
        _tmp: tmp,
        root,
        db,
    }
}

impl Setup {
    fn id(&self, name: &str) -> i64 {
        Connection::open(&self.db)
            .unwrap()
            .query_row("SELECT id FROM files WHERE name = ?1", [name], |r| r.get(0))
            .unwrap()
    }

    fn searcher(&self, meaning: Fake) -> Searcher {
        Searcher::open(&self.db)
            .unwrap()
            .with_meaning(Arc::new(meaning))
    }
}

/// Answers the given documents' first chunk, in order, for any query; or is unavailable.
struct Fake(Result<Vec<i64>, MeaningUnavailable>);

impl Meaning for Fake {
    fn rank(&self, _query: &str, n: usize) -> Result<Vec<MeaningHit>, MeaningUnavailable> {
        let ids = self.0.clone()?;
        Ok(ids
            .into_iter()
            .enumerate()
            .map(|(i, file_id)| MeaningHit {
                file_id,
                ordinal: 0,
                score: 1.0 - i as f32 / 10.0,
            })
            .take(n)
            .collect())
    }
}

fn text(q: &str) -> Query {
    Query::Text(q.into())
}

/// VFS-29 AC-10 (R11, R12): «gastos médicos» finds no document with both words; the
/// dentist's invoice, found by meaning, is in the top 5 marked «relacionado por tema», with
/// its passage as the snippet and nothing highlighted.
#[test]
fn a_document_found_by_meaning_alone_is_marked() {
    let s = setup();
    let dentist = s.id("dentista.txt");
    let r = s
        .searcher(Fake(Ok(vec![dentist])))
        .retrieve(&text("gastos dentales"), &SearchFilters::default(), 5, 0)
        .unwrap();
    assert_eq!(r.meaning, MeaningUse::Used);
    let hit = r
        .hits
        .iter()
        .find(|h| h.id == dentist)
        .expect("in the top 5");
    assert!(hit.by_meaning);
    assert!(hit.highlights.is_empty());
    assert!(hit.snippet.starts_with("Clínica dental"), "{}", hit.snippet);
    assert_eq!(hit.passages[0].ordinal, 0);
    assert_eq!(hit.dir, s.root.join("salud").to_string_lossy());
}

/// VFS-29 AC-10, D15: a question retrieves with a structured query through the same
/// `retrieve` (vfs-ai's `LocalCorpus`), so it gets the passage found by meaning with no
/// change of its own.
#[test]
fn a_question_gets_the_passage_found_by_meaning() {
    let s = setup();
    let dentist = s.id("dentista.txt");
    let q = Query::Structured(StructuredQuery {
        any: vec!["gastos".into(), "dentista".into()],
        ..Default::default()
    });
    let r = s
        .searcher(Fake(Ok(vec![dentist])))
        .retrieve(&q, &SearchFilters::default(), 20, 0)
        .unwrap();
    let hit = r.hits.iter().find(|h| h.id == dentist).unwrap();
    assert!(hit.by_meaning);
    assert_eq!(
        (hit.passages[0].file_id, hit.passages[0].ordinal),
        (dentist, 0)
    );
}

/// VFS-29 R11 (D7): with every query word in a document, BM25's list and the ranker's are
/// fused; a document in both is a word hit (highlighted, not «relacionado por tema»).
#[test]
fn word_hits_and_meaning_hits_are_fused() {
    let s = setup();
    let (rent, dentist) = (s.id("alquiler.txt"), s.id("dentista.txt"));
    let r = s
        .searcher(Fake(Ok(vec![rent, dentist])))
        .retrieve(&text("alquiler fianza"), &SearchFilters::default(), 5, 0)
        .unwrap();
    let ids: Vec<i64> = r.hits.iter().map(|h| h.id).collect();
    assert_eq!(ids[0], rent, "in both lists, first");
    assert!(ids.contains(&dentist));
    assert!(!r.hits[0].by_meaning && !r.hits[0].highlights.is_empty());
    assert!(!r.partial);
}

/// VFS-29 D17: when no document has every query word, BM25's any-word list is left out of
/// the fusion: the hits are the ranker's.
#[test]
fn a_partial_word_list_is_left_out() {
    let s = setup();
    let dentist = s.id("dentista.txt");
    let plain = Searcher::open(&s.db).unwrap();
    let bm25 = plain
        .retrieve(&text("gastos dentales"), &SearchFilters::default(), 5, 0)
        .unwrap();
    assert!(
        bm25.partial && !bm25.hits.is_empty(),
        "«gastos» alone matches"
    );
    let r = s
        .searcher(Fake(Ok(vec![dentist])))
        .retrieve(&text("gastos dentales"), &SearchFilters::default(), 5, 0)
        .unwrap();
    let ids: Vec<i64> = r.hits.iter().map(|h| h.id).collect();
    assert_eq!(ids, vec![dentist]);
    assert!(!r.partial);
}

/// VFS-29 R11: documents found by meaning obey the filters on screen like word hits.
#[test]
fn meaning_hits_obey_the_filters() {
    let s = setup();
    let (dentist, water) = (s.id("dentista.txt"), s.id("agua.txt"));
    let filters = SearchFilters {
        folder: Some(s.root.join("casa").to_string_lossy().into_owned()),
        ..Default::default()
    };
    let r = s
        .searcher(Fake(Ok(vec![dentist, water])))
        .retrieve(&text("consumo mensual"), &filters, 5, 0)
        .unwrap();
    let ids: Vec<i64> = r.hits.iter().map(|h| h.id).collect();
    assert!(ids.contains(&water));
    assert!(!ids.contains(&dentist), "outside the folder");
}

/// VFS-29 AC-11 (R13): when the ranker cannot answer, `retrieve` returns BM25's hits and says
/// meaning search was unavailable, for the «Documentos» tab's notice.
#[test]
fn an_unavailable_ranker_falls_back_to_words() {
    let s = setup();
    let plain = Searcher::open(&s.db).unwrap();
    let want = plain
        .retrieve(&text("alquiler fianza"), &SearchFilters::default(), 5, 0)
        .unwrap();
    assert_eq!(want.meaning, MeaningUse::Off);
    for why in [
        MeaningUnavailable::NotReady,
        MeaningUnavailable::Failed("timeout".into()),
    ] {
        let r = s
            .searcher(Fake(Err(why)))
            .retrieve(&text("alquiler fianza"), &SearchFilters::default(), 5, 0)
            .unwrap();
        assert_eq!(r.meaning, MeaningUse::Unavailable);
        let a: Vec<i64> = r.hits.iter().map(|h| h.id).collect();
        let b: Vec<i64> = want.hits.iter().map(|h| h.id).collect();
        assert_eq!(a, b);
    }
}

/// VFS-29 AC-1 (R15): with meaning search off, searching creates no vector store and the
/// response says meaning search did not take part.
#[test]
fn with_meaning_off_nothing_is_created() {
    let s = setup();
    let r = Searcher::open(&s.db)
        .unwrap()
        .retrieve(&text("alquiler"), &SearchFilters::default(), 5, 0)
        .unwrap();
    assert_eq!(r.meaning, MeaningUse::Off);
    assert!(!s.db.with_file_name("vectors.db").exists());
}

/// VFS-29 R11: paging the fused list returns each document once.
#[test]
fn fused_pages_do_not_repeat() {
    let s = setup();
    let ids_all = ["dentista.txt", "agua.txt", "alquiler.txt", "gastos.txt"].map(|n| s.id(n));
    let m = || Fake(Ok(ids_all.to_vec()));
    let first = s
        .searcher(m())
        .retrieve(&text("recibo"), &SearchFilters::default(), 2, 0)
        .unwrap();
    let second = s
        .searcher(m())
        .retrieve(&text("recibo"), &SearchFilters::default(), 2, 2)
        .unwrap();
    let mut seen: Vec<i64> = first
        .hits
        .iter()
        .chain(&second.hits)
        .map(|h| h.id)
        .collect();
    assert_eq!(seen.len(), 4);
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 4);
}

/// VFS-29 D17, R11: when nothing found by meaning is inside the filters, the word hits stay,
/// even a partial list: the fusion never empties a search BM25 answered.
#[test]
fn word_hits_stay_when_meaning_finds_nothing_inside_the_filters() {
    let s = setup();
    let filters = SearchFilters {
        folder: Some(s.root.join("salud").to_string_lossy().into_owned()),
        ..Default::default()
    };
    let plain = Searcher::open(&s.db).unwrap();
    let want = plain
        .retrieve(&text("gastos dentales"), &filters, 5, 0)
        .unwrap();
    assert!(want.partial && !want.hits.is_empty());
    let water = s.id("agua.txt");
    let r = s
        .searcher(Fake(Ok(vec![water])))
        .retrieve(&text("gastos dentales"), &filters, 5, 0)
        .unwrap();
    let a: Vec<i64> = r.hits.iter().map(|h| h.id).collect();
    let b: Vec<i64> = want.hits.iter().map(|h| h.id).collect();
    assert_eq!(a, b);
}

/// VFS-53 AC-5 (R6): related documents become hits found by meaning, in the order given, each
/// with the given chunk as its passage; a document no longer in the index is left out.
#[test]
fn related_documents_become_hits_with_their_passage() {
    let s = setup();
    let (dentist, water) = (s.id("dentista.txt"), s.id("agua.txt"));
    let hits = Searcher::open(&s.db)
        .unwrap()
        .related_hits(&[(water, 0, 0.6), (999_999, 0, 0.5), (dentist, 0, 0.4)])
        .unwrap();
    let ids: Vec<i64> = hits.iter().map(|h| h.id).collect();
    assert_eq!(ids, vec![water, dentist]);
    assert!(hits.iter().all(|h| h.by_meaning && h.highlights.is_empty()));
    assert!(
        hits[1].snippet.starts_with("Clínica dental"),
        "{}",
        hits[1].snippet
    );
    assert_eq!(hits[1].passages[0].ordinal, 0);
}
