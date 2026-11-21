//! Tauri commands (VFS-1 contract, `vfs_core::api`). Every command is `async` so none runs on
//! the main thread, and rejects with [`ApiError`]. No command opens a file.

use std::path::PathBuf;

use tauri::ipc::Invoke;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use vfs_core::api::{ApiError, RevealOutcome};
use vfs_core::{IndexProgress, LinkKind, LinkTarget, Root, SearchQuery, SearchResponse};

use crate::{reveal, AppState};

type CmdResult<T> = Result<T, ApiError>;

fn io_error(e: impl std::fmt::Display) -> ApiError {
    vfs_core::Error::Io(std::io::Error::other(e.to_string())).into()
}

/// Every command of the contract, for `Builder::invoke_handler`.
pub fn handler<R: Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        list_roots,
        pick_folder,
        add_root,
        remove_root,
        search,
        reveal,
        reveal_target,
        open_link_url,
        get_progress,
        rescan,
        set_paused,
        crate::bookmarks::search_sources,
        crate::bookmarks::get_bookmarks_status,
        crate::bookmarks::set_bookmarks_enabled,
        crate::bookmarks::open_bookmark,
    ]
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

/// "Mostrar el destino" (VFS-18 R6): Explorer/Finder with the target of a shortcut or link
/// selected. The target is the one the index holds, never resolved or searched for; a
/// missing target (moved, deleted, its disk not connected) is `not_found` and nothing is
/// opened. The first and only access to the target happens here, at the user's request; a
/// UNC target may wait for the network, off the UI thread (the command is async).
#[tauri::command(async)]
pub fn reveal_target(state: State<'_, AppState>, id: i64) -> CmdResult<RevealOutcome> {
    let path = target_to_reveal(id, state.vfs.link_target(id)?)?;
    reveal::reveal(&path).map_err(io_error)
}

/// The path "Mostrar el destino" reveals for link `id`: its recorded target when that is an
/// absolute path that exists now (checked without following it), else `not_found`; a link
/// with no path target (an Internet Shortcut, or none recorded) is `unavailable`.
fn target_to_reveal(id: i64, link: LinkTarget) -> Result<PathBuf, vfs_core::Error> {
    let target = match (link.kind, link.target) {
        (LinkKind::Shortcut | LinkKind::Symlink, Some(t)) => t,
        _ => return Err(vfs_core::Error::Unavailable(format!("link {id}"))),
    };
    let path = PathBuf::from(&target);
    if !path.is_absolute() || std::fs::symlink_metadata(&path).is_err() {
        return Err(vfs_core::Error::NotFound(target));
    }
    Ok(path)
}

/// "Abrir en Firefox" on an Internet Shortcut (VFS-18 R7): its address, `http`/`https` only,
/// through the same path as a bookmark (VFS-6 R10).
#[tauri::command(async)]
pub fn open_link_url(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    let link = state.vfs.link_target(id)?;
    match link.target {
        Some(url) if link.openable => {
            crate::bookmarks::open_in_firefox(&url).map_err(|e| vfs_core::Error::Io(e).into())
        }
        _ => Err(vfs_core::Error::Unavailable(format!("link {id}")).into()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn link(kind: LinkKind, target: Option<&str>) -> LinkTarget {
        LinkTarget {
            kind,
            target: target.map(str::to_owned),
            openable: false,
        }
    }

    /// VFS-18 AC-6: a shortcut or link whose target exists reveals that target; a missing
    /// target or a drive that is not connected is `not_found` (the UI shows the
    /// missing-target message); a link with no path target is never revealed.
    #[test]
    fn reveal_target_checks_the_recorded_target() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Boda 2003.jpg");
        std::fs::write(&file, b"x").unwrap();
        let t = file.to_str().unwrap();
        assert_eq!(
            target_to_reveal(1, link(LinkKind::Shortcut, Some(t))).unwrap(),
            file
        );
        assert_eq!(
            target_to_reveal(
                1,
                link(LinkKind::Symlink, Some(dir.path().to_str().unwrap()))
            )
            .unwrap(),
            dir.path()
        );

        let missing = dir.path().join("borrado.jpg");
        let absent_drive = if cfg!(windows) {
            // A volume id that no disk has: the drive is not connected.
            r"\\?\Volume{00000000-0000-0000-0000-000000000000}\Fotos".to_owned()
        } else {
            "/Volumes/vfs-no-such-disk/Fotos".to_owned()
        };
        for gone in [missing.to_str().unwrap().to_owned(), absent_drive] {
            let err = target_to_reveal(1, link(LinkKind::Shortcut, Some(&gone))).unwrap_err();
            assert_eq!(ApiError::from(err).code, "not_found", "{gone}");
        }

        for no_path in [
            link(LinkKind::Shortcut, None),
            link(LinkKind::Internet, Some("https://example.com")),
            link(LinkKind::Shortcut, Some("Fotos\\relativo.jpg")),
        ] {
            assert!(target_to_reveal(1, no_path).is_err());
        }
    }
}
