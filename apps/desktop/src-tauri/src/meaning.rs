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
const RELEASES: &str = "https://github.com/AndresHG/VFS/releases/download";

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

/// The app's HTTP client for the download (the reqwest already in the binary).
struct HttpFetch(reqwest::blocking::Client);

impl HttpFetch {
    fn new() -> Result<HttpFetch, String> {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(None)
            .build()
            .map(HttpFetch)
            .map_err(|e| e.to_string())
    }
}

impl Fetch for HttpFetch {
    fn get(&self, url: &str) -> std::io::Result<Box<dyn Read + Send>> {
        let r = self
            .0
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(std::io::Error::other)?;
        Ok(Box::new(r))
    }
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
        let fetch = match HttpFetch::new() {
            Ok(f) => f,
            Err(e) => {
                log::warn!("search by meaning: no http client: {e}");
                return;
            }
        };
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
}

/// The offer for an archive of `chunks` chunks and a download of `download_bytes`.
pub fn offer_for(chunks: u64, download_bytes: u64) -> MeaningOffer {
    MeaningOffer {
        download_bytes,
        chunks,
        seconds_fast: chunks * MS_PER_CHUNK_FAST / 1000,
        seconds_slow: chunks * MS_PER_CHUNK_SLOW / 1000,
        ram_bytes: WORKER_RAM_BYTES,
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
pub fn meaning_set_enabled(runs: State<'_, MeaningRuns>, on: bool) -> Result<Status, ApiError> {
    let s = runs.service().ok_or_else(unavailable)?;
    s.set_enabled(on);
    Ok(s.status())
}

#[tauri::command(async)]
pub fn meaning_set_paused(runs: State<'_, MeaningRuns>, paused: bool) -> Result<Status, ApiError> {
    let s = runs.service().ok_or_else(unavailable)?;
    s.set_paused(paused);
    Ok(s.status())
}

/// The «Documentos» tab opened: the vectors and the query model load ahead of the first
/// search (R13: no search waits for them).
#[tauri::command(async)]
pub fn meaning_warm(runs: State<'_, MeaningRuns>) -> Result<(), ApiError> {
    if let Some(s) = runs.service() {
        s.warm();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-29 R1a: the confirmation gives the download size, the time for this archive on a
    /// fast and a slow PC, and the memory needed.
    #[test]
    fn the_offer_scales_with_the_archive() {
        let o = offer_for(540_000, 158_000_000);
        assert_eq!(o.download_bytes, 158_000_000);
        assert!(o.seconds_fast < o.seconds_slow);
        // 540,000 chunks: between about 13 h and 35 h (D16, D18).
        assert!((12 * 3600..15 * 3600).contains(&o.seconds_fast), "{o:?}");
        assert!((30 * 3600..36 * 3600).contains(&o.seconds_slow), "{o:?}");
        assert!(o.ram_bytes > 600 << 20);
        assert_eq!(offer_for(0, 1).seconds_slow, 0);
    }
}
