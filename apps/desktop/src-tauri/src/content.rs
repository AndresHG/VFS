//! Content search's worker in the app (VFS-27 PR 27a): where the bundled `vfs-extract`
//! sidecar is (R1), and the handle of a content run that an update stops before the installer
//! starts (R3). The background content indexer that starts runs comes with PR 27b; nothing
//! here starts one, so with content search off no worker ever runs (R2).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// File name of the extraction worker, as the bundler places the sidecar next to the app's
/// executable (Tauri strips the target triple of `binaries/vfs-extract-<triple>`).
#[cfg(windows)]
pub const WORKER_NAME: &str = "vfs-extract.exe";
#[cfg(not(windows))]
pub const WORKER_NAME: &str = "vfs-extract";

/// How long a stopping run may take to finish the document in flight before its worker is
/// killed (VFS-27 R3).
pub const STOP_GRACE: Duration = Duration::from_secs(5);

/// How much longer than the grace the app waits for the run's thread (kill and reap).
const STOP_SLACK: Duration = Duration::from_secs(2);

/// Where the worker is for an app executable in `exe_dir`.
pub fn worker_in(exe_dir: &Path) -> PathBuf {
    exe_dir.join(WORKER_NAME)
}

/// The bundled worker next to the running app (R1). `None` only when the app's own path is
/// unknown; whether the file is there is the run's concern (a missing worker stops it with
/// "worker cannot start", VFS-2 R17).
pub fn worker_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(worker_in(exe.parent()?))
}

/// A content run on its own thread, with the flag that stops it.
pub struct ContentRun {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ContentRun {
    /// Runs `work` on a new thread; `work` must return soon after the flag is set (the
    /// engine's run does, killing its worker after [`STOP_GRACE`]).
    pub fn spawn(work: impl FnOnce(&AtomicBool) + Send + 'static) -> ContentRun {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("vfs-content".into())
            .spawn(move || work(&flag))
            .expect("spawn the content run");
        ContentRun {
            stop,
            thread: Some(thread),
        }
    }

    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Asks the run to stop and waits for it, at most `wait`. Returns whether it ended.
    pub fn stop(&mut self, wait: Duration) -> bool {
        self.stop.store(true, Ordering::SeqCst);
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let deadline = Instant::now() + wait;
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if thread.is_finished() {
            let _ = thread.join();
            true
        } else {
            false
        }
    }
}

/// The app's current content run, if any (managed Tauri state).
#[derive(Default)]
pub struct ContentRuns(pub Mutex<Option<ContentRun>>);

impl ContentRuns {
    /// Stops the current run, if any, before the app is replaced (VFS-27 R3): the document in
    /// flight gets [`STOP_GRACE`], then the worker is killed. Returns whether no run is left.
    pub fn stop_for_update(&self) -> bool {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(run) => {
                let ended = run.stop(STOP_GRACE + STOP_SLACK);
                if !ended {
                    log::warn!("content run still running after the update stop");
                }
                *guard = None;
                ended
            }
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-27 AC-1: the worker is looked for next to the app's executable, under the name the
    /// bundler gives the sidecar.
    #[test]
    fn worker_sits_next_to_the_app() {
        let dir = Path::new("/Applications/VFS.app/Contents/MacOS");
        let p = worker_in(dir);
        assert_eq!(p.parent(), Some(dir));
        assert!(p
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("vfs-extract"));
        assert_eq!(
            worker_path().unwrap().parent(),
            std::env::current_exe().unwrap().parent()
        );
    }

    /// VFS-27 AC-3: stopping for an update ends a run that honours the flag, and gives up
    /// after the wait on one that does not, so the update is never blocked.
    #[test]
    fn stop_for_update_ends_the_run() {
        let runs = ContentRuns::default();
        assert!(runs.stop_for_update(), "no run: nothing to stop");

        let saw_stop = Arc::new(AtomicBool::new(false));
        let seen = saw_stop.clone();
        *runs.0.lock().unwrap() = Some(ContentRun::spawn(move |stop| {
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(10));
            }
            seen.store(true, Ordering::SeqCst);
        }));
        assert!(runs.0.lock().unwrap().as_ref().unwrap().is_running());
        let t = Instant::now();
        assert!(runs.stop_for_update());
        assert!(t.elapsed() < Duration::from_secs(1));
        assert!(saw_stop.load(Ordering::SeqCst));
        assert!(runs.0.lock().unwrap().is_none());

        let mut stuck = ContentRun::spawn(|_| std::thread::sleep(Duration::from_secs(3)));
        let t = Instant::now();
        assert!(!stuck.stop(Duration::from_millis(200)));
        assert!(t.elapsed() < Duration::from_secs(1));
    }
}
