//! Throwaway probe for issue 109 (never merged): generates a tree, runs the real background
//! indexer on it and prints a timeline (ms since start) of progress, search latency and the
//! scheduler's debug records.
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use vfs_core::{IndexProgress, IndexerState, ProgressSink, SearchQuery, Vfs};

static T0: OnceLock<Instant> = OnceLock::new();
static OUT: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

fn ms() -> u128 {
    T0.get().unwrap().elapsed().as_millis()
}
fn out(line: &str) {
    let mut f = OUT.get().unwrap().lock().unwrap();
    let _ = writeln!(f, "{} {}", ms(), line);
}

struct L;
impl log::Log for L {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        if r.target().starts_with("vfs") {
            out(&format!("{} {}", r.target(), r.args()));
        } else if r.level() <= log::Level::Warn {
            out(&format!("{} {} {}", r.level(), r.target(), r.args()));
        }
    }
    fn flush(&self) {}
}

struct Sink;
impl ProgressSink for Sink {
    fn on_progress(&self, _: &IndexProgress) {}
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tree = PathBuf::from(&args[1]);
    let db = PathBuf::from(&args[2]);
    let log_path = PathBuf::from(&args[3]);
    let n: usize = args.get(4).map_or(100_000, |s| s.parse().unwrap());
    let calm: u64 = args.get(5).map_or(0, |s| s.parse().unwrap());
    T0.set(Instant::now()).unwrap();
    OUT.set(Mutex::new(std::fs::File::create(&log_path).unwrap()))
        .unwrap();
    log::set_logger(&L).unwrap();
    log::set_max_level(log::LevelFilter::Debug);

    if !tree.exists() {
        out("gen start");
        let per_dir = 100;
        for d in 0..n.div_ceil(per_dir) {
            let dir = tree
                .join(format!("Año {}", 2000 + d % 25))
                .join(format!("Carpeta {:03}", d / 25))
                .join(format!("Fotos de verano {d}"));
            std::fs::create_dir_all(&dir).unwrap();
            for f in 0..per_dir {
                std::fs::write(dir.join(format!("Recibo número {f} canción.pdf")), b"x").unwrap();
            }
        }
        out("gen end");
    }
    std::thread::sleep(Duration::from_secs(calm));

    let vfs = Vfs::open(&db).unwrap();
    vfs.start_indexer(Arc::new(Sink)).unwrap();
    out("add_root");
    vfs.add_root(&tree).unwrap();
    let start = Instant::now();
    let mut first_hit: Option<u128> = None;
    let mut last = 0u64;
    let mut last_change = Instant::now();
    let mut longest_gap = Duration::ZERO;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let p = vfs.progress();
        if p.files_indexed != last {
            longest_gap = longest_gap.max(last_change.elapsed());
            last_change = Instant::now();
            last = p.files_indexed;
        }
        let q = SearchQuery {
            text: "recibo cancion".into(),
            filters: Default::default(),
            limit: 10,
            offset: 0,
        };
        let s0 = Instant::now();
        let hits = vfs.search(&q).map(|r| r.hits.len()).unwrap_or(0);
        let lat = s0.elapsed().as_millis();
        if hits > 0 && first_hit.is_none() {
            first_hit = Some(start.elapsed().as_millis());
        }
        out(&format!(
            "progress files={} state={:?} search_ms={lat} hits={hits}",
            p.files_indexed, p.state
        ));
        if p.state == IndexerState::Idle && p.files_indexed as usize >= n {
            break;
        }
        if start.elapsed() > Duration::from_secs(600) {
            out("timeout");
            break;
        }
    }
    let secs = start.elapsed().as_secs_f64();
    out(&format!(
        "summary files={last} secs={secs:.1} files_per_s={:.0} longest_gap_s={:.1} first_hit_ms={first_hit:?}",
        last as f64 / secs,
        longest_gap.as_secs_f64()
    ));
    vfs.shutdown();
}
