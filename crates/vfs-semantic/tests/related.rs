//! Document vectors and «Relacionados» (VFS-53) against a real `content.db` and `vectors.db`,
//! with a fake embedder whose vectors follow the subject of each text.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use vfs_semantic::quant;
use vfs_semantic::related::{self, MAX_RELATED, MIN_SIMILARITY};
use vfs_semantic::store::{self, Store};
use vfs_semantic::worker::fake_vector;
use vfs_semantic::{run, EmbedError, Embedder, ModelId, RunOptions, RunSummary, LOCAL_MODEL};

const DIM: usize = 32;

fn model() -> ModelId {
    ModelId {
        name: "m",
        dim: DIM,
        ..LOCAL_MODEL
    }
}

/// Vectors near one axis per subject (by a word in the text), plus a small per-text part and
/// a component every vector shares, as e5's vectors do (VFS-29 D19).
struct Topical;

const SUBJECTS: [&str; 3] = ["dentista", "coche", "jardin"];

fn topical(text: &str) -> Vec<f32> {
    let noise = fake_vector(text, DIM);
    let mut v: Vec<f32> = (0..DIM).map(|i| 0.5 + 0.15 * noise[i]).collect();
    for (axis, word) in SUBJECTS.iter().enumerate() {
        if text.contains(word) {
            v[axis] += 1.0;
        }
    }
    quant::normalise(&mut v);
    v
}

impl Embedder for Topical {
    fn model(&self) -> ModelId {
        model()
    }
    fn embed(
        &mut self,
        texts: &[String],
        _query: bool,
        _stop: &AtomicBool,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts.iter().map(|t| topical(t)).collect())
    }
}

struct Archive {
    _dir: tempfile::TempDir,
    db: PathBuf,
}

impl Archive {
    fn new(docs: &[(i64, String)]) -> Archive {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        for (id, text) in docs {
            vfs_content::testing::write_doc(&db, *id, text).unwrap();
        }
        Archive { _dir: dir, db }
    }
}

/// A text of `chunks` different chunks about `subject`.
fn text(subject: &str, seed: &str, chunks: usize) -> String {
    (0..chunks)
        .map(|i| format!("{subject} {seed} parte {i}. ").repeat(80) + "\n\n")
        .collect()
}

fn embed(db: &Path) -> RunSummary {
    run(
        db,
        &mut Topical,
        &RunOptions { batch: 8 },
        &AtomicBool::new(false),
        &mut |_| true,
    )
    .unwrap()
}

fn doc_count(db: &Path) -> i64 {
    rusqlite::Connection::open(store::store_path(db))
        .unwrap()
        .query_row("SELECT count(*) FROM docs", [], |r| r.get(0))
        .unwrap()
}

/// Three documents per subject, ids 1–3 dentist, 4–6 car, 7–9 garden.
fn three_subjects() -> Archive {
    let mut docs = Vec::new();
    for (s, subject) in SUBJECTS.iter().enumerate() {
        for k in 0..3 {
            let id = (s * 3 + k + 1) as i64;
            docs.push((id, text(subject, &format!("doc{id}"), 1 + k)));
        }
    }
    Archive::new(&docs)
}

/// VFS-53 AC-1 (R1): a completed run stores one document vector per document, the centred,
/// normalised mean of its chunk vectors.
#[test]
fn a_completed_run_stores_document_vectors() {
    let a = Archive::new(&[(1, text("dentista", "a", 2)), (2, text("coche", "b", 1))]);
    let sum = embed(&a.db);
    assert!(sum.complete);
    assert_eq!(sum.docs_written, 2);
    assert_eq!(doc_count(&a.db), 2);

    let s = Store::open(&a.db).unwrap();
    let set = s.set_id(&model()).unwrap();
    let mean = s.mean(set).unwrap().unwrap();
    let chunks = s.chunk_hashes(1).unwrap();
    assert_eq!(chunks.len(), 2);
    let mut expected = vec![0f32; DIM];
    for (_, h) in &chunks {
        let (q8, scale) = s.q8(set, h).unwrap().unwrap();
        for (e, x) in expected.iter_mut().zip(&q8) {
            *e += *x as f32 * scale / 2.0;
        }
    }
    expected.iter_mut().zip(&mean).for_each(|(e, m)| *e -= m);
    quant::normalise(&mut expected);
    let (q8, scale) = s.doc_q8(set, 1).unwrap().unwrap();
    let cos = quant::dot_int8(&expected, &q8, scale);
    assert!(cos > 0.99, "document vector off by cosine {cos}");
}

/// VFS-53 AC-2 (R2): an unchanged archive writes no document row; a changed document
/// rewrites its own row only; a removed document's row is dropped.
#[test]
fn document_vectors_follow_changes_only() {
    let a = three_subjects();
    embed(&a.db);
    assert_eq!(doc_count(&a.db), 9);

    let sum = embed(&a.db);
    assert_eq!(
        sum.docs_written, 0,
        "an unchanged archive rewrote document rows"
    );

    vfs_content::testing::write_doc(&a.db, 2, &text("dentista", "cambiado", 2)).unwrap();
    let sum = embed(&a.db);
    assert_eq!(sum.docs_written, 1);

    vfs_content::testing::remove_doc(&a.db, 9).unwrap();
    embed(&a.db);
    assert_eq!(doc_count(&a.db), 8);
}

/// VFS-53 AC-3 (R3): a version-2 store keeps its vectors, gains the `docs` table, and the next
/// run fills it without embedding any text again.
#[test]
fn a_version_2_store_gains_document_vectors_without_embedding() {
    let a = three_subjects();
    embed(&a.db);
    let vectors_before;
    {
        let c = rusqlite::Connection::open(store::store_path(&a.db)).unwrap();
        c.execute_batch("DROP TABLE docs; PRAGMA user_version = 2;")
            .unwrap();
        vectors_before = c
            .query_row("SELECT count(*) FROM vectors", [], |r| r.get::<_, i64>(0))
            .unwrap();
    }
    // Read-only, before the run, a related request finds nothing and does not fail.
    assert!(related::related(&a.db, 1, MAX_RELATED).unwrap().is_empty());
    let sum = embed(&a.db);
    assert_eq!(sum.embedded, 0, "kept vectors were embedded again");
    assert_eq!(sum.docs_written, 9);
    let c = rusqlite::Connection::open(store::store_path(&a.db)).unwrap();
    let v: i64 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, store::VECTORS_VERSION);
    let after: i64 = c
        .query_row("SELECT count(*) FROM vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, vectors_before);
}

/// VFS-53 AC-4 (R5, R7): the documents of the same subject come first, never the asked
/// document itself, and nothing below the threshold.
#[test]
fn related_documents_share_the_subject() {
    let a = three_subjects();
    embed(&a.db);
    let r = related::related(&a.db, 4, MAX_RELATED).unwrap();
    let ids: Vec<i64> = r.iter().map(|x| x.file_id).collect();
    assert!(!ids.contains(&4));
    assert!(ids.len() >= 2, "{r:?}");
    let mut first_two = ids[..2].to_vec();
    first_two.sort();
    assert_eq!(first_two, vec![5, 6], "{r:?}");
    assert!(r.iter().all(|x| x.score >= MIN_SIMILARITY), "{r:?}");
    assert!(r.windows(2).all(|w| w[0].score >= w[1].score));
    // A document with no vector has no related documents.
    assert!(related::related(&a.db, 99, MAX_RELATED).unwrap().is_empty());
}

/// VFS-53 AC-5 (R6): a related document's passage is its chunk closest to the asked document.
#[test]
fn the_passage_is_the_closest_chunk() {
    // Document 2: one chunk about the garden, then two about the dentist.
    let mixed = format!("{}{}", text("jardin", "x", 1), text("dentista", "y", 2));
    // Other subjects around them, so the set's mean is not the dentist's.
    let mut docs = vec![(1, text("dentista", "a", 2)), (2, mixed)];
    for id in 3..9 {
        let subject = if id % 2 == 0 { "coche" } else { "jardin" };
        docs.push((id, text(subject, &format!("o{id}"), 1)));
    }
    let a = Archive::new(&docs);
    embed(&a.db);
    let s = Store::open(&a.db).unwrap();
    let chunks = s.chunk_hashes(2).unwrap();
    assert_eq!(chunks.len(), 3);
    drop(s);
    let r = related::related(&a.db, 1, MAX_RELATED).unwrap();
    let hit = r
        .iter()
        .find(|x| x.file_id == 2)
        .expect("the mixed document is related");
    assert_ne!(
        hit.ordinal, chunks[0].0,
        "the garden chunk was picked: {r:?}"
    );
}

/// VFS-53 AC-6 (R9): with 60,000 document rows of e5-small's 384 dimensions, related documents
/// come back within 50 ms in a release build (lenient in debug; the time is printed).
#[test]
fn related_is_fast_on_sixty_thousand_documents() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let real = ModelId {
        name: "real",
        ..LOCAL_MODEL
    };
    let dim = real.dim;
    {
        let mut s = Store::open(&db).unwrap();
        let set = s.set_id(&real).unwrap();
        // The first batch fixes the set's mean; the set serves searches.
        let chunks: Vec<_> = (0..64)
            .map(|i| {
                let t = format!("c{i}");
                (store::text_hash(&t), fake_vector(&t, dim))
            })
            .collect();
        s.insert(set, &chunks).unwrap();
        s.set_complete(set, true).unwrap();
        let unit = |t: &str| {
            let mut v = fake_vector(t, dim);
            quant::normalise(&mut v);
            v
        };
        let target = unit("target");
        let mut rows: Vec<(i64, [u8; 16], Vec<f32>)> = vec![(1, [0u8; 16], target.clone())];
        // Five documents close to the target, the rest random.
        for id in 2..7 {
            let mut v: Vec<f32> = target
                .iter()
                .zip(unit(&format!("n{id}")))
                .map(|(a, b)| a + 0.5 * b)
                .collect();
            quant::normalise(&mut v);
            rows.push((id, [0u8; 16], v));
        }
        rows.extend((10..60_010).map(|id| (id, [0u8; 16], unit(&format!("d{id}")))));
        for batch in rows.chunks(5_000) {
            s.put_docs(set, batch).unwrap();
        }
    }
    // The first request reads the rows from disk; the timed one finds them in the OS cache.
    let first = related::related(&db, 1, MAX_RELATED).unwrap();
    let mut got: Vec<i64> = first.iter().map(|r| r.file_id).collect();
    got.sort();
    assert_eq!(got, vec![2, 3, 4, 5, 6]);
    let t = Instant::now();
    related::related(&db, 1, MAX_RELATED).unwrap();
    let ms = t.elapsed().as_millis();
    eprintln!("related over 60,000 documents: {ms} ms");
    // R9 is a release figure; a debug build (CI) only checks it stays far from seconds.
    let bound = if cfg!(debug_assertions) { 2_000 } else { 50 };
    assert!(ms < bound, "{ms} ms");
}

/// VFS-53 AC-9 (R9): without search by meaning there is no `vectors.db`, and a related
/// request does not create one.
#[test]
fn no_vector_store_is_created_by_a_related_request() {
    let a = Archive::new(&[(1, text("dentista", "a", 1))]);
    assert!(related::related(&a.db, 1, MAX_RELATED).unwrap().is_empty());
    assert!(!store::exists(&a.db));
}

/// VFS-53 R9 (cost): building the document vectors of 6,000 documents of 9 chunks each of 384
/// dimensions, one tenth of the target archive, from stored chunk vectors; the time is printed for the PR, and
/// a second update over the unchanged archive writes nothing.
#[test]
fn building_document_vectors_cost() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let mut s = Store::open(&db).unwrap();
    let set = s.set_id(&model()).unwrap();
    let mut locations = Vec::new();
    let mut batch = Vec::new();
    // 6,000 documents in release for the PR's figure; 600 in a debug build to keep CI fast.
    let n_docs: i64 = if cfg!(debug_assertions) { 600 } else { 6_000 };
    for doc in 0..n_docs {
        for ord in 0..9u32 {
            let t = format!("d{doc} c{ord}");
            let h = store::text_hash(&t);
            locations.push((h, doc, ord));
            batch.push((h, fake_vector(&t, LOCAL_MODEL.dim)));
            if batch.len() == 4_096 {
                s.insert(set, &batch).unwrap();
                batch.clear();
            }
        }
    }
    s.insert(set, &batch).unwrap();
    locations.sort_unstable();
    s.set_locations(&locations).unwrap();
    let t = Instant::now();
    let u = related::update_docs(&mut s, set, &AtomicBool::new(false)).unwrap();
    eprintln!(
        "document vectors for {n_docs} documents: {} ms",
        t.elapsed().as_millis()
    );
    assert_eq!(u.written, n_docs as u64);
    let t = Instant::now();
    let u = related::update_docs(&mut s, set, &AtomicBool::new(false)).unwrap();
    eprintln!("unchanged update: {} ms", t.elapsed().as_millis());
    assert_eq!(u.written, 0);
}
