//! The ranker by meaning (VFS-29 R11, R13, R15, AC-13, AC-14), against a real `vectors.db`
//! built by the embedding run and the worker process running its fake model, whose vector
//! for a text is the same whether it is a query or a passage.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use vfs_content::{Meaning, MeaningUnavailable};
use vfs_semantic::client::WorkerConfig;
use vfs_semantic::search::{MeaningConfig, MeaningIndex};
use vfs_semantic::worker::fake_vector;
use vfs_semantic::{run, EmbedError, Embedder, ModelId, RunOptions, LOCAL_MODEL};

const DIM: usize = 32;

fn model() -> ModelId {
    ModelId {
        dim: DIM,
        ..LOCAL_MODEL
    }
}

struct Fake;

impl Embedder for Fake {
    fn model(&self) -> ModelId {
        model()
    }
    fn embed(
        &mut self,
        texts: &[String],
        _query: bool,
        _stop: &AtomicBool,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts.iter().map(|t| fake_vector(t, DIM)).collect())
    }
}

fn index(docs: &[(i64, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    for (id, text) in docs {
        vfs_content::testing::write_doc(&db, *id, text).unwrap();
    }
    let sum = run(
        &db,
        &mut Fake,
        &RunOptions::default(),
        &AtomicBool::new(false),
        &mut |_| true,
    )
    .unwrap();
    assert!(sum.complete);
    (dir, db)
}

fn ranker(db: &Path, hooks: &[&str], timeout: Duration, idle: Duration) -> MeaningIndex {
    let worker = WorkerConfig {
        program: PathBuf::from(env!("CARGO_BIN_EXE_vfs-embed")),
        args: hooks.iter().map(|h| format!("--test-hook={h}")).collect(),
        model_dir: PathBuf::from("no-model-needed"),
        model: model(),
        threads: 1,
        foreground: true,
        mem_cap: 512 << 20,
        timeout,
    };
    MeaningIndex::new(db.to_path_buf(), MeaningConfig { worker, idle })
}

fn ready(db: &Path) -> MeaningIndex {
    let m = ranker(
        db,
        &["fake:32"],
        Duration::from_secs(5),
        Duration::from_secs(600),
    );
    m.warm();
    assert!(m.wait_ready(Duration::from_secs(30)));
    m
}

fn docs() -> Vec<(i64, String)> {
    (1..=30)
        .map(|i| (i, format!("Documento {i}: texto propio número {i}.")))
        .collect()
}

/// VFS-29 R11: the chunk whose vector is closest to the query's comes first, through the
/// 1-bit scan around the set's mean and the int8 rescore.
#[test]
fn the_closest_chunk_comes_first() {
    let d = docs();
    let refs: Vec<(i64, &str)> = d.iter().map(|(i, t)| (*i, t.as_str())).collect();
    let (_t, db) = index(&refs);
    let m = ready(&db);
    for (id, text) in d.iter().step_by(7) {
        let hits = m.rank(text, 5).unwrap();
        assert_eq!((hits[0].file_id, hits[0].ordinal), (*id, 0), "{text}");
        assert!(hits[0].score > 0.9);
        assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
        assert_eq!(hits.len(), 5);
    }
}

/// VFS-29 AC-14 (R6, D10), retrieval side: a text stored once for two copies is found in
/// both documents.
#[test]
fn both_copies_are_found() {
    let mut d = docs();
    d.push((40, d[3].1.clone()));
    let refs: Vec<(i64, &str)> = d.iter().map(|(i, t)| (*i, t.as_str())).collect();
    let (_t, db) = index(&refs);
    let m = ready(&db);
    let hits = m.rank(&d[3].1, 5).unwrap();
    let mut first: Vec<i64> = hits[..2].iter().map(|h| h.file_id).collect();
    first.sort();
    assert_eq!(first, vec![d[3].0, 40]);
}

/// VFS-29 R13: before the rows are loaded and the query worker is ready, the ranker says so
/// at once instead of making the search wait; with no complete vector set it never starts a
/// worker and creates no file.
#[test]
fn not_ready_answers_at_once() {
    let d = docs();
    let refs: Vec<(i64, &str)> = d.iter().map(|(i, t)| (*i, t.as_str())).collect();
    let (_t, db) = index(&refs);
    let m = ranker(
        &db,
        &["fake:32"],
        Duration::from_secs(5),
        Duration::from_secs(600),
    );
    let t = Instant::now();
    assert_eq!(m.rank("hola", 5), Err(MeaningUnavailable::NotReady));
    assert!(t.elapsed() < Duration::from_millis(500));
    assert!(m.wait_ready(Duration::from_secs(30)));
    assert!(m.rank("hola", 5).is_ok());

    let empty = tempfile::tempdir().unwrap();
    let none = ranker(
        &empty.path().join("index.db"),
        &["fake:32"],
        Duration::from_secs(5),
        Duration::from_secs(600),
    );
    assert_eq!(none.rank("hola", 5), Err(MeaningUnavailable::NotReady));
    assert!(
        !none.wait_ready(Duration::from_secs(10)),
        "no set, no worker"
    );
    assert_eq!(none.rank("hola", 5), Err(MeaningUnavailable::NotReady));
    assert!(!vfs_semantic::store::exists(&empty.path().join("index.db")));
}

/// VFS-29 R13: a query embedding past the time limit fails within it, so the search falls
/// back to BM25.
#[test]
fn a_slow_query_embedding_fails_within_the_limit() {
    let d = docs();
    let refs: Vec<(i64, &str)> = d.iter().map(|(i, t)| (*i, t.as_str())).collect();
    let (_t, db) = index(&refs);
    let m = ranker(
        &db,
        &["fake:32", "hang"],
        Duration::from_millis(400),
        Duration::from_secs(600),
    );
    m.warm();
    assert!(m.wait_ready(Duration::from_secs(30)));
    let t = Instant::now();
    assert!(matches!(
        m.rank("hola", 5),
        Err(MeaningUnavailable::Failed(_))
    ));
    assert!(t.elapsed() < Duration::from_secs(2));
}

/// VFS-29 AC-13 (R15): unused for the idle time, the rows and the worker are released; the
/// next search loads them again.
#[test]
fn unused_rows_and_worker_are_released() {
    let d = docs();
    let refs: Vec<(i64, &str)> = d.iter().map(|(i, t)| (*i, t.as_str())).collect();
    let (_t, db) = index(&refs);
    let m = ranker(
        &db,
        &["fake:32"],
        Duration::from_secs(5),
        Duration::from_millis(300),
    );
    m.warm();
    assert!(m.wait_ready(Duration::from_secs(30)));
    m.rank("hola", 5).unwrap();
    assert!(m.is_loaded());
    let t = Instant::now();
    while m.is_loaded() && t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!m.is_loaded(), "released after the idle time");
    assert_eq!(m.rank("hola", 5), Err(MeaningUnavailable::NotReady));
    assert!(m.wait_ready(Duration::from_secs(30)));
    assert!(m.rank("hola", 5).is_ok());
}
