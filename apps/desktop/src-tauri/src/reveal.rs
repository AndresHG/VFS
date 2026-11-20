//! "Mostrar en carpeta" (VFS-1 AC-13): asks the OS file manager to show an item selected.
//! macOS: `open -R <path>`. Windows: `explorer.exe /select,"<path>"`. Explorer cannot select
//! items through a `\\?\` path or a path over `MAX_PATH`, so those open the nearest usable
//! ancestor folder instead and report [`RevealOutcome::OpenedParent`]. Nothing is opened
//! with its default application.

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

/// Shows `path` (which exists) selected in Finder/Explorer.
#[cfg(windows)]
pub fn reveal(path: &Path) -> io::Result<RevealOutcome> {
    use std::os::windows::process::CommandExt;
    // Explorer parses its own command line: `/select,` must be followed by the quoted path in
    // the same argument, so it is passed raw. Windows paths cannot contain `"`.
    let (arg, outcome) = match plan_windows(path, |p| p.exists()) {
        Some(Action::Select(p)) => (
            format!("/select,\"{}\"", p.display()),
            RevealOutcome::Revealed,
        ),
        Some(Action::OpenDir(p)) => (format!("\"{}\"", p.display()), RevealOutcome::OpenedParent),
        None => {
            return Err(io::Error::other(format!(
                "no folder of {} can be shown in Explorer",
                path.display()
            )))
        }
    };
    // Explorer's exit code is meaningless (1 on success); only a failed spawn is an error.
    Command::new("explorer.exe").raw_arg(arg).spawn()?;
    Ok(outcome)
}

/// Shows `path` (which exists) selected in Finder/Explorer.
#[cfg(target_os = "macos")]
pub fn reveal(path: &Path) -> io::Result<RevealOutcome> {
    if Command::new("open").arg("-R").arg(path).status()?.success() {
        return Ok(RevealOutcome::Revealed);
    }
    let dir = nearest_ancestor(path, Path::is_dir)
        .ok_or_else(|| io::Error::other(format!("no folder of {}", path.display())))?;
    let status = Command::new("open").arg(&dir).status()?;
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

    /// VFS-1 AC-13: the Explorer length limit counts UTF-16 units and rejects `\\?\` paths.
    #[test]
    fn explorer_limit() {
        let ok = format!("C:\\{}", "a".repeat(EXPLORER_MAX_PATH - 3));
        assert!(explorer_usable(Path::new(&ok)));
        let long = format!("C:\\{}", "a".repeat(EXPLORER_MAX_PATH - 2));
        assert!(!explorer_usable(Path::new(&long)));
        assert!(!explorer_usable(Path::new(r"\\?\C:\x")));
    }

    /// VFS-1 AC-8 + AC-13: short paths are selected (verbatim prefix dropped); paths over
    /// MAX_PATH open the nearest existing ancestor Explorer can address.
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
