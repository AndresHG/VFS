//! The embedding worker process and its client (VFS-29 R16), with the worker's test hooks in
//! place of a model: a fake model, a worker that hangs, one that outgrows its memory cap.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use vfs_semantic::client::{WorkerConfig, WorkerEmbedder};
use vfs_semantic::worker::fake_vector;
use vfs_semantic::{EmbedError, Embedder, ModelId, LOCAL_MODEL};

const DIM: usize = 8;

fn config(hooks: &[&str]) -> WorkerConfig {
    WorkerConfig {
        program: PathBuf::from(env!("CARGO_BIN_EXE_vfs-embed")),
        args: hooks.iter().map(|h| format!("--test-hook={h}")).collect(),
        model_dir: PathBuf::from("no-model-needed"),
        model: ModelId {
            dim: DIM,
            ..LOCAL_MODEL
        },
        threads: 1,
        foreground: false,
        mem_cap: 512 << 20,
        timeout: Duration::from_secs(5),
    }
}

fn texts(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| s.to_string()).collect()
}

/// VFS-29 R16: the worker starts in its own process, says hello with its version and
/// dimension, answers each batch with one vector per text, and stays up between batches.
#[test]
fn the_worker_answers_batches() {
    let mut w = WorkerEmbedder::new(config(&["fake:8"]));
    let never = AtomicBool::new(false);
    let v = w
        .embed(&texts(&["factura", "recibo"]), false, &never)
        .unwrap();
    assert_eq!(
        v,
        vec![fake_vector("factura", DIM), fake_vector("recibo", DIM)]
    );
    assert!(w.is_running());
    let q = w.embed(&texts(&["gastos médicos"]), true, &never).unwrap();
    assert_eq!(q.len(), 1);
    w.kill();
    assert!(!w.is_running());
}

/// VFS-29 R16: a worker reporting another dimension than the model's is refused.
#[test]
fn a_worker_of_another_model_is_refused() {
    let mut w = WorkerEmbedder::new(config(&["fake:4"]));
    let err = w
        .embed(&texts(&["x"]), false, &AtomicBool::new(false))
        .unwrap_err();
    assert!(matches!(err, EmbedError::Failed(m) if m.contains("dimensions")));
    assert!(!w.is_running());
}

/// VFS-29 R16: a build without the model runtime and no fake model fails to start, and the
/// caller gets the reason instead of a hang.
#[test]
fn a_worker_without_a_model_fails_cleanly() {
    let mut cfg = config(&[]);
    cfg.model_dir = PathBuf::from("does-not-exist");
    let mut w = WorkerEmbedder::new(cfg);
    let err = w
        .embed(&texts(&["x"]), false, &AtomicBool::new(false))
        .unwrap_err();
    assert!(matches!(err, EmbedError::Failed(m) if m.contains("model")));
}

/// VFS-29 R7, R16: a worker that never answers is killed at the batch time limit.
#[test]
fn a_hung_worker_is_killed_at_the_time_limit() {
    let mut cfg = config(&["fake:8", "hang"]);
    cfg.timeout = Duration::from_millis(500);
    let mut w = WorkerEmbedder::new(cfg);
    let t = Instant::now();
    let err = w
        .embed(&texts(&["x"]), false, &AtomicBool::new(false))
        .unwrap_err();
    assert!(matches!(err, EmbedError::Failed(m) if m.contains("in time")));
    assert!(t.elapsed() < Duration::from_secs(4));
    assert!(!w.is_running());
}

/// VFS-29 R7 (VFS-27 R7 pause): raising the stop flag abandons the batch in flight and ends
/// the worker at once.
#[test]
fn stop_abandons_the_batch() {
    let mut cfg = config(&["fake:8", "hang"]);
    cfg.timeout = Duration::from_secs(60);
    let mut w = WorkerEmbedder::new(cfg);
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            std::thread::sleep(Duration::from_millis(300));
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let t = Instant::now();
        assert_eq!(
            w.embed(&texts(&["x"]), false, &stop).unwrap_err(),
            EmbedError::Stopped
        );
        assert!(t.elapsed() < Duration::from_secs(5));
    });
    assert!(!w.is_running());
}

/// VFS-29 R16: a worker that outgrows its memory cap is killed and the batch fails; the
/// caller is unharmed.
#[test]
fn a_worker_over_its_memory_cap_is_killed() {
    let mut cfg = config(&["fake:8", "alloc"]);
    cfg.mem_cap = 256 << 20;
    cfg.timeout = Duration::from_secs(30);
    let mut w = WorkerEmbedder::new(cfg);
    let err = w
        .embed(&texts(&["x"]), false, &AtomicBool::new(false))
        .unwrap_err();
    assert!(matches!(err, EmbedError::Failed(_)), "{err:?}");
    assert!(!w.is_running());
}
