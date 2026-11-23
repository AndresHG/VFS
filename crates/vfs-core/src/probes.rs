//! Probes and pacing shared with the content indexer (VFS-27 R6, R7): the name indexer's
//! disk-load sampler and busy-disk back-off (VFS-5 R4), the time since the user's last
//! keyboard or mouse input, and the free space of a volume. Read-only queries; none needs
//! administrator rights.

use std::path::Path;
use std::time::Duration;

pub use crate::indexer::sched::{BackOff, Step};
use crate::platform;
use crate::{IndexProgress, IndexerState, RootStatus};

/// Whether the name indexer is walking a root or running a large pass, which content
/// indexing waits for (VFS-27 R7 rule 1): a root in its first scan or full re-walk, or more
/// pending folders than VFS-5's `incremental_max_dirs`.
pub fn name_indexing_busy(progress: &IndexProgress) -> bool {
    let limit = crate::indexer::sched::SchedConfig::default().incremental_max_dirs;
    progress.state == IndexerState::Scanning
        && (progress
            .roots
            .iter()
            .any(|r| r.status == RootStatus::Scanning)
            || progress.roots.iter().map(|r| r.dirs_pending).sum::<u64>() > limit)
}

/// Average disk queue length of the disk holding one path (VFS-5 R4). Exists on Windows only:
/// macOS has no such probe (VFS-5 R6).
pub struct DiskSampler {
    load: Box<dyn platform::DiskLoad>,
    drives: Vec<String>,
}

impl DiskSampler {
    /// The sampler for the disk holding `path`; `None` where the system has no probe.
    pub fn for_path(path: &Path) -> Option<DiskSampler> {
        Some(DiskSampler {
            load: platform::disk_load()?,
            drives: platform::drive_of(path).into_iter().collect(),
        })
    }

    /// One sample; `None` when it failed.
    pub fn queue_length(&mut self) -> Option<f64> {
        let samples = self.load.sample()?;
        platform::queue_for(&samples, &self.drives)
    }
}

/// Time since the last keyboard or mouse input in the user's session; `None` when it cannot be
/// read. Windows: `GetLastInputInfo` against `GetTickCount64`. macOS:
/// `CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateHIDSystemState,
/// kCGAnyInputEventType)`.
pub fn idle_time() -> Option<Duration> {
    imp::idle_time()
}

/// Bytes free for the user on the volume holding `path`; `None` when it cannot be read.
/// Windows: `GetDiskFreeSpaceExW`. macOS: the volume's available capacity for important usage,
/// which counts purgeable space (VFS-27 R6), not `statvfs`.
pub fn free_space(path: &Path) -> Option<u64> {
    imp::free_space(path)
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::time::Duration;

    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    pub(super) fn idle_time() -> Option<Duration> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: `info` is a live LASTINPUTINFO with its size set, as the call requires.
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return None;
        }
        // `dwTime` is the low 32 bits of the tick count; wrapping subtraction keeps it right
        // across the 49.7-day wrap.
        // SAFETY: no arguments.
        let now = unsafe { GetTickCount64() } as u32;
        Some(Duration::from_millis(now.wrapping_sub(info.dwTime) as u64))
    }

    pub(super) fn free_space(path: &Path) -> Option<u64> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0u64;
        // SAFETY: NUL-terminated path; the only out pointer is a live u64, the others null.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(free)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::time::Duration;

    type CFTypeRef = *const c_void;

    /// `kCGEventSourceStateHIDSystemState`.
    const HID_SYSTEM_STATE: i32 = 1;
    /// `kCGAnyInputEventType`.
    const ANY_INPUT_EVENT: u32 = u32::MAX;
    /// `kCFNumberSInt64Type`.
    const NUMBER_SINT64: isize = 4;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFURLVolumeAvailableCapacityForImportantUsageKey: CFTypeRef;
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: CFTypeRef,
            buffer: *const u8,
            len: isize,
            is_directory: u8,
        ) -> CFTypeRef;
        fn CFURLCopyResourcePropertyForKey(
            url: CFTypeRef,
            key: CFTypeRef,
            value: *mut CFTypeRef,
            error: *mut CFTypeRef,
        ) -> u8;
        fn CFNumberGetValue(number: CFTypeRef, the_type: isize, value: *mut c_void) -> u8;
        fn CFRelease(cf: CFTypeRef);
    }

    pub(super) fn idle_time() -> Option<Duration> {
        // SAFETY: a pure query with constant arguments.
        let secs =
            unsafe { CGEventSourceSecondsSinceLastEventType(HID_SYSTEM_STATE, ANY_INPUT_EVENT) };
        (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
    }

    pub(super) fn free_space(path: &Path) -> Option<u64> {
        let bytes = path.as_os_str().as_bytes();
        // SAFETY: CoreFoundation calls on objects created here and released exactly once;
        // `value` is checked before use and the key is a CoreFoundation constant.
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                bytes.as_ptr(),
                bytes.len() as isize,
                1,
            );
            if url.is_null() {
                return None;
            }
            let mut value: CFTypeRef = std::ptr::null();
            let ok = CFURLCopyResourcePropertyForKey(
                url,
                kCFURLVolumeAvailableCapacityForImportantUsageKey,
                &mut value,
                std::ptr::null_mut(),
            );
            CFRelease(url);
            if ok == 0 || value.is_null() {
                return None;
            }
            let mut n: i64 = 0;
            let got = CFNumberGetValue(value, NUMBER_SINT64, &mut n as *mut i64 as *mut c_void);
            CFRelease(value);
            (got != 0 && n >= 0).then_some(n as u64)
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use std::path::Path;
    use std::time::Duration;

    pub(super) fn idle_time() -> Option<Duration> {
        None
    }

    pub(super) fn free_space(_path: &Path) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-27 AC-6: free space reads on the platforms the app ships on. A CI runner's
    /// volume can report 0 bytes free, so only the answer is checked, not its size.
    #[test]
    fn free_space_reads() {
        let dir = tempfile::tempdir().unwrap();
        let free = free_space(dir.path());
        if cfg!(any(windows, target_os = "macos")) {
            assert!(free.is_some(), "{free:?}");
        }
    }

    /// VFS-27 AC-7: the idle probe answers without panicking; a missing answer counts as
    /// "in use" at the caller.
    #[test]
    fn idle_time_answers() {
        let _ = idle_time();
        let dir = tempfile::tempdir().unwrap();
        let sampler = DiskSampler::for_path(dir.path());
        if cfg!(target_os = "macos") {
            assert!(sampler.is_none(), "macOS has no disk probe (VFS-5 R6)");
        }
    }

    fn progress(state: IndexerState, roots: &[(RootStatus, u64)]) -> IndexProgress {
        IndexProgress {
            state,
            files_indexed: 0,
            roots: roots
                .iter()
                .enumerate()
                .map(|(i, &(status, dirs_pending))| crate::RootProgress {
                    root_id: i as i64,
                    path: String::new(),
                    status,
                    dirs_done: 0,
                    dirs_pending,
                    files_indexed: 0,
                    current_dir: None,
                })
                .collect(),
        }
    }

    /// VFS-27 AC-7 (R7 rule 1): content waits while a root is walked or a name pass has more
    /// pending folders than the scheduler's incremental limit, not for a small pass or when
    /// the indexer is idle.
    #[test]
    fn name_indexing_busy_follows_the_scheduler() {
        let limit = crate::indexer::sched::SchedConfig::default().incremental_max_dirs;
        let scanning = IndexerState::Scanning;
        assert!(name_indexing_busy(&progress(
            scanning,
            &[(RootStatus::Scanning, 0)]
        )));
        assert!(name_indexing_busy(&progress(
            scanning,
            &[(RootStatus::Ready, limit), (RootStatus::Ready, 1)]
        )));
        assert!(!name_indexing_busy(&progress(
            scanning,
            &[(RootStatus::Ready, limit)]
        )));
        assert!(!name_indexing_busy(&progress(
            IndexerState::Idle,
            &[(RootStatus::Scanning, limit + 1)]
        )));
    }
}
