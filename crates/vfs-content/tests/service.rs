//! The app's background content indexer (VFS-27 27b): the service with fake probes and the
//! real worker binary.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vfs_content::indexer::StopReason;
use vfs_content::pace::PaceConfig;
use vfs_content::service::{
    ContentService, Events, Phase, Probes, ServiceConfig, Status, RUN_MARKER,
};
use vfs_content::{store, IndexOptions};
use vfs_core::Vfs;

/// A machine nobody touches: idle for 10 minutes, plenty of space, no disk probe.
struct Quiet {
    idle: Option<Duration>,
}

impl Probes for Quiet {
    fn name_busy(&self) -> bool {
        false
    }
    fn idle(&self) -> Option<Duration> {
        self.idle
    }
    fn free_bytes(&self) -> Option<u64> {
        Some(100 << 30)
    }
    fn disk_queue(&mut self, _path: &Path) -> Option<f64> {
        None
    }
}

#[derive(Default)]
struct Recorder {
    statuses: Mutex<Vec<Status>>,
    diag: Mutex<Vec<String>>,
}

impl Events for Recorder {
    fn status(&self, status: &Status) {
        self.statuses.lock().unwrap().push(status.clone());
    }
    fn diag(&self, record: &str) {
        self.diag.lock().unwrap().push(record.to_string());
    }
}

struct Setup {
    tmp: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
    data: PathBuf,
    vfs: Vfs,
}

fn setup(files: usize) -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Expedientes");
    let data = tmp.path().join("datos");
    std::fs::create_dir_all(root.join("cartas")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    for i in 0..files {
        std::fs::write(
            root.join(format!("cartas/carta{i:03}.txt")),
            format!("Carta número {i} sobre la cigüeña del campanario."),
        )
        .unwrap();
    }
    let db = data.join("index.db");
    let vfs = Vfs::open(&db).unwrap();
    vfs.add_root(&root).unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    Setup {
        tmp,
        root,
        db,
        data,
        vfs,
    }
}

impl Setup {
    fn config(&self, worker_args: Vec<String>) -> ServiceConfig {
        ServiceConfig {
            index_db: self.db.clone(),
            data_dir: self.data.clone(),
            index: IndexOptions::new(env!("CARGO_BIN_EXE_vfs-extract"), worker_args),
            pace: PaceConfig::default(),
        }
    }

    fn start(&self, cfg: ServiceConfig, idle: Option<Duration>) -> (ContentService, Arc<Recorder>) {
        let rec = Arc::new(Recorder::default());
        let svc = ContentService::start(cfg, Box::new(Quiet { idle }), rec.clone());
        (svc, rec)
    }

    fn openlog(&self) -> PathBuf {
        self.tmp.path().join("opens.log")
    }
}

/// Polls the service until `f` holds, for at most a minute.
fn wait_for(svc: &ContentService, what: &str, f: impl Fn(&Status) -> bool) -> Status {
    let t = Instant::now();
    loop {
        let s = svc.status();
        if f(&s) {
            return s;
        }
        assert!(t.elapsed() < Duration::from_secs(60), "{what}: {s:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

const IDLE: Option<Duration> = Some(Duration::from_secs(600));

/// VFS-27 AC-2 (R2): with content search never switched on, no `content.db` exists, no
/// worker starts and no document is opened.
#[test]
fn off_by_default_creates_nothing() {
    let s = setup(3);
    let log = s.openlog();
    let (svc, _rec) = s.start(
        s.config(vec![format!("--test-hook=openlog:{}", log.display())]),
        IDLE,
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(svc.status().phase, Phase::Off);
    assert!(!svc.status().settings.enabled);
    assert!(!store::exists(&s.db));
    assert!(!log.exists(), "no worker ran");
}

/// VFS-27 AC-8 (R8, R9): paused after some documents and restarted, the run stays paused
/// with its count shown; resumed, it reads only the rest, and its rate counts working time
/// only, not the pause.
#[test]
fn pause_survives_a_restart_and_resumes_with_the_rest() {
    let s = setup(12);
    let mut cfg = s.config(Vec::new());
    // The PC in use, with a short gap, so the run can be paused part way.
    cfg.pace.in_use_gap = Duration::from_millis(400);
    let (svc, _rec) = s.start(cfg.clone(), Some(Duration::from_secs(5)));
    svc.set_enabled(true);
    wait_for(&svc, "three read", |st| st.done >= 3);
    svc.pause();
    let paused = wait_for(&svc, "paused", |st| st.phase == Phase::Paused);
    let done_at_pause = paused.done;
    assert!(done_at_pause < 12, "{paused:?}");
    drop(svc);

    let (svc, rec) = s.start(cfg, IDLE);
    let st = wait_for(&svc, "count shown", |st| st.total > 0);
    assert_eq!(st.phase, Phase::Paused);
    assert!(st.settings.paused && st.settings.enabled);
    assert_eq!((st.done, st.total), (done_at_pause, 12), "{st:?}");
    std::thread::sleep(Duration::from_millis(1500)); // the pause, which must not count
    let resumed = Instant::now();
    svc.resume();
    let fin = wait_for(&svc, "finished", |st| st.phase == Phase::Finished);
    assert_eq!(fin.done, 12, "{fin:?}");
    // The resumed run read only the documents left: its own stop record counts them.
    let left = 12 - done_at_pause;
    let all = rec.diag.lock().unwrap().join("\n");
    assert!(
        all.contains(&format!("content run stop (finished): {left} indexed")),
        "{all}"
    );
    let per_hour = fin.per_hour.expect("a rate once documents were read");
    let wall = resumed.elapsed().as_secs_f64() + 1.5;
    let read = (12 - done_at_pause) as f64;
    assert!(
        per_hour > read * 3600.0 / wall,
        "{per_hour} documents/hour counts the pause"
    );
}

/// VFS-27 AC-9 (R10): after a finished run, a new document found by the name indexer is
/// read and found by content search.
#[test]
fn new_documents_are_read_after_a_finished_run() {
    let s = setup(2);
    let (svc, _rec) = s.start(s.config(Vec::new()), IDLE);
    svc.set_enabled(true);
    wait_for(&svc, "finished", |st| st.phase == Phase::Finished);
    std::fs::write(
        s.root.join("cartas/nueva.txt"),
        "Una carta nueva sobre el olivo del patio.",
    )
    .unwrap();
    let root = PathBuf::from(&s.vfs.list_roots().unwrap()[0].path);
    s.vfs.mark_dirty(&[root.join("cartas")]).unwrap();
    s.vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    svc.notify_names_changed();
    wait_for(&svc, "read the new one", |st| {
        st.done == 3 && st.phase == Phase::Finished
    });
    let hits = vfs_content::Searcher::open(&s.db)
        .unwrap()
        .retrieve(
            &vfs_content::Query::Text("olivo".into()),
            &vfs_core::SearchFilters::default(),
            10,
            0,
        )
        .unwrap();
    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].name, "nueva.txt");
}

/// VFS-27 AC-10 (R11): a store damaged on purpose with the run-open marker present is
/// checked at start and reported damaged; it is deleted only after «Volver a leer los
/// documentos», and name search works meanwhile.
#[test]
fn damaged_store_waits_for_the_user() {
    let s = setup(3);
    let (svc, _rec) = s.start(s.config(Vec::new()), IDLE);
    svc.set_enabled(true);
    wait_for(&svc, "finished", |st| st.phase == Phase::Finished);
    drop(svc);

    // An unclean stop: the marker is left behind, and the store is damaged.
    std::fs::write(s.data.join(RUN_MARKER), b"").unwrap();
    let path = store::store_path(&s.db);
    // Everything into the main file first: pages still in the WAL would hide the damage.
    rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_update(None, "wal_checkpoint", "TRUNCATE")
        .unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    for b in bytes.iter_mut().skip(100).take(4096) {
        *b = 0xA5;
    }
    std::fs::write(&path, &bytes).unwrap();

    let (svc, _rec) = s.start(s.config(Vec::new()), IDLE);
    wait_for(&svc, "damaged", |st| st.phase == Phase::Damaged);
    assert!(
        path.exists(),
        "nothing is deleted without the user's choice"
    );
    let names = s
        .vfs
        .search(&vfs_core::SearchQuery {
            text: "carta".into(),
            filters: vfs_core::SearchFilters::default(),
            limit: 10,
            offset: 0,
            sort: Default::default(),
            scope: Default::default(),
        })
        .unwrap();
    let files = names.hits.iter().filter(|h| !h.is_dir).count();
    assert_eq!(files, 3, "name search works meanwhile");
    svc.rebuild();
    let fin = wait_for(&svc, "read again", |st| st.phase == Phase::Finished);
    assert_eq!(fin.done, 3);
    assert!(!s.data.join(RUN_MARKER).exists());
}

/// VFS-27 AC-14 (R15): with the worker missing, the status shows the worker reason (for
/// «Reintentar») and no document changes state.
#[test]
fn missing_worker_is_a_run_level_reason() {
    let s = setup(2);
    let mut cfg = s.config(Vec::new());
    cfg.index.worker.program = s.tmp.path().join("no-such-worker");
    let (svc, _rec) = s.start(cfg, IDLE);
    svc.set_enabled(true);
    let st = wait_for(&svc, "stopped", |st| {
        matches!(st.phase, Phase::Stopped { .. })
    });
    assert!(
        matches!(
            st.phase,
            Phase::Stopped {
                reason: StopReason::WorkerCannotStart,
                ..
            }
        ),
        "{st:?}"
    );
    let conn = rusqlite::Connection::open(&s.db).unwrap();
    let changed: i64 = conn
        .query_row(
            "SELECT count(*) FROM files WHERE content_state <> 0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(changed, 0);
}

/// VFS-27 AC-15 (R16): the diagnostic records of a run have its start, waits and stop with
/// counts and the rate, and no file or folder name.
#[test]
fn diagnostics_carry_no_names() {
    let s = setup(4);
    let mut cfg = s.config(Vec::new());
    cfg.pace.in_use_gap = Duration::from_millis(200);
    let (svc, rec) = s.start(cfg, Some(Duration::from_secs(5)));
    svc.set_enabled(true);
    wait_for(&svc, "finished", |st| st.phase == Phase::Finished);
    let diag = rec.diag.lock().unwrap().clone();
    let all = diag.join("\n");
    assert!(all.contains("content run start: 0 of 4"), "{all}");
    assert!(all.contains("content run waits: InUse"), "{all}");
    assert!(
        all.contains("content run stop (finished): 4 indexed"),
        "{all}"
    );
    assert!(all.contains("documents/hour"), "{all}");
    assert!(all.contains("store"), "{all}");
    for name in ["Expedientes", "cartas", "carta000", ".txt", "datos"] {
        assert!(!all.contains(name), "«{name}» in the log:\n{all}");
    }
}

/// VFS-27 AC-5 (R5, D4): switching off stops a run that is going and keeps the store; the
/// thread idles with no worker. Deleting what was read returns promptly, removes the store
/// and leaves content search off.
#[test]
fn switch_off_stops_the_run_and_delete_is_prompt() {
    let s = setup(10);
    let mut cfg = s.config(Vec::new());
    cfg.pace.in_use_gap = Duration::from_millis(400);
    let (svc, _rec) = s.start(cfg, Some(Duration::from_secs(5)));
    svc.set_enabled(true);
    wait_for(&svc, "two read", |st| st.done >= 2);
    svc.set_enabled(false);
    let off = wait_for(&svc, "off", |st| st.phase == Phase::Off);
    assert!(off.done < 10, "{off:?}");
    assert!(svc.wait_idle(Duration::from_secs(10)), "the run ended");
    assert!(store::exists(&s.db), "switching off keeps the store");

    let t = Instant::now();
    svc.delete_store().unwrap();
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert!(!store::exists(&s.db));
    let st = svc.status();
    assert!(!st.settings.enabled);
    assert_eq!(st.phase, Phase::Off);
}

/// VFS-27 AC-14 (R15): after a run-level stop, «Reintentar» starts a new run once the cause
/// is gone.
#[test]
fn retry_after_a_run_level_stop() {
    let s = setup(2);
    let mut cfg = s.config(Vec::new());
    let worker = s.tmp.path().join("vfs-extract-copy");
    cfg.index.worker.program = worker.clone();
    let (svc, _rec) = s.start(cfg, IDLE);
    svc.set_enabled(true);
    wait_for(&svc, "stopped", |st| {
        matches!(st.phase, Phase::Stopped { .. })
    });
    std::fs::copy(env!("CARGO_BIN_EXE_vfs-extract"), &worker).unwrap();
    svc.retry();
    let fin = wait_for(&svc, "finished", |st| st.phase == Phase::Finished);
    assert_eq!(fin.done, 2);
}

/// VFS-27 AC-2 (R2): with content search off, the name indexer's change notices write no
/// settings file and start nothing.
#[test]
fn notices_while_off_write_nothing() {
    let s = setup(1);
    let (svc, _rec) = s.start(s.config(Vec::new()), IDLE);
    for _ in 0..5 {
        svc.notify_names_changed();
    }
    std::thread::sleep(Duration::from_millis(200));
    assert!(!s.data.join(vfs_content::service::SETTINGS_FILE).exists());
    assert!(!store::exists(&s.db));
}

/// VFS-27 AC-15 (R16): a user's pause writes a record with its reason.
#[test]
fn user_pause_is_recorded() {
    let s = setup(6);
    let mut cfg = s.config(Vec::new());
    cfg.pace.in_use_gap = Duration::from_millis(400);
    let (svc, rec) = s.start(cfg, Some(Duration::from_secs(5)));
    svc.set_enabled(true);
    wait_for(&svc, "one read", |st| st.done >= 1);
    svc.pause();
    wait_for(&svc, "paused", |st| st.phase == Phase::Paused);
    let all = rec.diag.lock().unwrap().join("\n");
    assert!(all.contains("content run paused by the user"), "{all}");
}

/// VFS-27 AC-9 (R10): after a finished run, the name indexer's notices with nothing new start
/// no run: no queue rebuild, no writes, no log records.
#[test]
fn notices_with_nothing_new_start_no_run() {
    let s = setup(2);
    let (svc, rec) = s.start(s.config(Vec::new()), IDLE);
    svc.set_enabled(true);
    wait_for(&svc, "finished", |st| st.phase == Phase::Finished);
    for _ in 0..3 {
        svc.notify_names_changed();
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(300));
    let starts = rec
        .diag
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.starts_with("content run start"))
        .count();
    assert_eq!(starts, 1);
}

/// VFS-27 AC-3 (R3; VFS-21 R5): after the stop for an update whose install then failed,
/// content indexing goes on and finishes.
#[test]
fn reading_resumes_after_a_failed_update() {
    let s = setup(8);
    let mut cfg = s.config(Vec::new());
    cfg.pace.in_use_gap = Duration::from_millis(300);
    let (svc, _rec) = s.start(cfg, Some(Duration::from_secs(5)));
    svc.set_enabled(true);
    wait_for(&svc, "one read", |st| st.done >= 1);
    assert!(
        svc.shutdown(Duration::from_secs(10)),
        "the update stop ended the run"
    );
    let stopped_at = svc.status().done;
    assert!(stopped_at < 8, "{stopped_at}");
    svc.restart_after_failed_update();
    let fin = wait_for(&svc, "finished", |st| st.phase == Phase::Finished);
    assert_eq!(fin.done, 8);
}

/// VFS-27 AC-10 (R11; VFS-2 R19): a store from another version is reported damaged, not
/// rebuilt behind the user's back, even after a clean stop.
#[test]
fn store_of_another_version_waits_for_the_user() {
    let s = setup(2);
    let (svc, _rec) = s.start(s.config(Vec::new()), IDLE);
    svc.set_enabled(true);
    wait_for(&svc, "finished", |st| st.phase == Phase::Finished);
    drop(svc);
    rusqlite::Connection::open(store::store_path(&s.db))
        .unwrap()
        .pragma_update(None, "user_version", store::STORE_VERSION + 1)
        .unwrap();
    let (svc, _rec) = s.start(s.config(Vec::new()), IDLE);
    wait_for(&svc, "damaged", |st| st.phase == Phase::Damaged);
    assert!(store::exists(&s.db));
    svc.rebuild();
    let fin = wait_for(&svc, "read again", |st| st.phase == Phase::Finished);
    assert_eq!(fin.done, 2);
}

/// VFS-27 AC-10 (R11, R5): a store reported damaged and then deleted with «Borrar lo leído
/// de los documentos» is not reported damaged again: switched back on, reading starts afresh.
#[test]
fn deleted_damaged_store_starts_afresh() {
    let s = setup(2);
    let (svc, _rec) = s.start(s.config(Vec::new()), IDLE);
    svc.set_enabled(true);
    wait_for(&svc, "finished", |st| st.phase == Phase::Finished);
    drop(svc);
    rusqlite::Connection::open(store::store_path(&s.db))
        .unwrap()
        .pragma_update(None, "user_version", store::STORE_VERSION + 1)
        .unwrap();
    let (svc, _rec) = s.start(s.config(Vec::new()), IDLE);
    wait_for(&svc, "damaged", |st| st.phase == Phase::Damaged);
    svc.delete_store().unwrap();
    svc.set_enabled(true);
    let fin = wait_for(&svc, "read afresh", |st| st.phase == Phase::Finished);
    assert_eq!(fin.done, 2);
}

impl Setup {
    /// A second root, `Disco`, with two documents, indexed by name.
    fn second_root(&self) -> (PathBuf, i64) {
        let other = self.tmp.path().join("Disco");
        std::fs::create_dir_all(other.join("b")).unwrap();
        for i in 0..2 {
            std::fs::write(other.join(format!("b/otra{i}.txt")), "el olivo del patio").unwrap();
        }
        let root = self.vfs.add_root(&other).unwrap();
        self.vfs
            .scan_to_completion(&AtomicBool::new(false))
            .unwrap();
        (other, root.id)
    }
}

/// VFS-27 AC-8 (R9): after a finished run with one folder offline, the folder coming back is
/// read with no user action (a notice, or the periodic look).
#[test]
fn offline_folder_is_read_when_it_returns() {
    let s = setup(2);
    let (other, _) = s.second_root();
    let away = s.tmp.path().join("Disco-desconectado");
    std::fs::rename(&other, &away).unwrap();
    let (svc, _rec) = s.start(s.config(Vec::new()), IDLE);
    svc.set_enabled(true);
    let fin = wait_for(&svc, "finished without it", |st| {
        st.phase == Phase::Finished
    });
    assert_eq!((fin.done, fin.total), (2, 4), "{fin:?}");
    std::fs::rename(&away, &other).unwrap();
    svc.notify_names_changed();
    wait_for(&svc, "read once back", |st| {
        st.phase == Phase::Finished && st.done == 4
    });
}

/// VFS-27 AC-8 (R8): after a finished run with a folder paused through VFS-32, resuming it
/// reads its documents.
#[test]
fn paused_folder_is_read_when_resumed() {
    let s = setup(2);
    let (_, root_id) = s.second_root();
    s.vfs.set_root_paused(root_id, true).unwrap();
    let (svc, _rec) = s.start(s.config(Vec::new()), IDLE);
    svc.set_enabled(true);
    let fin = wait_for(&svc, "finished without it", |st| {
        st.phase == Phase::Finished
    });
    assert_eq!((fin.done, fin.total), (2, 4), "{fin:?}");
    s.vfs.set_root_paused(root_id, false).unwrap();
    svc.notify_names_changed();
    wait_for(&svc, "read once resumed", |st| {
        st.phase == Phase::Finished && st.done == 4
    });
}
