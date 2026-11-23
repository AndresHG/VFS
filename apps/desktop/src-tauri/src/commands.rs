//! Tauri commands (VFS-1 contract, `vfs_core::api`). Every command is `async` so none runs on
//! the main thread, and rejects with [`ApiError`]. No command opens a file.

use std::path::PathBuf;

use tauri::ipc::Invoke;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use vfs_core::api::{ApiError, RevealOutcome};
use vfs_core::{
    IndexProgress, LinkKind, LinkTarget, Root, SearchQuery, SearchResponse, Suggestion,
    UnreadableDir,
};

use crate::{reveal, AppState};

type CmdResult<T> = Result<T, ApiError>;

fn io_error(e: impl std::fmt::Display) -> ApiError {
    vfs_core::Error::Io(std::io::Error::other(e.to_string())).into()
}

/// Every command of the contract, for `Builder::invoke_handler`.
pub fn handler<R: Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        system_locale,
        set_language,
        wait_core,
        list_roots,
        list_unreadable,
        pick_folder,
        add_root,
        remove_root,
        set_root_paused,
        search,
        suggest,
        reveal,
        reveal_target,
        open_link_url,
        get_progress,
        rescan,
        set_paused,
        get_include_all_folders,
        set_include_all_folders,
        get_file_exclusions,
        count_file_exclusion,
        add_file_exclusion,
        remove_file_exclusion,
        reset_file_exclusions,
        get_content_max_mb,
        set_content_max_mb,
        crate::bookmarks::search_sources,
        crate::bookmarks::get_bookmarks_status,
        crate::bookmarks::set_bookmarks_enabled,
        crate::bookmarks::open_bookmark,
        crate::updates::get_update_settings,
        crate::updates::set_update_check,
        crate::updates::check_for_update,
        crate::updates::install_update,
        crate::feedback::feedback_report,
        crate::feedback::feedback_save,
        crate::content::content_status,
        crate::content::content_set_enabled,
        crate::content::content_set_paused,
        crate::content::content_retry,
        crate::content::content_rebuild,
        crate::content::content_mark_offered,
        crate::content::content_offer,
        crate::content::content_delete_store,
        crate::content::content_search,
        crate::content::content_unread_counts,
        crate::content::content_unread,
    ]
}

/// Resolves once the index is open, or rejects with `Unavailable` when it could not open
/// (VFS-39 R2). The UI calls it before its first index command; the wait runs on a blocking
/// thread, so a long open never holds an async worker that other commands need.
#[tauri::command]
pub async fn wait_core<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || match app.try_state::<AppState>() {
        Some(state) => state.wait(),
        None => Err(vfs_core::Error::Unavailable("the index is not opening".into()).into()),
    })
    .await
    .map_err(io_error)?
}

/// The OS locale as a BCP 47 tag ("es-ES"), or "" when unknown (VFS-22 R2). The UI picks
/// its language from it; WKWebView's `navigator.language` follows the bundle, not the OS.
#[tauri::command(async)]
pub fn system_locale() -> String {
    sys_locale::get_locale().unwrap_or_default()
}

/// Makes `lang` ("es" or "en", the UI's language) the shell's: window title, macOS menu and
/// folder picker (VFS-22 R5). Any other code is ignored.
#[tauri::command(async)]
pub fn set_language<R: Runtime>(app: AppHandle<R>, lang: String) -> CmdResult<()> {
    let Some(lang) = crate::i18n::Lang::from_code(&lang) else {
        return Ok(());
    };
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if let Err(e) = crate::i18n::apply(&handle, lang) {
            log::warn!("shell language: {e}");
        }
    })
    .map_err(io_error)
}

#[tauri::command(async)]
pub fn list_roots(state: State<'_, AppState>) -> CmdResult<Vec<Root>> {
    Ok(state.vfs()?.list_roots()?)
}

/// The folders of a root that could not be listed (VFS-10 R4), at most 500 in path order.
#[tauri::command(async)]
pub fn list_unreadable(state: State<'_, AppState>, root_id: i64) -> CmdResult<Vec<UnreadableDir>> {
    Ok(state.vfs()?.unreadable_dirs(root_id)?)
}

/// Native folder dialog; `None` when the user cancels.
#[tauri::command(async)]
pub fn pick_folder<R: Runtime>(app: AppHandle<R>) -> CmdResult<Option<String>> {
    let picked = app
        .dialog()
        .file()
        .set_title(crate::i18n::current(&app).strings().choose_folder)
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
    Ok(state.vfs()?.add_root(&PathBuf::from(path))?)
}

#[tauri::command(async)]
pub fn remove_root(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    Ok(state.vfs()?.remove_root(id)?)
}

/// Pauses or resumes indexing of one folder (VFS-32).
#[tauri::command(async)]
pub fn set_root_paused(state: State<'_, AppState>, id: i64, paused: bool) -> CmdResult<()> {
    Ok(state.vfs()?.set_root_paused(id, paused)?)
}

#[tauri::command(async)]
pub fn search(state: State<'_, AppState>, query: SearchQuery) -> CmdResult<SearchResponse> {
    Ok(state.vfs()?.search(&query)?)
}

/// One search that finds something for a file search with no hits (VFS-13), or none.
#[tauri::command(async)]
pub fn suggest(state: State<'_, AppState>, query: SearchQuery) -> CmdResult<Option<Suggestion>> {
    Ok(state.vfs()?.suggest(&query)?)
}

/// "Mostrar en carpeta" (VFS-1 AC-13): Finder/Explorer with the file or folder selected.
/// `isDir` selects the id space (`dirs.id` for folder hits, `files.id` otherwise).
#[tauri::command(async)]
pub fn reveal(state: State<'_, AppState>, id: i64, is_dir: bool) -> CmdResult<RevealOutcome> {
    let vfs = state.vfs()?;
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
    let path = target_to_reveal(id, state.vfs()?.link_target(id)?)?;
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
    let url = url_to_open(id, state.vfs()?.link_target(id)?)?;
    crate::bookmarks::open_in_firefox(&url).map_err(|e| vfs_core::Error::Io(e).into())
}

/// The address "Abrir en Firefox" opens for link `id`: an Internet Shortcut's `http`/`https`
/// address, else `unavailable`.
fn url_to_open(id: i64, link: LinkTarget) -> Result<String, vfs_core::Error> {
    match link.target {
        Some(url) if link.openable => Ok(url),
        _ => Err(vfs_core::Error::Unavailable(format!("link {id}"))),
    }
}

#[tauri::command(async)]
pub fn get_progress(state: State<'_, AppState>) -> CmdResult<IndexProgress> {
    Ok(state.vfs()?.progress())
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
    Ok(state.vfs()?.rescan(id)?)
}

#[tauri::command(async)]
pub fn set_paused(state: State<'_, AppState>, paused: bool) -> CmdResult<()> {
    state.vfs()?.set_paused(paused);
    Ok(())
}

/// «Incluir carpetas del sistema y de programas» (VFS-33 R9): off by default.
#[tauri::command(async)]
pub fn get_include_all_folders(state: State<'_, AppState>) -> CmdResult<bool> {
    Ok(state.vfs()?.include_all_folders()?)
}

/// Sets it and returns the stored value; a change re-walks every folder (VFS-33 R8).
#[tauri::command(async)]
pub fn set_include_all_folders(state: State<'_, AppState>, on: bool) -> CmdResult<bool> {
    state.vfs()?.set_include_all_folders(on)?;
    Ok(state.vfs()?.include_all_folders()?)
}

/// «Filtro de archivos» (VFS-34 R1): the user's patterns.
#[tauri::command(async)]
pub fn get_file_exclusions(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    Ok(state.vfs()?.file_exclusions()?)
}

/// Indexed files `pattern` matches, before it is added; rejects with `invalid_pattern` and
/// the reason as the message (VFS-34 R2).
#[tauri::command(async)]
pub fn count_file_exclusion(state: State<'_, AppState>, pattern: String) -> CmdResult<u64> {
    Ok(state.vfs()?.count_file_exclusion(&pattern)?)
}

/// Adds a pattern and returns the list; the matching files leave the index in the
/// background (VFS-34 R4).
#[tauri::command(async)]
pub fn add_file_exclusion(state: State<'_, AppState>, pattern: String) -> CmdResult<Vec<String>> {
    Ok(state.vfs()?.add_file_exclusion(&pattern)?)
}

/// Removes a pattern and returns the list; every folder is checked again (VFS-34 R5).
#[tauri::command(async)]
pub fn remove_file_exclusion(
    state: State<'_, AppState>,
    pattern: String,
) -> CmdResult<Vec<String>> {
    Ok(state.vfs()?.remove_file_exclusion(&pattern)?)
}

/// «Restablecer la lista» (VFS-34 R7).
#[tauri::command(async)]
pub fn reset_file_exclusions(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    Ok(state.vfs()?.reset_file_exclusions()?)
}

/// «Tamaño máximo para leer el contenido (MB)»; 0 means no limit (VFS-34 R6).
#[tauri::command(async)]
pub fn get_content_max_mb(state: State<'_, AppState>) -> CmdResult<u64> {
    Ok(state.vfs()?.content_max_mb()?)
}

/// Stores it and returns the stored value (VFS-34 R6).
#[tauri::command(async)]
pub fn set_content_max_mb(state: State<'_, AppState>, mb: u64) -> CmdResult<u64> {
    state.vfs()?.set_content_max_mb(mb)?;
    Ok(state.vfs()?.content_max_mb()?)
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

    struct QuietSink;
    impl vfs_core::ProgressSink for QuietSink {
        fn on_progress(&self, _: &IndexProgress) {}
    }

    /// VFS-18 AC-11 (R8): after a `.lnk` and a `.url` are indexed and both actions are
    /// prepared on them ("Mostrar el destino" finds the target, "Abrir en Firefox" the
    /// address), the link files' bytes and modification times are unchanged.
    #[test]
    fn link_files_are_unchanged_by_indexing_and_both_actions() {
        let tree = tempfile::tempdir().unwrap();
        let db = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tree.path()).unwrap();
        std::fs::create_dir_all(root.join("Boda")).unwrap();
        std::fs::create_dir_all(root.join("Accesos")).unwrap();
        let album = root.join("Boda").join("album.jpg");
        std::fs::write(&album, b"x").unwrap();
        // A minimal Shell Link: HasRelativePath | IsUnicode, then the relative path.
        let rel: Vec<u16> = "..\\Boda\\album.jpg".encode_utf16().collect();
        let mut lnk = vec![0u8; 0x4C];
        lnk[0] = 0x4C;
        lnk[4..20].copy_from_slice(&[
            0x01, 0x14, 0x02, 0x00, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46,
        ]);
        lnk[0x14] = 0x08 | 0x80;
        lnk.extend((rel.len() as u16).to_le_bytes());
        lnk.extend(rel.iter().flat_map(|u| u.to_le_bytes()));
        lnk.extend([0, 0, 0, 0]);
        let url = b"[InternetShortcut]\r\nURL=https://www.mibanco.es/\r\n".to_vec();
        let files = [
            (root.join("Accesos").join("fotos.lnk"), lnk),
            (root.join("Accesos").join("Banco.url"), url),
        ];
        for (path, bytes) in &files {
            std::fs::write(path, bytes).unwrap();
        }
        let before: Vec<_> = files
            .iter()
            .map(|(p, _)| std::fs::metadata(p).unwrap().modified().unwrap())
            .collect();

        let vfs = vfs_core::Vfs::open(&db.path().join("index.db")).unwrap();
        vfs.add_root(&root).unwrap();
        vfs.start_indexer_with(
            std::sync::Arc::new(QuietSink),
            vfs_core::IndexerOptions {
                boot_grace: std::time::Duration::ZERO,
            },
        )
        .unwrap();
        let id_of = |text: &str, name: &str| -> Option<i64> {
            vfs.search(&SearchQuery {
                text: text.into(),
                filters: Default::default(),
                limit: 10,
                offset: 0,
                sort: Default::default(),
                scope: Default::default(),
            })
            .unwrap()
            .hits
            .into_iter()
            .find(|h| h.name == name && h.link.as_ref().is_some_and(|l| l.target.is_some()))
            .map(|h| h.id)
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let (lnk_id, url_id) = loop {
            if let (Some(a), Some(b)) = (id_of("fotos", "fotos.lnk"), id_of("banco", "Banco.url")) {
                break (a, b);
            }
            assert!(std::time::Instant::now() < deadline, "links never read");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };

        let target = target_to_reveal(lnk_id, vfs.link_target(lnk_id).unwrap()).unwrap();
        assert_eq!(std::fs::canonicalize(target).unwrap(), album);
        assert_eq!(
            url_to_open(url_id, vfs.link_target(url_id).unwrap()).unwrap(),
            "https://www.mibanco.es/"
        );
        vfs.shutdown();
        for ((path, bytes), mtime) in files.iter().zip(before) {
            assert_eq!(&std::fs::read(path).unwrap(), bytes, "{}", path.display());
            assert_eq!(std::fs::metadata(path).unwrap().modified().unwrap(), mtime);
        }
    }
}
