//! Path spelling shared by the indexer side and the worker.

use std::path::{Path, PathBuf};

/// Windows: absolute paths of 248+ characters get the `\\?\` prefix (same rule as
/// `vfs_core::platform::long_path`, VFS-1 AC-8). Elsewhere: unchanged.
pub fn long_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.as_os_str().to_string_lossy();
        if s.len() >= 248 && !s.starts_with(r"\\?\") {
            if let Some(unc) = s.strip_prefix(r"\\") {
                return PathBuf::from(format!(r"\\?\UNC\{unc}"));
            }
            if path.is_absolute() {
                return PathBuf::from(format!(r"\\?\{s}"));
            }
        }
    }
    path.to_path_buf()
}
