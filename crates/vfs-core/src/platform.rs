//! Platform specifics: volume identity, drive kind, long paths, indexer thread priority, disk
//! load, uptime and memory (VFS-5).
//! Most of the crate's `cfg(windows)` code lives here.

use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// Scheduling attributes of the indexer thread (VFS-5 R1, R2). The walk profile is the
/// thread's base state; background mode (very-low I/O) is entered only around a directory
/// listing, so no index write can ever run in it (VFS-5 R3).
pub(crate) trait ThreadPriority: Send {
    /// Normal I/O priority, lowest CPU priority, low memory priority.
    fn walk_profile(&mut self);
    /// Very-low I/O priority until `end_background`.
    fn begin_background(&mut self);
    /// Leaves background mode and restores the walk profile.
    fn end_background(&mut self);
}

/// The calling thread's real attributes. Each kind of failure is logged once and ignored:
/// a thread at the wrong priority still indexes correctly.
#[derive(Default)]
pub(crate) struct OsThreadPriority {
    warned: [bool; 3],
}

impl OsThreadPriority {
    fn check(&mut self, what: usize, r: std::io::Result<()>) {
        if let Err(e) = r {
            if !std::mem::replace(&mut self.warned[what], true) {
                let name = ["walk profile", "background begin", "background end"][what];
                log::warn!("indexer thread priority ({name}): {e}");
            }
        }
    }
}

impl ThreadPriority for OsThreadPriority {
    fn walk_profile(&mut self) {
        self.check(0, imp::walk_profile());
    }

    fn begin_background(&mut self) {
        self.check(1, imp::begin_background());
    }

    fn end_background(&mut self) {
        self.check(2, imp::end_background());
    }
}

/// Average disk queue length per physical disk (VFS-5 R4).
pub(crate) trait DiskLoad: Send {
    /// One sample: (instance name, average queue length since the previous sample), or `None`
    /// when this sample failed.
    fn sample(&mut self) -> Option<Vec<(String, f64)>>;
}

/// The system's disk-load probe: on Windows a PDH query on
/// `\PhysicalDisk(*)\Avg. Disk Queue Length`, added by its English name because counter names
/// are localised on a Spanish Windows. `None` where there is no probe (macOS, Linux) or when the
/// query cannot be opened.
pub(crate) fn disk_load() -> Option<Box<dyn DiskLoad>> {
    imp::disk_load()
}

/// Drive of an absolute Windows path, upper case with its colon ("D:"); `None` for UNC paths,
/// relative paths and every path on other systems.
pub(crate) fn drive_of(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    match path.components().next()? {
        Component::Prefix(p) => match p.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => {
                Some(format!("{}:", (d as char).to_ascii_uppercase()))
            }
            _ => None,
        },
        _ => None,
    }
}

/// PDH name of the instance summing every physical disk.
const TOTAL_INSTANCE: &str = "_Total";

/// Queue length of the disks holding `drives`: the largest among the instances whose name
/// lists one of them (PDH names physical disks "0 C:", "1 D: E:"), else the `_Total`
/// instance, which over-counts rather than missing a busy disk.
pub(crate) fn queue_for(samples: &[(String, f64)], drives: &[String]) -> Option<f64> {
    let matching = samples
        .iter()
        .filter(|(name, _)| {
            name.split_whitespace()
                .any(|t| drives.iter().any(|d| t.eq_ignore_ascii_case(d)))
        })
        .map(|(_, q)| *q)
        .fold(None, |acc: Option<f64>, q| {
            Some(acc.map_or(q, |a| a.max(q)))
        });
    matching.or_else(|| {
        samples
            .iter()
            .find(|(name, _)| name == TOTAL_INSTANCE)
            .map(|(_, q)| *q)
    })
}

/// Time since the system booted (Windows `GetTickCount64`, macOS `CLOCK_MONOTONIC`, which
/// counts from boot and includes sleep); `None` where unknown.
pub(crate) fn uptime() -> Option<Duration> {
    imp::uptime()
}

/// Resident memory of this process (Windows working set, macOS resident size).
pub(crate) fn working_set_bytes() -> Option<u64> {
    imp::working_set_bytes()
}

/// Footprint counters of this process since it started, for the diagnostic log (VFS-41):
/// peak resident memory, CPU time, and bytes read and written (macOS: disk I/O; Windows: all
/// I/O the process issued, files and devices). `None` where unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProcStats {
    pub peak_bytes: Option<u64>,
    pub cpu: Option<Duration>,
    pub read_bytes: Option<u64>,
    pub written_bytes: Option<u64>,
}

pub(crate) fn proc_stats() -> ProcStats {
    imp::proc_stats()
}

/// The Windows attributes of `path`, read by name (`GetFileAttributesW`: no handle is
/// opened, so a cloud placeholder is not downloaded); `None` elsewhere or when unreadable.
#[cfg(windows)]
pub(crate) fn file_attributes(path: &Path) -> Option<u32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
    let wide: Vec<u16> = long_path(path)
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    let attrs = unsafe { GetFileAttributesW(wide.as_ptr()) };
    (attrs != INVALID_FILE_ATTRIBUTES).then_some(attrs)
}

#[cfg(not(windows))]
pub(crate) fn file_attributes(_path: &Path) -> Option<u32> {
    None
}

/// Windows: sets `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED` on `dir`, so Windows Search does not
/// re-index the index's WAL churn (VFS-5 R9). Elsewhere nothing.
pub(crate) fn exclude_from_content_index(dir: &Path) -> std::io::Result<()> {
    imp::exclude_from_content_index(dir)
}

/// File Synology Drive Client keeps at the top of every folder it syncs. Its presence in
/// every client version is unverified (`windows-recommendations.md` R2).
const SYNOLOGY_MARKER: &str = ".SynologyWorkingDirectory";

/// Whether `dir` or one of its ancestors is the top of a Synology sync task (VFS-5 R9).
pub(crate) fn inside_sync_root(dir: &Path) -> bool {
    dir.ancestors().any(|a| a.join(SYNOLOGY_MARKER).exists())
}

#[cfg(target_os = "macos")]
mod imp {
    use super::DriveKind;
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    /// `sys/qos.h`; not exported by the `libc` crate.
    const QOS_CLASS_UTILITY: libc::c_uint = 0x11;
    /// `sys/resource.h`.
    const IOPOL_TYPE_DISK: libc::c_int = 0;
    const IOPOL_SCOPE_THREAD: libc::c_int = 1;
    pub(super) const IOPOL_DEFAULT: libc::c_int = 0;
    pub(super) const IOPOL_THROTTLE: libc::c_int = 3;
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
        #[cfg(test)]
        fn getiopolicy_np(iotype: libc::c_int, scope: libc::c_int) -> libc::c_int;
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

    fn set_io_policy(policy: libc::c_int) -> std::io::Result<()> {
        // SAFETY: only changes the disk I/O policy of the calling thread.
        if unsafe { setiopolicy_np(IOPOL_TYPE_DISK, IOPOL_SCOPE_THREAD, policy) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Utility QoS lowers CPU priority without the I/O throttling that background QoS adds;
    /// the thread's disk policy goes back to the process default (normal).
    pub(super) fn walk_profile() -> std::io::Result<()> {
        // SAFETY: only changes the QoS class of the calling thread.
        let rc = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0) };
        if rc != 0 {
            return Err(std::io::Error::from_raw_os_error(rc));
        }
        set_io_policy(IOPOL_DEFAULT)
    }

    pub(super) fn begin_background() -> std::io::Result<()> {
        set_io_policy(IOPOL_THROTTLE)
    }

    pub(super) fn end_background() -> std::io::Result<()> {
        set_io_policy(IOPOL_DEFAULT)
    }

    /// Disk I/O policy of the calling thread, for tests.
    #[cfg(test)]
    pub(super) fn io_policy() -> libc::c_int {
        // SAFETY: reads a scheduling attribute of the calling thread.
        unsafe { getiopolicy_np(IOPOL_TYPE_DISK, IOPOL_SCOPE_THREAD) }
    }

    pub(super) fn disk_load() -> Option<Box<dyn super::DiskLoad>> {
        None
    }

    pub(super) fn uptime() -> Option<std::time::Duration> {
        // SAFETY: `ts` is a plain C struct that `clock_gettime` fills on success.
        unsafe {
            let mut ts: libc::timespec = std::mem::zeroed();
            (libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) == 0)
                .then(|| std::time::Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32))
        }
    }

    pub(super) fn working_set_bytes() -> Option<u64> {
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        // SAFETY: `info` is a plain C struct of `size` bytes that `proc_pidinfo` fills.
        unsafe {
            let mut info: libc::proc_taskinfo = std::mem::zeroed();
            let n = libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDTASKINFO,
                0,
                (&mut info as *mut libc::proc_taskinfo).cast(),
                size,
            );
            (n == size).then_some(info.pti_resident_size)
        }
    }

    pub(super) fn proc_stats() -> super::ProcStats {
        let tv =
            |t: libc::timeval| std::time::Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
        let mut stats = super::ProcStats::default();
        // SAFETY: `ru` is a plain C struct that `getrusage` fills on success.
        unsafe {
            let mut ru: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut ru) == 0 {
                // macOS reports `ru_maxrss` in bytes.
                stats.peak_bytes = Some(ru.ru_maxrss as u64);
                stats.cpu = Some(tv(ru.ru_utime) + tv(ru.ru_stime));
            }
        }
        // SAFETY: `info` is a plain C struct of the V2 layout that `proc_pid_rusage` fills.
        unsafe {
            let mut info: libc::rusage_info_v2 = std::mem::zeroed();
            let ptr = (&mut info as *mut libc::rusage_info_v2).cast();
            if libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V2, ptr) == 0 {
                stats.read_bytes = Some(info.ri_diskio_bytesread);
                stats.written_bytes = Some(info.ri_diskio_byteswritten);
            }
        }
        stats
    }

    pub(super) fn exclude_from_content_index(_dir: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use super::DriveKind;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetFileAttributesW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
        SetFileAttributesW, FILE_ATTRIBUTE_NOT_CONTENT_INDEXED, INVALID_FILE_ATTRIBUTES,
    };
    use windows_sys::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
        PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
        PDH_MORE_DATA,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentThread, SetThreadInformation, SetThreadPriority,
        ThreadMemoryPriority, MEMORY_PRIORITY_INFORMATION, MEMORY_PRIORITY_LOW,
        THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END, THREAD_PRIORITY_LOWEST,
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
        // `GetVolumePathNameW` answers for paths that no longer exist; a missing folder is
        // unknown on every platform, so an unplugged or deleted root is never called local.
        if std::fs::symlink_metadata(path).is_err() {
            return DriveKind::Unknown;
        }
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

    fn set_priority(priority: i32) -> std::io::Result<()> {
        // SAFETY: the pseudo-handle of the current thread needs no closing.
        if unsafe { SetThreadPriority(GetCurrentThread(), priority) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// `THREAD_PRIORITY_LOWEST` plus `MEMORY_PRIORITY_LOW`; I/O stays normal because
    /// background mode is off. On Windows 8+ background mode does not lower CPU priority
    /// (`windows-io.md` §1.1), so the CPU priority also applies while it is on.
    pub(super) fn walk_profile() -> std::io::Result<()> {
        set_priority(THREAD_PRIORITY_LOWEST)?;
        let info = MEMORY_PRIORITY_INFORMATION {
            MemoryPriority: MEMORY_PRIORITY_LOW,
        };
        // SAFETY: `info` is a valid MEMORY_PRIORITY_INFORMATION of the size passed.
        let ok = unsafe {
            SetThreadInformation(
                GetCurrentThread(),
                ThreadMemoryPriority,
                (&info as *const MEMORY_PRIORITY_INFORMATION).cast(),
                std::mem::size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn begin_background() -> std::io::Result<()> {
        set_priority(THREAD_MODE_BACKGROUND_BEGIN)
    }

    /// Leaving background mode resets memory priority to normal, so the walk profile is
    /// applied again.
    pub(super) fn end_background() -> std::io::Result<()> {
        set_priority(THREAD_MODE_BACKGROUND_END)?;
        walk_profile()
    }

    /// PDH query on the average queue length of every physical disk.
    struct Pdh {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
    }

    // SAFETY: PDH query and counter handles are process-wide and used by one thread at a time.
    unsafe impl Send for Pdh {}

    impl Drop for Pdh {
        fn drop(&mut self) {
            // SAFETY: `query` was opened by `PdhOpenQueryW` and is closed once.
            unsafe { PdhCloseQuery(self.query) };
        }
    }

    /// `PDH_CSTATUS_NEW_DATA`: the highest status of a usable value (0 is valid data).
    const PDH_CSTATUS_NEW_DATA: u32 = 1;

    impl super::DiskLoad for Pdh {
        fn sample(&mut self) -> Option<Vec<(String, f64)>> {
            let (mut size, mut count) = (0u32, 0u32);
            // SAFETY: the query and counter are open; the first call only asks for the size
            // and the second writes at most `size` bytes into a buffer of at least that size,
            // aligned for the item struct (u64).
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }
                let rc = PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut size,
                    &mut count,
                    std::ptr::null_mut(),
                );
                if rc != PDH_MORE_DATA {
                    return None;
                }
                let mut buf = vec![0u64; (size as usize).div_ceil(8)];
                let items = buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
                if PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut size,
                    &mut count,
                    items,
                ) != 0
                {
                    return None;
                }
                let items = std::slice::from_raw_parts(items, count as usize);
                Some(
                    items
                        .iter()
                        .filter(|i| i.FmtValue.CStatus <= PDH_CSTATUS_NEW_DATA)
                        .map(|i| (pwstr(i.szName), i.FmtValue.Anonymous.doubleValue))
                        .collect(),
                )
            }
        }
    }

    /// # Safety
    /// `p` is null or a NUL-terminated UTF-16 string.
    unsafe fn pwstr(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut n = 0;
        // SAFETY: the caller guarantees a terminating NUL.
        unsafe {
            while *p.add(n) != 0 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
        }
    }

    pub(super) fn disk_load() -> Option<Box<dyn super::DiskLoad>> {
        let path: Vec<u16> = r"\PhysicalDisk(*)\Avg. Disk Queue Length"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut query: PDH_HQUERY = std::ptr::null_mut();
        let mut counter: PDH_HCOUNTER = std::ptr::null_mut();
        // SAFETY: out-pointers are valid; `path` is NUL-terminated; the query is closed once,
        // here on failure or by `Pdh::drop`.
        unsafe {
            if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                PdhCloseQuery(query);
                return None;
            }
            // A rate counter needs a first raw sample to compare against.
            PdhCollectQueryData(query);
        }
        Some(Box::new(Pdh { query, counter }))
    }

    pub(super) fn uptime() -> Option<std::time::Duration> {
        // SAFETY: no arguments, no side effects.
        Some(std::time::Duration::from_millis(unsafe {
            GetTickCount64()
        }))
    }

    pub(super) fn working_set_bytes() -> Option<u64> {
        let mut pmc: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        // SAFETY: `pmc` is a valid struct of `cb` bytes; the process pseudo-handle needs no
        // closing.
        let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) };
        (ok != 0).then_some(pmc.WorkingSetSize as u64)
    }

    pub(super) fn proc_stats() -> super::ProcStats {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{
            GetProcessIoCounters, GetProcessTimes, IO_COUNTERS,
        };
        let mut stats = super::ProcStats::default();
        // SAFETY: each struct is plain data of the size the call expects; the process
        // pseudo-handle needs no closing.
        unsafe {
            let process = GetCurrentProcess();
            let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if GetProcessMemoryInfo(process, &mut pmc, pmc.cb) != 0 {
                stats.peak_bytes = Some(pmc.PeakWorkingSetSize as u64);
            }
            let mut t: [FILETIME; 4] = std::mem::zeroed();
            let [created, exited, kernel, user] = &mut t;
            if GetProcessTimes(process, created, exited, kernel, user) != 0 {
                let ticks =
                    |f: &FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
                // FILETIME counts 100 ns ticks.
                stats.cpu = Some(std::time::Duration::from_nanos(
                    (ticks(kernel) + ticks(user)) * 100,
                ));
            }
            let mut io: IO_COUNTERS = std::mem::zeroed();
            if GetProcessIoCounters(process, &mut io) != 0 {
                stats.read_bytes = Some(io.ReadTransferCount);
                stats.written_bytes = Some(io.WriteTransferCount);
            }
        }
        stats
    }

    pub(super) fn exclude_from_content_index(dir: &Path) -> std::io::Result<()> {
        let p = wide(dir);
        // SAFETY: `p` is NUL-terminated.
        unsafe {
            let attrs = GetFileAttributesW(p.as_ptr());
            if attrs == INVALID_FILE_ATTRIBUTES {
                return Err(std::io::Error::last_os_error());
            }
            if attrs & FILE_ATTRIBUTE_NOT_CONTENT_INDEXED == 0
                && SetFileAttributesW(p.as_ptr(), attrs | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
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

    pub(super) fn walk_profile() -> std::io::Result<()> {
        Ok(())
    }

    pub(super) fn begin_background() -> std::io::Result<()> {
        Ok(())
    }

    pub(super) fn end_background() -> std::io::Result<()> {
        Ok(())
    }

    pub(super) fn disk_load() -> Option<Box<dyn super::DiskLoad>> {
        None
    }

    pub(super) fn uptime() -> Option<std::time::Duration> {
        None
    }

    pub(super) fn working_set_bytes() -> Option<u64> {
        None
    }

    pub(super) fn proc_stats() -> super::ProcStats {
        super::ProcStats::default()
    }

    pub(super) fn exclude_from_content_index(_dir: &Path) -> std::io::Result<()> {
        Ok(())
    }
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

    /// VFS-5 AC-4 (macOS): background mode throttles the thread's disk I/O and leaving it
    /// restores the default policy; the walk profile itself does not throttle.
    #[cfg(target_os = "macos")]
    #[test]
    fn background_mode_toggles_io_policy() {
        std::thread::spawn(|| {
            let mut p = OsThreadPriority::default();
            p.walk_profile();
            assert_eq!(imp::io_policy(), imp::IOPOL_DEFAULT);
            p.begin_background();
            assert_eq!(imp::io_policy(), imp::IOPOL_THROTTLE);
            p.end_background();
            assert_eq!(imp::io_policy(), imp::IOPOL_DEFAULT);
            assert!(!p.warned.iter().any(|w| *w), "a priority call failed");
        })
        .join()
        .unwrap();
    }

    /// VFS-5 AC-4 (Windows CI): the walk profile sets lowest CPU and low memory priority;
    /// background mode really toggles (a second begin or end fails with the documented
    /// error), and leaving it restores the walk profile.
    #[cfg(windows)]
    #[test]
    fn background_mode_toggles_thread_priority() {
        use windows_sys::Win32::Foundation::{
            GetLastError, ERROR_THREAD_MODE_ALREADY_BACKGROUND, ERROR_THREAD_MODE_NOT_BACKGROUND,
        };
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, GetThreadInformation, GetThreadPriority, SetThreadPriority,
            ThreadMemoryPriority, MEMORY_PRIORITY_INFORMATION, MEMORY_PRIORITY_LOW,
            THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END, THREAD_PRIORITY_LOWEST,
        };
        fn memory_priority() -> u32 {
            let mut info = MEMORY_PRIORITY_INFORMATION { MemoryPriority: 0 };
            // SAFETY: `info` is a valid struct of the size passed.
            let ok = unsafe {
                GetThreadInformation(
                    GetCurrentThread(),
                    ThreadMemoryPriority,
                    (&mut info as *mut MEMORY_PRIORITY_INFORMATION).cast(),
                    std::mem::size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
                )
            };
            assert_ne!(ok, 0, "{}", std::io::Error::last_os_error());
            info.MemoryPriority
        }
        fn set_fails_with(mode: i32, code: u32) {
            // SAFETY: changes the current thread only.
            unsafe {
                assert_eq!(SetThreadPriority(GetCurrentThread(), mode), 0);
                assert_eq!(GetLastError(), code);
            }
        }
        std::thread::spawn(|| {
            let mut p = OsThreadPriority::default();
            p.walk_profile();
            // SAFETY: reads the current thread's priority.
            assert_eq!(
                unsafe { GetThreadPriority(GetCurrentThread()) },
                THREAD_PRIORITY_LOWEST
            );
            assert_eq!(memory_priority(), MEMORY_PRIORITY_LOW);
            p.begin_background();
            set_fails_with(
                THREAD_MODE_BACKGROUND_BEGIN,
                ERROR_THREAD_MODE_ALREADY_BACKGROUND,
            );
            p.end_background();
            set_fails_with(THREAD_MODE_BACKGROUND_END, ERROR_THREAD_MODE_NOT_BACKGROUND);
            // SAFETY: reads the current thread's priority.
            assert_eq!(
                unsafe { GetThreadPriority(GetCurrentThread()) },
                THREAD_PRIORITY_LOWEST
            );
            assert_eq!(memory_priority(), MEMORY_PRIORITY_LOW);
            assert!(!p.warned.iter().any(|w| *w), "a priority call failed");
        })
        .join()
        .unwrap();
    }

    /// VFS-5 AC-5 (Windows CI): the PDH probe opens for a standard user and a second sample
    /// reports the `_Total` instance. Elsewhere there is no probe.
    #[test]
    fn disk_load_probe() {
        let probe = disk_load();
        if cfg!(windows) {
            let mut probe = probe.expect("PDH query opens");
            std::thread::sleep(Duration::from_millis(1100));
            let s = probe.sample().expect("a sample");
            assert!(
                s.iter().any(|(n, q)| n == TOTAL_INSTANCE && *q >= 0.0),
                "{s:?}"
            );
            assert!(queue_for(&s, &["C:".to_owned()]).is_some());
        } else {
            assert!(probe.is_none());
        }
    }

    /// VFS-5 AC-5: the queue of the disks holding the roots is the largest among PDH
    /// instances naming one of their drives, else `_Total`.
    #[test]
    fn queue_for_picks_the_roots_disk() {
        let s: Vec<(String, f64)> = vec![
            ("0 C:".into(), 0.5),
            ("1 D: E:".into(), 3.0),
            ("2 F:".into(), 1.0),
            (TOTAL_INSTANCE.into(), 4.5),
        ];
        let d = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(queue_for(&s, &d(&["e:"])), Some(3.0));
        assert_eq!(queue_for(&s, &d(&["C:", "F:"])), Some(1.0));
        assert_eq!(queue_for(&s, &d(&["Z:"])), Some(4.5));
        assert_eq!(queue_for(&s, &[]), Some(4.5));
        assert_eq!(queue_for(&[], &d(&["C:"])), None);
    }

    /// VFS-5 AC-5: drive letters come from Windows paths only.
    #[test]
    fn drive_of_windows_paths() {
        if cfg!(windows) {
            assert_eq!(drive_of(Path::new(r"d:\fotos")).as_deref(), Some("D:"));
            assert_eq!(drive_of(Path::new(r"\\?\E:\x")).as_deref(), Some("E:"));
            assert_eq!(drive_of(Path::new(r"\\nas\share\x")), None);
        } else {
            assert_eq!(drive_of(Path::new("/Volumes/datos")), None);
        }
        assert_eq!(drive_of(Path::new("relativo")), None);
    }

    /// VFS-5 AC-10: a folder below a `.SynologyWorkingDirectory` marker is inside a sync
    /// root; a sibling tree is not.
    #[test]
    fn sync_root_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let synced = tmp.path().join("Drive");
        let deep = synced.join("AppData").join("Local").join("app.vfs.desktop");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(tmp.path().join("otra")).unwrap();
        assert!(!inside_sync_root(&deep));
        std::fs::create_dir(synced.join(SYNOLOGY_MARKER)).unwrap();
        assert!(inside_sync_root(&deep));
        assert!(inside_sync_root(&synced));
        assert!(!inside_sync_root(&tmp.path().join("otra")));
    }

    /// VFS-5 AC-10: on Windows the index folder becomes not content-indexed, and setting it
    /// twice is harmless; elsewhere the call does nothing.
    #[test]
    fn index_folder_not_content_indexed() {
        let tmp = tempfile::tempdir().unwrap();
        exclude_from_content_index(tmp.path()).unwrap();
        exclude_from_content_index(tmp.path()).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;
            let attrs = std::fs::metadata(tmp.path()).unwrap().file_attributes();
            assert_ne!(attrs & FILE_ATTRIBUTE_NOT_CONTENT_INDEXED, 0);
        }
    }

    /// VFS-5 AC-7, AC-8: uptime and working set are known on both target systems.
    #[test]
    fn uptime_and_working_set() {
        if cfg!(any(windows, target_os = "macos")) {
            assert!(uptime().unwrap() > Duration::ZERO);
            assert!(working_set_bytes().unwrap() > 1 << 20);
        }
    }

    /// VFS-41 AC-1: the footprint counters are known on both platforms, the peak is at least
    /// the current working set, and CPU time and bytes written only grow (a synced 4 MB write
    /// shows up in the written bytes).
    #[test]
    fn process_counters_grow() {
        if !cfg!(any(windows, target_os = "macos")) {
            return;
        }
        let before = proc_stats();
        assert!(before.peak_bytes.unwrap() >= working_set_bytes().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("x")).unwrap();
        std::io::Write::write_all(&mut f, &vec![7u8; 4 << 20]).unwrap();
        f.sync_all().unwrap();
        let mut x = 0u64;
        for i in 0..20_000_000u64 {
            x = x.wrapping_mul(31).wrapping_add(i);
        }
        std::hint::black_box(x);
        let after = proc_stats();
        assert!(after.cpu.unwrap() > before.cpu.unwrap());
        // Half of the 4 MB written: the OS may count a write it merges with another later.
        assert!(after.written_bytes.unwrap() >= before.written_bytes.unwrap() + (2 << 20));
        assert!(after.read_bytes.unwrap() >= before.read_bytes.unwrap());
    }
}
