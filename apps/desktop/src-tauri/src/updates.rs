//! App updates (VFS-21): one check per launch against the GitHub release manifest, the user's
//! "check on open" setting, and the download → verify → install → relaunch flow. This is the
//! app's only network connection; with the setting off nothing here touches the network.

use std::fmt::Display;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::async_runtime::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use vfs_core::api::ApiError;

use crate::AppState;

/// Settings file in the local app-data directory, apart from the index (VFS-21 D3).
pub const SETTINGS_FILE: &str = "updates.json";
/// The check fetches about 1 KB; past this the network counts as unavailable (VFS-21 R4).
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// The download is several MB, possibly over a slow line.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Download progress for the notice: `{downloaded, total}` in bytes, `total` may be null.
pub const EVENT_UPDATE_PROGRESS: &str = "update-progress";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    pub check_on_open: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        UpdateSettings {
            check_on_open: true,
        }
    }
}

/// Reads the settings; a missing or unreadable file means the defaults (check on).
pub fn load_settings(path: &Path) -> UpdateSettings {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes the settings through a temporary file, so a crash never leaves half a file.
pub fn save_settings(path: &Path, settings: &UpdateSettings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec(settings).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path)
}

/// Whether `remote` should be offered to a build running `current`: strictly newer, and a
/// pre-release only to a pre-release build (VFS-21 D7).
pub fn is_newer(current: &Version, remote: &Version) -> bool {
    remote > current && (remote.pre.is_empty() || !current.pre.is_empty())
}

/// The result of the one check per launch (VFS-21 R1). Concurrent callers wait for the first
/// check and share its result; a failed check counts as "no update" until the next launch.
pub struct CheckOnce<T> {
    slot: Mutex<Option<Option<T>>>,
}

impl<T: Clone> Default for CheckOnce<T> {
    fn default() -> Self {
        CheckOnce {
            slot: Mutex::new(None),
        }
    }
}

impl<T: Clone> CheckOnce<T> {
    pub async fn get_or_check<F, Fut, E>(&self, fetch: F) -> Option<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Option<T>, E>>,
        E: Display,
    {
        let mut slot = self.slot.lock().await;
        if let Some(done) = slot.as_ref() {
            return done.clone();
        }
        let found = fetch().await.unwrap_or_else(|e| {
            log::warn!("update check failed: {e}");
            None
        });
        *slot = Some(found.clone());
        found
    }

    /// The update the check found, if it ran and found one.
    pub async fn found(&self) -> Option<T> {
        self.slot.lock().await.clone().flatten()
    }
}

/// Download, then `install`, then `after_install`; `recover` runs when `install` fails
/// (VFS-21 R3, R5, R8). `download` verifies the signature (the updater plugin checks it once the
/// bytes arrive), so a failed download or a bad signature returns before anything is installed
/// or stopped. Windows: the indexer is stopped inside `install` (the updater's `on_before_exit` hook)
/// and on success the process exits for the installer. macOS: `install` replaces the bundle
/// while the indexer runs, then `after_install` stops it and relaunches.
pub async fn install_flow<D, DFut, I, A, F, E>(
    download: D,
    install: I,
    after_install: A,
    recover: F,
) -> Result<(), E>
where
    D: FnOnce() -> DFut,
    DFut: Future<Output = Result<Vec<u8>, E>>,
    I: FnOnce(&[u8]) -> Result<(), E>,
    A: FnOnce(),
    F: FnOnce(),
{
    let bytes = download().await?;
    if let Err(e) = install(&bytes) {
        recover();
        return Err(e);
    }
    after_install();
    Ok(())
}

/// Managed state for the update commands.
pub struct UpdateState {
    settings_path: PathBuf,
    check: CheckOnce<tauri_plugin_updater::Update>,
    /// Set by the Windows `on_before_exit` hook once the indexer has been stopped.
    index_stopped: Arc<AtomicBool>,
}

impl UpdateState {
    pub fn new(local_data: &Path) -> UpdateState {
        UpdateState {
            settings_path: local_data.join(SETTINGS_FILE),
            check: CheckOnce::default(),
            index_stopped: Arc::default(),
        }
    }
}

/// After a failed install: if the indexer was already stopped (the Windows installer could
/// not start), relaunch the current version so the index session and the window come back
/// whole; otherwise nothing was touched and the app simply carries on (VFS-21 R5).
pub fn recover_after_failed_install(index_stopped: &AtomicBool, relaunch: impl FnOnce()) {
    if index_stopped.load(Ordering::SeqCst) {
        relaunch();
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
}

fn io_error(e: impl Display) -> ApiError {
    vfs_core::Error::Io(std::io::Error::other(e.to_string())).into()
}

/// Stops the indexer so the index is consistent before the app is replaced (VFS-21 R8).
fn shutdown_index<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<AppState>() {
        state.vfs.shutdown();
    }
}

async fn fetch_update<R: Runtime>(
    app: &AppHandle<R>,
    index_stopped: Arc<AtomicBool>,
) -> tauri_plugin_updater::Result<Option<tauri_plugin_updater::Update>> {
    use tauri_plugin_updater::UpdaterExt;
    let exiting = app.clone();
    app.updater_builder()
        .version_comparator(|current, release| is_newer(&current, &release.version))
        .timeout(CHECK_TIMEOUT)
        // Replaces the plugin's default hook (`cleanup_before_exit`, which hides the windows):
        // if the installer then fails to start, the app must still be usable.
        .on_before_exit(move || {
            shutdown_index(&exiting);
            index_stopped.store(true, Ordering::SeqCst);
        })
        .build()?
        .check()
        .await
}

#[tauri::command(async)]
pub fn get_update_settings(state: State<'_, UpdateState>) -> UpdateSettings {
    load_settings(&state.settings_path)
}

#[tauri::command(async)]
pub fn set_update_check(
    state: State<'_, UpdateState>,
    enabled: bool,
) -> Result<UpdateSettings, ApiError> {
    let settings = UpdateSettings {
        check_on_open: enabled,
    };
    save_settings(&state.settings_path, &settings).map_err(io_error)?;
    Ok(settings)
}

/// The UI asks once after load; with the setting off this returns `None` without any
/// connection (VFS-21 R6), and later calls in the same launch reuse the first answer.
#[tauri::command]
pub async fn check_for_update<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, UpdateState>,
) -> Result<Option<UpdateInfo>, ApiError> {
    if !load_settings(&state.settings_path).check_on_open {
        return Ok(None);
    }
    let stopped = state.index_stopped.clone();
    let found = state
        .check
        .get_or_check(|| fetch_update(&app, stopped))
        .await;
    Ok(found.map(|u| UpdateInfo { version: u.version }))
}

/// "Actualizar": download with progress events, verify, install and relaunch. Rejects, with
/// nothing installed, when the download, the signature or the install step fails (VFS-21 R5).
#[tauri::command]
pub async fn install_update<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, UpdateState>,
) -> Result<(), ApiError> {
    let Some(mut update) = state.check.found().await else {
        return Err(vfs_core::Error::NotFound("update".into()).into());
    };
    update.timeout = Some(DOWNLOAD_TIMEOUT);
    let events = app.clone();
    let finished = app.clone();
    let counter = AtomicU64::new(0);
    let downloaded = &counter;
    let mut last_percent = None;
    let download = || {
        update.download(
            move |chunk, total| {
                let downloaded =
                    downloaded.fetch_add(chunk as u64, Ordering::Relaxed) + chunk as u64;
                // With a known size, one event per percent at most (a 10 MB download comes in
                // ~1000 chunks); without one, every chunk.
                let percent = total.map(|t| downloaded * 100 / t.max(1));
                if percent.is_none() || percent != last_percent {
                    last_percent = percent;
                    let _ =
                        events.emit(EVENT_UPDATE_PROGRESS, UpdateProgress { downloaded, total });
                }
            },
            // Also when the server sent no size: the UI switches to "Instalando…" on this.
            move || {
                let downloaded = downloaded.load(Ordering::Relaxed);
                let total = Some(downloaded);
                let _ = finished.emit(EVENT_UPDATE_PROGRESS, UpdateProgress { downloaded, total });
            },
        )
    };
    let result = install_flow(
        download,
        |bytes| update.install(bytes),
        || {
            // macOS: the bundle was replaced in place; relaunch on the new version.
            shutdown_index(&app);
            app.restart();
        },
        || recover_after_failed_install(&state.index_stopped, || app.restart()),
    )
    .await;
    result.map_err(|e| {
        log::warn!("update install failed: {e}");
        io_error(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::AtomicUsize;

    fn v(s: &str) -> Version {
        Version::parse(s.trim_start_matches('v')).unwrap()
    }

    /// VFS-21 AC-1: only a strictly newer version is offered.
    #[test]
    fn newer_versions_only() {
        assert!(is_newer(&v("0.2.0"), &v("0.2.1")));
        assert!(is_newer(&v("0.2.0"), &v("v0.2.1")));
        assert!(is_newer(&v("0.2.0"), &v("1.0.0")));
        assert!(!is_newer(&v("0.2.0"), &v("0.2.0")));
        assert!(!is_newer(&v("0.2.0"), &v("0.1.9")));
    }

    /// VFS-21 AC-2: a stable build is never offered a pre-release; a pre-release build is
    /// offered the stable release and later pre-releases.
    #[test]
    fn prereleases_only_for_prerelease_builds() {
        assert!(!is_newer(&v("0.2.0"), &v("0.3.0-beta.1")));
        assert!(is_newer(&v("0.3.0-beta.1"), &v("0.3.0")));
        assert!(is_newer(&v("0.3.0-beta.1"), &v("0.3.0-beta.2")));
        assert!(!is_newer(&v("0.3.0"), &v("0.3.0-beta.2")));
    }

    /// VFS-21 AC-3: on by default, saved choice survives a restart, a corrupt file means on.
    #[test]
    fn setting_defaults_persists_and_survives_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join(SETTINGS_FILE);
        assert!(load_settings(&path).check_on_open);

        save_settings(
            &path,
            &UpdateSettings {
                check_on_open: false,
            },
        )
        .unwrap();
        assert!(!load_settings(&path).check_on_open);

        std::fs::write(&path, b"{not json").unwrap();
        assert!(load_settings(&path).check_on_open);
        save_settings(
            &path,
            &UpdateSettings {
                check_on_open: false,
            },
        )
        .unwrap();
        assert!(!load_settings(&path).check_on_open);
    }

    /// VFS-21 AC-5: the second ask in a launch reuses the first answer, no second request.
    #[test]
    fn check_runs_once_per_launch() {
        let calls = AtomicUsize::new(0);
        let once = CheckOnce::<String>::default();
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(Some("0.2.1".to_string()))
        };
        tauri::async_runtime::block_on(async {
            assert_eq!(once.get_or_check(fetch).await.as_deref(), Some("0.2.1"));
            assert_eq!(once.get_or_check(fetch).await.as_deref(), Some("0.2.1"));
            assert_eq!(once.found().await.as_deref(), Some("0.2.1"));
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// VFS-21 AC-6: an unreachable endpoint or a bad manifest is "no update", and is not
    /// retried in the same launch.
    #[test]
    fn failed_check_is_no_update_until_next_launch() {
        let calls = AtomicUsize::new(0);
        let once = CheckOnce::<String>::default();
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err::<Option<String>, _>("network unreachable")
        };
        tauri::async_runtime::block_on(async {
            assert_eq!(once.get_or_check(fetch).await, None);
            assert_eq!(once.get_or_check(fetch).await, None);
            assert_eq!(once.found().await, None);
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    fn run_flow(download_ok: bool, install_ok: bool) -> (Result<(), String>, Vec<&'static str>) {
        let steps = RefCell::new(Vec::new());
        let result = tauri::async_runtime::block_on(install_flow(
            || async {
                steps.borrow_mut().push("download");
                if download_ok {
                    Ok(vec![1, 2, 3])
                } else {
                    Err("connection reset".to_string())
                }
            },
            |bytes: &[u8]| {
                assert_eq!(bytes, [1, 2, 3]);
                steps.borrow_mut().push("install");
                if install_ok {
                    Ok(())
                } else {
                    Err("signature mismatch".to_string())
                }
            },
            || steps.borrow_mut().push("shutdown+relaunch"),
            || steps.borrow_mut().push("recover"),
        ));
        (result, steps.into_inner())
    }

    /// VFS-21 AC-7: a failed download installs nothing and leaves the index running (no
    /// shutdown, no relaunch).
    #[test]
    fn failed_download_installs_nothing_and_keeps_running() {
        let (result, steps) = run_flow(false, true);
        assert!(result.is_err());
        assert_eq!(steps, ["download"]);
    }

    /// VFS-21 AC-12: a bad signature or an installer that cannot run rejects, skips the
    /// post-install relaunch and runs the recovery step.
    #[test]
    fn failed_install_runs_recovery() {
        let (result, steps) = run_flow(true, false);
        assert_eq!(result.unwrap_err(), "signature mismatch");
        assert_eq!(steps, ["download", "install", "recover"]);
    }

    /// VFS-21 AC-12: recovery relaunches the current version only when the indexer was
    /// already stopped (Windows installer failed to start); otherwise the app carries on.
    #[test]
    fn recovery_relaunches_only_after_the_index_stopped() {
        let relaunched = AtomicUsize::new(0);
        let relaunch = || {
            relaunched.fetch_add(1, Ordering::SeqCst);
        };
        recover_after_failed_install(&AtomicBool::new(false), relaunch);
        assert_eq!(relaunched.load(Ordering::SeqCst), 0);
        recover_after_failed_install(&AtomicBool::new(true), relaunch);
        assert_eq!(relaunched.load(Ordering::SeqCst), 1);
    }

    /// VFS-21 AC-8: the index is shut down only after the update is installed and before the
    /// relaunch.
    #[test]
    fn index_stops_after_install_before_relaunch() {
        let (result, steps) = run_flow(true, true);
        assert!(result.is_ok());
        assert_eq!(steps, ["download", "install", "shutdown+relaunch"]);
    }
}
