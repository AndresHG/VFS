//! Listing one directory without opening files, with the exclusion list (VFS-1 AC-4, AC-8).
//! Owned by task T3a.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::UnreadableReason;

pub(crate) enum EntryKind {
    File {
        size: u64,
        mtime: i64,
    },
    Dir {
        mtime: i64,
    },
    /// A symbolic link or junction: recorded as a link row, never entered (VFS-18 R1, D5).
    Link {
        mtime: i64,
    },
}

pub(crate) struct Entry {
    /// `to_str()` of the name, or its lossy form when not valid Unicode.
    pub name: String,
    pub lossy: bool,
    pub kind: EntryKind,
    /// A file whose data is only in the cloud (VFS-25 R1); never set on folders or links.
    pub cloud_only: bool,
}

/// Exact names (compared ignoring ASCII case) that are never indexed, nor watched.
const EXCLUDED: &[&str] = &[
    "$RECYCLE.BIN",
    "System Volume Information",
    ".Trashes",
    ".Trash",
    ".Spotlight-V100",
    ".fseventsd",
    ".DocumentRevisions-V100",
    ".TemporaryItems",
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    // Synology Drive client and DSM metadata, recycle bin and snapshots.
    ".SynologyWorkingDirectory",
    "@eaDir",
    "#recycle",
    "#snapshot",
];

/// System, sync-client and temporary names never indexed, and whose watcher events are
/// dropped (`$RECYCLE.BIN`, `.DS_Store`, `._*`, `~$*`, `.~lock.*#`, `~*.tmp`, ...). The one
/// list both the walker and the watcher use (VFS-1 I8).
pub(crate) fn is_excluded(name: &str) -> bool {
    let tmp_suffix = name.len() >= 4
        && name.is_char_boundary(name.len() - 4)
        && name[name.len() - 4..].eq_ignore_ascii_case(".tmp");
    name.starts_with("._")
        || name.starts_with("~$")
        || (name.starts_with('~') && tmp_suffix)
        || (name.starts_with(".~lock.") && name.ends_with('#'))
        || EXCLUDED.iter().any(|e| e.eq_ignore_ascii_case(name))
}

/// Unix seconds of a timestamp, floored (negative before 1970, so 1969-12-31 23:59:59.5
/// is -1, not 0); 0 when the platform has none.
pub(crate) fn unix_secs(t: std::io::Result<SystemTime>) -> i64 {
    match t {
        Ok(t) => match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => {
                let d = e.duration();
                -(d.as_secs() as i64) - i64::from(d.subsec_nanos() > 0)
            }
        },
        Err(_) => 0,
    }
}

/// True when `root` or a directory of `rel` below it (`/`-separated) is now a symbolic link,
/// junction or other name-surrogate reparse point (`FileType::is_symlink`). An indexed
/// directory replaced by one is treated as gone: the walk never follows a link, even where a
/// real directory used to be.
pub(crate) fn behind_link(root: &Path, rel: &str) -> bool {
    let is_link = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink());
    let mut p = root.to_path_buf();
    if is_link(&p) {
        return true;
    }
    for c in rel.split('/').filter(|c| !c.is_empty()) {
        p.push(c);
        if is_link(&p) {
            return true;
        }
    }
    false
}

/// Entries of `dir` from `read_dir` metadata; symlinks, junctions and other name-surrogate
/// reparse points are listed as links and never followed (VFS-18 R1); excluded names and
/// `skip` (the app's data directory) are skipped.
///
/// No file is opened: on Windows type and metadata come from the enumeration record, on
/// Unix from `lstat`. `FileType::is_symlink` is true for symlinks and for Windows
/// name-surrogate reparse points (junctions, mount points), so those are never followed;
/// cloud placeholders are not name surrogates and are listed from their metadata without
/// being downloaded. Entries that vanish or fail while listing are skipped, except that an
/// entry whose metadata is denied (a Unix folder readable but not searchable) fails the
/// whole listing, so the folder is retried like an unlistable one instead of being marked
/// clean without those entries.
pub(crate) fn list_dir(dir: &Path, skip: Option<&Path>) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let os_name = entry.file_name();
        let (name, lossy) = match os_name.to_str() {
            Some(s) => (s.to_owned(), false),
            None => (os_name.to_string_lossy().into_owned(), true),
        };
        if is_excluded(&name) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Err(e),
            Err(_) => continue,
        };
        let mtime = unix_secs(meta.modified());
        // `DirEntry::metadata` does not follow links (lstat on Unix, the enumeration record
        // on Windows), so this is the link's own time.
        let kind = if file_type.is_symlink() {
            EntryKind::Link { mtime }
        } else if file_type.is_dir() {
            if skip.is_some_and(|s| entry.path() == s) {
                continue;
            }
            EntryKind::Dir { mtime }
        } else if file_type.is_file() {
            EntryKind::File {
                size: meta.len(),
                mtime,
            }
        } else {
            continue;
        };
        let cloud_only = matches!(kind, EntryKind::File { .. }) && cloud_only(&meta);
        out.push(Entry {
            name,
            lossy,
            kind,
            cloud_only,
        });
    }
    Ok(out)
}

/// Attribute bits of a cloud placeholder whose data is not on the disk (VFS-25 R1):
/// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`, `FILE_ATTRIBUTE_RECALL_ON_OPEN`,
/// `FILE_ATTRIBUTE_OFFLINE`.
#[cfg_attr(not(windows), allow(dead_code))]
const CLOUD_ONLY_ATTRIBUTES: u32 = 0x0040_0000 | 0x0004_0000 | 0x0000_1000;

/// True when Windows file attributes mark a cloud-only file (VFS-25 R1).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn cloud_only_attributes(attributes: u32) -> bool {
    attributes & CLOUD_ONLY_ATTRIBUTES != 0
}

/// Whether listing metadata marks a cloud-only file: on Windows from the enumeration
/// record's attributes (no extra call, no open); never elsewhere (VFS-25 non-goal).
fn cloud_only(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        cloud_only_attributes(meta.file_attributes())
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// Every `ERROR_CLOUD_FILE_*` code of `winerror.h` (Windows SDK, windows-sys 0.61): a listing
/// that fails with one of them is a cloud-only folder (VFS-10 R3). Checked against
/// windows-sys on Windows by `tests::cloud_codes_match_windows_sys`.
const CLOUD_FILE_ERRORS: [i32; 32] = [
    358, 362, 363, 364, 365, 366, 374, 375, 377, 378, 379, 380, 381, 382, 383, 386, 387, 388, 389,
    390, 391, 392, 393, 394, 395, 396, 397, 398, 404, 426, 434, 475,
];

/// Windows codes of a device or network that is absent or not answering (VFS-10 R3):
/// `ERROR_NOT_READY`, `ERROR_BAD_NETPATH`, `ERROR_DEV_NOT_EXIST`, `ERROR_UNEXP_NET_ERR`,
/// `ERROR_NETNAME_DELETED`, `ERROR_BAD_NET_NAME`, `ERROR_SEM_TIMEOUT`,
/// `ERROR_NETWORK_UNREACHABLE`.
const WINDOWS_UNAVAILABLE: [i32; 8] = [21, 53, 55, 59, 64, 67, 121, 1231];

/// Why a listing failed, from the error alone (VFS-10 R3). `windows` says whether
/// `raw_os_error` is a Win32 code or an `errno`, so both tables are tested everywhere.
pub(crate) fn unreadable_reason(e: &std::io::Error, windows: bool) -> UnreadableReason {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::PermissionDenied => return UnreadableReason::NoPermission,
        K::TimedOut
        | K::HostUnreachable
        | K::NetworkUnreachable
        | K::NetworkDown
        | K::StaleNetworkFileHandle => return UnreadableReason::Unavailable,
        _ => {}
    }
    let Some(code) = e.raw_os_error() else {
        return UnreadableReason::DiskError;
    };
    if windows {
        if CLOUD_FILE_ERRORS.contains(&code) {
            return UnreadableReason::CloudOnly;
        }
        if WINDOWS_UNAVAILABLE.contains(&code) {
            return UnreadableReason::Unavailable;
        }
    } else if unix_unavailable(code) {
        return UnreadableReason::Unavailable;
    }
    UnreadableReason::DiskError
}

/// `ENXIO`, `ENODEV`, `EHOSTDOWN` (VFS-10 R3).
#[cfg(unix)]
fn unix_unavailable(code: i32) -> bool {
    [libc::ENXIO, libc::ENODEV, libc::EHOSTDOWN].contains(&code)
}

#[cfg(not(unix))]
fn unix_unavailable(_: i32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-25 AC-1: each of the three placeholder bits marks a file cloud-only; plain,
    /// hidden, archive, read-only and reparse-point words do not; the stored flag is bit 2.
    #[test]
    fn cloud_only_attribute_bits() {
        for bit in [0x0040_0000, 0x0004_0000, 0x1000] {
            assert!(cloud_only_attributes(bit | 0x20), "{bit:#x}");
        }
        // FILE_ATTRIBUTE_NORMAL, HIDDEN, ARCHIVE, READONLY, REPARSE_POINT.
        for word in [0x80, 0x2, 0x20, 0x1, 0x400, 0x0] {
            assert!(!cloud_only_attributes(word), "{word:#x}");
        }
        use crate::db::ingest::{FLAG_CLOUD_ONLY, FLAG_LINK};
        assert_eq!(FLAG_CLOUD_ONLY, 1 << 2);
        assert_eq!(
            FLAG_CLOUD_ONLY & (FLAG_LINK | 1),
            0,
            "distinct from the link and lossy bits"
        );
    }

    /// VFS-10 AC-3: each row of the reason table maps to its reason.
    #[test]
    fn listing_errors_map_to_reasons() {
        use std::io::{Error, ErrorKind as K};
        use UnreadableReason as R;
        let kind = |k| Error::from(k);
        for windows in [true, false] {
            assert_eq!(
                unreadable_reason(&kind(K::PermissionDenied), windows),
                R::NoPermission
            );
            for k in [
                K::TimedOut,
                K::HostUnreachable,
                K::NetworkUnreachable,
                K::NetworkDown,
            ] {
                assert_eq!(
                    unreadable_reason(&kind(k), windows),
                    R::Unavailable,
                    "{k:?}"
                );
            }
            assert_eq!(unreadable_reason(&kind(K::Other), windows), R::DiskError);
        }
        let raw = Error::from_raw_os_error;
        for code in WINDOWS_UNAVAILABLE {
            assert_eq!(
                unreadable_reason(&raw(code), true),
                R::Unavailable,
                "{code}"
            );
        }
        // The first and last cloud codes, and one between.
        for code in [358, 395, 475] {
            assert_eq!(unreadable_reason(&raw(code), true), R::CloudOnly, "{code}");
        }
        // 367 (ERROR_CHILD_PROCESS_BLOCKED) lies inside the range but is no cloud error;
        // 1117 is ERROR_IO_DEVICE.
        for code in [367, 1117] {
            assert_eq!(unreadable_reason(&raw(code), true), R::DiskError, "{code}");
        }
        #[cfg(unix)]
        {
            for code in [libc::ENXIO, libc::ENODEV, libc::EHOSTDOWN] {
                assert_eq!(
                    unreadable_reason(&raw(code), false),
                    R::Unavailable,
                    "{code}"
                );
            }
            assert_eq!(unreadable_reason(&raw(libc::EIO), false), R::DiskError);
        }
    }

    /// VFS-10 R3: the cloud-file codes are exactly windows-sys's `ERROR_CLOUD_FILE_*`.
    #[cfg(windows)]
    #[test]
    fn cloud_codes_match_windows_sys() {
        use windows_sys::Win32::Foundation::*;
        let named = [
            ERROR_CLOUD_FILE_SYNC_ROOT_METADATA_CORRUPT,
            ERROR_CLOUD_FILE_PROVIDER_NOT_RUNNING,
            ERROR_CLOUD_FILE_METADATA_CORRUPT,
            ERROR_CLOUD_FILE_METADATA_TOO_LARGE,
            ERROR_CLOUD_FILE_PROPERTY_BLOB_TOO_LARGE,
            ERROR_CLOUD_FILE_PROPERTY_BLOB_CHECKSUM_MISMATCH,
            ERROR_CLOUD_FILE_TOO_MANY_PROPERTY_BLOBS,
            ERROR_CLOUD_FILE_PROPERTY_VERSION_NOT_SUPPORTED,
            ERROR_CLOUD_FILE_NOT_IN_SYNC,
            ERROR_CLOUD_FILE_ALREADY_CONNECTED,
            ERROR_CLOUD_FILE_NOT_SUPPORTED,
            ERROR_CLOUD_FILE_INVALID_REQUEST,
            ERROR_CLOUD_FILE_READ_ONLY_VOLUME,
            ERROR_CLOUD_FILE_CONNECTED_PROVIDER_ONLY,
            ERROR_CLOUD_FILE_VALIDATION_FAILED,
            ERROR_CLOUD_FILE_AUTHENTICATION_FAILED,
            ERROR_CLOUD_FILE_INSUFFICIENT_RESOURCES,
            ERROR_CLOUD_FILE_NETWORK_UNAVAILABLE,
            ERROR_CLOUD_FILE_UNSUCCESSFUL,
            ERROR_CLOUD_FILE_NOT_UNDER_SYNC_ROOT,
            ERROR_CLOUD_FILE_IN_USE,
            ERROR_CLOUD_FILE_PINNED,
            ERROR_CLOUD_FILE_REQUEST_ABORTED,
            ERROR_CLOUD_FILE_PROPERTY_CORRUPT,
            ERROR_CLOUD_FILE_ACCESS_DENIED,
            ERROR_CLOUD_FILE_INCOMPATIBLE_HARDLINKS,
            ERROR_CLOUD_FILE_PROPERTY_LOCK_CONFLICT,
            ERROR_CLOUD_FILE_REQUEST_CANCELED,
            ERROR_CLOUD_FILE_PROVIDER_TERMINATED,
            ERROR_CLOUD_FILE_REQUEST_TIMEOUT,
            ERROR_CLOUD_FILE_DEHYDRATION_DISALLOWED,
            ERROR_CLOUD_FILE_US_MESSAGE_TIMEOUT,
        ];
        assert_eq!(named.map(|c| c as i32), CLOUD_FILE_ERRORS);
        let unavailable = [
            ERROR_NOT_READY,
            ERROR_BAD_NETPATH,
            ERROR_DEV_NOT_EXIST,
            ERROR_UNEXP_NET_ERR,
            ERROR_NETNAME_DELETED,
            ERROR_BAD_NET_NAME,
            ERROR_SEM_TIMEOUT,
            ERROR_NETWORK_UNREACHABLE,
        ];
        assert_eq!(unavailable.map(|c| c as i32), WINDOWS_UNAVAILABLE);
    }

    /// VFS-1 AC-4, I8: system, sync-client and temporary names are excluded, ordinary names
    /// are not.
    #[test]
    fn exclusions() {
        for n in [
            "$RECYCLE.BIN",
            "$Recycle.Bin",
            "System Volume Information",
            ".DS_Store",
            "thumbs.db",
            "Desktop.ini",
            "._IMG_0001.JPG",
            "~$informe.docx",
            ".fseventsd",
            ".SynologyWorkingDirectory",
            "@eaDir",
            "#recycle",
            "#Snapshot",
            ".~lock.informe.odt#",
            "~WRL0001.tmp",
            "~DF1234.TMP",
        ] {
            assert!(is_excluded(n), "{n}");
        }
        for n in [
            "Notaría.pdf",
            ".bashrc",
            "Trash",
            "informe~$.docx",
            "_x",
            "recycle",
            "copia.tmp",
            "~notas.txt",
            ".~lock.informe.odt",
        ] {
            assert!(!is_excluded(n), "{n}");
        }
    }
}
