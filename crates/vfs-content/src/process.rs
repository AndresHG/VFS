//! Process-level helpers shared by the worker processes VFS starts (the extraction worker,
//! VFS-2 R5, and the embedding worker, VFS-29 R16): background priority, and on Windows a job
//! object with a memory limit. No extractor dependency, so any worker binary can use them.

/// Lowest CPU and I/O priority available without administrator rights, for the whole
/// worker process (VFS-2 R5).
pub fn enter_background_priority() {
    #[cfg(target_os = "macos")]
    unsafe {
        // Darwin background band: throttled CPU and disk I/O for every thread.
        libc::setpriority(libc::PRIO_DARWIN_PROCESS, 0, libc::PRIO_DARWIN_BG);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, SetPriorityClass, IDLE_PRIORITY_CLASS, PROCESS_MODE_BACKGROUND_BEGIN,
        };
        // Background mode for I/O and memory priority, then the idle class for CPU.
        SetPriorityClass(GetCurrentProcess(), PROCESS_MODE_BACKGROUND_BEGIN);
        SetPriorityClass(GetCurrentProcess(), IDLE_PRIORITY_CLASS);
    }
}

/// The OS's own report of this process's priority (VFS-2 AC-23): whether it is in the
/// background band, and the raw value read (macOS `getpriority(PRIO_DARWIN_PROCESS)`, Linux
/// nice, Windows priority class), for diagnosis. On Windows the band means the idle class
/// with background mode on; background mode is read back by asking for it again, which the
/// OS refuses with `ERROR_PROCESS_MODE_ALREADY_BACKGROUND` while it is on.
pub fn background_priority() -> (bool, i64) {
    #[cfg(target_os = "macos")]
    // SAFETY: plain query of this process's own priority.
    unsafe {
        let p = libc::getpriority(libc::PRIO_DARWIN_PROCESS, 0);
        (p != 0, p as i64)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    // SAFETY: plain query of this process's own priority.
    unsafe {
        let p = libc::getpriority(libc::PRIO_PROCESS, 0);
        (p == 19, p as i64)
    }
    #[cfg(windows)]
    // SAFETY: plain queries of this process's own priority; the second call changes nothing
    // when background mode is already on, which is what it detects.
    unsafe {
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_PROCESS_MODE_ALREADY_BACKGROUND};
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, GetPriorityClass, SetPriorityClass, IDLE_PRIORITY_CLASS,
            PROCESS_MODE_BACKGROUND_BEGIN,
        };
        let class = GetPriorityClass(GetCurrentProcess());
        let mode_on = SetPriorityClass(GetCurrentProcess(), PROCESS_MODE_BACKGROUND_BEGIN) == 0
            && GetLastError() == ERROR_PROCESS_MODE_ALREADY_BACKGROUND;
        (class == IDLE_PRIORITY_CLASS && mode_on, class as i64)
    }
}

#[cfg(windows)]
pub mod job {
    //! A Windows job object that caps a child process's committed memory and kills it with
    //! the job (VFS-2 R18).
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };

    pub struct Job(HANDLE);

    // SAFETY: a job handle may be used and closed from any thread.
    unsafe impl Send for Job {}

    impl Job {
        /// Creates a job with the limits and puts `child` in it.
        pub fn hold(child: &Child, memory_limit: usize) -> std::io::Result<Job> {
            // SAFETY: plain Win32 calls on handles owned here or by `child`; every pointer
            // refers to a live local of the right type and size.
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let job = Job(h);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags =
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                info.ProcessMemoryLimit = memory_limit;
                if SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                if AssignProcessToJobObject(h, child.as_raw_handle() as HANDLE) == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(job)
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle was created by `hold` and is closed exactly once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
