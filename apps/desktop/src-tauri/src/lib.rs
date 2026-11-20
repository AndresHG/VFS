//! VFS desktop shell (VFS-1): opens the core index in the per-user app data directory, runs
//! the background indexer, and exposes the command and event contract of `vfs_core::api` to
//! the static frontend in `apps/desktop/ui`.

pub mod commands;
pub mod reveal;

use std::path::Path;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

pub use vfs_core::api::{ApiError, RevealOutcome, EVENT_INDEX_CHANGED, EVENT_INDEX_PROGRESS};
pub use vfs_core::{
    DriveKind, FileKind, IndexProgress, IndexerState, MatchKind, Root, RootProgress, RootStatus,
    SearchFilters, SearchHit, SearchQuery, SearchResponse, TruncatedBy,
};

use vfs_core::{ProgressSink, SearchConfig, Vfs};

/// File name of the index inside the app data directory.
pub const INDEX_FILE: &str = "index.db";

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
    Ok(())
}

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

    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(commands::handler())
        .setup(|app| {
            let handle = app.handle().clone();
            let started = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())
                .and_then(|dir| init_core(&handle, &dir.join(INDEX_FILE)));
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
