//! Search by meaning in the app (VFS-29 R1, R1a, R4, R7, R11–R16): the service from
//! `vfs-semantic` started next to the index, the HTTP fetcher its download uses, the
//! numbers the confirmation shows (download size, time for this archive, memory), its status
//! events, the commands the interface calls, and the ranker every content search and
//! question fuses with BM25. With the switch off it creates nothing and starts no thread.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use vfs_core::api::ApiError;
use vfs_core::Vfs;
use vfs_semantic::client::{WorkerConfig, DEFAULT_MEM_CAP};
use vfs_semantic::install::{self, Fetch};
use vfs_semantic::search::{MeaningConfig, IDLE_RELEASE, QUERY_TIMEOUT};
use vfs_semantic::service::{self, MeaningService, ServiceConfig, Status};
use vfs_semantic::LOCAL_MODEL;

/// Event carrying the service status.
pub const EVENT_MEANING_STATUS: &str = "meaning-status";

/// Where this version's release assets are (the worker, D21).
pub const RELEASES: &str = "https://github.com/AndresHG/VFS/releases/download";

/// Threads the background embedding uses (29b measured 2; the rest of the PC stays usable).
const EMBED_THREADS: usize = 2;
/// Wall clock per batch of the background run.
const BATCH_TIMEOUT: Duration = Duration::from_secs(300);

/// Time per chunk at two threads: measured on the development Mac (29c, D18), and the upper
/// estimate for the slow target PC (D16), for the range the confirmation shows.
const MS_PER_CHUNK_FAST: u64 = 90;
const MS_PER_CHUNK_SLOW: u64 = 230;
/// Memory the worker needs while embedding (0.64–0.70 GB measured, D18).
const WORKER_RAM_BYTES: u64 = 700 * 1024 * 1024;
/// Memory the query model needs while «Documentos» or a chat is in use (0.57 GB after load,
/// measured in 29c), released after [`IDLE_RELEASE`] without a search.
const SEARCH_RAM_BYTES: u64 = 570 * 1024 * 1024;

/// The app's HTTP client for the download (the reqwest already in the binary), built on the
/// first request: a blocking client starts a thread, which must not exist while search by
/// meaning is off (R15).
#[derive(Default)]
struct HttpFetch {
    client: std::sync::OnceLock<Result<reqwest::blocking::Client, String>>,
}

impl HttpFetch {
    fn client(&self) -> std::io::Result<&reqwest::blocking::Client> {
        self.client
            .get_or_init(|| {
                if rustls::crypto::CryptoProvider::get_default().is_none() {
                    let _ = rustls::crypto::ring::default_provider().install_default();
                }
                reqwest::blocking::Client::builder()
                    .connect_timeout(Duration::from_secs(20))
                    .timeout(None)
                    .build()
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(|e| std::io::Error::other(e.clone()))
    }
}

impl Fetch for HttpFetch {
    fn get(&self, url: &str) -> std::io::Result<Box<dyn Read + Send>> {
        let r = self
            .client()?
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(std::io::Error::other)?;
        Ok(Box::new(r))
    }
}

/// Whether a content status means new text may have been read: a finished content run (R4).
pub fn content_status_reports_text(status: &vfs_content::service::Status) -> bool {
    status.phase == vfs_content::service::Phase::Finished
}

/// The app's search by meaning and the index it serves (managed Tauri state).
#[derive(Default)]
pub struct MeaningRuns(Mutex<Option<(Arc<MeaningService>, PathBuf)>>);

impl MeaningRuns {
    /// Starts the service next to the index; with the switch off it starts no thread.
    pub fn start<R: Runtime>(&self, app: &AppHandle<R>, vfs: Vfs, index_db: &Path) {
        let Some(data_dir) = index_db.parent().map(Path::to_path_buf) else {
            return;
        };
        let fetch = HttpFetch::default();
        let model_dir = service::model_dir(&data_dir);
        let worker = WorkerConfig {
            program: model_dir.join(install::worker_file()),
            args: Vec::new(),
            model_dir,
            model: LOCAL_MODEL,
            threads: EMBED_THREADS,
            foreground: false,
            mem_cap: DEFAULT_MEM_CAP,
            timeout: BATCH_TIMEOUT,
        };
        let query = MeaningConfig {
            worker: WorkerConfig {
                threads: 1,
                foreground: true,
                timeout: QUERY_TIMEOUT,
                ..worker.clone()
            },
            idle: IDLE_RELEASE,
        };
        let version = app.package_info().version.to_string();
        let base = format!("{RELEASES}/v{version}/");
        let cfg = ServiceConfig {
            index_db: index_db.to_path_buf(),
            data_dir: data_dir.clone(),
            downloads: install::local_model(&base, &updater_pubkey(app)),
            worker,
            query,
            pace: vfs_content::pace::PaceConfig::default(),
            change_quiet: service::CHANGE_QUIET,
            retry_first: service::RETRY_FIRST,
        };
        let emitter = app.clone();
        let svc = MeaningService::start(
            cfg,
            Box::new(fetch),
            crate::content::app_probes(vfs, data_dir),
            Box::new(move |status: &Status| {
                if let Err(e) = emitter.emit(EVENT_MEANING_STATUS, status) {
                    log::warn!("emit {EVENT_MEANING_STATUS}: {e}");
                }
            }),
        );
        *self.lock() = Some((Arc::new(svc), index_db.to_path_buf()));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(Arc<MeaningService>, PathBuf)>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn service(&self) -> Option<Arc<MeaningService>> {
        self.lock().as_ref().map(|(s, _)| Arc::clone(s))
    }

    fn index_db(&self) -> Option<PathBuf> {
        self.lock().as_ref().map(|(_, db)| db.clone())
    }

    /// The ranker content searches and questions fuse with BM25 (R11), while the switch is on
    /// and a complete vector set exists.
    pub fn ranker(&self) -> Option<Arc<dyn vfs_content::Meaning>> {
        self.service()?.ranker()
    }

    /// Content indexing read new or changed documents (R4).
    pub fn notify_content_changed(&self) {
        if let Some(s) = self.service() {
            s.notify_content_changed();
        }
    }

    /// The update did not install: search by meaning goes on.
    pub fn restart_after_failed_update(&self) {
        if let Some(s) = self.service() {
            s.restart_after_failed_update();
        }
    }

    /// Ends the thread and the workers before the app is replaced or closes.
    pub fn stop_for_update(&self) {
        if let Some(s) = self.service() {
            if !s.shutdown(crate::content::STOP_GRACE) {
                log::warn!("search by meaning still running after the stop");
            }
        }
    }
}

/// The searcher of `index_db` with the ranker attached when search by meaning serves (R11,
/// D15): every caller of `retrieve`, the «Documentos» tab and questions alike, uses this.
pub fn searcher<R: Runtime>(
    app: &AppHandle<R>,
    index_db: &Path,
) -> Result<vfs_content::Searcher, vfs_content::Error> {
    let s = vfs_content::Searcher::open(index_db)?;
    Ok(
        match app.try_state::<MeaningRuns>().and_then(|m| m.ranker()) {
            Some(r) => s.with_meaning(r),
            None => s,
        },
    )
}

/// The updater's public key from `tauri.conf.json`, which also signs the worker (D21).
fn updater_pubkey<R: Runtime>(app: &AppHandle<R>) -> String {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .unwrap_or_default()
        .to_owned()
}

fn unavailable() -> ApiError {
    vfs_core::Error::Unavailable("search by meaning".into()).into()
}

/// What the confirmation shows before anything is downloaded (R1a).
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeaningOffer {
    pub download_bytes: u64,
    /// Chunks of this archive's read documents.
    pub chunks: u64,
    /// Time to embed them, on a fast PC and on a slow one.
    pub seconds_fast: u64,
    pub seconds_slow: u64,
    pub ram_bytes: u64,
    /// Memory while searching, and the minutes without a search after which it is released.
    pub search_ram_bytes: u64,
    pub idle_minutes: u64,
}

/// The offer for an archive of `chunks` chunks and a download of `download_bytes`.
pub fn offer_for(chunks: u64, download_bytes: u64) -> MeaningOffer {
    MeaningOffer {
        download_bytes,
        chunks,
        seconds_fast: chunks * MS_PER_CHUNK_FAST / 1000,
        seconds_slow: chunks * MS_PER_CHUNK_SLOW / 1000,
        ram_bytes: WORKER_RAM_BYTES,
        search_ram_bytes: SEARCH_RAM_BYTES,
        idle_minutes: IDLE_RELEASE.as_secs() / 60,
    }
}

#[tauri::command(async)]
pub fn meaning_status(runs: State<'_, MeaningRuns>) -> Result<Status, ApiError> {
    Ok(runs.service().ok_or_else(unavailable)?.status())
}

/// The numbers of the confirmation (R1a).
#[tauri::command(async)]
pub fn meaning_offer<R: Runtime>(
    app: AppHandle<R>,
    runs: State<'_, MeaningRuns>,
) -> Result<MeaningOffer, ApiError> {
    let db = runs.index_db().ok_or_else(unavailable)?;
    let chunks = vfs_content::ChunkReader::open(&db)
        .and_then(|r| r.chunk_count())
        .unwrap_or(0);
    let version = app.package_info().version.to_string();
    let downloads = install::local_model(&format!("{RELEASES}/v{version}/"), "");
    Ok(offer_for(chunks, install::download_bytes(&downloads)))
}

/// «Búsqueda por significado» (R1): on, after the confirmation, downloads and embeds; off,
/// stops and releases (it may wait a few seconds for the run, so never on the main thread).
#[tauri::command(async)]
pub fn meaning_set_enabled(
    runs: State<'_, MeaningRuns>,
    content: State<'_, crate::content::ContentRuns>,
    on: bool,
) -> Result<Status, ApiError> {
    let s = runs.service().ok_or_else(unavailable)?;
    // R1: available only while content search is on.
    if on && !content.enabled() {
        return Err(vfs_core::Error::Unavailable("content search is off".into()).into());
    }
    s.set_enabled(on);
    Ok(s.status())
}

/// Content search was switched off: search by meaning, which needs it, goes off too (R1).
pub fn content_switched_off<R: Runtime>(app: &AppHandle<R>) {
    if let Some(s) = app.try_state::<MeaningRuns>().and_then(|m| m.service()) {
        if s.status().settings.enabled {
            s.set_enabled(false);
        }
    }
}

#[tauri::command(async)]
pub fn meaning_set_paused(runs: State<'_, MeaningRuns>, paused: bool) -> Result<Status, ApiError> {
    let s = runs.service().ok_or_else(unavailable)?;
    s.set_paused(paused);
    Ok(s.status())
}

/// The «Documentos» tab opened: the vectors and the query model load ahead of the first
/// search, which otherwise waits for them up to the query limit (R13).
#[tauri::command(async)]
pub fn meaning_warm(runs: State<'_, MeaningRuns>) -> Result<(), ApiError> {
    if let Some(s) = runs.service() {
        s.warm();
    }
    Ok(())
}

/// «Relacionados» (VFS-53 R4–R7): the documents closest to `file_id` by meaning, as
/// document hits with their closest passage; computed now, nothing kept. Unavailable while
/// search by meaning does not serve (R4, R9: then `vectors.db` is not even opened).
#[tauri::command(async)]
pub fn related_documents(
    runs: State<'_, MeaningRuns>,
    file_id: i64,
) -> Result<Vec<vfs_content::ContentHit>, ApiError> {
    if runs.ranker().is_none() {
        return Err(unavailable());
    }
    let db = runs.index_db().ok_or_else(unavailable)?;
    let io = |e: String| ApiError::from(vfs_core::Error::Io(std::io::Error::other(e)));
    let related = vfs_semantic::related::related(&db, file_id, vfs_semantic::related::MAX_RELATED)
        .map_err(|e| io(e.to_string()))?;
    let triples: Vec<(i64, u32, f32)> = related
        .iter()
        .map(|r| (r.file_id, r.ordinal, r.score))
        .collect();
    vfs_content::Searcher::open(&db)
        .and_then(|s| s.related_hits(&triples))
        .map_err(|e| io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-29 AC-1 (R15): the download's HTTP client, and the thread it starts, exist only
    /// after the first request.
    #[test]
    fn the_http_client_waits_for_the_first_request() {
        let f = HttpFetch::default();
        assert!(f.client.get().is_none());
    }

    /// VFS-29 AC-4 (R4): a finished content run reports new text; a run in progress does not.
    #[test]
    fn a_finished_content_run_reports_text() {
        use vfs_content::service::{Phase, Settings, Status as C};
        let st = |phase| C {
            settings: Settings::default(),
            phase,
            done: 1,
            total: 1,
            per_hour: None,
            seconds_left: None,
        };
        assert!(content_status_reports_text(&st(Phase::Finished)));
        assert!(!content_status_reports_text(&st(Phase::Reading)));
        assert!(!content_status_reports_text(&st(Phase::Off)));
    }

    /// VFS-29 R1a: the confirmation gives the download size, the time for this archive on a
    /// fast and a slow PC, the memory needed while preparing and while searching, and when the
    /// search memory is released (R15).
    #[test]
    fn the_offer_scales_with_the_archive() {
        let o = offer_for(540_000, 158_000_000);
        assert_eq!(o.download_bytes, 158_000_000);
        assert!(o.seconds_fast < o.seconds_slow);
        // 540,000 chunks: between about 13 h and 35 h (D16, D18).
        assert!((12 * 3600..15 * 3600).contains(&o.seconds_fast), "{o:?}");
        assert!((30 * 3600..36 * 3600).contains(&o.seconds_slow), "{o:?}");
        assert!(o.ram_bytes > 600 << 20);
        // The memory while searching and when it is released (VFS-29 R15).
        assert!(o.search_ram_bytes > 500 << 20 && o.search_ram_bytes < o.ram_bytes);
        assert_eq!(o.idle_minutes, IDLE_RELEASE.as_secs() / 60);
        assert_eq!(offer_for(0, 1).seconds_slow, 0);
    }
}
