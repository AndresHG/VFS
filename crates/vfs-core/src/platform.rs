//! Platform specifics: volume identity and background priority. Placeholder bodies; owned by
//! task T3.

use std::path::Path;

/// Windows: volume GUID path; macOS: mount point. T3 replaces this placeholder.
#[allow(dead_code)]
pub(crate) fn volume_key(path: &Path) -> String {
    path.ancestors()
        .last()
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Puts the calling thread in background CPU and I/O priority; failures are logged and
/// ignored. T3 replaces this placeholder.
#[allow(dead_code)]
pub(crate) fn enter_background_priority() {}
