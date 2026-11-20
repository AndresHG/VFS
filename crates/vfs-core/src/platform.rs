//! Platform specifics: volume identity, drive kind, long paths and background priority.
//! Owned by task T3b; only this file, `indexer/walk.rs`, `indexer/watch.rs` and the desktop
//! `reveal.rs` contain `cfg(windows)` code.

use std::path::{Path, PathBuf};

use crate::types::DriveKind;

/// Windows: volume GUID path (`GetVolumePathNameW` + `GetVolumeNameForVolumeMountPointW`,
/// falling back to the volume path, then the drive prefix); macOS: `statfs.f_mntonname`.
/// Elsewhere, or when the system call fails, the path's first component.
pub(crate) fn volume_key(path: &Path) -> String {
    imp::volume_key(path).unwrap_or_else(|| first_component(path))
}

fn first_component(path: &Path) -> String {
    path.components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Windows: `GetDriveTypeW` on the volume path; macOS: `statfs.f_flags` (`MNT_LOCAL`,
/// `MNT_REMOVABLE`). A USB hard disk usually reports `Local` on both systems.
pub(crate) fn drive_kind(path: &Path) -> DriveKind {
    imp::drive_kind(path)
}

/// Windows: the `\\?\` (or `\\?\UNC\`) form of an absolute path, for direct Win32 calls that
/// would otherwise stop at 260 characters (the standard library adds it on its own). Paths
/// already verbatim, relative, or containing `..` are returned unchanged. Elsewhere a copy.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn long_path(path: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::path::{Component, Prefix};

    let mut comps = path.components();
    let mut out = match comps.next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(_) => {
                let mut s = OsString::from(r"\\?\");
                s.push(p.as_os_str());
                s.push(r"\");
                PathBuf::from(s)
            }
            Prefix::UNC(server, share) => {
                let mut s = OsString::from(r"\\?\UNC\");
                s.push(server);
                s.push(r"\");
                s.push(share);
                s.push(r"\");
                PathBuf::from(s)
            }
            _ => return path.to_path_buf(),
        },
        _ => return path.to_path_buf(),
    };
    for c in comps {
        match c {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => out.push(name),
            Component::ParentDir | Component::Prefix(_) => return path.to_path_buf(),
        }
    }
    out
}

/// Puts the calling thread in background CPU and I/O priority; failures are logged and
/// ignored.
pub(crate) fn enter_background_priority() {
    imp::enter_background_priority();
}

#[cfg(target_os = "macos")]
mod imp {
    use super::DriveKind;
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    /// `sys/qos.h`; not exported by the `libc` crate.
    const QOS_CLASS_BACKGROUND: libc::c_uint = 0x09;
    /// `sys/resource.h`.
    const IOPOL_TYPE_DISK: libc::c_int = 0;
    const IOPOL_SCOPE_THREAD: libc::c_int = 1;
    const IOPOL_THROTTLE: libc::c_int = 3;
    /// `sys/mount.h`: mounted on removable media.
    const MNT_REMOVABLE: u32 = 0x0000_0200;

    extern "C" {
        fn pthread_set_qos_class_self_np(
            qos_class: libc::c_uint,
            relative_priority: libc::c_int,
        ) -> libc::c_int;
        fn setiopolicy_np(
            iotype: libc::c_int,
            scope: libc::c_int,
            policy: libc::c_int,
        ) -> libc::c_int;
    }

    fn statfs(path: &Path) -> Option<libc::statfs> {
        let c = CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: `c` is a valid NUL-terminated string; `buf` is a plain C struct that
        // `statfs` fills on success.
        unsafe {
            let mut buf: libc::statfs = std::mem::zeroed();
            (libc::statfs(c.as_ptr(), &mut buf) == 0).then_some(buf)
        }
    }

    pub(super) fn volume_key(path: &Path) -> Option<String> {
        let st = statfs(path)?;
        // SAFETY: the kernel NUL-terminates `f_mntonname` within its fixed-size array.
        let mnt = unsafe { CStr::from_ptr(st.f_mntonname.as_ptr()) };
        let key = mnt.to_string_lossy().into_owned();
        (!key.is_empty()).then_some(key)
    }

    pub(super) fn drive_kind(path: &Path) -> DriveKind {
        match statfs(path) {
            None => DriveKind::Unknown,
            Some(st) if st.f_flags & libc::MNT_LOCAL as u32 == 0 => DriveKind::Network,
            Some(st) if st.f_flags & MNT_REMOVABLE != 0 => DriveKind::Removable,
            Some(_) => DriveKind::Local,
        }
    }

    pub(super) fn enter_background_priority() {
        // SAFETY: both calls only change scheduling attributes of the calling thread.
        unsafe {
            if pthread_set_qos_class_self_np(QOS_CLASS_BACKGROUND, 0) != 0 {
                log::warn!("could not set background QoS on the indexer thread");
            }
            if setiopolicy_np(IOPOL_TYPE_DISK, IOPOL_SCOPE_THREAD, IOPOL_THROTTLE) != 0 {
                log::warn!(
                    "could not throttle indexer I/O: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::DriveKind;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
    };

    /// `GetDriveTypeW` results (`WindowsProgramming`, a feature this crate does not enable).
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn from_wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    /// Mount point of the volume holding `path`, with its trailing backslash, NUL-terminated.
    fn volume_path(path: &Path) -> Option<Vec<u16>> {
        let p = wide(path);
        let mut buf = vec![0u16; 1024];
        // SAFETY: `p` is NUL-terminated; `buf` holds `buf.len()` u16s.
        let ok = unsafe { GetVolumePathNameW(p.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        (ok != 0).then_some(buf)
    }

    pub(super) fn volume_key(path: &Path) -> Option<String> {
        let vol = volume_path(path)?;
        let mut guid = vec![0u16; 64];
        // SAFETY: `vol` is NUL-terminated (returned by `GetVolumePathNameW`); `guid` holds
        // `guid.len()` u16s (a volume GUID path is 49 characters).
        let ok = unsafe {
            GetVolumeNameForVolumeMountPointW(vol.as_ptr(), guid.as_mut_ptr(), guid.len() as u32)
        };
        let key = if ok != 0 {
            from_wide(&guid)
        } else {
            from_wide(&vol)
        };
        (!key.is_empty()).then_some(key)
    }

    pub(super) fn drive_kind(path: &Path) -> DriveKind {
        let Some(vol) = volume_path(path) else {
            return DriveKind::Unknown;
        };
        // SAFETY: `vol` is NUL-terminated.
        match unsafe { GetDriveTypeW(vol.as_ptr()) } {
            DRIVE_REMOTE => DriveKind::Network,
            DRIVE_REMOVABLE | DRIVE_CDROM => DriveKind::Removable,
            DRIVE_FIXED | DRIVE_RAMDISK => DriveKind::Local,
            _ => DriveKind::Unknown,
        }
    }

    pub(super) fn enter_background_priority() {
        // SAFETY: the pseudo-handle of the current thread needs no closing. Background mode
        // lowers both CPU and I/O priority of this thread only.
        let ok = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) };
        if ok == 0 {
            log::warn!(
                "could not enter background mode on the indexer thread: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::DriveKind;
    use std::path::Path;

    pub(super) fn volume_key(_path: &Path) -> Option<String> {
        None
    }

    pub(super) fn drive_kind(_path: &Path) -> DriveKind {
        DriveKind::Unknown
    }

    pub(super) fn enter_background_priority() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-1 AC-8: a folder on the system disk has a non-empty volume key shared by its
    /// subfolders, and is a local drive.
    #[test]
    fn volume_key_and_drive_kind_of_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let key = volume_key(&root);
        assert!(!key.is_empty());
        assert_eq!(volume_key(&sub), key);
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(drive_kind(&root), DriveKind::Local);
        }
        enter_background_priority();
    }

    /// VFS-1 AC-8: long paths get the verbatim prefix on Windows and are untouched elsewhere.
    #[test]
    fn long_path_prefix() {
        if cfg!(windows) {
            assert_eq!(
                long_path(Path::new(r"C:\datos\Año 2019")),
                Path::new(r"\\?\C:\datos\Año 2019")
            );
            assert_eq!(
                long_path(Path::new(r"\\nas\share\fotos")),
                Path::new(r"\\?\UNC\nas\share\fotos")
            );
            assert_eq!(
                long_path(Path::new(r"\\?\C:\already")),
                Path::new(r"\\?\C:\already")
            );
        } else {
            assert_eq!(long_path(Path::new("/datos/x")), Path::new("/datos/x"));
        }
        assert_eq!(long_path(Path::new("relative")), Path::new("relative"));
    }
}
