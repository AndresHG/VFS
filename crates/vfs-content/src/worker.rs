//! Worker side of extraction (VFS-2 R2, R5): runs in its own process, at background CPU and
//! I/O priority, reads requests from stdin and answers on stdout. Each file is read once,
//! sequentially, into memory and handed to the extractor; a panic in the extractor becomes a
//! `Failed` answer, an allocation over the heap cap aborts the process (the parent then marks
//! the file failed and starts a new worker).

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::extract::{extract_bytes, Extracted};
use crate::protocol::{FailReason, Hello, Outcome, Request, Response, SkipReason, WORKER_VERSION};

/// Global allocator for the worker binary that refuses allocations beyond a cap. Refusal
/// makes Rust abort with "memory allocation of N bytes failed", which the supervisor reports
/// as [`FailReason::Memory`]. Portable replacement for an OS memory limit (VFS-2 D6).
///
/// The allocator is process-wide state by nature: the only module-level mutable state in the
/// crate. A binary installs it as its `#[global_allocator]`: `vfs-extract`, and the `vfs` CLI
/// when built with `--features content` (the CLI is its own worker). Uncapped, it costs one
/// atomic add per allocation.
pub struct CappedAlloc {
    used: AtomicUsize,
    cap: AtomicUsize,
}

impl CappedAlloc {
    pub const fn new() -> CappedAlloc {
        CappedAlloc {
            used: AtomicUsize::new(0),
            cap: AtomicUsize::new(usize::MAX),
        }
    }

    pub fn set_cap(&self, bytes: usize) {
        self.cap.store(bytes, Ordering::Relaxed);
    }

    fn reserve(&self, n: usize) -> bool {
        let prev = self.used.fetch_add(n, Ordering::Relaxed);
        if prev.saturating_add(n) > self.cap.load(Ordering::Relaxed) {
            self.used.fetch_sub(n, Ordering::Relaxed);
            return false;
        }
        true
    }
}

impl Default for CappedAlloc {
    fn default() -> Self {
        CappedAlloc::new()
    }
}

unsafe impl GlobalAlloc for CappedAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !self.reserve(layout.size()) {
            return std::ptr::null_mut();
        }
        let p = System.alloc(layout);
        if p.is_null() {
            self.used.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !self.reserve(layout.size()) {
            return std::ptr::null_mut();
        }
        let p = System.alloc_zeroed(layout);
        if p.is_null() {
            self.used.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        self.used.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old = layout.size();
        if new_size > old && !self.reserve(new_size - old) {
            return std::ptr::null_mut();
        }
        let p = System.realloc(ptr, layout, new_size);
        if p.is_null() {
            if new_size > old {
                self.used.fetch_sub(new_size - old, Ordering::Relaxed);
            }
        } else if new_size < old {
            self.used.fetch_sub(old - new_size, Ordering::Relaxed);
        }
        p
    }
}

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

/// Windows: absolute paths of 248+ characters get the `\\?\` prefix (same rule as
/// `vfs_core::platform::long_path`, VFS-1 AC-8). Elsewhere: unchanged.
pub(crate) fn long_path(path: &Path) -> PathBuf {
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

/// Test-only behaviours, chosen with `--test-hook=<kind>:<arg>` worker arguments
/// (repeatable). The content indexer never passes them, so they change nothing outside tests.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TestHook {
    /// Never answer for the file named `arg` (VFS-2 AC-3).
    Hang(String),
    /// Allocate without bound for the file named `arg` (AC-5).
    Alloc(String),
    /// Abort without a word for the file named `arg` (AC-20).
    Crash(String),
    /// Report `arg` as the worker version (AC-20).
    Version(String),
    /// Append `open <name>` / `close <name>` lines to the file `arg` around each file (AC-23).
    OpenLog(PathBuf),
}

fn test_hooks() -> Vec<TestHook> {
    std::env::args()
        .filter_map(|a| {
            let v = a.strip_prefix("--test-hook=")?.to_string();
            let (kind, arg) = v.split_once(':')?;
            let arg = arg.to_string();
            match kind {
                "hang" => Some(TestHook::Hang(arg)),
                "alloc" => Some(TestHook::Alloc(arg)),
                "crash" => Some(TestHook::Crash(arg)),
                "version" => Some(TestHook::Version(arg)),
                "openlog" => Some(TestHook::OpenLog(arg.into())),
                _ => None,
            }
        })
        .collect()
}

fn run_hooks(hooks: &[TestHook], path: &Path) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let is = |n: &String| name.as_deref() == Some(n.as_str());
    for hook in hooks {
        match hook {
            TestHook::Hang(n) if is(n) => loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            },
            TestHook::Alloc(n) if is(n) => {
                let mut hog: Vec<Vec<u8>> = Vec::new();
                loop {
                    hog.push(vec![1u8; 16 << 20]);
                }
            }
            TestHook::Crash(n) if is(n) => std::process::abort(),
            _ => {}
        }
    }
}

fn log_open(hooks: &[TestHook], what: &str, path: &Path) {
    for hook in hooks {
        if let TestHook::OpenLog(log) = hook {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .expect("open log");
            writeln!(f, "{what} {name}").expect("write open log");
        }
    }
}

/// Cuts `s` to at most `max` bytes at a character boundary; returns whether it cut.
fn cap_text(s: &mut String, max: usize) -> bool {
    if s.len() <= max {
        return false;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    true
}

/// Normalizes each page on its own (so no hyphen is joined across a page break) and returns
/// the new body with the new page start offsets.
fn normalize_pages(body: &str, page_starts: &[usize]) -> (String, Vec<usize>) {
    if page_starts.is_empty() {
        return (crate::analyze::normalize(body), Vec::new());
    }
    let mut out = crate::analyze::normalize(&body[..page_starts[0]]);
    let mut starts = Vec::with_capacity(page_starts.len());
    for (i, &s) in page_starts.iter().enumerate() {
        let e = page_starts.get(i + 1).copied().unwrap_or(body.len());
        starts.push(out.len());
        out.push_str(&crate::analyze::normalize(&body[s..e]));
    }
    (out, starts)
}

/// Handles one request inside the worker.
pub fn handle(req: &Request) -> Response {
    let started = Instant::now();
    let path = long_path(&req.path);
    let done = |outcome, bytes_read| Response {
        outcome,
        bytes_read,
        ms: started.elapsed().as_millis() as u64,
    };
    let mut file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            return done(
                Outcome::Failed {
                    reason: FailReason::Open,
                    detail: e.to_string(),
                },
                0,
            )
        }
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len > req.max_input_bytes {
        return done(Outcome::Skipped(SkipReason::TooLarge), 0);
    }
    // One sequential read of the whole file: on a spinning disk this is the cheapest access
    // pattern, and every supported format needs most of the file anyway (ZIP directories sit
    // at the end, PDF cross-reference tables too).
    let mut bytes = Vec::with_capacity(len as usize);
    if let Err(e) = (&mut file)
        .take(req.max_input_bytes + 1)
        .read_to_end(&mut bytes)
    {
        return done(
            Outcome::Failed {
                reason: FailReason::Open,
                detail: e.to_string(),
            },
            bytes.len() as u64,
        );
    }
    drop(file);
    let read = bytes.len() as u64;
    if read > req.max_input_bytes {
        return done(Outcome::Skipped(SkipReason::TooLarge), read);
    }
    let ext = req
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| extract_bytes(&ext, bytes)));
    let outcome = match result {
        Ok(Extracted::Text {
            title,
            body,
            page_starts,
        }) => {
            let (mut body, mut page_starts) = normalize_pages(&body, &page_starts);
            let truncated = cap_text(&mut body, req.max_text_bytes);
            page_starts.retain(|&s| s < body.len());
            if body.trim().is_empty() {
                Outcome::Skipped(SkipReason::NoText)
            } else {
                Outcome::Text {
                    title,
                    body,
                    truncated,
                    page_starts,
                }
            }
        }
        Ok(Extracted::Skipped(r)) => Outcome::Skipped(r),
        Ok(Extracted::Failed(detail)) => Outcome::Failed {
            reason: FailReason::Corrupt,
            detail,
        },
        Err(panic) => {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic".into());
            Outcome::Failed {
                reason: FailReason::Corrupt,
                detail: format!("panic: {msg}"),
            }
        }
    };
    done(outcome, read)
}

/// Entry point of a worker process: `alloc` is the binary's global allocator, capped from
/// `VFS_EXTRACT_MEM_CAP`. Writes a [`Hello`] once at background priority, then answers one
/// request per line. Exits as soon as stdin closes, even in the middle of a file: a closed
/// pipe means the parent is gone (VFS-2 R18).
pub fn worker_main(alloc: &CappedAlloc) {
    enter_background_priority();
    if let Some(cap) = std::env::var(crate::protocol::MEM_CAP_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
    {
        alloc.set_cap(cap);
    }
    // Panics are answered as `Failed`; keep the default hook from spamming stderr.
    std::panic::set_hook(Box::new(|_| {}));
    let hooks = test_hooks();
    let version = hooks
        .iter()
        .find_map(|h| match h {
            TestHook::Version(v) => Some(v.clone()),
            _ => None,
        })
        .unwrap_or_else(|| WORKER_VERSION.to_string());
    let mut stdout = std::io::stdout().lock();
    let (background, priority) = background_priority();
    let hello = serde_json::to_string(&Hello {
        version,
        background,
        priority,
    })
    .expect("hello serializes");
    if writeln!(stdout, "{hello}")
        .and_then(|_| stdout.flush())
        .is_err()
    {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(l) = line else { break };
            if tx.send(l).is_err() {
                break;
            }
        }
        std::process::exit(0);
    });
    for line in rx {
        let resp = match serde_json::from_str::<Request>(&line) {
            Ok(req) => {
                log_open(&hooks, "open", &req.path);
                run_hooks(&hooks, &req.path);
                let resp = handle(&req);
                log_open(&hooks, "close", &req.path);
                resp
            }
            Err(e) => Response::failed(FailReason::Crash, format!("bad request: {e}")),
        };
        let mut out = serde_json::to_string(&resp).expect("response serializes");
        out.push('\n');
        if stdout
            .write_all(out.as_bytes())
            .and_then(|_| stdout.flush())
            .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-2 AC-5: the output cap cuts at a character boundary.
    #[test]
    fn caps_text_at_char_boundary() {
        let mut s = "añoñ".to_string(); // a(1) ñ(2) o(1) ñ(2)
        assert!(cap_text(&mut s, 3));
        assert_eq!(s, "añ");
        let mut s = "año".to_string();
        assert!(cap_text(&mut s, 2));
        assert_eq!(s, "a");
        let mut t = "hola".to_string();
        assert!(!cap_text(&mut t, 10));
    }

    /// VFS-2 AC-5: the allocator refuses allocations above its cap and recovers on free.
    #[test]
    fn capped_alloc_refuses_over_cap() {
        let a = CappedAlloc::new();
        a.set_cap(1000);
        let l = Layout::from_size_align(800, 8).unwrap();
        unsafe {
            let p = a.alloc(l);
            assert!(!p.is_null());
            assert!(a.alloc(l).is_null());
            a.dealloc(p, l);
            let q = a.alloc(l);
            assert!(!q.is_null());
            a.dealloc(q, l);
        }
    }

    /// VFS-2 AC-4: a missing file fails with `Open`, an oversized one is skipped unread.
    #[test]
    fn open_errors_and_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let missing = handle(&Request {
            path: dir.path().join("no.txt"),
            max_input_bytes: 100,
            max_text_bytes: 100,
        });
        assert!(matches!(
            missing.outcome,
            Outcome::Failed {
                reason: FailReason::Open,
                ..
            }
        ));
        let big = dir.path().join("big.txt");
        std::fs::write(&big, vec![b'a'; 200]).unwrap();
        let r = handle(&Request {
            path: big,
            max_input_bytes: 100,
            max_text_bytes: 100,
        });
        assert_eq!(r.outcome, Outcome::Skipped(SkipReason::TooLarge));
        assert_eq!(r.bytes_read, 0);
    }
}
