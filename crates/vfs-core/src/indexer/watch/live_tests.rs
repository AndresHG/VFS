//! Live watcher tests on a real temp directory (VFS-1 AC-7). Every wait polls for up to
//! 30 s; the tests run on Windows CI too.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{Filter, RootWatcher, WatchEvent, WatchSink};

const ROOT_ID: i64 = 7;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Recorded {
    dirty: HashSet<PathBuf>,
    rescans: Vec<i64>,
}

/// Field order matters: the watcher closes its handles before the temp dir is deleted.
struct Fixture {
    watcher: RootWatcher,
    root: PathBuf,
    events: Arc<Mutex<Recorded>>,
    _tmp: tempfile::TempDir,
}

/// Starts a watcher on a fresh canonical temp root (`root/`), ignoring `root/appdata`, and
/// waits until it demonstrably delivers events.
fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let base = dunce::canonicalize(tmp.path()).unwrap();
    let root = base.join("root");
    fs::create_dir(&root).unwrap();
    let events = Arc::new(Mutex::new(Recorded::default()));
    let rec = Arc::clone(&events);
    let sink: WatchSink = Arc::new(move |e| {
        let mut r = rec.lock().unwrap();
        match e {
            WatchEvent::Dirty(p) => {
                r.dirty.insert(p);
            }
            WatchEvent::Overflow(id) | WatchEvent::Failed(id) => r.rescans.push(id),
            WatchEvent::Renamed { .. } => {}
        }
    });
    let mut watcher = RootWatcher::start(sink, Filter::new(root.join("appdata"))).unwrap();
    watcher.watch(ROOT_ID, &root).unwrap();
    let mut f = Fixture {
        watcher,
        root,
        events,
        _tmp: tmp,
    };
    f.wait_live();
    f
}

impl Fixture {
    /// Writes probe files until one is reported, so a slow-starting stream (FSEvents) cannot
    /// make the real assertions miss events; then clears the record.
    fn wait_live(&mut self) {
        let start = Instant::now();
        let mut n = 0;
        loop {
            let probe = self.root.join(format!("probe-{n}"));
            fs::write(&probe, b"x").unwrap();
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                if self.events.lock().unwrap().dirty.contains(&probe) {
                    std::thread::sleep(Duration::from_millis(300));
                    *self.events.lock().unwrap() = Recorded::default();
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(
                start.elapsed() < TIMEOUT,
                "watcher never delivered an event"
            );
            n += 1;
        }
    }

    /// Polls up to 30 s until `done` holds for the recorded events.
    fn wait_for(&self, what: &str, done: impl Fn(&Recorded) -> bool) {
        let start = Instant::now();
        while start.elapsed() < TIMEOUT {
            if done(&self.events.lock().unwrap()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let r = self.events.lock().unwrap();
        panic!(
            "timed out waiting for {what}; got {} dirty paths, rescans {:?}: {:?}",
            r.dirty.len(),
            r.rescans,
            r.dirty.iter().take(20).collect::<Vec<_>>()
        );
    }

    fn wait_dirty(&self, paths: &[&Path]) {
        self.wait_for(&format!("{paths:?}"), |r| {
            paths.iter().all(|p| r.dirty.contains(*p))
        });
    }
}

/// VFS-1 AC-7: creating a file reports its path.
#[test]
fn create_is_reported() {
    let f = fixture();
    let file = f.root.join("Notaría 2019.pdf");
    fs::write(&file, b"pdf").unwrap();
    f.wait_dirty(&[&file]);
}

/// VFS-1 AC-7: deleting a file reports its path.
#[test]
fn delete_is_reported() {
    let f = fixture();
    let file = f.root.join("borrar.txt");
    fs::write(&file, b"x").unwrap();
    f.wait_dirty(&[&file]);
    *f.events.lock().unwrap() = Recorded::default();
    fs::remove_file(&file).unwrap();
    f.wait_dirty(&[&file]);
}

/// VFS-1 AC-7: renaming a file in place reports both the old and the new path.
#[test]
fn rename_reports_both_paths() {
    let f = fixture();
    let old = f.root.join("viejo.txt");
    let new = f.root.join("nuevo.txt");
    fs::write(&old, b"x").unwrap();
    f.wait_dirty(&[&old]);
    fs::rename(&old, &new).unwrap();
    f.wait_dirty(&[&old, &new]);
}

/// VFS-1 AC-7: moving a file between directories reports both paths, so both parents are
/// marked dirty.
#[test]
fn move_between_directories_reports_both_paths() {
    let f = fixture();
    let a = f.root.join("a");
    let b = f.root.join("b").join("c");
    fs::create_dir(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let src = a.join("factura.pdf");
    let dst = b.join("factura.pdf");
    fs::write(&src, b"x").unwrap();
    f.wait_dirty(&[&src]);
    fs::rename(&src, &dst).unwrap();
    f.wait_dirty(&[&src, &dst]);
}

/// VFS-1 AC-7: changes under the app data directory are dropped, others still arrive.
#[test]
fn app_data_events_are_ignored() {
    let f = fixture();
    let appdata = f.root.join("appdata");
    fs::create_dir(&appdata).unwrap();
    for i in 0..20 {
        fs::write(appdata.join(format!("index-{i}.db")), b"x").unwrap();
    }
    let marker = f.root.join("marker.txt");
    fs::write(&marker, b"x").unwrap();
    f.wait_dirty(&[&marker]);
    std::thread::sleep(Duration::from_millis(500));
    let r = f.events.lock().unwrap();
    let leaked: Vec<_> = r.dirty.iter().filter(|p| p.starts_with(&appdata)).collect();
    assert!(leaked.is_empty(), "app data events leaked: {leaked:?}");
}

/// VFS-1 AC-7: a burst of several thousand creations ends with every path reported or a
/// re-walk of the root requested (overflow is never silent).
#[test]
fn burst_reports_all_or_rescan() {
    const N: usize = 5000;
    let f = fixture();
    let dir = f.root.join("rafaga");
    fs::create_dir(&dir).unwrap();
    let files: Vec<PathBuf> = (0..N).map(|i| dir.join(format!("f{i:05}.txt"))).collect();
    for p in &files {
        fs::write(p, b"").unwrap();
    }
    f.wait_for("all paths or a rescan", |r| {
        r.rescans.contains(&ROOT_ID) || files.iter().all(|p| r.dirty.contains(p))
    });
    let r = f.events.lock().unwrap();
    assert!(r.rescans.iter().all(|id| *id == ROOT_ID), "{:?}", r.rescans);
}

/// VFS-1 AC-7: after `unwatch` no further events arrive for that root.
#[test]
fn unwatch_stops_events() {
    let mut f = fixture();
    f.watcher.unwatch(ROOT_ID);
    std::thread::sleep(Duration::from_millis(500));
    *f.events.lock().unwrap() = Recorded::default();
    fs::write(f.root.join("tarde.txt"), b"x").unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let r = f.events.lock().unwrap();
    assert!(r.dirty.is_empty() && r.rescans.is_empty(), "{:?}", r.dirty);
}
