//! Tauri commands (VFS-1 contract, `vfs_core::api`). Every command is `async` so none runs on
//! the main thread, and rejects with [`ApiError`]. No command opens a file.

use std::path::PathBuf;

use tauri::ipc::Invoke;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use vfs_core::api::{ApiError, RevealOutcome};
use vfs_core::{IndexProgress, Root, SearchQuery, SearchResponse};

use crate::{reveal, AppState};

type CmdResult<T> = Result<T, ApiError>;

fn io_error(e: impl std::fmt::Display) -> ApiError {
    vfs_core::Error::Io(std::io::Error::other(e.to_string())).into()
}

/// Every command of the contract, for `Builder::invoke_handler`.
pub fn handler<R: Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        system_locale,
        list_roots,
        pick_folder,
        add_root,
        remove_root,
        search,
        reveal,
        get_progress,
        rescan,
        set_paused,
        crate::bookmarks::search_sources,
        crate::bookmarks::get_bookmarks_status,
        crate::bookmarks::set_bookmarks_enabled,
        crate::bookmarks::open_bookmark,
    ]
}

/// The OS locale as a BCP 47 tag ("es-ES"), or "" when unknown (VFS-22 R2). The UI picks
/// its language from it; WKWebView's `navigator.language` follows the bundle, not the OS.
#[tauri::command(async)]
pub fn system_locale() -> String {
    sys_locale::get_locale().unwrap_or_default()
}

#[tauri::command(async)]
pub fn list_roots(state: State<'_, AppState>) -> CmdResult<Vec<Root>> {
    Ok(state.vfs.list_roots()?)
}

/// Native folder dialog; `None` when the user cancels.
#[tauri::command(async)]
pub fn pick_folder<R: Runtime>(app: AppHandle<R>) -> CmdResult<Option<String>> {
    let picked = app
        .dialog()
        .file()
        .set_title("Elegir carpeta")
        .blocking_pick_folder();
    match picked {
        None => Ok(None),
        Some(fp) => {
            let path = fp.into_path().map_err(io_error)?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
    }
}

#[tauri::command(async)]
pub fn add_root(state: State<'_, AppState>, path: String) -> CmdResult<Root> {
    Ok(state.vfs.add_root(&PathBuf::from(path))?)
}

#[tauri::command(async)]
pub fn remove_root(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    Ok(state.vfs.remove_root(id)?)
}

#[tauri::command(async)]
pub fn search(state: State<'_, AppState>, query: SearchQuery) -> CmdResult<SearchResponse> {
    Ok(state.vfs.search(&query)?)
}

/// "Mostrar en carpeta" (VFS-1 AC-13): Finder/Explorer with the file or folder selected.
/// `isDir` selects the id space (`dirs.id` for folder hits, `files.id` otherwise).
#[tauri::command(async)]
pub fn reveal(state: State<'_, AppState>, id: i64, is_dir: bool) -> CmdResult<RevealOutcome> {
    let vfs = &state.vfs;
    let path = if is_dir {
        vfs.dir_path(id)?
    } else {
        vfs.file_path(id)?
    };
    if std::fs::symlink_metadata(&path).is_err() {
        // Gone from disk: have the indexer re-list its folder, and tell the UI.
        if let Err(e) = vfs.mark_dirty(std::slice::from_ref(&path)) {
            log::warn!("mark_dirty {}: {e}", path.display());
        }
        return Err(vfs_core::Error::NotFound(path.to_string_lossy().into_owned()).into());
    }
    reveal::reveal(&path).map_err(io_error)
}

#[tauri::command(async)]
pub fn get_progress(state: State<'_, AppState>) -> CmdResult<IndexProgress> {
    Ok(state.vfs.progress())
}

/// "Buscar cambios ahora" also reads changed Firefox profiles (VFS-6 R2).
#[tauri::command(async)]
pub fn rescan<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: Option<i64>,
) -> CmdResult<()> {
    if id.is_none() {
        if let Some(b) = app.try_state::<crate::bookmarks::BookmarkState>() {
            b.refresh_now();
        }
    }
    Ok(state.vfs.rescan(id)?)
}

#[tauri::command(async)]
pub fn set_paused(state: State<'_, AppState>, paused: bool) -> CmdResult<()> {
    state.vfs.set_paused(paused);
    Ok(())
}
