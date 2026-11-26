//! The embedding run against a real `content.db` and `vectors.db` (VFS-29 R5–R7, R10, D10),
//! with an in-process fake embedder that records every text it is sent.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use vfs_semantic::store::{self, Store};
use vfs_semantic::worker::fake_vector;
use vfs_semantic::{run, EmbedError, Embedder, ModelId, RunOptions, RunSummary, LOCAL_MODEL};

const DIM: usize = 16;

fn model(name: &'static str) -> ModelId {
    ModelId {
        name,
        dim: DIM,
        ..LOCAL_MODEL
    }
}

/// Records each text; refuses (as if killed) once `fail_after` texts have been embedded.
struct Fake {
    model: ModelId,
    sent: Vec<String>,
    fail_after: Option<usize>,
}

impl Fake {
    fn new(model: ModelId) -> Fake {
        Fake {
            model,
            sent: Vec::new(),
            fail_after: None,
        }
    }
}

impl Embedder for Fake {
    fn model(&self) -> ModelId {
        self.model
    }
    fn embed(
        &mut self,
        texts: &[String],
        _query: bool,
        _stop: &AtomicBool,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        if let Some(n) = self.fail_after {
            if self.sent.len() + texts.len() > n {
                return Err(EmbedError::Stopped);
            }
        }
        self.sent.extend(texts.iter().cloned());
        Ok(texts.iter().map(|t| fake_vector(t, DIM)).collect())
    }
}

/// A content store with the given documents (name, text), as the content indexer writes it.
struct Archive {
    _dir: tempfile::TempDir,
    db: PathBuf,
}

impl Archive {
    fn new(docs: &[(i64, &str)]) -> Archive {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        write_content(&db, docs);
        Archive { _dir: dir, db }
    }
}

fn write_content(db: &Path, docs: &[(i64, &str)]) {
    for (id, text) in docs {
        vfs_content::testing::write_doc(db, *id, text).unwrap();
    }
}

/// A text of about `chunks` chunks, each different, built from `seed`.
fn long_text(seed: &str, chunks: usize) -> String {
    (0..chunks)
        .map(|i| format!("{seed} parte {i}. ").repeat(80) + "\n\n")
        .collect()
}

fn embed(db: &Path, e: &mut Fake) -> RunSummary {
    run(
        db,
        e,
        &RunOptions { batch: 2 },
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap()
}

fn chunk_count(db: &Path) -> usize {
    let c = vfs_content::ChunkReader::open(db).unwrap();
    c.doc_ids()
        .unwrap()
        .into_iter()
        .map(|id| c.chunk_texts(id).unwrap().len())
        .sum()
}

/// VFS-29 AC-14 (R6, D10): the same document in two folders sends each distinct chunk text
/// once, and both copies are found by meaning: the set holds a vector for every text.
#[test]
fn copies_send_each_text_once() {
    let text = long_text("Factura del dentista", 3);
    let a = Archive::new(&[(1, &text), (2, &text), (3, "Recibo del agua de julio.")]);
    let mut e = Fake::new(model("m"));
    let sum = embed(&a.db, &mut e);
    let distinct: HashSet<&String> = e.sent.iter().collect();
    assert_eq!(distinct.len(), e.sent.len(), "a text was sent twice");
    assert_eq!(sum.texts as usize, e.sent.len());
    assert_eq!(chunk_count(&a.db), 2 * (e.sent.len() - 1) + 1);
    assert!(sum.complete);
    let s = Store::open(&a.db).unwrap();
    let set = s.set_id(&model("m")).unwrap();
    for t in &e.sent {
        assert!(s.hashes(set).unwrap().contains(&store::text_hash(t)));
    }
}

/// VFS-29 AC-6 (R7): a run killed at 40 % resumes from the last stored batch, and no chunk
/// text is sent twice across the two runs.
#[test]
fn a_killed_run_resumes_without_resending() {
    let docs: Vec<String> = (0..5).map(|i| long_text(&format!("Doc {i}"), 2)).collect();
    let refs: Vec<(i64, &str)> = docs
        .iter()
        .enumerate()
        .map(|(i, t)| (i as i64 + 1, t.as_str()))
        .collect();
    let a = Archive::new(&refs);
    let total = chunk_count(&a.db);
    let mut first = Fake::new(model("m"));
    first.fail_after = Some(total * 4 / 10);
    let sum = embed(&a.db, &mut first);
    assert!(sum.stopped && !sum.complete);
    assert!(!first.sent.is_empty() && first.sent.len() < total);
    let mut second = Fake::new(model("m"));
    let sum = embed(&a.db, &mut second);
    assert!(sum.complete);
    let all: Vec<&String> = first.sent.iter().chain(&second.sent).collect();
    let distinct: HashSet<&&String> = all.iter().collect();
    assert_eq!(all.len(), total, "every text once");
    assert_eq!(distinct.len(), total);
}

/// VFS-29 AC-5 (R6): with a complete set, deleting and rebuilding `content.db` with one
/// document changed sends only that document's changed chunks; `vectors.db` survives and the
/// old text's vector is dropped.
#[test]
fn a_rebuilt_content_store_sends_only_changed_chunks() {
    let one = long_text("Contrato de alquiler", 3);
    let two = long_text("Póliza del hogar", 3);
    let a = Archive::new(&[(1, &one), (2, &two)]);
    let mut e = Fake::new(model("m"));
    assert!(embed(&a.db, &mut e).complete);

    vfs_content::testing::delete_store(&a.db).unwrap();
    assert!(store::exists(&a.db), "vectors survive the content store");
    let changed = one.replacen("parte 1.", "parte uno.", 1);
    write_content(&a.db, &[(1, &changed), (2, &two)]);
    let mut again = Fake::new(model("m"));
    let sum = embed(&a.db, &mut again);
    assert_eq!(again.sent.len(), 1, "only the changed chunk");
    assert!(again.sent[0].contains("parte uno."));
    assert_eq!(sum.dropped, 1);
    let s = Store::open(&a.db).unwrap();
    let set = s.set_id(&model("m")).unwrap();
    assert_eq!(s.count(set).unwrap() as usize, chunk_count(&a.db));
}

/// VFS-29 R6: while documents wait in the content queue, vectors of texts not seen yet are
/// kept: they may be those documents' texts.
#[test]
fn nothing_is_dropped_while_documents_wait() {
    let a = Archive::new(&[(1, "Primer documento."), (2, "Segundo documento.")]);
    let mut e = Fake::new(model("m"));
    embed(&a.db, &mut e);
    vfs_content::testing::remove_doc(&a.db, 2).unwrap();
    vfs_content::testing::set_queued(&a.db, 2, true).unwrap();
    let sum = embed(&a.db, &mut Fake::new(model("m")));
    assert_eq!(sum.dropped, 0);
    vfs_content::testing::set_queued(&a.db, 2, false).unwrap();
    let sum = embed(&a.db, &mut Fake::new(model("m")));
    assert_eq!(sum.dropped, 1);
}

/// VFS-29 AC-9 (R10): with set A complete, a new model's set B is built beside it; searches
/// use A until B is complete, then B, and A's rows are gone.
#[test]
fn a_new_model_replaces_the_old_set_when_complete() {
    let docs: Vec<String> = (0..4).map(|i| long_text(&format!("Doc {i}"), 2)).collect();
    let refs: Vec<(i64, &str)> = docs
        .iter()
        .enumerate()
        .map(|(i, t)| (i as i64 + 1, t.as_str()))
        .collect();
    let a = Archive::new(&refs);
    embed(&a.db, &mut Fake::new(model("a")));
    let s = Store::open(&a.db).unwrap();
    let set_a = s.set_id(&model("a")).unwrap();
    assert_eq!(s.search_set().unwrap(), Some((set_a, DIM)));

    let mut b = Fake::new(model("b"));
    b.fail_after = Some(3);
    embed(&a.db, &mut b);
    assert_eq!(
        s.search_set().unwrap(),
        Some((set_a, DIM)),
        "A until B is complete"
    );
    assert!(s.count(set_a).unwrap() > 0);

    embed(&a.db, &mut Fake::new(model("b")));
    let set_b = s.set_id(&model("b")).unwrap();
    assert_eq!(s.search_set().unwrap(), Some((set_b, DIM)));
    assert_eq!(s.count(set_a).unwrap(), 0, "A's rows are gone");
}

/// VFS-29 R10: while documents wait in the content queue, a new model's set is not complete
/// even with every stored text embedded, and the old complete set keeps serving searches.
#[test]
fn a_new_set_is_not_complete_while_documents_wait() {
    let a = Archive::new(&[(1, "Primer documento."), (2, "Segundo documento.")]);
    embed(&a.db, &mut Fake::new(model("a")));
    vfs_content::testing::set_queued(&a.db, 3, true).unwrap();
    let sum = embed(&a.db, &mut Fake::new(model("b")));
    assert!(!sum.complete && sum.error.is_none() && !sum.stopped);
    let s = Store::open(&a.db).unwrap();
    let set_a = s.set_id(&model("a")).unwrap();
    assert_eq!(s.search_set().unwrap(), Some((set_a, DIM)));
    assert_eq!(s.count(set_a).unwrap(), 2);
    vfs_content::testing::set_queued(&a.db, 3, false).unwrap();
    assert!(embed(&a.db, &mut Fake::new(model("b"))).complete);
    assert_eq!(s.count(set_a).unwrap(), 0);
}
