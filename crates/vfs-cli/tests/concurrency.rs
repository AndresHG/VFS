//! Concurrency (VFS-3 AC-6..AC-9): searches on several threads while the indexer writes and
//! the watcher is flooded, newer queries interrupting older ones, shutdown at any moment,
//! and two processes (the app and the command line) on one index.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use support::*;
use vfs_core::{IndexProgress, IndexerState, ProgressSink, RootStatus, SearchQuery, Vfs};

/// Forwards "up to date" progress to a channel.
struct Sink(Mutex<mpsc::Sender<()>>);

impl ProgressSink for Sink {
    fn on_progress(&self, p: &IndexProgress) {
        if p.state == IndexerState::Idle
            && !p.roots.is_empty()
            && p.roots
                .iter()
                .all(|r| r.status == RootStatus::Ready && r.dirs_pending == 0)
        {
            let _ = self.0.lock().unwrap().send(());
        }
    }
}

fn sink() -> (Arc<Sink>, mpsc::Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    (Arc::new(Sink(Mutex::new(tx))), rx)
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        filters: Default::default(),
        limit: 50,
        offset: 0,
    }
}

const QUERIES: &[&str] = &[
    "factura",
    "notaria 2",
    "año",
    "ca",
    "fcatura",
    "banco escritura",
    "renombrada",
    "x",
    "presupuesto 1",
];

struct Tree {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
}

fn tree(seed: u64, files: usize) -> Tree {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("archivo");
    fs::create_dir(&root).unwrap();
    gen_tree(&root, &mut Rng::new(seed), files);
    let db_dir = tmp.path().join("datos");
    fs::create_dir(&db_dir).unwrap();
    Tree {
        db: db_dir.join("index.db"),
        root,
        _tmp: tmp,
    }
}

/// Creates, renames and deletes files and folders under `root` until `stop`.
fn flood(root: &Path, seed: u64, stop: &AtomicBool) -> usize {
    let mut rng = Rng::new(seed);
    let base = root.join(format!("flujo {seed}"));
    fs::create_dir_all(&base).unwrap();
    let mut n = 0u64;
    let mut ops = 0;
    while !stop.load(Ordering::SeqCst) {
        n += 1;
        let dir = base.join(format!("lote {}", n % 7));
        let _ = fs::create_dir_all(&dir);
        let f = dir.join(name(&mut rng, n, true));
        let _ = fs::write(&f, b"x");
        match rng.below(4) {
            0 => {
                let _ = fs::remove_file(&f);
            }
            1 => {
                let _ = fs::rename(&f, dir.join(format!("renombrada {n}.pdf")));
            }
            2 if n.is_multiple_of(50) => {
                let _ = fs::rename(&dir, base.join(format!("lote movido {n}")));
            }
            _ => {}
        }
        ops += 1;
    }
    ops
}

/// VFS-3 AC-6: four threads (two handles, one shared by clones) search without pause while
/// the indexer runs its first scan and a flood of creations, renames and deletions hits
/// the watcher. No search fails or panics, every hit's path rebuilds, and once the flood
/// stops the index reaches "up to date" and equals the disk.
#[test]
fn searches_while_indexing_and_watching() {
    let t = tree(11, scaled(3_000, 40_000));
    let app = Vfs::open(&t.db).unwrap();
    app.add_root(&t.root).unwrap();
    let (sink, up) = sink();
    app.start_indexer(sink).unwrap();
    let other = Vfs::open(&t.db).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let searches = Arc::new(AtomicUsize::new(0));
    let mut threads = Vec::new();
    for i in 0..4 {
        let vfs = if i % 2 == 0 {
            app.clone()
        } else {
            other.clone()
        };
        let (stop, searches) = (Arc::clone(&stop), Arc::clone(&searches));
        threads.push(std::thread::spawn(move || {
            let mut k = i;
            while !stop.load(Ordering::SeqCst) {
                let q = query(QUERIES[k % QUERIES.len()]);
                let r = vfs
                    .search(&q)
                    .unwrap_or_else(|e| panic!("search {:?} failed: {e}", q.text));
                for h in r.hits.iter().take(3) {
                    // A hit may be deleted between the search and this lookup.
                    let p = if h.is_dir {
                        vfs.dir_path(h.id)
                    } else {
                        vfs.file_path(h.id)
                    };
                    if let Err(e) = p {
                        assert_eq!(e.code(), "not_found", "path of hit {}: {e}", h.id);
                    }
                }
                searches.fetch_add(1, Ordering::SeqCst);
                k += 1;
            }
        }));
    }
    let flooder = {
        let (root, stop) = (t.root.clone(), Arc::clone(&stop));
        std::thread::spawn(move || flood(&root, 7, &stop))
    };
    std::thread::sleep(Duration::from_secs(scaled(4, 60) as u64));
    stop.store(true, Ordering::SeqCst);
    let ops = flooder.join().unwrap();
    for th in threads {
        th.join().unwrap();
    }
    assert!(searches.load(Ordering::SeqCst) > 0 && ops > 0);
    // The watcher coalesces for a few seconds; then the index must settle on the disk.
    while up.try_recv().is_ok() {}
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        up.recv_timeout(left)
            .expect("never up to date after the flood");
        let snapshot = index_snapshot(&raw_conn(&t.db), &stored_root(&t.root));
        if snapshot == disk_truth(&t.root) {
            break;
        }
    }
    app.shutdown();
    other.shutdown();
    check_structure(&t.db, 500, "after the flood");
}

/// VFS-3 AC-7, VFS-1 AC-10 (S6): many threads fire queries at one handle, each newer
/// query interrupting the one running. Every call returns `Ok` (a cut-short one says so
/// with `truncated_by`), and afterwards an uninterrupted query returns the full answer,
/// the same as on a fresh handle.
#[test]
fn newer_queries_interrupt_older_ones() {
    let t = tree(12, scaled(5_000, 50_000));
    index(&t.db, &[&t.root]);
    let vfs = Vfs::open(&t.db).unwrap();
    let cut = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..6)
        .map(|i| {
            let (vfs, cut) = (vfs.clone(), Arc::clone(&cut));
            std::thread::spawn(move || {
                for k in 0..scaled(60, 600) {
                    let q = query(QUERIES[(i + k) % QUERIES.len()]);
                    let r = vfs.search(&q).unwrap();
                    if r.truncated_by == Some(vfs_core::TruncatedBy::Deadline) {
                        cut.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();
    for th in threads {
        th.join().unwrap();
    }
    let fresh = Vfs::open(&t.db).unwrap();
    for text in QUERIES {
        let a = vfs.search(&query(text)).unwrap();
        let b = fresh.search(&query(text)).unwrap();
        let ids = |r: &vfs_core::SearchResponse| -> Vec<(bool, i64)> {
            r.hits.iter().map(|h| (h.is_dir, h.id)).collect()
        };
        assert_eq!(
            ids(&a),
            ids(&b),
            "{text}: interrupted handle answers differently"
        );
        assert_eq!(a.total, b.total, "{text}");
    }
    eprintln!(
        "{} of the racing searches were cut short",
        cut.load(Ordering::SeqCst)
    );
}

/// VFS-3 AC-8, VFS-1 AC-6: the app is shut down at a random moment of its first scan,
/// sometimes while searches run; `shutdown` returns promptly, and the next start resumes
/// to an index equal to the disk.
#[test]
fn shutdown_at_any_moment() {
    let t = tree(13, scaled(3_000, 30_000));
    let mut rng = Rng::new(99);
    for i in 0..scaled(8, 60) {
        let vfs = Vfs::open(&t.db).unwrap();
        if vfs.list_roots().unwrap().is_empty() {
            vfs.add_root(&t.root).unwrap();
        }
        let (sink, _up) = sink();
        vfs.start_indexer(sink).unwrap();
        let searcher = rng.chance(50).then(|| {
            let vfs = vfs.clone();
            std::thread::spawn(move || {
                for text in QUERIES.iter().cycle().take(40) {
                    vfs.search(&query(text)).unwrap();
                }
            })
        });
        std::thread::sleep(Duration::from_millis(rng.below(1_500)));
        let t0 = Instant::now();
        vfs.shutdown();
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "shutdown {i} took {:?}",
            t0.elapsed()
        );
        if let Some(s) = searcher {
            s.join().unwrap();
        }
        drop(vfs);
        check_structure(&t.db, 100, &format!("shutdown {i}"));
    }
    let vfs = Vfs::open(&t.db).unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    drop(vfs);
    assert_matches_disk(
        &raw_conn(&t.db),
        &stored_root(&t.root),
        &t.root,
        "after shutdowns",
    );
}

/// VFS-3 AC-9, VFS-1 AC-6: the app (`vfs run`) and the command line share one index: the
/// command line indexes a second folder and searches while the app indexes and watches the
/// first. Neither fails, the app keeps its session, and both folders end equal to the disk.
#[test]
fn app_and_cli_share_an_index() {
    let t = tree(14, scaled(3_000, 30_000));
    let second = t._tmp.path().join("segundo");
    fs::create_dir(&second).unwrap();
    gen_tree(&second, &mut Rng::new(15), scaled(2_000, 20_000));
    let mut app = App::start(&t.db, &[&t.root]);
    std::thread::sleep(Duration::from_millis(300));
    index(&t.db, &[&second]);
    for text in QUERIES {
        let out = std::process::Command::new(VFS_BIN)
            .args(["search", "--db"])
            .arg(&t.db)
            .arg(text)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "cli search {text}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // The app learns of the second root only at its next start; the first must settle,
    // including files created while it runs.
    let mut rng = Rng::new(16);
    let base = t.root.join("en vivo");
    fs::create_dir_all(&base).unwrap();
    for n in 0..200 {
        fs::write(base.join(name(&mut rng, n, true)), b"y").unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if !app.wait_up_to_date(deadline.saturating_duration_since(Instant::now())) {
            assert_matches_disk(
                &raw_conn(&t.db),
                &stored_root(&t.root),
                &t.root,
                &format!("app never settled; last progress {:?}", app.last),
            );
        }
        let snap = index_snapshot(&raw_conn(&t.db), &stored_root(&t.root));
        if snap == disk_truth(&t.root) {
            break;
        }
    }
    app.stop();
    let mut bad = t.db.as_os_str().to_owned();
    bad.push(".bad");
    assert!(
        !std::path::Path::new(&bad).exists(),
        "a healthy shared index was moved aside"
    );
    check_structure(&t.db, 500, "shared index");
    assert_matches_disk(&raw_conn(&t.db), &stored_root(&t.root), &t.root, "app root");
    assert_matches_disk(&raw_conn(&t.db), &stored_root(&second), &second, "cli root");
}
