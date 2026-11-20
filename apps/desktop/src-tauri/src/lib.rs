//! VFS desktop shell (VFS-1): opens the core index in the per-user local app data directory, runs
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

/// Largest log file before it rotates; two old files are kept, so logs stay under ~3 MB.
const LOG_MAX_BYTES: u128 = 1024 * 1024;

/// Writes warnings and errors to `<local_data>/logs/vfs.log`, rotating at
/// [`LOG_MAX_BYTES`], so a problem on the target PC can be diagnosed afterwards. Only
/// warnings and errors are written; nothing at info level or below, where messages could
/// name the user's files in passing.
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
            .max_file_size(LOG_MAX_BYTES)
            .rotation_strategy(RotationStrategy::KeepSome(2))
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

    // VFS-5: e2e builds only (`--features webdriver`); listens on TAURI_WEBDRIVER_PORT.
    #[cfg(feature = "webdriver")]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());

    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(commands::handler())
        .setup(|app| {
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
