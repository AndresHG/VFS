//! Whether a document is a cloud-only placeholder, checked just before it would be opened
//! (VFS-2 R12, D12). Reading such a file would make the sync client download it, so it is
//! never opened. The check reads attributes only: it opens nothing.

use std::path::Path;

/// Windows attribute bits of a placeholder whose data is not on the disk:
/// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`, `FILE_ATTRIBUTE_RECALL_ON_OPEN`,
/// `FILE_ATTRIBUTE_OFFLINE` (the same set as VFS-25 R1).
#[cfg_attr(not(windows), allow(dead_code))]
const CLOUD_ATTRIBUTES: u32 = 0x0040_0000 | 0x0004_0000 | 0x0000_1000;

/// macOS `SF_DATALESS` (`sys/stat.h`): the file's data lives in a file provider.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SF_DATALESS: u32 = 0x4000_0000;

/// True when Windows file attributes mark a cloud-only file.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn cloud_attributes(attributes: u32) -> bool {
    attributes & CLOUD_ATTRIBUTES != 0
}

/// True when macOS `st_flags` mark a dataless file.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn dataless(st_flags: u32) -> bool {
    st_flags & SF_DATALESS != 0
}

/// Whether `path` is cloud-only right now. False when the attributes cannot be read: the
/// open that follows then reports the real error.
pub fn is_cloud_only(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
        };
        let wide: Vec<u16> = crate::paths::long_path(path)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call.
        let a = unsafe { GetFileAttributesW(wide.as_ptr()) };
        a != INVALID_FILE_ATTRIBUTES && cloud_attributes(a)
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        std::fs::symlink_metadata(path).is_ok_and(|m| dataless(m.st_flags()))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-2 AC-13: the macOS `SF_DATALESS` flag and the Windows placeholder attributes are
    /// read as cloud-only; ordinary flags and attributes are not.
    #[test]
    fn recognises_cloud_only_flags() {
        assert!(dataless(SF_DATALESS));
        assert!(dataless(SF_DATALESS | 0x20));
        assert!(!dataless(0x20)); // SF_ARCHIVED
        assert!(!dataless(0));
        assert!(cloud_attributes(0x1000)); // offline
        assert!(cloud_attributes(0x0004_0000)); // recall on open
        assert!(cloud_attributes(0x0040_0000 | 0x20)); // recall on data access, archive
        assert!(!cloud_attributes(0x20 | 0x80)); // archive, normal
    }

    /// VFS-2 AC-13: an ordinary local file is not cloud-only.
    #[test]
    fn local_file_is_not_cloud_only() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, "hola").unwrap();
        assert!(!is_cloud_only(&p));
    }
}
