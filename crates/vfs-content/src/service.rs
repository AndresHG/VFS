//! The background content indexer the app runs (VFS-27 R2, R5, R7–R11, R15, R16): the
//! switch and the pause, kept in `content.json` in the app data folder; one thread that runs
//! the engine under the pace rules while content search is on; progress with documents per
//! hour and time left; the "content run open" marker and the integrity check after an
//! unclean stop; run-level reasons; and the diagnostic records. The app supplies the probes
//! and receives status changes; nothing here knows about Tauri.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::indexer::{self, Admit, Gate, IndexOptions, OutcomeKind, RunSummary, StopReason};
use crate::pace::{Decision, Pace, PaceConfig, Readings, WaitReason};
use crate::store;
use vfs_core::probes::CpuSample;

/// Settings file in the app data folder.
pub const SETTINGS_FILE: &str = "content.json";
/// Present while a content run is open; found at start, it means the last run did not end
/// cleanly (R11).
pub const RUN_MARKER: &str = "content-run-open";

/// VFS-27 D3: time per document on the target PC, as a range (replaced by 27m's numbers).
const MS_PER_DOC: (u64, u64) = (170, 550);
/// VFS-27 D3: store growth per document, as a range (13–16 MB per 1,000 documents).
const BYTES_PER_DOC: (u64, u64) = (13_000, 16_000);
/// Time per document before 200 have been read: the middle of D3's range.
const ESTIMATE_MS_PER_DOC: u64 = (MS_PER_DOC.0 + MS_PER_DOC.1) / 2;
/// Documents the time left is computed from, once read (R8).
const RATE_WINDOW: usize = 200;
/// After a finished run, how long to wait for new documents before looking anyway (R10).
const IDLE_RECHECK: Duration = Duration::from_secs(10 * 60);

/// What the app tells the service about the machine (VFS-27 R6, R7).
pub trait Probes: Send {
    /// A name walk, or a name pass over many folders, runs.
    fn name_busy(&self) -> bool;
    /// Time since the last keyboard or mouse input; `None` when the probe failed.
    fn idle(&self) -> Option<Duration>;
    /// Free bytes on the app-data drive; `None` when unknown.
    fn free_bytes(&self) -> Option<u64>;
    /// One disk-load sample (queue length) for the disk holding `path`; `None` where there is
    /// no probe.
    fn disk_queue(&mut self, path: &Path) -> Option<f64>;
    /// The machine's processor use since the previous call; `None` when unknown.
    fn cpu(&mut self) -> Option<CpuSample>;
}

/// Where status changes and diagnostic records go: Tauri events and the log in the app.
pub trait Events: Send + Sync {
    fn status(&self, status: &Status);
    /// One diagnostic record, with no file or folder name in it (R16).
    fn diag(&self, record: &str);
}

/// The persisted switch and pause (R5, R8).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// «Buscar también dentro de los documentos».
    #[serde(default)]
    pub enabled: bool,
    /// The user paused; holds across restarts until resumed (R8).
    #[serde(default)]
    pub paused: bool,
    /// Content search was offered once (R4; read by the interface).
    #[serde(default)]
    pub offered: bool,
}

/// What the run is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Phase {
    /// Content search is off.
    Off,
    /// The user paused.
    Paused,
    /// Checking the store after an unclean stop (R11).
    Checking,
    /// The store failed the check or is from another version (R11); waits for the user.
    Damaged,
    Reading,
    /// Waiting between documents, with the reason (R6, R7).
    Waiting {
        reason: WaitKind,
    },
    /// The queue is empty; new documents are read when the name indexer finds them (R10).
    Finished,
    /// The run stopped for a run-level reason and waits for «Reintentar» (R15).
    Stopped {
        reason: StopReason,
        detail: String,
    },
}

/// Why the run waits, as shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WaitKind {
    NameIndexing,
    DiskBusy,
    CpuBusy,
    InUse,
    LowSpace,
}

impl From<WaitReason> for WaitKind {
    fn from(r: WaitReason) -> WaitKind {
        match r {
            WaitReason::NameIndexing => WaitKind::NameIndexing,
            WaitReason::DiskBusy => WaitKind::DiskBusy,
            WaitReason::CpuBusy => WaitKind::CpuBusy,
            WaitReason::InUse => WaitKind::InUse,
            WaitReason::LowSpace => WaitKind::LowSpace,
        }
    }
}

/// The content run as the status area shows it (R8).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub settings: Settings,
    pub phase: Phase,
    /// Documents processed (read, skipped or failed) out of all documents in the index.
    pub done: u64,
    pub total: u64,
    /// Documents per hour over working time only (not pauses or waits).
    pub per_hour: Option<f64>,
    /// Time left: from the last 200 documents, or from the D3 estimate until then.
    pub seconds_left: Option<u64>,
}

/// Paths and limits of the service.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub index_db: PathBuf,
    /// The app data folder: settings and the run marker live here.
    pub data_dir: PathBuf,
    /// The engine's options, with the worker command (the bundled sidecar in the app).
    pub index: IndexOptions,
    pub pace: PaceConfig,
}

struct Shared {
    cfg: ServiceConfig,
    events: Arc<dyn Events>,
    state: Mutex<State>,
    wake: Condvar,
    /// Stops the current engine run (pause, switch off, retry, shutdown).
    stop_run: Arc<AtomicBool>,
}

struct State {
    settings: Settings,
    status: Status,
    /// Set by `notify`, `resume`, `retry`, `rebuild` and `shutdown`; cleared by the loop.
    poked: bool,
    shutdown: bool,
    /// The user asked to read the documents again after damage (R11).
    rebuild: bool,
    /// The user asked to retry after a run-level stop (R15).
    retry: bool,
    /// The user resumed: a low-space pause ends (R6).
    resumed: bool,
    /// An engine run is in progress.
    running: bool,
    /// «Borrar lo leído» asked; the content thread, the only one that opens the store,
    /// deletes it and leaves the outcome in `deleted`.
    delete: bool,
    deleted: Option<Result<(), String>>,
    /// The first and the last report of changed names not yet acted on (VFS-50 R3).
    names_first: Option<Instant>,
    names_last: Option<Instant>,
}

/// The app's background content indexer.
pub struct ContentService {
    shared: Arc<Shared>,
    /// The thread, which hands the probes back when it ends.
    thread: Mutex<Option<JoinHandle<Box<dyn Probes>>>>,
    /// The probes while no thread holds them.
    probes: Mutex<Option<Box<dyn Probes>>>,
}

impl ContentService {
    /// Loads the settings and, when content search is on, starts the thread. With it off,
    /// nothing is created and no worker runs (R2).
    pub fn start(
        cfg: ServiceConfig,
        probes: Box<dyn Probes>,
        events: Arc<dyn Events>,
    ) -> ContentService {
        // Only the small settings file is read here: `start` runs on the app's main thread,
        // so the counts and the store check happen on the content thread (#178).
        let settings = load_settings(&cfg.data_dir);
        let (done, total) = (0, 0);
        let status = Status {
            settings: settings.clone(),
            phase: if settings.paused && settings.enabled {
                Phase::Paused
            } else {
                Phase::Off
            },
            done,
            total,
            per_hour: None,
            seconds_left: None,
        };
        let shared = Arc::new(Shared {
            cfg,
            events,
            state: Mutex::new(State {
                settings: settings.clone(),
                status,
                poked: false,
                shutdown: false,
                rebuild: false,
                retry: false,
                resumed: false,
                running: false,
                delete: false,
                deleted: None,
                names_first: None,
                names_last: None,
            }),
            wake: Condvar::new(),
            stop_run: Arc::new(AtomicBool::new(false)),
        });
        let svc = ContentService {
            shared,
            thread: Mutex::new(None),
            probes: Mutex::new(Some(probes)),
        };
        if settings.enabled {
            svc.spawn();
        }
        svc
    }

    fn spawn(&self) {
        let mut thread = self.thread.lock().unwrap_or_else(|e| e.into_inner());
        if thread.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        let mut slot = self.probes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(done) = thread.take() {
            if let Ok(p) = done.join() {
                *slot = Some(p);
            }
        }
        let Some(probes) = slot.take() else { return };
        drop(slot);
        let shared = self.shared.clone();
        *thread = Some(
            std::thread::Builder::new()
                .name("vfs-content".into())
                .spawn(move || run_loop(shared, probes))
                .expect("spawn the content thread"),
        );
    }

    pub fn status(&self) -> Status {
        self.lock().status.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn change(&self, f: impl FnOnce(&mut State)) {
        let mut st = self.lock();
        let before = st.settings.clone();
        f(&mut st);
        st.poked = true;
        st.status.settings = st.settings.clone();
        // Saved only when the settings changed: a poke alone writes nothing (R2).
        let saved = if st.settings == before {
            Ok(())
        } else {
            save_settings(&self.shared.cfg.data_dir, &st.settings)
        };
        let status = st.status.clone();
        drop(st);
        if saved.is_err() {
            self.shared.events.diag("content settings not saved");
        }
        self.shared.wake.notify_all();
        self.shared.events.status(&status);
    }

    /// The switch (R5, D4): off stops the run and keeps the store; on clears any pause and
    /// resumes.
    pub fn set_enabled(&self, on: bool) {
        if !on {
            self.shared.stop_run.store(true, Ordering::SeqCst);
            self.shared
                .events
                .diag("content search switched off by the user");
        }
        self.change(|st| {
            st.settings.enabled = on;
            st.settings.offered = true;
            if on {
                st.settings.paused = false;
                st.resumed = true;
            }
        });
        if on {
            self.spawn();
        }
    }

    /// «Pausar»: holds across restarts (R8).
    pub fn pause(&self) {
        self.shared.stop_run.store(true, Ordering::SeqCst);
        self.shared.events.diag("content run paused by the user");
        self.change(|st| st.settings.paused = true);
    }

    /// «Reanudar»: also ends a low-space pause (R6).
    pub fn resume(&self) {
        self.change(|st| {
            st.settings.paused = false;
            st.resumed = true;
        });
    }

    /// «Reintentar» after a run-level stop (R15).
    pub fn retry(&self) {
        self.change(|st| st.retry = true);
    }

    /// «Volver a leer los documentos» after damage (R11): the store is deleted only now.
    pub fn rebuild(&self) {
        self.change(|st| st.rebuild = true);
    }

    /// The offer was answered «Ahora no» (R4): not asked again, switch left off.
    pub fn mark_offered(&self) {
        self.change(|st| st.settings.offered = true);
    }

    /// The name indexer added or changed documents: read them under the same pace (R10).
    pub fn notify_names_changed(&self) {
        // Only wakes the thread: no settings write, no status event (R2). The run waits for
        // calm (VFS-50 R3).
        {
            let mut st = self.lock();
            let now = Instant::now();
            st.names_first.get_or_insert(now);
            st.names_last = Some(now);
            st.poked = true;
        }
        self.shared.wake.notify_all();
    }

    /// «Borrar lo leído de los documentos» (R5): stops the run, deletes `content.db`, and
    /// switches content search off. Name search is not touched.
    pub fn delete_store(&self) -> crate::Result<()> {
        let threaded = self
            .thread
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|t| !t.is_finished());
        if !threaded {
            self.set_enabled(false);
            delete_now(&self.shared.cfg)?;
            self.change(|st| st.status.phase = Phase::Off);
            return Ok(());
        }
        // The content thread deletes it between steps, so no check of its own holds the file
        // open meanwhile (Windows cannot delete an open file).
        self.shared.stop_run.store(true, Ordering::SeqCst);
        self.change(|st| {
            st.settings.enabled = false;
            st.delete = true;
            st.deleted = None;
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(result) = self.lock().deleted.take() {
                return result.map_err(|e| crate::Error::Io(std::io::Error::other(e)));
            }
            if Instant::now() >= deadline {
                return Err(crate::Error::Io(std::io::Error::other(
                    "the content run did not stop in time",
                )));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits until no engine run is in progress, at most `wait`; returns whether none is.
    pub fn wait_idle(&self, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        while self.lock().running {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    /// After an update that failed to install: undoes [`ContentService::shutdown`] and
    /// starts the thread again if content search is on.
    pub fn restart_after_failed_update(&self) {
        {
            let mut st = self.lock();
            st.shutdown = false;
            st.poked = true;
        }
        self.shared.stop_run.store(false, Ordering::SeqCst);
        if self.lock().settings.enabled {
            self.spawn();
        }
    }

    /// Stops the run and the thread, waiting at most `wait` (the worker gets the engine's
    /// grace for the document in flight). Returns whether the thread ended.
    pub fn shutdown(&self, wait: Duration) -> bool {
        self.shared.stop_run.store(true, Ordering::SeqCst);
        {
            let mut st = self.lock();
            st.shutdown = true;
            st.poked = true;
        }
        self.shared.wake.notify_all();
        self.join(wait)
    }

    fn join(&self, wait: Duration) -> bool {
        let mut guard = self.thread.lock().unwrap_or_else(|e| e.into_inner());
        let Some(thread) = guard.as_ref() else {
            return true;
        };
        let deadline = Instant::now() + wait;
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if !thread.is_finished() {
            return false;
        }
        if let Some(Ok(p)) = guard.take().map(JoinHandle::join) {
            *self.probes.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
        }
        true
    }
}

impl Drop for ContentService {
    fn drop(&mut self) {
        self.shutdown(Duration::from_secs(10));
    }
}

/// What the offer shows (R4, R6): the documents a run would read, the time and the disk it
/// would take, as ranges from D3, and whether the app-data drive is short of space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    /// Documents in the index, cloud-only files excluded.
    pub documents: u64,
    pub seconds: (u64, u64),
    pub bytes: (u64, u64),
    pub free_bytes: Option<u64>,
    /// Free space is below the larger of 2 GB and twice the estimated store (R6).
    pub low_space: bool,
}

/// The offer's numbers for the index at `index_db`, with `free_bytes` on the app-data drive.
pub fn offer(index_db: &Path, free_bytes: Option<u64>) -> Offer {
    let documents = Connection::open_with_flags(index_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|c| {
            c.query_row(
                "SELECT count(*) FROM files WHERE kind = ?1 AND flags & ?2 = 0",
                rusqlite::params![
                    vfs_core::FileKind::Document.code(),
                    vfs_core::db::ingest::FLAG_CLOUD_ONLY
                ],
                |r| r.get::<_, i64>(0),
            )
        })
        .unwrap_or(0)
        .max(0) as u64;
    let bytes = (documents * BYTES_PER_DOC.0, documents * BYTES_PER_DOC.1);
    // The offer's floor is the low-space rule's resume level: 2 GB.
    let floor = PaceConfig::default().low_space_resume.max(2 * bytes.1);
    Offer {
        documents,
        seconds: (
            documents * MS_PER_DOC.0 / 1000,
            documents * MS_PER_DOC.1 / 1000,
        ),
        bytes,
        free_bytes,
        low_space: free_bytes.is_some_and(|f| f < floor),
    }
}

pub fn load_settings(data_dir: &Path) -> Settings {
    std::fs::read(data_dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Writes the settings atomically (a temporary file renamed over the old one).
fn save_settings(data_dir: &Path, s: &Settings) -> std::io::Result<()> {
    let tmp = data_dir.join(format!("{SETTINGS_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    std::fs::rename(&tmp, data_dir.join(SETTINGS_FILE))
}

/// Whether the store passes SQLite's `quick_check` and has this build's version (R11, VFS-2
/// R19). A missing store is fine: there is nothing to check.
fn store_is_sound(index_db: &Path) -> bool {
    let path = store::store_path(index_db);
    if !path.exists() {
        return true;
    }
    let Ok(conn) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return false;
    };
    let version: Option<i64> = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .ok();
    if version != Some(store::STORE_VERSION) {
        return false;
    }
    conn.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
        .is_ok_and(|r| r == "ok")
}

/// Deletes the store and resets every `content_state` (R5), and the run marker.
fn delete_now(cfg: &ServiceConfig) -> crate::Result<()> {
    crate::switch_off(&cfg.index_db)?;
    let _ = std::fs::remove_file(cfg.data_dir.join(RUN_MARKER));
    Ok(())
}

/// Whether a store exists with another version than this build's (VFS-2 R19). One
/// read-only `PRAGMA`; no other check.
fn store_version_differs(index_db: &Path) -> bool {
    let path = store::store_path(index_db);
    if !path.exists() {
        return false;
    }
    !Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|c| c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0)))
        .is_ok_and(|v| v == store::STORE_VERSION)
}

/// What a run would read from, cheaply: the documents' count, highest id, total
/// modification time and cloud-only count, and the roots passed over (offline or paused).
/// Unchanged after a finished run, there is nothing new to read (R8–R10).
#[derive(Debug, Clone, PartialEq)]
struct Signature {
    documents: (i64, i64, f64, i64),
    roots: Roots,
}

/// The roots and those passed over (offline or paused), both sorted: cheap to read, so they
/// are looked at even when no change was reported. A removed root reports no name change
/// (VFS-50 R5).
#[derive(Debug, Clone, PartialEq)]
struct Roots {
    all: Vec<i64>,
    passed_over: Vec<i64>,
}

fn roots_now(index_db: &Path) -> Option<Roots> {
    let conn = Connection::open_with_flags(index_db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let all = conn
        .prepare("SELECT id FROM roots ORDER BY id")
        .ok()?
        .query_map([], |r| r.get::<_, i64>(0))
        .ok()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .ok()?;
    Some(Roots {
        all,
        passed_over: indexer::passed_over_roots(index_db).ok()?,
    })
}

fn documents_signature(index_db: &Path) -> Option<Signature> {
    let conn = Connection::open_with_flags(index_db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let documents = conn
        .query_row(
            "SELECT count(*), coalesce(max(id), 0), total(mtime),
                    sum(flags & ?2 <> 0)
             FROM files WHERE kind = ?1",
            rusqlite::params![
                vfs_core::FileKind::Document.code(),
                vfs_core::db::ingest::FLAG_CLOUD_ONLY
            ],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                ))
            },
        )
        .ok()?;
    let roots = roots_now(index_db)?;
    Some(Signature { documents, roots })
}

/// Documents in the index, and those the store has processed.
fn progress_counts(index_db: &Path) -> (u64, u64) {
    let total = Connection::open_with_flags(index_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|c| {
            c.query_row(
                "SELECT count(*) FROM files WHERE kind = ?1",
                [vfs_core::FileKind::Document.code()],
                |r| r.get::<_, i64>(0),
            )
        })
        .unwrap_or(0)
        .max(0) as u64;
    let done = indexer::stats(index_db)
        .map(|s| s.indexed + s.failed + s.skipped)
        .unwrap_or(0);
    (done, total)
}

fn set_phase(shared: &Shared, phase: Phase) {
    let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    if st.status.phase == phase {
        return;
    }
    st.status.phase = phase;
    let status = st.status.clone();
    drop(st);
    shared.events.status(&status);
}

/// Waits until poked or `timeout`; returns whether the service is shutting down.
fn wait(shared: &Shared, timeout: Option<Duration>) -> bool {
    let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    let deadline = timeout.map(|t| Instant::now() + t);
    while !st.poked && !st.shutdown {
        let left = deadline.map(|d| d.saturating_duration_since(Instant::now()));
        if left == Some(Duration::ZERO) {
            break;
        }
        st = match left {
            Some(l) => {
                shared
                    .wake
                    .wait_timeout(st, l)
                    .unwrap_or_else(|e| e.into_inner())
                    .0
            }
            None => shared.wake.wait(st).unwrap_or_else(|e| e.into_inner()),
        };
    }
    st.poked = false;
    st.shutdown
}

fn run_loop(shared: Arc<Shared>, mut probes: Box<dyn Probes>) -> Box<dyn Probes> {
    let cfg = shared.cfg.clone();
    let marker = cfg.data_dir.join(RUN_MARKER);
    let mut checked = false;
    // The store failed the check; not checked again until the user acts (R11).
    let mut damaged = false;
    // The documents as they were when the last run finished with nothing left, and when
    // (R10).
    let mut finished_at: Option<(Signature, Instant)> = None;
    // When a run stopped by `index.db` refusing writes starts again on its own (R15).
    let mut index_retry_at: Option<Instant> = None;
    let mut pace = Pace::new(cfg.pace.clone());
    // Shown from the start, also while paused (R8: «300 de 1.000» after a restart).
    let (done, total) = progress_counts(&cfg.index_db);
    {
        let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        st.status.done = done;
        st.status.total = total;
        let status = st.status.clone();
        drop(st);
        shared.events.status(&status);
    }
    loop {
        let (settings, shutdown, rebuild, retry, resumed) = {
            let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            (
                st.settings.clone(),
                st.shutdown,
                std::mem::take(&mut st.rebuild),
                std::mem::take(&mut st.retry),
                std::mem::take(&mut st.resumed),
            )
        };
        if shutdown {
            break;
        }
        // «Borrar lo leído», between steps: nothing of this thread has the store open.
        let delete = std::mem::take(
            &mut shared
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .delete,
        );
        if delete {
            let result = delete_now(&cfg).map_err(|e| e.to_string());
            damaged = false;
            checked = false;
            finished_at = None;
            {
                let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                st.status.done = 0;
                st.status.phase = Phase::Off;
                st.deleted = Some(result);
            }
            continue;
        }
        if !settings.enabled {
            // Switched off: idle without a worker until switched on again or shut down. A
            // store found damaged is judged again when it is switched on (it may be deleted).
            damaged = false;
            checked = false;
            set_phase(&shared, Phase::Off);
            if wait(&shared, None) {
                break;
            }
            continue;
        }
        if resumed {
            pace.user_resumed();
        }
        if settings.paused {
            set_phase(&shared, Phase::Paused);
            if wait(&shared, None) {
                break;
            }
            continue;
        }
        // R11: after an unclean stop, check the store before reading anything; search keeps
        // working meanwhile.
        // A store of another version is damage too (R11): the engine would rebuild it, and
        // VFS-27 asks the user first.
        if !rebuild && !damaged && store_version_differs(&cfg.index_db) {
            damaged = true;
        }
        if ((!checked && marker.exists()) || damaged) && !rebuild {
            if !damaged {
                set_phase(&shared, Phase::Checking);
                damaged = !store_is_sound(&cfg.index_db);
            }
            if damaged {
                set_phase(&shared, Phase::Damaged);
                shared
                    .events
                    .diag("content store damaged after an unclean stop");
                if wait(&shared, None) {
                    break;
                }
                continue;
            }
        }
        if rebuild {
            damaged = false;
            finished_at = None;
            let _ = store::delete(&cfg.index_db);
            shared
                .events
                .diag("content store deleted to read the documents again");
        }
        checked = true;
        // A run-level stop waits for «Reintentar» (R15), or, when `index.db` refused writes,
        // for its retry time. The guard is dropped before waiting.
        let stopped = matches!(
            shared
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .status
                .phase,
            Phase::Stopped { .. }
        );
        if stopped && !retry {
            let left = index_retry_at.map(|at| at.saturating_duration_since(Instant::now()));
            if left != Some(Duration::ZERO) {
                if wait(&shared, left) {
                    break;
                }
                continue;
            }
        }
        index_retry_at = None;

        // A run starts with a full rebuild of the queue in SQL: never while a name walk or a
        // large name pass runs (R7 rule 1; the post-update re-walk of #178).
        if probes.name_busy() {
            set_phase(
                &shared,
                Phase::Waiting {
                    reason: WaitKind::NameIndexing,
                },
            );
            if wait(&shared, Some(cfg.pace.name_wait)) {
                break;
            }
            continue;
        }
        // After a finished run, changed names start a run only once they calm down; a command
        // (switch-on, resume, retry, rebuild) does not wait (VFS-50 R3). With no change
        // reported, only the folders passed over and the one retry of a timed-out document can
        // bring work: the documents signature, a scan of every file row, is not computed
        // (VFS-50 R5).
        if let Some((sig, at)) = finished_at
            .as_ref()
            .filter(|_| !(retry || rebuild || resumed))
        {
            let (first, last) = {
                let st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                (st.names_first, st.names_last)
            };
            match first {
                Some(first) => {
                    let due = (last.unwrap_or(first) + cfg.pace.names_quiet)
                        .min(first + cfg.pace.names_max_wait);
                    let now = Instant::now();
                    if now < due {
                        if wait(&shared, Some(due - now)) {
                            break;
                        }
                        continue;
                    }
                }
                None => {
                    let same_roots = roots_now(&cfg.index_db).as_ref() == Some(&sig.roots);
                    if same_roots
                        && (at.elapsed() < IDLE_RECHECK
                            || !indexer::has_timeout_retries(&cfg.index_db))
                    {
                        set_phase(&shared, Phase::Finished);
                        if wait(&shared, Some(IDLE_RECHECK)) {
                            break;
                        }
                        continue;
                    }
                }
            }
        }
        {
            let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            st.names_first = None;
            st.names_last = None;
        }
        // After a finished run, a look with no document added, removed or changed reads
        // nothing: no queue rebuild, no writes, no log records. The periodic look runs only for
        // the one retry of a timed-out document (VFS-2 R3).
        let signature = documents_signature(&cfg.index_db);
        let unchanged = finished_at.as_ref().is_some_and(|(sig, at)| {
            Some(sig) == signature.as_ref()
                && (at.elapsed() < IDLE_RECHECK || !indexer::has_timeout_retries(&cfg.index_db))
        });
        if unchanged && !retry {
            set_phase(&shared, Phase::Finished);
            if wait(&shared, Some(IDLE_RECHECK)) {
                break;
            }
            continue;
        }
        // A pause, switch-off or shutdown that came during the checks above wins: re-read
        // under the lock and clear the stop flag only then.
        {
            let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if st.shutdown || !st.settings.enabled || st.settings.paused {
                continue;
            }
            shared.stop_run.store(false, Ordering::SeqCst);
            st.running = true;
        }
        let _ = std::fs::write(&marker, b"");
        // The status changes only once the run knows how much it reads (VFS-50 R4).
        let counts = progress_counts(&cfg.index_db);
        shared.events.diag(&format!(
            "content run start: {} of {} documents",
            counts.0, counts.1
        ));
        let mut gate = AppGate {
            counts,
            silent: false,
            shared: &shared,
            probes: probes.as_mut(),
            pace: &mut pace,
            work_ms: 0,
            run_docs: 0,
            recent: VecDeque::new(),
            failures: Default::default(),
            waiting: None,
            admitted: None,
        };
        let result = indexer::run_with(&cfg.index_db, &cfg.index, &shared.stop_run, &mut gate);
        let (rate, failures, silent) = (gate.per_hour(), gate.failures.clone(), gate.silent);
        drop(gate);
        // A run that returns, finished or stopped, ended cleanly (R11).
        let _ = std::fs::remove_file(&marker);
        // Recount: a document read again (R10) was counted twice in the run.
        let (done, total) = progress_counts(&cfg.index_db);
        {
            let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            st.running = false;
            st.status.done = done;
            st.status.total = total;
        }
        let store_bytes = indexer::stats(&cfg.index_db)
            .map(|s| s.store_bytes)
            .unwrap_or(0);
        match result {
            Ok(sum) => {
                diag_stop(&shared, &sum, rate, &failures, store_bytes);
                if let Some(reason) = sum.stopped {
                    set_phase(
                        &shared,
                        Phase::Stopped {
                            reason,
                            detail: sum.stop_detail.clone().unwrap_or_default(),
                        },
                    );
                    // `index.db` refusing writes is the name indexer's or the disk's, never
                    // the run's: the guard above starts the run again later on its own (R15, VFS-43 R3).
                    if reason == StopReason::IndexWrite {
                        index_retry_at = Some(Instant::now() + cfg.pace.index_write_retry);
                    }
                } else if sum.remaining == 0 && !shared.stop_run.load(Ordering::SeqCst) {
                    finished_at = signature.clone().map(|sig| (sig, Instant::now()));
                    set_phase(&shared, Phase::Finished);
                    // A silent run sent nothing, and its phase was already `finished`: its new
                    // counts go out once (VFS-50 R4).
                    if silent {
                        let status = shared
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .status
                            .clone();
                        shared.events.status(&status);
                    }
                    if wait(&shared, Some(IDLE_RECHECK)) {
                        break;
                    }
                }
            }
            Err(e) => {
                shared.events.diag("content run failed to open the stores");
                set_phase(
                    &shared,
                    Phase::Stopped {
                        reason: StopReason::StoreWrite,
                        detail: e.to_string(),
                    },
                );
                if wait(&shared, None) {
                    break;
                }
            }
        }
    }
    probes
}

fn diag_stop(
    shared: &Shared,
    sum: &RunSummary,
    per_hour: Option<f64>,
    failures: &std::collections::BTreeMap<String, u64>,
    store_bytes: u64,
) {
    let reason = sum.stopped.map(|r| format!("{r:?}")).unwrap_or_else(|| {
        if sum.remaining == 0 {
            "finished".into()
        } else {
            "stopped".into()
        }
    });
    let rate = per_hour
        .map(|r| format!("{r:.0}"))
        .unwrap_or_else(|| "-".into());
    let fails: Vec<String> = failures.iter().map(|(k, n)| format!("{k}={n}")).collect();
    shared.events.diag(&format!(
        "content run stop ({reason}): {} indexed, {} skipped, {} failed, {} left; {rate} documents/hour; failures [{}]; store {store_bytes} bytes; prep_ms={}",
        sum.indexed,
        sum.skipped,
        sum.failed,
        sum.remaining,
        fails.join(", "),
        sum.prep_ms
    ));
}

/// The gate the service runs the engine with: pace (R6, R7), progress (R8), diagnostics.
struct AppGate<'a> {
    shared: &'a Shared,
    /// Documents processed and in the index when the run began, shown once it reads.
    counts: (u64, u64),
    /// A small run: the shown status is left alone until it ends (VFS-50 R4).
    silent: bool,
    probes: &'a mut dyn Probes,
    pace: &'a mut Pace,
    /// Worker time spent on documents in this run, for the rate over working time.
    work_ms: u64,
    run_docs: u64,
    /// Worker time of the last documents, for the time left.
    recent: VecDeque<u64>,
    failures: std::collections::BTreeMap<String, u64>,
    waiting: Option<WaitKind>,
    /// When the document in flight was admitted: its wall time sets the in-use gap (R7).
    admitted: Option<Instant>,
}

impl AppGate<'_> {
    fn per_hour(&self) -> Option<f64> {
        (self.work_ms > 0).then(|| self.run_docs as f64 * 3_600_000.0 / self.work_ms as f64)
    }

    fn seconds_left(&self, left: u64) -> u64 {
        let per_doc = if self.recent.len() >= RATE_WINDOW {
            self.recent.iter().sum::<u64>() / self.recent.len() as u64
        } else {
            ESTIMATE_MS_PER_DOC
        };
        left * per_doc / 1000
    }
}

impl Gate for AppGate<'_> {
    fn before_document(&mut self, path: &Path) -> Admit {
        {
            let st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if st.shutdown || !st.settings.enabled || st.settings.paused {
                return Admit::Stop;
            }
        }
        // Read the probes first; the disk and processor samples share them for the pace call.
        let name_busy = self.probes.name_busy();
        let idle = self.probes.idle();
        let free_bytes = self.probes.free_bytes();
        let probes = std::cell::RefCell::new(&mut *self.probes);
        let mut disk = || probes.borrow_mut().disk_queue(path);
        let mut cpu = || probes.borrow_mut().cpu();
        let decision = self.pace.decide(
            Instant::now(),
            Readings {
                name_busy,
                idle,
                free_bytes,
                disk: &mut disk,
                cpu: &mut cpu,
            },
        );
        match decision {
            Decision::Read => {
                self.admitted = Some(Instant::now());
                if self.waiting.take().is_some() && !self.silent {
                    set_phase(self.shared, Phase::Reading);
                }
                Admit::Read
            }
            // The gap while the PC is in use is part of reading, not a wait (R7 rule 4, D6).
            Decision::Wait(d, WaitReason::InUse) => Admit::Wait(d),
            Decision::Wait(d, reason) => {
                let kind = WaitKind::from(reason);
                if self.waiting != Some(kind) {
                    self.waiting = Some(kind);
                    if !self.silent {
                        set_phase(self.shared, Phase::Waiting { reason: kind });
                    }
                    self.shared
                        .events
                        .diag(&format!("content run waits: {kind:?}"));
                }
                Admit::Wait(d)
            }
        }
    }

    fn run_started(&mut self, queued: u64) {
        // Silent only over a calm status: a stop or a wait on screen gives way to «Leyendo».
        let calm = matches!(
            self.shared
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .status
                .phase,
            Phase::Finished
        );
        self.silent = calm && queued <= self.shared.cfg.pace.silent_max;
        if self.silent {
            return;
        }
        {
            let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            (st.status.done, st.status.total) = self.counts;
        }
        set_phase(self.shared, Phase::Reading);
    }

    fn after_document(&mut self, outcome: OutcomeKind, worker_ms: u64) {
        // A document's time is capped at the engine's limit: time asleep is not work.
        let ms = worker_ms.min(self.shared.cfg.index.worker.timeout.as_millis() as u64);
        let now = Instant::now();
        let took = self
            .admitted
            .take()
            .map_or(Duration::from_millis(ms), |t| now - t);
        self.pace.finished(now, took);
        self.work_ms += ms;
        self.run_docs += 1;
        self.recent.push_back(ms);
        while self.recent.len() > RATE_WINDOW {
            self.recent.pop_front();
        }
        if !matches!(outcome, OutcomeKind::Indexed) {
            *self.failures.entry(outcome.to_string()).or_default() += 1;
        }
        if self.silent {
            return;
        }
        let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        st.status.done = (st.status.done + 1).min(st.status.total);
        let left = st.status.total.saturating_sub(st.status.done);
        st.status.per_hour = self.per_hour();
        st.status.seconds_left = Some(self.seconds_left(left));
        let status = st.status.clone();
        drop(st);
        self.shared.events.status(&status);
    }
}
