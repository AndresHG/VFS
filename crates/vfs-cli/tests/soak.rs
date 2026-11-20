//! Long-run soak (VFS-3 AC-10): the app (`vfs run`) indexes and watches a tree that is
//! churned without pause (creations, renames, moves, deletions and bursts of files); then
//! the churn stops and the index must reach "up to date", equal the disk, and the app's
//! resident memory must not have kept growing.
//!
//! Duration: `VFS_SOAK_SECS` (default 20 s; the scheduled CI job runs 1800 s, with
//! `VFS_STRESS=1` for bursts of 50,000 files).

mod support;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use support::*;

fn soak_secs() -> u64 {
    std::env::var("VFS_SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
}

/// One churn step: mostly small changes, sometimes a burst of `burst` files in a new folder,
/// a folder rename or move, or the deletion of an earlier burst.
fn churn_step(root: &Path, rng: &mut Rng, step: u64, burst: usize) {
    let work = root.join("trabajo");
    let _ = fs::create_dir_all(&work);
    match rng.below(100) {
        0 => {
            let dir = work.join(format!("rafaga {step}"));
            fs::create_dir_all(&dir).unwrap();
            for n in 0..burst {
                let _ = fs::write(dir.join(name(rng, n as u64, true)), b"r");
            }
        }
        1..=3 => {
            // Delete a whole earlier folder.
            if let Some(d) = pick_dir(&work, rng) {
                let _ = fs::remove_dir_all(d);
            }
        }
        4..=10 => {
            // Rename or move a folder.
            if let (Some(a), Some(b)) = (pick_dir(&work, rng), pick_dir(&work, rng)) {
                let to = if rng.chance(50) || b.starts_with(&a) {
                    a.with_file_name(format!("renombrada {step}"))
                } else {
                    b.join(format!("movida {step}"))
                };
                let _ = fs::rename(&a, to);
            }
        }
        _ => {
            let dir = pick_dir(&work, rng).unwrap_or_else(|| work.clone());
            let f = dir.join(name(rng, step, true));
            let _ = fs::write(&f, b"c");
            if rng.chance(30) {
                let _ = fs::rename(&f, dir.join(format!("cambiada {step}.txt")));
            } else if rng.chance(30) {
                let _ = fs::remove_file(&f);
            }
        }
    }
}

fn pick_dir(dir: &Path, rng: &mut Rng) -> Option<std::path::PathBuf> {
    let dirs: Vec<_> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    (!dirs.is_empty()).then(|| rng.pick(&dirs).clone())
}

/// VFS-3 AC-10: after `VFS_SOAK_SECS` of churn under the running app, the index reaches up
/// to date and equals the disk, the index is structurally sound, and resident memory in
/// the second half of the run stays within a fixed margin of the first half's peak.
#[test]
fn soak_churn_under_the_app() {
    let secs = soak_secs();
    let burst = scaled(2_000, 50_000);
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("archivo");
    fs::create_dir(&root).unwrap();
    gen_tree(&root, &mut Rng::new(21), scaled(3_000, 50_000));
    fs::create_dir(tmp.path().join("datos")).unwrap();
    let db = tmp.path().join("datos").join("index.db");

    let mut app = App::start(&db, &[&root]);
    let pid = app.child.id();
    let stop = Arc::new(AtomicBool::new(false));
    let churner = {
        let (root, stop) = (root.clone(), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut rng = Rng::new(22);
            let mut step = 0u64;
            while !stop.load(Ordering::SeqCst) {
                churn_step(&root, &mut rng, step, burst);
                step += 1;
                std::thread::sleep(Duration::from_millis(2));
            }
            step
        })
    };
    let start = Instant::now();
    let mut samples: Vec<(Duration, u64)> = Vec::new();
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_secs(1).min(Duration::from_secs(secs)));
        app.drain();
        if let Some(rss) = rss_bytes(pid) {
            samples.push((start.elapsed(), rss));
        }
        assert!(
            app.child.try_wait().unwrap().is_none(),
            "the app exited during the soak"
        );
    }
    stop.store(true, Ordering::SeqCst);
    let steps = churner.join().unwrap();

    // Settle: the index must reach up to date and equal the disk.
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if !app.wait_up_to_date(left) {
            assert_matches_disk(
                &raw_conn(&db),
                &stored_root(&root),
                &root,
                &format!("never up to date after the soak; last {:?}", app.last),
            );
        }
        if index_snapshot(&raw_conn(&db), &stored_root(&root)) == disk_truth(&root) {
            break;
        }
    }
    app.stop();
    check_structure(&db, 2_000, "after the soak");
    assert_nothing_pending(&db, "after the soak");

    // Memory: the second half's peak stays within 64 MiB of the first half's.
    let half = Duration::from_secs(secs / 2);
    let peak =
        |f: &dyn Fn(Duration) -> bool| samples.iter().filter(|(t, _)| f(*t)).map(|(_, r)| *r).max();
    if let (Some(first), Some(second)) = (peak(&|t| t <= half), peak(&|t| t > half)) {
        eprintln!(
            "soak: {steps} churn steps in {secs} s; peak RSS {} MiB then {} MiB",
            first >> 20,
            second >> 20
        );
        assert!(
            second <= first + (64 << 20),
            "resident memory kept growing: {} MiB, then {} MiB",
            first >> 20,
            second >> 20
        );
    }
}
