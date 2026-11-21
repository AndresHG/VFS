//! Change cursors (VFS-9): a position in a volume's change history, so a start replays what
//! changed while the app was closed instead of re-walking every root.
//!
//! A root's stored cursor (`meta` key `cursor/<root id>`) means every change made before it is
//! in the index or pending in `dirs`. Windows: `usn:<volume serial>:<journal id>:<usn>`, read
//! with the unprivileged USN read (no admin) and keyed on each record's parent directory,
//! resolved by file id (docs/research/windows-io.md §4.2). macOS:
//! `fsevents:<event store uuid>:<event id>`, replayed through a one-shot FSEvents stream with
//! `sinceWhen` (docs/research/prior-art.md §2.8). Anything else has no history: every start
//! walks, as before.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};

use crate::error::Result;

/// Directories that changed after a cursor, and the cursor to store once they are marked;
/// `Err` says why the history cannot be trusted (VFS-9 R3).
pub(crate) type Replayed = std::result::Result<(Vec<PathBuf>, String), String>;

/// Source of change cursors; `OsHistory` in the app, a fake in tests.
pub(crate) trait History {
    /// The cursor of now for the volume holding `root`, or `None` when it keeps no usable
    /// history (another file system, a network drive, no journal).
    fn current(&self, root: &Path) -> Option<String>;
    /// The directories under `root`'s volume that changed after `cursor`. Paths may lie
    /// outside `root`; the caller filters them.
    fn replay(&self, root: &Path, cursor: &str) -> Replayed;
}

/// The operating system's change history.
pub(crate) struct OsHistory;

impl History for OsHistory {
    fn current(&self, root: &Path) -> Option<String> {
        imp::current(root)
    }

    fn replay(&self, root: &Path, cursor: &str) -> Replayed {
        imp::replay(root, cursor)
    }
}

fn key(root_id: i64) -> String {
    format!("cursor/{root_id}")
}

pub(crate) fn load(conn: &Connection, root_id: i64) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT value FROM meta WHERE key = ?1")?
        .query_row([key(root_id)], |r| r.get(0))
        .optional()?)
}

/// Stores `cursor` for `root_id`, or deletes it when `None`.
pub(crate) fn store(conn: &Connection, root_id: i64, cursor: Option<&str>) -> Result<()> {
    match cursor {
        Some(c) => conn
            .prepare_cached("INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)")?
            .execute((key(root_id), c))?,
        None => conn
            .prepare_cached("DELETE FROM meta WHERE key = ?1")?
            .execute([key(root_id)])?,
    };
    Ok(())
}

/// Parent file reference numbers of the `USN_RECORD_V2` records in `bytes` (the output of a
/// journal read after its leading next-USN). `Err` carries the major version of a record that
/// is not version 2 (ReFS writes version 3, with 128-bit ids): the replay is then lost.
#[cfg(any(windows, test))]
fn parent_refs(
    bytes: &[u8],
    out: &mut std::collections::HashSet<u64>,
) -> std::result::Result<(), u16> {
    let mut off = 0usize;
    while off + 24 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap_or_default()) as usize;
        let major = u16::from_le_bytes([bytes[off + 4], bytes[off + 5]]);
        if major != 2 {
            return Err(major);
        }
        out.insert(u64::from_le_bytes(
            bytes[off + 16..off + 24].try_into().unwrap_or_default(),
        ));
        if len == 0 {
            break;
        }
        off += len;
    }
    Ok(())
}

/// `usn:<serial>:<journal id>:<usn>`, the Windows cursor `parse_usn` reads.
#[cfg(any(windows, test))]
fn usn_cursor(serial: u32, journal: u64, usn: i64) -> String {
    format!("usn:{serial:08x}:{journal:016x}:{usn}")
}

/// Splits `usn:<serial>:<journal id>:<usn>`.
#[cfg(any(windows, test))]
fn parse_usn(cursor: &str) -> Option<(u32, u64, i64)> {
    let mut parts = cursor.strip_prefix("usn:")?.split(':');
    let serial = u32::from_str_radix(parts.next()?, 16).ok()?;
    let journal = u64::from_str_radix(parts.next()?, 16).ok()?;
    let usn = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((serial, journal, usn))
}

/// `fsevents:<uuid>:<event id>`, the macOS cursor `parse_fsevents` reads.
#[cfg(any(target_os = "macos", test))]
fn fsevents_cursor(uuid: &str, id: u64) -> String {
    format!("fsevents:{uuid}:{id}")
}

/// Splits `fsevents:<uuid>:<event id>`.
#[cfg(any(target_os = "macos", test))]
fn parse_fsevents(cursor: &str) -> Option<(&str, u64)> {
    let (uuid, id) = cursor.strip_prefix("fsevents:")?.rsplit_once(':')?;
    Some((uuid, id.parse().ok()?))
}

/// FSEvents event flags that mean the history no longer covers every change below the root
/// (VFS-9 R5): rescan requests, dropped or wrapped ids, the root itself moved, a mount below.
#[cfg(any(target_os = "macos", test))]
const FSE_LOST: u32 = 0x01 | 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80;
#[cfg(any(target_os = "macos", test))]
const FSE_HISTORY_DONE: u32 = 0x10;

#[cfg(target_os = "macos")]
mod imp {
    //! A one-shot FSEvents stream on a private dispatch queue, stopped at `HistoryDone`.

    use std::ffi::{c_char, c_void, CStr};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};

    use super::{fsevents_cursor, parse_fsevents, Replayed, FSE_HISTORY_DONE, FSE_LOST};

    /// How long the history may take to arrive before the replay counts as lost (VFS-9 R5).
    const HISTORY_TIMEOUT: Duration = Duration::from_secs(60);
    const UTF8: u32 = 0x0800_0100;
    const CREATE_NO_DEFER: u32 = 0x02;
    const CREATE_WATCH_ROOT: u32 = 0x04;

    type Ref = *const c_void;
    type Callback = extern "C" fn(Ref, *mut c_void, usize, *mut c_void, *const u32, *const u64);

    #[repr(C)]
    struct Context {
        version: isize,
        info: *mut c_void,
        retain: Ref,
        release: Ref,
        copy_description: Ref,
    }

    #[repr(C)]
    struct ArrayCallBacks {
        _opaque: [u8; 0],
    }

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn FSEventsGetCurrentEventId() -> u64;
        fn FSEventsCopyUUIDForDevice(dev: libc::dev_t) -> Ref;
        fn FSEventStreamCreate(
            alloc: Ref,
            callback: Callback,
            context: *const Context,
            paths: Ref,
            since: u64,
            latency: f64,
            flags: u32,
        ) -> Ref;
        fn FSEventStreamSetDispatchQueue(stream: Ref, queue: *mut c_void);
        fn FSEventStreamStart(stream: Ref) -> u8;
        fn FSEventStreamStop(stream: Ref);
        fn FSEventStreamInvalidate(stream: Ref);
        fn FSEventStreamRelease(stream: Ref);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFTypeArrayCallBacks: ArrayCallBacks;
        fn CFStringCreateWithBytes(
            alloc: Ref,
            bytes: *const u8,
            len: isize,
            enc: u32,
            ext: u8,
        ) -> Ref;
        fn CFArrayCreate(
            alloc: Ref,
            values: *const Ref,
            n: isize,
            cb: *const ArrayCallBacks,
        ) -> Ref;
        fn CFUUIDCreateString(alloc: Ref, uuid: Ref) -> Ref;
        fn CFStringGetCString(s: Ref, buf: *mut c_char, size: isize, enc: u32) -> u8;
        fn CFRelease(r: Ref);
    }

    extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: Ref) -> *mut c_void;
        fn dispatch_release(object: *mut c_void);
    }

    /// The event-store UUID of the device holding `root`; `None` when it keeps no history
    /// (network and some removable volumes).
    fn store_uuid(root: &Path) -> Option<String> {
        let dev = std::fs::metadata(root).ok()?.dev();
        // SAFETY: plain C calls; every returned object is released below.
        unsafe {
            let uuid = FSEventsCopyUUIDForDevice(dev as libc::dev_t);
            if uuid.is_null() {
                return None;
            }
            let s = CFUUIDCreateString(std::ptr::null(), uuid);
            CFRelease(uuid);
            if s.is_null() {
                return None;
            }
            let mut buf = [0 as c_char; 64];
            let ok = CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8);
            CFRelease(s);
            (ok != 0).then(|| CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }

    pub(super) fn current(root: &Path) -> Option<String> {
        let uuid = store_uuid(root)?;
        // SAFETY: plain C call.
        let id = unsafe { FSEventsGetCurrentEventId() };
        Some(fsevents_cursor(&uuid, id))
    }

    #[derive(Default)]
    struct Collected {
        paths: Vec<PathBuf>,
        lost: Option<String>,
        done: bool,
    }

    struct Shared {
        state: Mutex<Collected>,
        cond: Condvar,
    }

    extern "C" fn on_events(
        _stream: Ref,
        info: *mut c_void,
        n: usize,
        paths: *mut c_void,
        flags: *const u32,
        _ids: *const u64,
    ) {
        // SAFETY: `info` is the `Shared` that `replay` keeps alive until the stream is
        // invalidated; without `UseCFTypes`, `paths` is an array of `n` C strings and `flags`
        // has `n` entries.
        let (shared, paths, flags) = unsafe {
            (
                &*(info as *const Shared),
                std::slice::from_raw_parts(paths as *const *const c_char, n),
                std::slice::from_raw_parts(flags, n),
            )
        };
        let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        for (&path, &flag) in paths.iter().zip(flags) {
            if flag & FSE_HISTORY_DONE != 0 {
                st.done = true;
                continue;
            }
            if st.done {
                continue;
            }
            // SAFETY: each path is a NUL-terminated C string owned by FSEvents.
            let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
            let path = PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
            if flag & FSE_LOST != 0 && st.lost.is_none() {
                st.lost = Some(format!("history flag {flag:#x} at {}", path.display()));
            }
            st.paths.push(path);
        }
        shared.cond.notify_all();
    }

    pub(super) fn replay(root: &Path, cursor: &str) -> Replayed {
        let (uuid, since) = parse_fsevents(cursor).ok_or("cursor of another kind")?;
        if store_uuid(root).as_deref() != Some(uuid) {
            return Err("event store changed".into());
        }
        // SAFETY: plain C call. Taken before the stream exists, so every event up to it is
        // part of the history the stream delivers before `HistoryDone`.
        let now = unsafe { FSEventsGetCurrentEventId() };
        if since > now {
            return Err("cursor ahead of the event store".into());
        }
        let shared = Shared {
            state: Mutex::new(Collected::default()),
            cond: Condvar::new(),
        };
        let bytes = root.as_os_str().as_bytes();
        // SAFETY: every object created here is released before returning; `shared` outlives
        // the stream, which is stopped and invalidated (no callback runs after that) first.
        unsafe {
            let path = CFStringCreateWithBytes(
                std::ptr::null(),
                bytes.as_ptr(),
                bytes.len() as isize,
                UTF8,
                0,
            );
            if path.is_null() {
                return Err("root path not representable".into());
            }
            let array = CFArrayCreate(std::ptr::null(), &path, 1, &kCFTypeArrayCallBacks);
            CFRelease(path);
            let ctx = Context {
                version: 0,
                info: &shared as *const Shared as *mut c_void,
                retain: std::ptr::null(),
                release: std::ptr::null(),
                copy_description: std::ptr::null(),
            };
            let stream = FSEventStreamCreate(
                std::ptr::null(),
                on_events,
                &ctx,
                array,
                since,
                0.0,
                CREATE_NO_DEFER | CREATE_WATCH_ROOT,
            );
            CFRelease(array);
            if stream.is_null() {
                return Err("cannot create the history stream".into());
            }
            let queue = dispatch_queue_create(c"vfs.fsevents.replay".as_ptr(), std::ptr::null());
            FSEventStreamSetDispatchQueue(stream, queue);
            let started = FSEventStreamStart(stream) != 0;
            let deadline = Instant::now() + HISTORY_TIMEOUT;
            if started {
                let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                while !st.done && st.lost.is_none() {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    st = shared
                        .cond
                        .wait_timeout(st, left)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
                drop(st);
                FSEventStreamStop(stream);
            }
            FSEventStreamInvalidate(stream);
            FSEventStreamRelease(stream);
            dispatch_release(queue);
            if !started {
                return Err("cannot start the history stream".into());
            }
        }
        let st = shared.state.into_inner().unwrap_or_else(|e| e.into_inner());
        if let Some(lost) = st.lost {
            return Err(lost);
        }
        if !st.done {
            return Err("history did not finish in time".into());
        }
        Ok((st.paths, fsevents_cursor(uuid, now)))
    }
}

#[cfg(windows)]
mod imp {
    //! `FSCTL_QUERY_USN_JOURNAL` and the undocumented `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` on
    //! the volume root opened as a directory with traverse access, which a standard user may
    //! do on Windows 10 1703 and later (docs/research/windows-io.md §4.1).

    use std::collections::HashSet;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetDriveTypeW, GetFinalPathNameByHandleW, GetVolumeInformationW,
        GetVolumePathNameW, OpenFileById, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_DESCRIPTOR,
        FILE_ID_DESCRIPTOR_0, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE,
        OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    use super::{parent_refs, parse_usn, usn_cursor, Replayed};

    const FSCTL_QUERY_USN_JOURNAL: u32 = 0x0009_00F4;
    const FSCTL_READ_UNPRIVILEGED_USN_JOURNAL: u32 = 0x0009_03AB;
    const READ_BYTES: usize = 64 * 1024;

    /// A kernel handle closed on drop.
    struct Owned(HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a valid handle owned by this value.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    struct Volume {
        handle: Owned,
        serial: u32,
    }

    /// The volume holding `root`, when it is a local NTFS drive (VFS-9 R4).
    fn open_volume(root: &Path) -> std::result::Result<Volume, String> {
        let root_w = wide(root.as_os_str());
        let mut vol = [0u16; 1024];
        // SAFETY: `root_w` is NUL-terminated and `vol` holds `vol.len()` characters.
        if unsafe { GetVolumePathNameW(root_w.as_ptr(), vol.as_mut_ptr(), vol.len() as u32) } == 0 {
            return Err(format!("volume of {}: {}", root.display(), last_error()));
        }
        // SAFETY: `vol` is NUL-terminated.
        let kind = unsafe { GetDriveTypeW(vol.as_ptr()) };
        if !super::local_drive(kind) {
            return Err(format!("not a local drive (type {kind})"));
        }
        let mut serial = 0u32;
        let mut fs = [0u16; 64];
        // SAFETY: `vol` is NUL-terminated; the output buffers have the lengths passed.
        let ok = unsafe {
            GetVolumeInformationW(
                vol.as_ptr(),
                std::ptr::null_mut(),
                0,
                &mut serial,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        if ok == 0 {
            return Err(format!("volume information: {}", last_error()));
        }
        let fs_name = String::from_utf16_lossy(&fs[..fs.iter().position(|&c| c == 0).unwrap_or(0)]);
        if !super::ntfs(&fs_name) {
            return Err(format!("file system {fs_name} keeps no usable journal"));
        }
        // SAFETY: `vol` is NUL-terminated; traverse access on a directory needs no admin.
        let h = unsafe {
            CreateFileW(
                vol.as_ptr(),
                FILE_TRAVERSE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(format!("open volume: {}", last_error()));
        }
        Ok(Volume {
            handle: Owned(h),
            serial,
        })
    }

    fn last_error() -> std::io::Error {
        std::io::Error::last_os_error()
    }

    /// `USN_JOURNAL_DATA_V0`: journal id, first and next USN.
    fn query(vol: &Volume) -> std::result::Result<(u64, i64, i64), String> {
        let mut out = [0u64; 7];
        let mut got = 0u32;
        // SAFETY: `out` is 56 bytes, the size of `USN_JOURNAL_DATA_V0`.
        let ok = unsafe {
            DeviceIoControl(
                vol.handle.0,
                FSCTL_QUERY_USN_JOURNAL,
                std::ptr::null(),
                0,
                out.as_mut_ptr().cast(),
                56,
                &mut got,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || got < 24 {
            return Err(format!("query journal: {}", last_error()));
        }
        Ok((out[0], out[1] as i64, out[2] as i64))
    }

    pub(super) fn current(root: &Path) -> Option<String> {
        let vol = open_volume(root)
            .inspect_err(|e| log::debug!("no change cursor for {}: {e}", root.display()))
            .ok()?;
        let (journal, _, next) = query(&vol).ok()?;
        Some(usn_cursor(vol.serial, journal, next))
    }

    /// The current path of the directory with file reference `frn`, opened with no access
    /// rights (VFS-9 R9); `None` when it is gone or the user may not open it.
    fn path_of(vol: &Volume, frn: u64) -> Option<PathBuf> {
        let id = FILE_ID_DESCRIPTOR {
            dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
            Type: 0,
            Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: frn as i64 },
        };
        // SAFETY: `id` is a valid descriptor; no access rights are requested.
        let h = unsafe {
            OpenFileById(
                vol.handle.0,
                &id,
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                FILE_FLAG_BACKUP_SEMANTICS,
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let h = Owned(h);
        let mut buf = vec![0u16; 32 * 1024];
        // SAFETY: `buf` holds `buf.len()` characters; flags 0 = normalized DOS path.
        let n = unsafe { GetFinalPathNameByHandleW(h.0, buf.as_mut_ptr(), buf.len() as u32, 0) };
        if n == 0 || n as usize >= buf.len() {
            return None;
        }
        let path = PathBuf::from(std::ffi::OsString::from_wide(&buf[..n as usize]));
        Some(dunce::simplified(&path).to_path_buf())
    }

    pub(super) fn replay(root: &Path, cursor: &str) -> Replayed {
        let (serial, journal, mut start) = parse_usn(cursor).ok_or("cursor of another kind")?;
        let vol = open_volume(root)?;
        if vol.serial != serial {
            return Err("another volume".into());
        }
        let (id, first, next) = query(&vol)?;
        if id != journal {
            return Err("journal recreated".into());
        }
        if start < first {
            return Err("journal trimmed past the cursor".into());
        }
        if start > next {
            return Err("cursor ahead of the journal".into());
        }
        let mut parents = HashSet::new();
        // `u64` elements keep the buffer aligned for the records.
        let mut buf = vec![0u64; READ_BYTES / 8];
        while start < next {
            // READ_USN_JOURNAL_DATA_V0: start USN, reason mask, return only on close,
            // timeout, bytes to wait for, journal id.
            let mut input = [0u8; 40];
            input[0..8].copy_from_slice(&start.to_le_bytes());
            input[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
            input[32..40].copy_from_slice(&journal.to_le_bytes());
            let mut got = 0u32;
            // SAFETY: `input` is a READ_USN_JOURNAL_DATA_V0; `buf` holds READ_BYTES bytes.
            let ok = unsafe {
                DeviceIoControl(
                    vol.handle.0,
                    FSCTL_READ_UNPRIVILEGED_USN_JOURNAL,
                    input.as_ptr().cast(),
                    input.len() as u32,
                    buf.as_mut_ptr().cast(),
                    READ_BYTES as u32,
                    &mut got,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(format!("read journal: {}", last_error()));
            }
            // SAFETY: the call wrote `got` (<= READ_BYTES) bytes into `buf`.
            let bytes =
                unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), got as usize) };
            if bytes.len() < 8 {
                break;
            }
            parent_refs(&bytes[8..], &mut parents)
                .map_err(|v| format!("journal record version {v}"))?;
            let after = i64::from_le_bytes(bytes[0..8].try_into().unwrap_or_default());
            if bytes.len() == 8 || after <= start {
                break;
            }
            start = after;
        }
        let paths = parents
            .into_iter()
            .filter_map(|frn| path_of(&vol, frn))
            .collect();
        Ok((paths, usn_cursor(serial, journal, next)))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use std::path::Path;

    use super::Replayed;

    pub(super) fn current(_root: &Path) -> Option<String> {
        None
    }

    pub(super) fn replay(_root: &Path, _cursor: &str) -> Replayed {
        Err("no change history on this system".into())
    }
}

/// Whether a `GetDriveTypeW` kind is a local disk (VFS-9 R4): fixed (3) or removable (2);
/// network (4), CD-ROM (5), RAM disk (6) and unknown kinds keep no journal VFS can read.
#[cfg(any(windows, test))]
fn local_drive(kind: u32) -> bool {
    matches!(kind, 2 | 3)
}

/// Whether a volume's file system keeps a journal VFS can read (VFS-9 R4): NTFS only.
#[cfg(any(windows, test))]
fn ntfs(fs_name: &str) -> bool {
    fs_name == "NTFS"
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn record(major: u16, parent: u64) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0..4].copy_from_slice(&64u32.to_le_bytes());
        r[4..6].copy_from_slice(&major.to_le_bytes());
        r[8..16].copy_from_slice(&7u64.to_le_bytes());
        r[16..24].copy_from_slice(&parent.to_le_bytes());
        r
    }

    /// VFS-9 AC-6: version 2 records yield their parent directory; any other version (ReFS
    /// writes 3) makes the replay lost.
    #[test]
    fn usn_records_yield_parents_and_refuse_other_versions() {
        let mut bytes = record(2, 11);
        bytes.extend(record(2, 12));
        bytes.extend(record(2, 11));
        let mut out = HashSet::new();
        assert_eq!(parent_refs(&bytes, &mut out), Ok(()));
        assert_eq!(out, HashSet::from([11, 12]));
        bytes.extend(record(3, 13));
        assert_eq!(parent_refs(&bytes, &mut HashSet::new()), Err(3));
    }

    /// VFS-9 AC-6, AC-3: only NTFS on a local drive keeps a usable journal; cursors of the other platform or
    /// garbled ones do not parse.
    #[test]
    fn cursor_kinds_and_file_systems() {
        assert!(ntfs("NTFS"));
        assert!(local_drive(2) && local_drive(3));
        for kind in [0, 1, 4, 5, 6] {
            assert!(!local_drive(kind), "drive type {kind}");
        }
        for fs in ["FAT32", "exFAT", "ReFS", ""] {
            assert!(!ntfs(fs), "{fs}");
        }
        assert_eq!(
            parse_usn("usn:1a2b3c4d:00000000000000ff:4096"),
            Some((0x1a2b_3c4d, 0xff, 4096))
        );
        assert_eq!(parse_usn(&usn_cursor(7, 9, -1)), Some((7, 9, -1)));
        assert_eq!(parse_fsevents(&fsevents_cursor("AB", 3)), Some(("AB", 3)));
        assert_eq!(parse_usn("usn:1a2b3c4d:ff"), None);
        assert_eq!(parse_usn("fsevents:ABC:5"), None);
        assert_eq!(parse_fsevents("fsevents:AB-CD:42"), Some(("AB-CD", 42)));
        assert_eq!(parse_fsevents("usn:1:2:3"), None);
        assert_eq!(parse_fsevents("fsevents:AB-CD:x"), None);
    }

    /// VFS-9 AC-6: the FSEvents flags that mean lost history, and `HistoryDone`, are distinct.
    #[test]
    fn fsevents_lost_flags() {
        for (flag, lost) in [
            (0x01, true),     // MustScanSubDirs
            (0x02, true),     // UserDropped
            (0x04, true),     // KernelDropped
            (0x08, true),     // EventIdsWrapped
            (0x20, true),     // RootChanged
            (0x40, true),     // Mount
            (0x80, true),     // Unmount
            (0x100, false),   // ItemCreated
            (0x10000, false), // ItemIsDir
        ] {
            assert_eq!(flag & FSE_LOST != 0, lost, "{flag:#x}");
        }
        assert_eq!(FSE_LOST & FSE_HISTORY_DONE, 0);
    }
}
