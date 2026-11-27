//! The app's search by meaning service (VFS-29 R1, R1a, R4, R7, R15): the switch, the
//! download, the paced embedding, new documents, the pause; with the worker's fake model and
//! a fake download server.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vfs_content::pace::PaceConfig;
use vfs_content::service::{Probes, WaitKind};
use vfs_core::probes::CpuSample;
use vfs_semantic::client::WorkerConfig;
use vfs_semantic::install::{Check, Download, Fetch};
use vfs_semantic::search::MeaningConfig;
use vfs_semantic::service::{self, MeaningService, Phase, ServiceConfig, Status};
use vfs_semantic::LOCAL_MODEL;

struct FakeProbes {
    name_busy: Arc<AtomicBool>,
}

impl Probes for FakeProbes {
    fn name_busy(&self) -> bool {
        self.name_busy.load(Ordering::Relaxed)
    }
    fn idle(&self) -> Option<Duration> {
        Some(Duration::from_secs(3600))
    }
    fn free_bytes(&self) -> Option<u64> {
        None
    }
    fn disk_queue(&mut self, _path: &Path) -> Option<f64> {
        None
    }
    fn cpu(&mut self) -> Option<CpuSample> {
        None
    }
}

struct Server(HashMap<String, Vec<u8>>);

impl Fetch for Server {
    fn get(&self, url: &str) -> std::io::Result<Box<dyn Read + Send>> {
        match self.0.get(url) {
            Some(b) => Ok(Box::new(std::io::Cursor::new(b.clone()))),
            None => Err(std::io::Error::other(format!("404 {url}"))),
        }
    }
}

/// SHA-256 of b"model".
const MODEL_SHA: &str = "9372c470eeadd5ecd9c3c74c2b3cb633f8e2f2fad799250a0f70d652b6b825e4";

struct App {
    _dir: tempfile::TempDir,
    db: PathBuf,
    data: PathBuf,
    busy: Arc<AtomicBool>,
}

impl App {
    fn new(docs: usize) -> App {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().to_path_buf();
        let db = data.join("index.db");
        for i in 0..docs {
            let text = format!("Documento {i}. ").repeat(150) + &format!("Final {i}.");
            vfs_content::testing::write_doc(&db, i as i64 + 1, &text).unwrap();
        }
        App {
            _dir: dir,
            db,
            data,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    fn start(&self, model_url: &str) -> MeaningService {
        let model_dir = service::model_dir(&self.data);
        let worker = WorkerConfig {
            program: PathBuf::from(env!("CARGO_BIN_EXE_vfs-embed")),
            args: vec!["--test-hook=fake:384".into()],
            model_dir,
            model: LOCAL_MODEL,
            threads: 1,
            foreground: false,
            mem_cap: 512 << 20,
            timeout: Duration::from_secs(30),
        };
        let query = MeaningConfig {
            worker: WorkerConfig {
                foreground: true,
                ..worker.clone()
            },
            idle: Duration::from_secs(600),
        };
        let cfg = ServiceConfig {
            index_db: self.db.clone(),
            data_dir: self.data.clone(),
            downloads: vec![Download {
                file: "model.onnx".into(),
                url: model_url.into(),
                check: Check::Sha256(MODEL_SHA),
                bytes: Some(5),
                executable: false,
            }],
            worker,
            query,
            pace: PaceConfig {
                name_wait: Duration::from_millis(50),
                ..PaceConfig::default()
            },
            change_quiet: Duration::from_millis(100),
        };
        let server = Server(HashMap::from([(
            "https://h/model".to_string(),
            b"model".to_vec(),
        )]));
        MeaningService::start(
            cfg,
            Box::new(server),
            Box::new(FakeProbes {
                name_busy: Arc::clone(&self.busy),
            }),
            Box::new(|_: &Status| {}),
        )
    }
}

fn wait_for(svc: &MeaningService, what: &str, f: impl Fn(&Status) -> bool) -> Status {
    let t = Instant::now();
    loop {
        let s = svc.status();
        if f(&s) {
            return s;
        }
        assert!(t.elapsed() < Duration::from_secs(60), "{what}: {s:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// VFS-29 AC-1, R15: with the switch off, the service starts nothing: no model folder, no
/// vector store, no ranker for the searches.
#[test]
fn off_creates_nothing() {
    let app = App::new(3);
    let svc = app.start("https://h/model");
    let s = svc.status();
    assert_eq!(s.phase, Phase::Off);
    assert!(!s.settings.enabled && !s.searchable);
    assert!(svc.ranker().is_none());
    std::thread::sleep(Duration::from_millis(200));
    assert!(!service::model_dir(&app.data).exists());
    assert!(!vfs_semantic::store::exists(&app.db));
}

/// VFS-29 R1, R1a, R7, R11: switched on, the model is downloaded and checked, every chunk
/// is embedded, the switch is kept across a restart, and searches get the ranker.
#[test]
fn switched_on_it_downloads_embeds_and_serves() {
    let app = App::new(4);
    let svc = app.start("https://h/model");
    svc.set_enabled(true);
    let s = wait_for(&svc, "finished", |s| {
        s.phase == Phase::Finished && s.searchable
    });
    assert_eq!(s.done, s.total);
    assert!(service::model_dir(&app.data).join("model.onnx").is_file());
    let ranker = svc.ranker().expect("a ranker while on and complete");
    svc.warm();
    let t = Instant::now();
    let hits = loop {
        match ranker.rank("Documento 2.", 3) {
            Ok(h) => break h,
            Err(_) if t.elapsed() < Duration::from_secs(30) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(e) => panic!("{e:?}"),
        }
    };
    assert_eq!(hits.len(), 3);
    drop(svc);
    assert!(
        service::load_settings(&app.data).enabled,
        "kept in meaning.json"
    );
}

/// VFS-29 AC-4 (R4): with the switch on, a document content indexing reads later is
/// embedded without asking again.
#[test]
fn new_documents_are_embedded_without_asking() {
    let app = App::new(2);
    let svc = app.start("https://h/model");
    svc.set_enabled(true);
    wait_for(&svc, "finished", |s| s.phase == Phase::Finished);
    let store = vfs_semantic::store::Store::open(&app.db).unwrap();
    let (set, _) = store.search_set().unwrap().unwrap();
    let before = store.count(set).unwrap();
    vfs_content::testing::write_doc(&app.db, 50, "Un documento nuevo, con otro texto.").unwrap();
    svc.notify_content_changed();
    let t = Instant::now();
    while store.count(set).unwrap() == before {
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "the new text was not embedded"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(store.count(set).unwrap(), before + 1);
}

/// VFS-29 R7 (VFS-27 R7): the run waits while the name indexer is busy, holds while paused,
/// and finishes once resumed.
#[test]
fn the_run_follows_the_pace_and_the_pause() {
    let app = App::new(60);
    app.busy.store(true, Ordering::Relaxed);
    let svc = app.start("https://h/model");
    svc.set_enabled(true);
    wait_for(&svc, "waiting for the name indexer", |s| {
        s.phase
            == Phase::Waiting {
                reason: WaitKind::NameIndexing,
            }
    });
    svc.set_paused(true);
    wait_for(&svc, "paused", |s| s.phase == Phase::Paused);
    app.busy.store(false, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(svc.status().phase, Phase::Paused, "holds while paused");
    svc.set_paused(false);
    wait_for(&svc, "finished", |s| {
        s.phase == Phase::Finished && s.searchable
    });
}

/// VFS-29 R7: a download that fails is reported and tried again later; nothing counts as
/// installed and nothing is embedded.
#[test]
fn a_failed_download_is_reported() {
    let app = App::new(2);
    let svc = app.start("https://h/missing");
    svc.set_enabled(true);
    let s = wait_for(&svc, "failed", |s| matches!(s.phase, Phase::Failed { .. }));
    assert!(!s.searchable);
    assert!(!vfs_semantic::store::exists(&app.db));
    svc.set_enabled(false);
    assert_eq!(svc.status().phase, Phase::Off);
}
