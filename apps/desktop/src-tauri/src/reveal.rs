//! "Mostrar en carpeta" (VFS-1 AC-13): asks the OS file manager to show an item selected.
//! macOS: `open -R <path>`. Windows: the Shell's `SHOpenFolderAndSelectItems`, which selects
//! the item and reuses an Explorer window already showing its folder, repeated into the new
//! Explorer window when it had to open one; `explorer.exe /select,"<path>"` only when that
//! call fails (issue 107). A path over `MAX_PATH` is selected the same way through its plain
//! (non-`\\?\`) form, which the Shell's ID-list calls accept (VFS-24 AC-1); only if that
//! fails does the nearest folder Explorer can address open, reporting
//! [`RevealOutcome::OpenedParent`]. On macOS a failed
//! `open -R` reveals the nearest existing ancestor with `open -R` too. Nothing is opened
//! with its default application, and no ancestor is ever launched (an `.app` bundle would
//! be).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", windows))]
use std::process::Command;

use vfs_core::api::RevealOutcome;

/// Longest path, in UTF-16 units, that Explorer accepts (`MAX_PATH` minus the NUL).
pub const EXPLORER_MAX_PATH: usize = 259;

/// What to ask the file manager for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open the parent folder with this item selected.
    Select(PathBuf),
    /// Open this folder (fallback; nothing selected).
    OpenDir(PathBuf),
}

/// The deepest proper ancestor of `path` for which `usable` holds.
pub fn nearest_ancestor(path: &Path, usable: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    path.ancestors()
        .skip(1)
        .filter(|a| !a.as_os_str().is_empty())
        .find(|a| usable(a))
        .map(Path::to_path_buf)
}

fn explorer_usable(p: &Path) -> bool {
    let s = p.to_string_lossy();
    !s.starts_with(r"\\?\") && s.encode_utf16().count() <= EXPLORER_MAX_PATH
}

/// Windows plan: select `path` when Explorer can address it, else open the nearest
/// existing ancestor that it can. `\\?\` prefixes are dropped where that is lossless.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn plan_windows(path: &Path, exists: impl Fn(&Path) -> bool) -> Option<Action> {
    let path = dunce::simplified(path);
    if explorer_usable(path) {
        return Some(Action::Select(path.to_path_buf()));
    }
    nearest_ancestor(path, |a| {
        let a = dunce::simplified(a);
        explorer_usable(a) && exists(a)
    })
    .map(|a| Action::OpenDir(dunce::simplified(&a).to_path_buf()))
}

/// The plain form of `path` that the Shell's ID-list calls accept past `MAX_PATH`:
/// `\\?\C:\…` becomes `C:\…` and `\\?\UNC\server\share\…` becomes `\\server\share\…`
/// (VFS-24 D1, measured on the Windows runner: `ILCreateFromPathW` returns no ID list for the
/// verbatim form of a 362-character path and a usable one for its plain form).
pub fn shell_form(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// The plain form of `path` for the Shell when only its length keeps Explorer from it: over
/// `MAX_PATH`, with no component ending in a dot or a space and no reserved device name
/// (`CON`, `NUL.txt`, `COM1`…). Without the prefix such names name another item, as would a
/// short path that still needs `\\?\`, so they get `None` and the nearest folder opens
/// instead (VFS-24 AC-1).
pub fn long_shell_path(path: &Path) -> Option<PathBuf> {
    let plain = shell_form(path);
    let s = plain.to_string_lossy();
    let long = s.encode_utf16().count() > EXPLORER_MAX_PATH;
    let lossless = s
        .split('\\')
        .skip(1)
        .all(|c| !c.ends_with('.') && !c.ends_with(' ') && !is_device_name(c));
    (long && lossless).then_some(plain)
}

/// Whether a path component is a Win32 reserved device name, with or without an extension
/// (`COM0`/`LPT0` included: newer Windows reserves them too, and a false match only opens the
/// folder instead).
fn is_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or_default().trim_end();
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit())
}

/// Explorer's argument for `action`: `/select,"<path>"` or `"<path>"`. A trailing backslash
/// (a drive root, `C:\`) is doubled, because under the Windows command-line rules
/// (`CommandLineToArgvW`) a backslash before the closing quote escapes it, so `"C:\"` would
/// read as `C:"` plus whatever follows; `"C:\\"` reads as `C:\`, and if Explorer splits the
/// line naively instead, `C:\\` still names the same folder. Unverified on a real Windows.
pub fn explorer_arg(action: &Action) -> String {
    let quoted = |p: &Path| {
        let s = p.display().to_string();
        let tail = if s.ends_with('\\') { "\\" } else { "" };
        format!("\"{s}{tail}\"")
    };
    match action {
        Action::Select(p) => format!("/select,{}", quoted(p)),
        Action::OpenDir(p) => quoted(p),
    }
}

/// Arguments of `open` on macOS for `action`. Both reveal in Finder (`-R`): the fallback
/// shows the ancestor selected in its own parent rather than opening it, so an ancestor that
/// is an application bundle is never launched.
pub fn open_args(action: &Action) -> Vec<OsString> {
    let p = match action {
        Action::Select(p) | Action::OpenDir(p) => p,
    };
    vec![OsString::from("-R"), p.clone().into_os_string()]
}

/// Shows `path` (which exists) selected in Finder/Explorer.
#[cfg(windows)]
pub fn reveal(path: &Path) -> io::Result<RevealOutcome> {
    match plan_windows(path, |p| p.exists()) {
        Some(Action::Select(p)) => {
            if let Err(e) = shell_select(&p) {
                log::warn!("SHOpenFolderAndSelectItems failed, using explorer /select: {e}");
                spawn_explorer(&Action::Select(p))?;
            }
            Ok(RevealOutcome::Revealed)
        }
        Some(Action::OpenDir(p)) => {
            // Past MAX_PATH Explorer's command line fails, but the Shell's ID-list call can
            // still select the item through its plain path (VFS-24 AC-1); only if it cannot
            // is the nearest folder opened instead.
            match long_shell_path(path).map_or_else(
                || Err(io::Error::other("needs its \\\\?\\ form")),
                |plain| shell_select(&plain),
            ) {
                Ok(()) => Ok(RevealOutcome::Revealed),
                Err(e) => {
                    log::warn!("long path not selectable ({e}); opening {}", p.display());
                    spawn_explorer(&Action::OpenDir(p))?;
                    Ok(RevealOutcome::OpenedParent)
                }
            }
        }
        None => Err(io::Error::other(format!(
            "no folder of {} can be shown in Explorer",
            path.display()
        ))),
    }
}

/// `explorer.exe` with [`explorer_arg`]. Explorer parses its own command line: `/select,`
/// must be followed by the quoted path in the same argument, so it is passed raw. Windows
/// paths cannot contain `"`. Its exit code is meaningless (1 on success); only a failed
/// spawn is an error.
#[cfg(windows)]
fn spawn_explorer(action: &Action) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    Command::new("explorer.exe")
        .raw_arg(explorer_arg(action))
        .spawn()
        .map(drop)
}

/// How long to wait for the Explorer window that a reveal opens.
#[cfg(windows)]
const NEW_WINDOW_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Delays, after a new Explorer window appeared, of the repeated select calls.
#[cfg(windows)]
const RESELECT_AFTER: [std::time::Duration; 2] = [
    std::time::Duration::from_millis(300),
    std::time::Duration::from_millis(1200),
];

/// Opens `path`'s folder in Explorer with `path` selected, through
/// `SHOpenFolderAndSelectItems` with the item's own absolute ID list and no child list (the
/// documented form for one item). Measured on the Windows CI runner (issue 107): when the
/// call has to create a new Explorer window, the folder opens with nothing selected, with
/// this and every other form of the call and with `explorer /select,`; into a window that
/// already shows the folder the same call selects at once. So when a new Explorer window
/// appears within [`NEW_WINDOW_WAIT`], the call is repeated into it ([`RESELECT_AFTER`]).
/// The Shell needs COM, so all of it runs on a thread of its own with a single-threaded
/// apartment; the command returns after the first call and never waits for the window.
#[cfg(windows)]
fn shell_select(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows_sys::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let (first_tx, first_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // SAFETY: plain Win32 calls; `wide` is NUL-terminated and outlives the calls, the ID
        // list is freed once, and COM is uninitialised only when it was initialised here.
        unsafe {
            let hr = CoInitializeEx(
                std::ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            );
            let pidl = ILCreateFromPathW(wide.as_ptr());
            if pidl.is_null() {
                let _ = first_tx.send(Err(io::Error::other(
                    "ILCreateFromPathW returned no ID list",
                )));
            } else {
                let before = explorer_windows();
                let first = SHOpenFolderAndSelectItems(pidl, 0, std::ptr::null(), 0);
                let ok = first >= 0;
                let _ = first_tx.send(if ok {
                    Ok(())
                } else {
                    Err(io::Error::other(format!("HRESULT {first:#010x}")))
                });
                if ok && wait_for_new_window(&before) {
                    for delay in RESELECT_AFTER {
                        std::thread::sleep(delay);
                        let hr = SHOpenFolderAndSelectItems(pidl, 0, std::ptr::null(), 0);
                        if hr < 0 {
                            log::warn!("repeated SHOpenFolderAndSelectItems: {hr:#010x}");
                        }
                    }
                }
                ILFree(pidl);
            }
            if hr >= 0 {
                CoUninitialize();
            }
        }
    });
    first_rx
        .recv()
        .unwrap_or_else(|_| Err(io::Error::other("reveal thread ended early")))
}

/// Whether an Explorer window that is not in `before` appears within [`NEW_WINDOW_WAIT`].
#[cfg(windows)]
fn wait_for_new_window(before: &[isize]) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < NEW_WINDOW_WAIT {
        if explorer_windows().iter().any(|w| !before.contains(w)) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// Window class of File Explorer's folder windows.
#[cfg(windows)]
const EXPLORER_WINDOW_CLASS: &str = "CabinetWClass";

/// Handles of the open File Explorer folder windows.
#[cfg(windows)]
fn explorer_windows() -> Vec<isize> {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW};

    unsafe extern "system" fn collect(hwnd: HWND, lp: LPARAM) -> BOOL {
        let mut class = [0u16; 64];
        // SAFETY: `class` is a live buffer of the length passed; `lp` is the `Vec` below,
        // borrowed for the duration of `EnumWindows`.
        let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
        if String::from_utf16_lossy(&class[..n.max(0) as usize]) == EXPLORER_WINDOW_CLASS {
            unsafe { (*(lp as *mut Vec<isize>)).push(hwnd as isize) };
        }
        1
    }
    let mut found: Vec<isize> = Vec::new();
    // SAFETY: the callback only runs during this call, while `found` is alive.
    unsafe { EnumWindows(Some(collect), &mut found as *mut Vec<isize> as LPARAM) };
    found
}

/// Shows `path` (which exists) selected in Finder/Explorer.
#[cfg(target_os = "macos")]
pub fn reveal(path: &Path) -> io::Result<RevealOutcome> {
    let select = Action::Select(path.to_path_buf());
    if Command::new("open")
        .args(open_args(&select))
        .status()?
        .success()
    {
        return Ok(RevealOutcome::Revealed);
    }
    let dir = nearest_ancestor(path, Path::is_dir)
        .ok_or_else(|| io::Error::other(format!("no folder of {}", path.display())))?;
    let status = Command::new("open")
        .args(open_args(&Action::OpenDir(dir.clone())))
        .status()?;
    if status.success() {
        Ok(RevealOutcome::OpenedParent)
    } else {
        Err(io::Error::other(format!(
            "open {}: {status}",
            dir.display()
        )))
    }
}

/// Shows `path` (which exists) selected in Finder/Explorer.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn reveal(path: &Path) -> io::Result<RevealOutcome> {
    Err(io::Error::other(format!(
        "reveal is not supported on this platform: {}",
        path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-1 AC-13: the fallback opens the deepest ancestor that is usable, never the item.
    #[test]
    fn nearest_ancestor_picks_deepest_usable() {
        let p = Path::new("/a/b/c/file.txt");
        assert_eq!(
            nearest_ancestor(p, |a| a == Path::new("/a/b") || a == Path::new("/a")),
            Some(PathBuf::from("/a/b"))
        );
        assert_eq!(nearest_ancestor(p, |_| false), None);
        assert_eq!(
            nearest_ancestor(Path::new("rel"), |_| true),
            None,
            "the empty path is never a candidate"
        );
    }

    /// VFS-1 AC-13: Explorer gets the path quoted after `/select,`; a drive root's trailing
    /// backslash is doubled so it cannot escape the closing quote.
    #[test]
    fn explorer_argument_strings() {
        let sel = |p: &str| explorer_arg(&Action::Select(PathBuf::from(p)));
        assert_eq!(
            sel(r"C:\Datos\Notaría.pdf"),
            r#"/select,"C:\Datos\Notaría.pdf""#
        );
        assert_eq!(sel("C:\\"), r#"/select,"C:\\""#);
        assert_eq!(
            explorer_arg(&Action::OpenDir(PathBuf::from(r"D:\Fotos"))),
            r#""D:\Fotos""#
        );
        assert_eq!(
            explorer_arg(&Action::OpenDir(PathBuf::from("D:\\"))),
            r#""D:\\""#
        );
    }

    /// VFS-1 AC-13: on macOS both the reveal and its fallback are `open -R`, so the fallback
    /// ancestor is shown in Finder, never opened or launched (an `.app` bundle would run).
    #[test]
    fn open_always_reveals() {
        for a in [
            Action::Select(PathBuf::from("/Volumes/Archivo/Notaría.pdf")),
            Action::OpenDir(PathBuf::from("/Applications/Calculator.app")),
        ] {
            let args = open_args(&a);
            assert_eq!(args[0], "-R");
            assert_eq!(args.len(), 2);
        }
    }

    /// VFS-24 AC-1: the Shell gets long paths without the verbatim prefix.
    #[test]
    fn shell_form_drops_the_verbatim_prefix() {
        let cases = [
            (r"\\?\C:\Datos\Año 2019\f.pdf", r"C:\Datos\Año 2019\f.pdf"),
            (r"\\?\UNC\nas\fotos\f.jpg", r"\\nas\fotos\f.jpg"),
            (r"C:\Datos\f.pdf", r"C:\Datos\f.pdf"),
        ];
        for (from, to) in cases {
            assert_eq!(shell_form(Path::new(from)), PathBuf::from(to));
        }
    }

    /// VFS-24 AC-1: only a path too long for Explorer, whose plain form names the same item,
    /// goes to the Shell without its prefix.
    #[test]
    fn long_shell_path_only_for_length() {
        let deep = format!(r"C:\Datos\{}\{}\f.pdf", "x".repeat(200), "y".repeat(100));
        let want = Some(PathBuf::from(&deep));
        assert_eq!(long_shell_path(Path::new(&format!(r"\\?\{deep}"))), want);
        assert_eq!(long_shell_path(Path::new(&deep)), want);
        assert_eq!(
            long_shell_path(Path::new(r"\\?\C:\Datos\punto.\f.pdf")),
            None
        );
        for odd in ["fin.", "fin ", "nul.txt", "CON", "com1.log", "Lpt9"] {
            let p = format!(r"\\?\C:\Datos\{}\{odd}\f.pdf", "x".repeat(300));
            assert_eq!(long_shell_path(Path::new(&p)), None, "{odd:?}");
        }
        for fine in ["console.txt", "com10", "nulo.pdf"] {
            let p = format!(r"\\?\C:\Datos\{}\{fine}", "x".repeat(300));
            assert!(long_shell_path(Path::new(&p)).is_some(), "{fine:?}");
        }
    }

    /// VFS-1 AC-13: the Explorer length limit counts UTF-16 units and rejects `\\?\` paths.
    #[test]
    fn explorer_limit() {
        let ok = format!("C:\\{}", "a".repeat(EXPLORER_MAX_PATH - 3));
        assert!(explorer_usable(Path::new(&ok)));
        let long = format!("C:\\{}", "a".repeat(EXPLORER_MAX_PATH - 2));
        assert!(!explorer_usable(Path::new(&long)));
        assert!(!explorer_usable(Path::new(r"\\?\C:\x")));
    }

    /// VFS-1 AC-8 + AC-13: short paths are selected (verbatim prefix dropped); for paths over
    /// MAX_PATH the fallback plan is the nearest existing ancestor Explorer can address
    /// (used only when the Shell cannot select the item, VFS-24 AC-1).
    #[cfg(windows)]
    #[test]
    fn plan_windows_long_paths() {
        assert_eq!(
            plan_windows(Path::new(r"\\?\C:\Datos\Notaría.pdf"), |_| true),
            Some(Action::Select(PathBuf::from(r"C:\Datos\Notaría.pdf")))
        );
        let deep = format!(r"C:\Datos\{}\{}\f.pdf", "x".repeat(200), "y".repeat(100));
        let verbatim = format!(r"\\?\{deep}");
        let want = PathBuf::from(format!(r"C:\Datos\{}", "x".repeat(200)));
        for p in [deep.as_str(), verbatim.as_str()] {
            assert_eq!(
                plan_windows(Path::new(p), |_| true),
                Some(Action::OpenDir(want.clone()))
            );
        }
        assert_eq!(
            plan_windows(Path::new(&deep), |a| a == Path::new(r"C:\")),
            Some(Action::OpenDir(PathBuf::from(r"C:\")))
        );
    }
}
