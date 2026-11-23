//! Content search in the app (VFS-27): the bundled `vfs-extract` sidecar (R1), the background
//! content indexer (`vfs_content::service`) fed with the machine's probes (R6, R7), its
//! status events and diagnostic records (R8, R16), the commands the interface calls (R5,
//! R8, R11–R15), and the stop before an update (R3). With content search off, the service
//! creates nothing and starts no worker (R2).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use vfs_content::pace::PaceConfig;
use vfs_content::service::{ContentService, Events, Offer, Probes, ServiceConfig, Status};
use vfs_content::{ContentResponse, IndexOptions, Query, Searcher, UnreadReason};
use vfs_core::api::ApiError;
use vfs_core::probes::{CpuSample, CpuSampler, DiskSampler};
use vfs_core::{SearchFilters, Vfs};

/// File name of the extraction worker, as the bundler places the sidecar next to the app's
/// executable (Tauri strips the target triple of `binaries/vfs-extract-<triple>`).
#[cfg(windows)]
pub const WORKER_NAME: &str = "vfs-extract.exe";
#[cfg(not(windows))]
pub const WORKER_NAME: &str = "vfs-extract";

/// How long a stopping run may take to finish the document in flight before its worker is
/// killed (VFS-27 R3). The engine's `stop_grace` is set from it.
pub const STOP_GRACE: Duration = Duration::from_secs(5);

/// How much longer than the grace the app waits for the content thread (kill and reap).
const STOP_SLACK: Duration = Duration::from_secs(2);

/// Event carrying the content status (R8).
pub const EVENT_CONTENT_STATUS: &str = "content-status";

/// Content hits per page (R12).
const PAGE: u32 = 20;

/// Where the worker is for an app executable in `exe_dir`.
pub fn worker_in(exe_dir: &Path) -> PathBuf {
    exe_dir.join(WORKER_NAME)
}

/// The bundled worker next to the running app (R1). `None` only when the app's own path is
/// unknown; a missing file stops the run with "worker cannot start" (VFS-2 R17).
pub fn worker_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(worker_in(exe.parent()?))
}

/// The machine as the pace rules see it (R6, R7).
struct AppProbes {
    vfs: Vfs,
    data_dir: PathBuf,
    /// One disk sampler per drive (the first component of a document's path).
    samplers: HashMap<PathBuf, Option<DiskSampler>>,
    cpu: CpuSampler,
}

impl Probes for AppProbes {
    fn name_busy(&self) -> bool {
        vfs_core::probes::name_indexing_busy(&self.vfs.progress())
    }

    fn idle(&self) -> Option<Duration> {
        vfs_core::probes::idle_time()
    }

    fn free_bytes(&self) -> Option<u64> {
        vfs_core::probes::free_space(&self.data_dir)
    }

    fn disk_queue(&mut self, path: &Path) -> Option<f64> {
        let drive: PathBuf = path
            .components()
            .take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
            .collect();
        self.samplers
            .entry(drive)
            .or_insert_with(|| DiskSampler::for_path(path))
            .as_mut()?
            .queue_length()
    }

    fn cpu(&mut self) -> Option<CpuSample> {
        self.cpu.sample()
    }
}

/// Status events to the window, diagnostic records to the log (R8, R16).
struct AppEvents<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> Events for AppEvents<R> {
    fn status(&self, status: &Status) {
        if let Err(e) = self.app.emit(EVENT_CONTENT_STATUS, status) {
            log::warn!("emit {EVENT_CONTENT_STATUS}: {e}");
        }
    }

    fn diag(&self, record: &str) {
        log::info!(target: vfs_core::DIAG_TARGET, "{record}");
    }
}

/// The app's content service and the index it reads (managed Tauri state); `None` until the
/// index is open.
#[derive(Default)]
pub struct ContentRuns(Mutex<Option<(Arc<ContentService>, PathBuf)>>);

impl ContentRuns {
    /// Starts the service next to the index (`data_dir` holds `index.db`). With content search
    /// off, this creates nothing (R2).
    pub fn start<R: Runtime>(&self, app: &AppHandle<R>, vfs: Vfs, index_db: &Path) {
        let Some(data_dir) = index_db.parent().map(Path::to_path_buf) else {
            return;
        };
        let worker = worker_path().unwrap_or_else(|| PathBuf::from(WORKER_NAME));
        let mut index = IndexOptions::new(worker, Vec::new());
        index.worker.stop_grace = STOP_GRACE;
        let cfg = ServiceConfig {
            index_db: index_db.to_path_buf(),
            data_dir: data_dir.clone(),
            index,
            pace: PaceConfig::default(),
        };
        let probes = AppProbes {
            vfs,
            data_dir,
            samplers: HashMap::new(),
            cpu: CpuSampler::default(),
        };
        let events = Arc::new(AppEvents { app: app.clone() });
        let svc = Arc::new(ContentService::start(cfg, Box::new(probes), events));
        *self.lock() = Some((svc, index_db.to_path_buf()));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(Arc<ContentService>, PathBuf)>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The service, taken out of the lock: a call that waits (delete, stop) never holds it,
    /// so the name indexer's notices do not stall behind it.
    fn service(&self) -> Option<Arc<ContentService>> {
        self.lock().as_ref().map(|(svc, _)| Arc::clone(svc))
    }

    fn with<T>(&self, f: impl FnOnce(&ContentService) -> T) -> Result<T, ApiError> {
        match self.service() {
            Some(svc) => Ok(f(&svc)),
            None => Err(unavailable()),
        }
    }

    /// The name index the service reads; `None` until it is open.
    fn index_db(&self) -> Option<PathBuf> {
        self.lock().as_ref().map(|(_, db)| db.clone())
    }

    /// A searcher over the content store; `Off` when content search was never switched on.
    fn searcher(&self) -> Result<Searcher, vfs_content::Error> {
        match self.index_db() {
            Some(db) => Searcher::open(&db),
            None => Err(vfs_content::Error::Off),
        }
    }

    /// The name indexer changed the index: new or changed documents are read (R10).
    pub fn notify_names_changed(&self) {
        if let Some(svc) = self.service() {
            svc.notify_names_changed();
        }
    }

    /// The update did not install: content indexing goes on (VFS-21 R5, VFS-27 R3).
    pub fn restart_after_failed_update(&self) {
        if let Some(svc) = self.service() {
            svc.restart_after_failed_update();
        }
    }

    /// Stops the content run before the app is replaced (R3): the document in flight gets
    /// [`STOP_GRACE`], then the worker is killed. Returns whether it ended in time.
    pub fn stop_for_update(&self) -> bool {
        match self.service() {
            Some(svc) => {
                let ended = svc.shutdown(STOP_GRACE + STOP_SLACK);
                if !ended {
                    log::warn!("content run still running after the update stop");
                }
                ended
            }
            None => true,
        }
    }
}

fn unavailable() -> ApiError {
    vfs_core::Error::Unavailable("content search".into()).into()
}

fn store_error(e: vfs_content::Error) -> ApiError {
    match e {
        vfs_content::Error::Off => vfs_core::Error::Unavailable("content search is off".into()),
        other => vfs_core::Error::Io(std::io::Error::other(other.to_string())),
    }
    .into()
}

#[tauri::command(async)]
pub fn content_status(runs: State<'_, ContentRuns>) -> Result<Status, ApiError> {
    runs.with(ContentService::status)
}

/// «Buscar también dentro de los documentos» (R5).
#[tauri::command(async)]
pub fn content_set_enabled(runs: State<'_, ContentRuns>, on: bool) -> Result<Status, ApiError> {
    runs.with(|s| {
        s.set_enabled(on);
        s.status()
    })
}

/// «Pausar» / «Reanudar» (R8).
#[tauri::command(async)]
pub fn content_set_paused(runs: State<'_, ContentRuns>, paused: bool) -> Result<Status, ApiError> {
    runs.with(|s| {
        if paused {
            s.pause();
        } else {
            s.resume();
        }
        s.status()
    })
}

/// «Reintentar» after a run-level stop (R15).
#[tauri::command(async)]
pub fn content_retry(runs: State<'_, ContentRuns>) -> Result<Status, ApiError> {
    runs.with(|s| {
        s.retry();
        s.status()
    })
}

/// «Volver a leer los documentos» after damage (R11).
#[tauri::command(async)]
pub fn content_rebuild(runs: State<'_, ContentRuns>) -> Result<Status, ApiError> {
    runs.with(|s| {
        s.rebuild();
        s.status()
    })
}

/// The offer answered «Ahora no» (R4).
#[tauri::command(async)]
pub fn content_mark_offered(runs: State<'_, ContentRuns>) -> Result<Status, ApiError> {
    runs.with(|s| {
        s.mark_offered();
        s.status()
    })
}

/// The offer's numbers (R4, R6): documents, time and disk ranges, and the low-space warning.
#[tauri::command(async)]
pub fn content_offer(runs: State<'_, ContentRuns>) -> Result<Offer, ApiError> {
    let db = runs.index_db().ok_or_else(unavailable)?;
    let free = db.parent().and_then(vfs_core::probes::free_space);
    Ok(vfs_content::service::offer(&db, free))
}

/// «Borrar lo leído de los documentos» (R5).
#[tauri::command(async)]
pub fn content_delete_store(runs: State<'_, ContentRuns>) -> Result<Status, ApiError> {
    runs.with(|s| s.delete_store().map(|_| s.status()))?
        .map_err(store_error)
}

/// One page of content hits (R12): `retrieve` with `offset`, 20 at a time.
#[tauri::command(async)]
pub fn content_search(
    runs: State<'_, ContentRuns>,
    query: String,
    filters: SearchFilters,
    offset: u32,
) -> Result<ContentResponse, ApiError> {
    runs.searcher()
        .map_err(store_error)?
        .retrieve(&Query::Text(query), &filters, PAGE, offset)
        .map_err(store_error)
}

/// Unread counts by reason (R14), the documents not read yet, and the store's size (R5).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadCounts {
    pub by_reason: Vec<(UnreadReason, u64)>,
    pub pending: u64,
    pub store_bytes: u64,
}

#[tauri::command(async)]
pub fn content_unread_counts(runs: State<'_, ContentRuns>) -> Result<UnreadCounts, ApiError> {
    let counts = runs
        .searcher()
        .and_then(|s| s.unread_counts())
        .map_err(store_error)?;
    let status = runs.with(ContentService::status)?;
    let store_bytes = runs
        .index_db()
        .and_then(|db| vfs_content::indexer::stats(&db).ok())
        .map_or(0, |s| s.store_bytes);
    Ok(UnreadCounts {
        by_reason: counts.into_iter().collect(),
        pending: status.total.saturating_sub(status.done),
        store_bytes,
    })
}

/// The unread reason of each name hit on the visible page (R14).
#[tauri::command(async)]
pub fn content_unread(
    runs: State<'_, ContentRuns>,
    file_ids: Vec<i64>,
) -> Result<Vec<(i64, Option<UnreadReason>)>, ApiError> {
    match runs.searcher() {
        Ok(s) => s.unread(&file_ids).map_err(store_error),
        // Content search off: no document carries a reason.
        Err(vfs_content::Error::Off) => Ok(file_ids.into_iter().map(|id| (id, None)).collect()),
        Err(e) => Err(store_error(e)),
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

    /// VFS-27 AC-3: with no service (the index not open yet), the update stop has nothing to
    /// wait for.
    #[test]
    fn stop_for_update_without_a_service() {
        assert!(ContentRuns::default().stop_for_update());
    }
}
