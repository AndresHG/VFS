//! The bookmark source in the app (VFS-6): managed state, the refresh thread, and the
//! commands `search_sources`, `get_bookmarks_status`, `set_bookmarks_enabled` and
//! `open_bookmark`. Opening a bookmark hands a web address to Firefox; no file is ever
//! opened (R10).

use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use vfs_bookmarks::firefox::Locator;
use vfs_bookmarks::sources::search_sources as run_search;
use vfs_bookmarks::{bookmarks_db_path, BookmarkStatus, Bookmarks, SourceOptions, SourcesResponse};
use vfs_core::api::ApiError;
use vfs_core::SearchQuery;

use crate::AppState;

/// Event sent after every bookmark read, payload [`BookmarkStatus`].
pub const EVENT_BOOKMARKS_CHANGED: &str = "bookmarks-changed";

/// Managed state of the bookmark source.
pub struct BookmarkState {
    pub bookmarks: Arc<Bookmarks>,
    /// Wakes the refresh thread ("Buscar cambios ahora", the setting turned on).
    refresh: Mutex<Option<Sender<()>>>,
}

impl BookmarkState {
    /// State without a refresh thread (tests read with [`Bookmarks::refresh`] directly).
    pub fn new(bookmarks: Arc<Bookmarks>) -> BookmarkState {
        BookmarkState {
            bookmarks,
            refresh: Mutex::new(None),
        }
    }

    pub fn refresh_now(&self) {
        if let Some(tx) = self
            .refresh
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = tx.send(());
        }
    }
}

/// Opens the bookmark store next to the index and starts the refresh thread, which reads
/// at once if the setting is on and then every 30 minutes (VFS-6 D6). A store that cannot
/// be opened leaves the app working with files only.
pub fn init<R: Runtime>(app: &AppHandle<R>, index_db: &Path) {
    let bookmarks = match Bookmarks::open(&bookmarks_db_path(index_db), Locator::system()) {
        Ok(b) => Arc::new(b),
        Err(e) => {
            log::warn!("bookmarks unavailable: {e}");
            return;
        }
    };
    let handle = app.clone();
    let tx = bookmarks.start_refresher(move |status| {
        if let Err(e) = handle.emit(EVENT_BOOKMARKS_CHANGED, status) {
            log::warn!("emit {EVENT_BOOKMARKS_CHANGED}: {e}");
        }
    });
    let state = BookmarkState::new(bookmarks);
    *state.refresh.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    app.manage(state);
}

fn missing() -> ApiError {
    vfs_core::Error::Unavailable("bookmarks unavailable".into()).into()
}

/// A search over the chosen sources, optionally inside one bookmark folder and in another
/// order; bookmarks only while the setting is on.
#[tauri::command(async)]
pub fn search_sources<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    query: SearchQuery,
    options: Option<SourceOptions>,
) -> Result<SourcesResponse, ApiError> {
    let marks = app.try_state::<BookmarkState>();
    let bookmarks = marks.as_ref().map(|s| s.bookmarks.as_ref());
    Ok(run_search(
        &state.vfs,
        bookmarks,
        &query,
        &options.unwrap_or_default(),
    )?)
}

#[tauri::command(async)]
pub fn get_bookmarks_status<R: Runtime>(app: AppHandle<R>) -> Result<BookmarkStatus, ApiError> {
    let state = app.try_state::<BookmarkState>().ok_or_else(missing)?;
    Ok(state.bookmarks.status())
}

/// "Incluir marcadores de Firefox": on reads at once (in the background); off deletes every
/// stored bookmark (R8).
#[tauri::command(async)]
pub fn set_bookmarks_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<BookmarkStatus, ApiError> {
    let state = app.try_state::<BookmarkState>().ok_or_else(missing)?;
    let status = state.bookmarks.set_enabled(enabled)?;
    if enabled {
        state.refresh_now();
    }
    Ok(status)
}

/// "Abrir en Firefox" (R10): the bookmark's `http`/`https` address in Firefox.
#[tauri::command(async)]
pub fn open_bookmark<R: Runtime>(app: AppHandle<R>, id: i64) -> Result<(), ApiError> {
    let state = app.try_state::<BookmarkState>().ok_or_else(missing)?;
    let url = state.bookmarks.url_to_open(id)?;
    open_in_firefox(&url).map_err(|e| vfs_core::Error::Io(e).into())
}

/// Opens a web address in Firefox; if Firefox cannot be started (moved or uninstalled
/// since its bookmarks were read), in the default browser instead. Only addresses that
/// [`firefox_arg`] accepts are passed on.
pub fn open_in_firefox(url: &str) -> std::io::Result<()> {
    let arg = firefox_arg(url).ok_or_else(|| std::io::Error::other("not a web address"))?;
    platform_open(arg).inspect_err(|e| log::warn!("open in Firefox: {e}"))
}

/// The address as one command-line argument, when [`vfs_bookmarks::is_openable`] accepts it:
/// it starts with the scheme, cannot be read as an option, and needs no quoting.
pub fn firefox_arg(url: &str) -> Option<&str> {
    vfs_bookmarks::is_openable(url).then_some(url)
}

#[cfg(target_os = "macos")]
fn platform_open(url: &str) -> std::io::Result<()> {
    let run = |args: &[&str]| -> std::io::Result<bool> {
        Ok(std::process::Command::new("/usr/bin/open")
            .args(args)
            .status()?
            .success())
    };
    if run(&["-a", "Firefox", url])? || run(&[url])? {
        Ok(())
    } else {
        Err(std::io::Error::other("open failed"))
    }
}

#[cfg(windows)]
fn platform_open(url: &str) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let verb = wide("open");
    let shell = |file: &[u16], params: Option<&[u16]>| -> isize {
        // SAFETY: every string is a NUL-terminated UTF-16 buffer that outlives the call.
        (unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                params.map_or(std::ptr::null(), |p| p.as_ptr()),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        }) as isize
    };
    // "firefox.exe" is found through App Paths (installer) or the app execution alias
    // (Microsoft Store build); the address needs no quoting (`firefox_arg`).
    // ShellExecuteW returns a value greater than 32 on success.
    let code = shell(&wide("firefox.exe"), Some(&wide(url)));
    if code > 32 || shell(&wide(url), None) > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "ShellExecuteW failed ({code})"
        )))
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_open(url: &str) -> std::io::Result<()> {
    std::process::Command::new("firefox")
        .arg(url)
        .spawn()
        .or_else(|_| std::process::Command::new("xdg-open").arg(url).spawn())
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-6 AC-12: only a plain web address becomes Firefox's argument.
    #[test]
    fn only_plain_web_addresses_reach_firefox() {
        assert_eq!(
            firefox_arg("https://cocina.es/flan?x=1"),
            Some("https://cocina.es/flan?x=1")
        );
        assert_eq!(firefox_arg("HTTP://a.es"), Some("HTTP://a.es"));
        for bad in [
            "file:///C:/a.pdf",
            "data:text/html,hola",
            "ftp://a.es",
            "-https://a.es",
            "https://a.es/\" x\"",
            "https://a.es/ -foreground",
            "https://a.es/\nb",
        ] {
            assert_eq!(firefox_arg(bad), None, "{bad}");
        }
    }
}
