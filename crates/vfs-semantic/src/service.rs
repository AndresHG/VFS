//! The app's search by meaning (VFS-29 R1, R1a, R4, R7, R15, R16): the switch and the pause,
//! kept in `meaning.json` in the app data folder; one thread that, while the switch is on,
//! downloads the local model on first use, then embeds the content chunks under the content
//! indexer's pace rules (VFS-27 R7), resumes after a restart, retries a failed run with
//! backoff, and embeds new documents when content indexing reads them; progress with time
//! left; and the ranker the app's searches use. With the switch off it starts no thread and
//! creates no file. Nothing here knows about Tauri.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use vfs_content::pace::{Decision, Pace, PaceConfig, Readings};
use vfs_content::service::{Probes, WaitKind};

use crate::client::{WorkerConfig, WorkerEmbedder};
use crate::install::{self, Download, Fetch};
use crate::search::{MeaningConfig, MeaningIndex};
use crate::store::Store;
use crate::{RunOptions, LOCAL_MODEL};

/// Settings file in the app data folder.
pub const SETTINGS_FILE: &str = "meaning.json";
/// Folder of the downloaded models in the app data folder.
pub const MODELS_DIR: &str = "models";

/// After content indexing reports new text, a run waits this long without a further report
/// (VFS-50: work after a change follows the change, not every document).
pub const CHANGE_QUIET: Duration = Duration::from_secs(120);
/// The first retry after a failed download or run, doubling up to [`RETRY_MAX`] (R7).
const RETRY_FIRST: Duration = Duration::from_secs(60);
const RETRY_MAX: Duration = Duration::from_secs(60 * 60);
/// A wait longer than this ends the worker, to give its memory back; the next batch starts
/// it again.
const KILL_WORKER_AFTER: Duration = Duration::from_secs(60);
/// Texts the time left is computed from.
const RATE_WINDOW: usize = 200;

/// The persisted switch and pause (R1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// «Búsqueda por significado».
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub paused: bool,
}

/// What the service is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Phase {
    /// The switch is off.
    Off,
    /// Downloading the model and the worker (R1a).
    Downloading {
        done: u64,
        total: u64,
    },
    /// Content search is off or has stored no text yet: nothing to embed.
    NoContent,
    Embedding,
    /// Waiting between batches, with the reason (VFS-27 R7).
    Waiting {
        reason: WaitKind,
    },
    Paused,
    /// Every text has its vector; new ones are embedded when content indexing reads them.
    Finished,
    /// The download or the run failed; it is tried again later (R7).
    Failed {
        detail: String,
    },
}

/// The service as the settings panel shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub settings: Settings,
    pub phase: Phase,
    /// Texts embedded in the current run, and those it set out to embed.
    pub done: u64,
    pub total: u64,
    pub seconds_left: Option<u64>,
    /// A complete vector set exists: searches use meaning (R11).
    pub searchable: bool,
}

/// Paths, model files and limits.
pub struct ServiceConfig {
    pub index_db: PathBuf,
    /// The app data folder: settings and models live here.
    pub data_dir: PathBuf,
    /// The model's files (`install::local_model` in the app).
    pub downloads: Vec<Download>,
    /// The worker command; its `model_dir` is the model folder.
    pub worker: WorkerConfig,
    /// The query worker the searches use.
    pub query: MeaningConfig,
    pub pace: PaceConfig,
    /// [`CHANGE_QUIET`] in the app.
    pub change_quiet: Duration,
}

/// The model folder for a data folder.
pub fn model_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(MODELS_DIR).join(LOCAL_MODEL.name)
}

/// Reads `meaning.json`; missing or unreadable means off.
pub fn load_settings(data_dir: &Path) -> Settings {
    std::fs::read_to_string(data_dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_settings(data_dir: &Path, s: &Settings) {
    let tmp = data_dir.join(format!("{SETTINGS_FILE}.tmp"));
    let ok = std::fs::write(&tmp, serde_json::to_vec(s).expect("json")).is_ok()
        && std::fs::rename(&tmp, data_dir.join(SETTINGS_FILE)).is_ok();
    if !ok {
        log::warn!("could not save {SETTINGS_FILE}");
    }
}

struct State {
    status: Status,
    poked: bool,
    changed_at: Option<Instant>,
    shutdown: bool,
}

struct Shared {
    cfg: ServiceConfig,
    fetch: Box<dyn Fetch + Send + Sync>,
    on_status: Box<dyn Fn(&Status) + Send + Sync>,
    state: Mutex<State>,
    wake: Condvar,
    /// Stops the download or run in flight (pause, switch off, shutdown).
    stop: AtomicBool,
    index: MeaningIndex,
}

/// The app's search by meaning for one index.
pub struct MeaningService {
    shared: Arc<Shared>,
    probes: Mutex<Option<Box<dyn Probes>>>,
    thread: Mutex<Option<JoinHandle<Box<dyn Probes>>>>,
}

impl MeaningService {
    /// Loads the settings; with the switch on, starts the thread. With it off, nothing is
    /// created and no worker runs (R15).
    pub fn start(
        cfg: ServiceConfig,
        fetch: Box<dyn Fetch + Send + Sync>,
        probes: Box<dyn Probes>,
        on_status: Box<dyn Fn(&Status) + Send + Sync>,
    ) -> MeaningService {
        let settings = load_settings(&cfg.data_dir);
        let searchable = searchable(&cfg.index_db);
        let index = MeaningIndex::new(cfg.index_db.clone(), cfg.query.clone());
        let svc = MeaningService {
            shared: Arc::new(Shared {
                cfg,
                fetch,
                on_status,
                state: Mutex::new(State {
                    status: Status {
                        settings: settings.clone(),
                        phase: Phase::Off,
                        done: 0,
                        total: 0,
                        seconds_left: None,
                        searchable,
                    },
                    poked: true,
                    changed_at: None,
                    shutdown: false,
                }),
                wake: Condvar::new(),
                stop: AtomicBool::new(false),
                index,
            }),
            probes: Mutex::new(Some(probes)),
            thread: Mutex::new(None),
        };
        if settings.enabled {
            svc.spawn();
        }
        svc
    }

    pub fn status(&self) -> Status {
        self.shared.lock().status.clone()
    }

    /// The ranker the app's searches fuse with BM25, while the switch is on and a complete
    /// vector set exists (R11).
    pub fn ranker(&self) -> Option<Arc<dyn vfs_content::Meaning>> {
        let st = self.shared.lock();
        let s = &st.status;
        (s.settings.enabled && s.searchable)
            .then(|| Arc::new(self.shared.index.clone()) as Arc<dyn vfs_content::Meaning>)
    }

    /// Loads the rows and starts the query worker ahead of a search (the «Documentos» tab
    /// opened). Returns at once.
    pub fn warm(&self) {
        if self.ranker().is_some() {
            self.shared.index.warm();
        }
    }

    /// «Búsqueda por significado» (R1). On: the download and the embedding start (R1a);
    /// off: they stop, and the rows and workers are released.
    pub fn set_enabled(&self, on: bool) {
        self.shared.stop.store(!on, Ordering::SeqCst);
        self.change(|st| {
            st.status.settings.enabled = on;
            if on {
                st.status.settings.paused = false;
            } else {
                st.status.phase = Phase::Off;
            }
        });
        if on {
            self.spawn();
        } else {
            self.join(Duration::from_secs(10));
            self.shared.index.release();
        }
    }

    pub fn set_paused(&self, paused: bool) {
        // The stop flag is raised here and lowered only here or on switching on, never by
        // the thread, so a pause or switch-off cannot slip past a run that is starting.
        self.shared.stop.store(paused, Ordering::SeqCst);
        self.change(|st| st.status.settings.paused = paused);
    }

    /// Content indexing read new or changed documents: they are embedded once it has been
    /// quiet for a while (R4).
    pub fn notify_content_changed(&self) {
        let mut st = self.shared.lock();
        st.changed_at = Some(Instant::now());
        st.poked = true;
        drop(st);
        self.shared.wake.notify_all();
    }

    /// Ends the thread and the worker; true when it ended within `wait`.
    pub fn shutdown(&self, wait: Duration) -> bool {
        self.shared.stop.store(true, Ordering::SeqCst);
        {
            let mut st = self.shared.lock();
            st.shutdown = true;
            st.poked = true;
        }
        self.shared.wake.notify_all();
        let ended = self.join(wait);
        self.shared.index.release();
        ended
    }

    fn change(&self, f: impl FnOnce(&mut State)) {
        let status = {
            let mut st = self.shared.lock();
            f(&mut st);
            st.poked = true;
            save_settings(&self.shared.cfg.data_dir, &st.status.settings);
            st.status.clone()
        };
        self.shared.wake.notify_all();
        (self.shared.on_status)(&status);
    }

    fn spawn(&self) {
        let mut guard = self.thread.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        if let Some(t) = guard.take() {
            if let Ok(p) = t.join() {
                *self.probes.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
            }
        }
        let Some(probes) = self.probes.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        self.shared.stop.store(false, Ordering::SeqCst);
        self.shared.lock().shutdown = false;
        let shared = Arc::clone(&self.shared);
        *guard = Some(std::thread::spawn(move || run_loop(&shared, probes)));
    }

    fn join(&self, wait: Duration) -> bool {
        let mut guard = self.thread.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = guard.as_ref() else {
            return true;
        };
        let deadline = Instant::now() + wait;
        while !t.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if !t.is_finished() {
            return false;
        }
        if let Some(Ok(p)) = guard.take().map(JoinHandle::join) {
            *self.probes.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
        }
        true
    }
}

impl Drop for MeaningService {
    fn drop(&mut self) {
        self.shutdown(Duration::from_secs(5));
    }
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, f: impl FnOnce(&mut Status)) {
        let status = {
            let mut st = self.lock();
            f(&mut st.status);
            st.status.clone()
        };
        (self.on_status)(&status);
    }

    /// Whether the thread should end or hold (switch off, pause, shutdown).
    fn holding(&self) -> bool {
        let st = self.lock();
        st.shutdown || !st.status.settings.enabled || st.status.settings.paused
    }

    /// Sleeps up to `d`, ending early when poked or stopped; true when it should go on.
    fn sleep(&self, d: Duration) -> bool {
        let st = self.lock();
        let (st, _) = self
            .wake
            .wait_timeout_while(st, d, |s| !s.poked && !self.stop.load(Ordering::Relaxed))
            .unwrap_or_else(|e| e.into_inner());
        drop(st);
        !self.stop.load(Ordering::Relaxed)
    }
}

fn searchable(index_db: &Path) -> bool {
    matches!(
        Store::open_ro(index_db)
            .ok()
            .flatten()
            .map(|s| s.search_set()),
        Some(Ok(Some(_)))
    )
}

/// The service thread: download, then embed, then wait for new text; until switched off.
fn run_loop(shared: &Shared, mut probes: Box<dyn Probes>) -> Box<dyn Probes> {
    let mut retry = RETRY_FIRST;
    loop {
        {
            let mut st = shared.lock();
            st.poked = false;
            if st.shutdown || !st.status.settings.enabled {
                return probes;
            }
        }
        if shared.lock().status.settings.paused {
            shared.set(|s| s.phase = Phase::Paused);
            wait_poke(shared, None);
            continue;
        }
        let outcome = step(shared, &mut *probes);
        match outcome {
            Step::Done => {
                retry = RETRY_FIRST;
                // Nothing runs until content indexing reports new text or the user acts.
                wait_poke(shared, None);
                // New text reported: let content indexing settle first (VFS-50).
                loop {
                    let quiet_for = shared.lock().changed_at.map(|t| t.elapsed());
                    match quiet_for {
                        Some(e) if e < shared.cfg.change_quiet && !shared.holding() => {
                            if !shared.sleep(shared.cfg.change_quiet - e) {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                shared.lock().changed_at = None;
            }
            Step::Held => {}
            Step::Failed(detail) => {
                log::warn!("search by meaning: {detail}");
                shared.set(|s| s.phase = Phase::Failed { detail });
                wait_poke(shared, Some(retry));
                retry = (retry * 2).min(RETRY_MAX);
            }
        }
    }
}

/// Waits for a poke (a change of settings, new text, shutdown), at most `d` when given.
fn wait_poke(shared: &Shared, d: Option<Duration>) {
    let st = shared.lock();
    match d {
        Some(d) => drop(
            shared
                .wake
                .wait_timeout_while(st, d, |s| !s.poked)
                .unwrap_or_else(|e| e.into_inner()),
        ),
        None => drop(
            shared
                .wake
                .wait_while(st, |s| !s.poked)
                .unwrap_or_else(|e| e.into_inner()),
        ),
    }
}

enum Step {
    /// Everything there is embedded (or nothing to embed).
    Done,
    /// Paused, switched off or shut down part way.
    Held,
    Failed(String),
}

/// One pass: the download if needed, then an embedding run.
fn step(shared: &Shared, probes: &mut dyn Probes) -> Step {
    let cfg = &shared.cfg;
    let dir = &cfg.worker.model_dir;
    if !install::installed(dir, &cfg.downloads) {
        let total = install::download_bytes(&cfg.downloads);
        shared.set(|s| s.phase = Phase::Downloading { done: 0, total });
        let mut last = Instant::now();
        let r = install::install(
            dir,
            &cfg.downloads,
            &*shared.fetch,
            &shared.stop,
            &mut |done, total| {
                if last.elapsed() > Duration::from_millis(500) {
                    last = Instant::now();
                    shared.set(|s| s.phase = Phase::Downloading { done, total });
                }
            },
        );
        match r {
            Ok(()) => {}
            Err(install::InstallError::Stopped) => return Step::Held,
            Err(e) => return Step::Failed(e.to_string()),
        }
    }
    if vfs_content::ChunkReader::open(&cfg.index_db).is_err() {
        shared.set(|s| s.phase = Phase::NoContent);
        return Step::Done;
    }
    let worker = std::rc::Rc::new(std::cell::RefCell::new(WorkerEmbedder::new(
        cfg.worker.clone(),
    )));
    let mut embedder = SharedWorker(std::rc::Rc::clone(&worker));
    let mut pace = Pace::new(cfg.pace.clone());
    let data_dir = cfg.data_dir.clone();
    let mut times: VecDeque<Duration> = VecDeque::new();
    let mut batch_started = Instant::now();
    let mut prev_done = 0u64;
    // The pace applies before the run reads the archive too, not only between batches.
    if !pace_wait(shared, &mut pace, probes, &data_dir, &mut || {}) {
        return Step::Held;
    }
    shared.set(|s| {
        s.phase = Phase::Embedding;
        s.done = 0;
        s.total = 0;
        s.seconds_left = None;
    });
    let result = crate::run(
        &cfg.index_db,
        &mut embedder,
        &RunOptions::default(),
        &shared.stop,
        &mut |p| {
            let took = batch_started.elapsed();
            pace.finished(Instant::now(), took);
            let texts = p.done.saturating_sub(prev_done).max(1);
            prev_done = p.done;
            for _ in 0..texts {
                times.push_back(took / texts as u32);
            }
            while times.len() > RATE_WINDOW {
                times.pop_front();
            }
            let per_text = times.iter().sum::<Duration>() / times.len().max(1) as u32;
            let left = per_text * p.total.saturating_sub(p.done) as u32;
            shared.set(|s| {
                s.phase = Phase::Embedding;
                s.done = p.done;
                s.total = p.total;
                s.seconds_left = Some(left.as_secs());
            });
            // The pace rules before the next batch (VFS-27 R7).
            let mut on_long_wait = || worker.borrow_mut().kill();
            if !pace_wait(shared, &mut pace, probes, &data_dir, &mut on_long_wait) {
                return false;
            }
            shared.set(|s| s.phase = Phase::Embedding);
            batch_started = Instant::now();
            true
        },
    );
    drop(embedder);
    drop(worker);
    match result {
        Err(e) => Step::Failed(e.to_string()),
        Ok(sum) if sum.error.is_some() => Step::Failed(sum.error.unwrap_or_default()),
        Ok(sum) if sum.stopped => Step::Held,
        Ok(sum) => {
            let searchable = searchable(&cfg.index_db);
            if sum.embedded > 0 || sum.dropped > 0 {
                shared.index.reload();
            }
            shared.set(|s| {
                s.phase = if sum.complete {
                    Phase::Finished
                } else {
                    Phase::NoContent
                };
                s.seconds_left = None;
                s.searchable = searchable;
            });
            Step::Done
        }
    }
}

/// Waits until the pace rules let the next batch run (VFS-27 R7), showing why it waits;
/// `on_long_wait` runs once a wait has lasted [`KILL_WORKER_AFTER`]. False when the service
/// is paused, switched off or stopped meanwhile.
fn pace_wait(
    shared: &Shared,
    pace: &mut Pace,
    probes: &mut dyn Probes,
    data_dir: &Path,
    on_long_wait: &mut dyn FnMut(),
) -> bool {
    let mut waited = Duration::ZERO;
    loop {
        if shared.holding() || shared.stop.load(Ordering::Relaxed) {
            return false;
        }
        // Embedding reads the app data drive (content.db, vectors.db).
        let mut disk = probes.disk_queue(data_dir);
        let mut cpu = probes.cpu();
        let decision = pace.decide(
            Instant::now(),
            Readings {
                name_busy: probes.name_busy(),
                idle: probes.idle(),
                free_bytes: probes.free_bytes(),
                disk: &mut || disk.take(),
                cpu: &mut || cpu.take(),
            },
        );
        match decision {
            Decision::Read => return true,
            Decision::Wait(d, reason) => {
                shared.set(|s| {
                    s.phase = Phase::Waiting {
                        reason: reason.into(),
                    }
                });
                waited += d;
                if waited > KILL_WORKER_AFTER {
                    on_long_wait();
                }
                if !shared.sleep(d) {
                    return false;
                }
                shared.lock().poked = false;
            }
        }
    }
}

/// The worker shared by the run and the pace, which ends it during a long wait.
struct SharedWorker(std::rc::Rc<std::cell::RefCell<WorkerEmbedder>>);

impl crate::Embedder for SharedWorker {
    fn model(&self) -> crate::ModelId {
        self.0.borrow().model()
    }

    fn embed(
        &mut self,
        texts: &[String],
        query: bool,
        stop: &AtomicBool,
    ) -> Result<Vec<Vec<f32>>, crate::EmbedError> {
        self.0.borrow_mut().embed(texts, query, stop)
    }
}
