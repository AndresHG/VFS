//! Platform specifics: volume identity and background priority. Owned by task T3b; only
//! this file, `indexer/walk.rs` and the desktop `reveal.rs` contain `cfg(windows)` code.

use std::path::Path;

/// Windows: volume GUID path (`GetVolumePathNameW` + `GetVolumeNameForVolumeMountPointW`,
/// falling back to the drive prefix); macOS: `statfs.f_mntonname`. The placeholder returns
/// the path's first component.
pub(crate) fn volume_key(path: &Path) -> String {
    path.components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Puts the calling thread in background CPU and I/O priority; failures are logged and
/// ignored.
#[allow(dead_code)] // T3a calls this at the start of the indexer thread.
pub(crate) fn enter_background_priority() {
    // T3b: THREAD_MODE_BACKGROUND_BEGIN on Windows, QOS_CLASS_BACKGROUND on macOS.
}
