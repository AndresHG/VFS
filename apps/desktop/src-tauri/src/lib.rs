//! VFS desktop shell (VFS-1): opens the core index in the per-user local app data directory, runs
//! the background indexer, and exposes the command and event contract of `vfs_core::api` to
//! the static frontend in `apps/desktop/ui`.

pub mod bookmarks;
pub mod commands;
pub mod reveal;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime, Window, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

pub use vfs_core::api::{ApiError, RevealOutcome, EVENT_INDEX_CHANGED, EVENT_INDEX_PROGRESS};
pub use vfs_core::{
    DriveKind, FileKind, IndexProgress, IndexerState, MatchKind, Root, RootProgress, RootStatus,
    SearchFilters, SearchHit, SearchQuery, SearchResponse, TruncatedBy,
};

use vfs_core::{ProgressSink, SearchConfig, Vfs};

/// File name of the index inside the local app data directory.
pub const INDEX_FILE: &str = "index.db";

/// Moves an index that an earlier build left in `old_dir` (on Windows the roaming app data
/// directory, which can be synced or backed up) to `new_dir`, the local app data directory.
/// Runs only when `old_dir` holds an index and `new_dir` does not, so it happens once. The
/// main file moves first; its WAL, shared memory, session folder and `.bad` copy follow.
/// Returns whether an index was moved. If the main file cannot be renamed (for example a
/// redirected roaming folder on another volume), nothing moves and the error is returned;
/// the caller then builds a new index in `new_dir`.
pub fn move_legacy_index(old_dir: &Path, new_dir: &Path) -> std::io::Result<bool> {
    let old_index = old_dir.join(INDEX_FILE);
    if old_dir == new_dir || !old_index.is_file() || new_dir.join(INDEX_FILE).exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(new_dir)?;
    std::fs::rename(&old_index, new_dir.join(INDEX_FILE))?;
    for entry in std::fs::read_dir(old_dir)? {
        let name = entry?.file_name();
        if name.to_string_lossy().starts_with(INDEX_FILE) {
            std::fs::rename(old_dir.join(&name), new_dir.join(&name))?;
        }
    }
    let _ = std::fs::remove_dir(old_dir); // only succeeds when nothing else is left there
    Ok(true)
}

/// Managed state shared by every command.
pub struct AppState {
    pub vfs: Vfs,
}

/// Forwards indexer progress to the webview as Tauri events.
struct EventSink<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> ProgressSink for EventSink<R> {
    fn on_progress(&self, progress: &IndexProgress) {
        if let Err(e) = self.app.emit(EVENT_INDEX_PROGRESS, progress) {
            log::warn!("emit {EVENT_INDEX_PROGRESS}: {e}");
        }
    }

    fn on_index_changed(&self) {
        if let Err(e) = self.app.emit(EVENT_INDEX_CHANGED, ()) {
            log::warn!("emit {EVENT_INDEX_CHANGED}: {e}");
        }
    }
}

/// Largest log file before it rotates...
const LOG_MAX_BYTES: u128 = 1024 * 1024;
/// ...and old files kept: at most 5 MB, about two weeks of mostly idle use or three days of
/// continuous indexing at one diagnostic record per minute (VFS-5 R7).
const LOG_KEEP: usize = 4;

/// Writes warnings, errors and the indexer's diagnostic records to
/// `<local_data>/logs/vfs.log`, rotating at [`LOG_MAX_BYTES`], so the target PC can be
/// diagnosed afterwards. At info level only the diagnostic target
/// ([`vfs_core::DIAG_TARGET`]) is written, whose records carry counts and durations, never a
/// file name; other info messages could name the user's files in passing (VFS-5 R8).
pub fn install_logger<R: Runtime>(app: &AppHandle<R>, local_data: &Path) -> tauri::Result<()> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
    app.plugin(
        tauri_plugin_log::Builder::new()
            .clear_targets()
            .target(Target::new(TargetKind::Folder {
                path: local_data.join("logs"),
                file_name: Some("vfs".into()),
            }))
            .level(log::LevelFilter::Warn)
            .level_for(vfs_core::DIAG_TARGET, log::LevelFilter::Info)
            .max_file_size(LOG_MAX_BYTES)
            .rotation_strategy(RotationStrategy::KeepSome(LOG_KEEP))
            .build(),
    )
}

/// Opens the index at `db_path` (creating its directory), manages it as [`AppState`] and
/// starts the background indexer. Shared by [`run`] and the integration test.
pub fn init_core<R: Runtime>(app: &AppHandle<R>, db_path: &Path) -> Result<(), String> {
    if let Some(dir) = db_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let vfs = Vfs::open_with(db_path, SearchConfig::default()).map_err(|e| e.to_string())?;
    vfs.start_indexer(Arc::new(EventSink { app: app.clone() }))
        .map_err(|e| e.to_string())?;
    app.manage(AppState { vfs });
    bookmarks::init(app, db_path);
    Ok(())
}

/// What the shell asks of the WebView2 controller when the main window is minimised or
/// restored (VFS-5 R11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebviewPower {
    /// Hide it and ask it to suspend, so Windows can reclaim the renderer's memory.
    Suspend,
    /// Make it visible again; a suspended WebView2 resumes on its own.
    Resume,
}

/// The action for a window that is now `minimized`, given whether it was `suspended`; only
/// Windows (WebView2) suspends, so elsewhere there is never an action.
pub fn webview_power(minimized: bool, suspended: bool) -> Option<WebviewPower> {
    if !cfg!(windows) {
        return None;
    }
    match (minimized, suspended) {
        (true, false) => Some(WebviewPower::Suspend),
        (false, true) => Some(WebviewPower::Resume),
        _ => None,
    }
}

/// Whether the main window's WebView2 was asked to suspend (managed state).
#[derive(Default)]
struct WebviewSuspended(AtomicBool);

/// On a resize, suspends the WebView2 of a minimised window or resumes it once restored.
fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if !matches!(event, WindowEvent::Resized(_)) {
        return;
    }
    let Some(state) = window.try_state::<WebviewSuspended>() else {
        return;
    };
    let minimized = window.is_minimized().unwrap_or(false);
    let Some(action) = webview_power(minimized, state.0.load(Ordering::SeqCst)) else {
        return;
    };
    state
        .0
        .store(action == WebviewPower::Suspend, Ordering::SeqCst);
    if let Some(webview) = window.get_webview_window(window.label()) {
        apply_webview_power(&webview, action);
    }
}

/// `ICoreWebView2Controller::put_IsVisible` and `ICoreWebView2_3::TrySuspend`, as in
/// Microsoft's own sample on `SIZE_MINIMIZED`. Never run on real hardware; failures are
/// logged and harmless (TrySuspend fails while the WebView is visible).
#[cfg(windows)]
fn apply_webview_power<R: Runtime>(webview: &tauri::WebviewWindow<R>, action: WebviewPower) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_3;
    use webview2_com::TrySuspendCompletedHandler;
    use windows_core::Interface;

    let r = webview.with_webview(move |pw| {
        let controller = pw.controller();
        // SAFETY: `with_webview` runs this on the WebView's UI thread, where its COM objects
        // live; every pointer comes from the live controller.
        let r = unsafe {
            match action {
                WebviewPower::Suspend => controller.SetIsVisible(false).and_then(|()| {
                    let core = controller.CoreWebView2()?.cast::<ICoreWebView2_3>()?;
                    let done = TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(())));
                    core.TrySuspend(&done)
                }),
                WebviewPower::Resume => controller.SetIsVisible(true),
            }
        };
        if let Err(e) = r {
            log::warn!("webview {action:?}: {e}");
        }
    });
    if let Err(e) = r {
        log::warn!("webview {action:?}: {e}");
    }
}

#[cfg(not(windows))]
fn apply_webview_power<R: Runtime>(_webview: &tauri::WebviewWindow<R>, _action: WebviewPower) {}

/// Shows a native error and quits: a startup failure must never die silently.
fn fail_startup<R: Runtime>(app: &AppHandle<R>, detail: &str) {
    log::error!("startup failed: {detail}");
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    let handle = app.clone();
    app.dialog()
        .message(format!(
            "No se ha podido iniciar VFS.\n\nDetalle: {detail}\n\n\
             Cierre el programa y vuelva a abrirlo. Si el problema continúa, \
             comuníquelo junto con este mensaje."
        ))
        .title("VFS")
        .kind(MessageDialogKind::Error)
        .show(move |_| handle.exit(1));
}

/// App entry point, called from `main.rs`.
pub fn run() {
    let builder = tauri::Builder::default();

    // Must be the first plugin: a second launch focuses the running window and exits.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }));

    // VFS-8: e2e builds only (`--features webdriver`); listens on TAURI_WEBDRIVER_PORT.
    #[cfg(feature = "webdriver")]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());

    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(commands::handler())
        .on_window_event(on_window_event)
        .setup(|app| {
            app.manage(WebviewSuspended::default());
            let handle = app.handle().clone();
            // The index lives in the local (never roaming) app data directory: a multi-GB
            // file must not be synced to a NAS or backed up with the user's profile.
            let started = app
                .path()
                .app_local_data_dir()
                .map_err(|e| e.to_string())
                .and_then(|dir| {
                    if let Err(e) = install_logger(&handle, &dir) {
                        eprintln!("VFS: log file unavailable: {e}");
                    }
                    if let Ok(old_dir) = app.path().app_data_dir() {
                        if let Err(e) = move_legacy_index(&old_dir, &dir) {
                            log::warn!("index not moved from {}: {e}", old_dir.display());
                        }
                    }
                    init_core(&handle, &dir.join(INDEX_FILE))
                });
            if let Err(detail) = started {
                fail_startup(&handle, &detail);
            }
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            // No window and no event loop exist yet; stderr is all that is left.
            eprintln!("VFS: no se ha podido iniciar: {e}");
            std::process::exit(1);
        }
    };

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            if let Some(state) = app.try_state::<AppState>() {
                state.vfs.shutdown();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-5 AC-12: minimising asks for one suspend and restoring for one resume, on Windows
    /// only; repeated resizes in the same state ask for nothing.
    #[test]
    fn webview_suspends_when_minimised_on_windows() {
        if cfg!(windows) {
            assert_eq!(webview_power(true, false), Some(WebviewPower::Suspend));
            assert_eq!(webview_power(false, true), Some(WebviewPower::Resume));
        } else {
            assert_eq!(webview_power(true, false), None);
            assert_eq!(webview_power(false, true), None);
        }
        assert_eq!(webview_power(true, true), None);
        assert_eq!(webview_power(false, false), None);
    }

    /// VFS-1 C6: an index left in the old (roaming) directory moves with its sibling files and
    /// its session folder, the emptied old directory goes away, and a second call does nothing.
    #[test]
    fn legacy_index_is_moved_once() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("Roaming"), tmp.path().join("Local"));
        std::fs::create_dir_all(old.join("index.db.session")).unwrap();
        for name in ["index.db", "index.db-wal", "index.db-shm"] {
            std::fs::write(old.join(name), name).unwrap();
        }
        assert!(move_legacy_index(&old, &new).unwrap());
        for name in ["index.db", "index.db-wal", "index.db-shm"] {
            assert_eq!(std::fs::read_to_string(new.join(name)).unwrap(), name);
        }
        assert!(new.join("index.db.session").is_dir());
        assert!(!old.exists());
        assert!(!move_legacy_index(&old, &new).unwrap());
    }

    /// VFS-1 C6: an index already in the new directory is never overwritten by an old one,
    /// other files in the old directory stay there, and identical directories move nothing.
    #[test]
    fn existing_index_wins_over_legacy_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("Roaming"), tmp.path().join("Local"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("index.db"), "old").unwrap();
        std::fs::write(old.join("otro.txt"), "x").unwrap();
        std::fs::write(new.join("index.db"), "new").unwrap();
        assert!(!move_legacy_index(&old, &new).unwrap());
        assert_eq!(
            std::fs::read_to_string(new.join("index.db")).unwrap(),
            "new"
        );
        assert!(old.join("index.db").exists());

        std::fs::remove_file(new.join("index.db")).unwrap();
        assert!(move_legacy_index(&old, &new).unwrap());
        assert!(old.join("otro.txt").exists());
        assert!(!move_legacy_index(&old, &old).unwrap());
    }
}
