//! VFS-27 R7 measurement, not a check: documents read per minute by the background content
//! indexer on this machine, with the real probes and the real worker.
//!
//! ```text
//! cargo build --release -p vfs-content
//! cargo run --release -p vfs-content --example measure_pace -- <folder> [seconds] [idle]
//! ```
//!
//! `<folder>` is indexed by name, then content indexing runs for `seconds` (default 120) from
//! the first document read. `idle`, when given, replaces the idle probe with that many
//! seconds since the last input, to measure light use (10) or no use (600) repeatably.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vfs_content::pace::PaceConfig;
use vfs_content::service::{ContentService, Events, Probes, ServiceConfig, Status};
use vfs_content::IndexOptions;
use vfs_core::probes::{CpuSample, CpuSampler, DiskSampler};
use vfs_core::Vfs;

/// This machine as the app sees it, but for an optional fixed idle time.
struct ThisMachine {
    idle: Option<Duration>,
    data: PathBuf,
    cpu: CpuSampler,
    disk: Option<Option<DiskSampler>>,
}

impl Probes for ThisMachine {
    fn name_busy(&self) -> bool {
        false
    }
    fn idle(&self) -> Option<Duration> {
        self.idle.or_else(vfs_core::probes::idle_time)
    }
    fn free_bytes(&self) -> Option<u64> {
        vfs_core::probes::free_space(&self.data)
    }
    fn disk_queue(&mut self, path: &Path) -> Option<f64> {
        self.disk
            .get_or_insert_with(|| DiskSampler::for_path(path))
            .as_mut()?
            .queue_length()
    }
    fn cpu(&mut self) -> Option<CpuSample> {
        self.cpu.sample()
    }
}

/// Counts the wait records by kind.
#[derive(Default)]
struct Waits(Mutex<BTreeMap<String, u32>>);

impl Events for Waits {
    fn status(&self, _status: &Status) {}
    fn diag(&self, record: &str) {
        if let Some(kind) = record.strip_prefix("content run waits: ") {
            *self.0.lock().unwrap().entry(kind.to_string()).or_default() += 1;
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let corpus = PathBuf::from(
        args.next()
            .expect("usage: measure_pace <folder> [seconds] [idle]"),
    );
    let secs: u64 = args.next().map_or(120, |s| s.parse().expect("seconds"));
    let idle = args
        .next()
        .map(|s| Duration::from_secs(s.parse().expect("idle seconds")));
    // The worker built next to this example (target/<profile>/vfs-extract).
    let exe = std::env::current_exe().unwrap();
    let worker = exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(if cfg!(windows) {
            "vfs-extract.exe"
        } else {
            "vfs-extract"
        });
    assert!(
        worker.exists(),
        "build the worker first: {}",
        worker.display()
    );

    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("datos");
    std::fs::create_dir_all(&data).unwrap();
    let db = data.join("index.db");
    let vfs = Vfs::open(&db).unwrap();
    vfs.add_root(&corpus).unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    let cfg = ServiceConfig {
        index_db: db,
        data_dir: data.clone(),
        index: IndexOptions::new(worker, Vec::new()),
        pace: PaceConfig::default(),
    };
    let waits = Arc::new(Waits::default());
    let machine = ThisMachine {
        idle,
        data,
        cpu: CpuSampler::default(),
        disk: None,
    };
    let svc = ContentService::start(cfg, Box::new(machine), waits.clone());
    svc.set_enabled(true);
    let first = Instant::now();
    while svc.status().done == 0 {
        assert!(
            first.elapsed() < Duration::from_secs(300),
            "no document read"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let (t0, d0) = (Instant::now(), svc.status().done);
    std::thread::sleep(Duration::from_secs(secs));
    let s = svc.status();
    let minutes = t0.elapsed().as_secs_f64() / 60.0;
    println!(
        "pace: {:.1} documents/min ({} in {:.1} min, idle {:?}); {} of {}; waits {:?}",
        (s.done - d0) as f64 / minutes,
        s.done - d0,
        minutes,
        idle,
        s.done,
        s.total,
        waits.0.lock().unwrap()
    );
}
