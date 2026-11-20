//! Filesystem watcher: events are hints that mark directories dirty (VFS-1 AC-7).
//! Owned by task T3b. macOS/Linux: `notify` (FSEvents), honouring its rescan flag. Windows:
//! own `ReadDirectoryChangesW` loop with a 64 KB buffer; 0 bytes or an error queues a re-walk
//! of the root and re-arms the watch. Events under the app data directory are dropped.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WatchEvent {
    /// Something changed at this absolute path; its parent (and itself, if a known directory)
    /// becomes pending shallow.
    Dirty(PathBuf),
    /// Events were lost for this root; queue a full re-walk.
    Rescan(i64),
}

/// Receives watcher events on the watcher's own thread(s); must not block.
pub(crate) type WatchSink = Arc<dyn Fn(WatchEvent) + Send + Sync>;

pub(crate) struct RootWatcher {
    backend: imp::Backend,
}

impl RootWatcher {
    /// Creates a watcher that reports through `sink` and drops events under `ignore` (the
    /// app's data directory).
    pub(crate) fn start(sink: WatchSink, ignore: PathBuf) -> Result<RootWatcher> {
        let mut ignore_list = Vec::with_capacity(2);
        if let Ok(canonical) = dunce::canonicalize(&ignore) {
            if canonical != ignore {
                ignore_list.push(canonical);
            }
        }
        ignore_list.push(ignore);
        Ok(RootWatcher {
            backend: imp::Backend::new(sink, Filter(Arc::new(ignore_list)))?,
        })
    }

    /// Watches `path` recursively, reporting events for root `root_id`. Watching an id again
    /// replaces its previous path.
    pub(crate) fn watch(&mut self, root_id: i64, path: &Path) -> Result<()> {
        self.backend.unwatch(root_id);
        self.backend.watch(root_id, path)
    }

    pub(crate) fn unwatch(&mut self, root_id: i64) {
        self.backend.unwatch(root_id);
    }
}

/// Paths whose events are dropped (the app data directory, as given and canonicalized).
#[derive(Clone)]
struct Filter(Arc<Vec<PathBuf>>);

impl Filter {
    fn ignored(&self, path: &Path) -> bool {
        self.0.iter().any(|p| path.starts_with(p))
    }
}

/// Walks the records of a `ReadDirectoryChangesW` buffer (`FILE_NOTIFY_INFORMATION`:
/// next-entry offset, action, name length in bytes, UTF-16 name) and calls `f` with each name.
#[cfg(any(windows, test))]
fn for_each_notify_name(bytes: &[u8], mut f: impl FnMut(&[u16])) {
    let u32_at =
        |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let mut off = 0usize;
    let mut name = Vec::new();
    while off + 12 <= bytes.len() {
        let next = u32_at(off) as usize;
        let len = u32_at(off + 8) as usize;
        let Some(raw) = bytes.get(off + 12..off + 12 + len) else {
            break;
        };
        name.clear();
        name.extend(
            raw.as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c)),
        );
        f(&name);
        if next == 0 {
            break;
        }
        off += next;
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, MutexGuard};

    use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

    use super::{Filter, WatchEvent, WatchSink};
    use crate::error::Result;

    fn io_error(e: notify::Error) -> crate::error::Error {
        std::io::Error::other(e.to_string()).into()
    }

    type Roots = Arc<Mutex<Vec<(i64, PathBuf)>>>;

    fn lock(roots: &Roots) -> MutexGuard<'_, Vec<(i64, PathBuf)>> {
        roots.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) struct Backend {
        watcher: RecommendedWatcher,
        roots: Roots,
    }

    impl Backend {
        pub(super) fn new(sink: WatchSink, filter: Filter) -> Result<Backend> {
            let roots: Roots = Arc::default();
            let shared = Arc::clone(&roots);
            let watcher = notify::recommended_watcher(move |res| {
                handle(&sink, &filter, &shared, res);
            })
            .map_err(io_error)?;
            Ok(Backend { watcher, roots })
        }

        pub(super) fn watch(&mut self, root_id: i64, path: &Path) -> Result<()> {
            // FSEvents reports canonical paths (`/private/var/...`, not `/var/...`).
            let path = dunce::canonicalize(path)?;
            // Registered first so a rescan event arriving during `watch` finds its root; the
            // lock is never held across `notify` calls (FSEvents restarts its stream there).
            lock(&self.roots).push((root_id, path.clone()));
            if let Err(e) = self.watcher.watch(&path, RecursiveMode::Recursive) {
                lock(&self.roots).retain(|(id, _)| *id != root_id);
                return Err(io_error(e));
            }
            Ok(())
        }

        pub(super) fn unwatch(&mut self, root_id: i64) {
            let removed: Vec<PathBuf> = {
                let mut roots = lock(&self.roots);
                let removed = roots
                    .iter()
                    .filter(|(id, _)| *id == root_id)
                    .map(|(_, p)| p.clone())
                    .collect();
                roots.retain(|(id, _)| *id != root_id);
                removed
            };
            for path in removed {
                if let Err(e) = self.watcher.unwatch(&path) {
                    log::debug!("unwatch {}: {e}", path.display());
                }
            }
        }
    }

    fn handle(sink: &WatchSink, filter: &Filter, roots: &Roots, res: notify::Result<Event>) {
        match res {
            Ok(event) if event.need_rescan() => rescan(sink, roots, &event.paths),
            Ok(event) => {
                if matches!(event.kind, EventKind::Access(_)) {
                    return;
                }
                for path in event.paths {
                    if !filter.ignored(&path) {
                        sink(WatchEvent::Dirty(path));
                    }
                }
            }
            Err(e) => {
                log::warn!("watcher error, queueing a re-walk: {e}");
                rescan(sink, roots, &e.paths);
            }
        }
    }

    /// Queues a re-walk of the roots containing `paths`, or of every root when none does.
    fn rescan(sink: &WatchSink, roots: &Roots, paths: &[PathBuf]) {
        let roots = lock(roots).clone();
        let hit: Vec<i64> = roots
            .iter()
            .filter(|(_, root)| paths.iter().any(|p| p.starts_with(root)))
            .map(|(id, _)| *id)
            .collect();
        let ids = if hit.is_empty() {
            roots.iter().map(|(id, _)| *id).collect()
        } else {
            hit
        };
        for id in ids {
            sink(WatchEvent::Rescan(id));
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};
    use std::thread::JoinHandle;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_NOTIFY_ENUM_DIR, HANDLE, INVALID_HANDLE_VALUE,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
        FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
        FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

    use super::{for_each_notify_name, Filter, WatchEvent, WatchSink};
    use crate::error::Result;

    /// 64 KB: the largest buffer `ReadDirectoryChangesW` accepts on network shares.
    const BUF_BYTES: usize = 64 * 1024;
    const FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE
        | FILE_NOTIFY_CHANGE_CREATION;
    /// Delay between attempts to reopen a root whose handle failed (volume gone, root moved).
    const REOPEN_MS: u32 = 2000;

    /// A kernel handle closed on drop.
    struct Owned(HANDLE);

    // SAFETY: kernel handles may be used and closed from any thread.
    unsafe impl Send for Owned {}
    unsafe impl Sync for Owned {}

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a valid handle owned by this value.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn new_event() -> Result<Owned> {
        // SAFETY: manual-reset, initially non-signalled, unnamed event.
        let h = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if h.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Owned(h))
    }

    fn open_dir(path: &Path) -> std::io::Result<Owned> {
        let wide: Vec<u16> = crate::platform::long_path(path)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // SAFETY: `wide` is NUL-terminated; the other arguments are plain flags.
        let h = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Owned(h))
    }

    struct RootThread {
        stop: std::sync::Arc<Owned>,
        join: Option<JoinHandle<()>>,
    }

    impl Drop for RootThread {
        fn drop(&mut self) {
            // SAFETY: `stop` is a valid event handle kept alive by the `Arc`.
            unsafe { SetEvent(self.stop.0) };
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }

    pub(super) struct Backend {
        sink: WatchSink,
        filter: Filter,
        threads: HashMap<i64, RootThread>,
    }

    impl Backend {
        pub(super) fn new(sink: WatchSink, filter: Filter) -> Result<Backend> {
            Ok(Backend {
                sink,
                filter,
                threads: HashMap::new(),
            })
        }

        pub(super) fn watch(&mut self, root_id: i64, path: &Path) -> Result<()> {
            let dir = open_dir(path)?;
            let stop = std::sync::Arc::new(new_event()?);
            let ctx = Loop {
                root_id,
                root: path.to_path_buf(),
                sink: self.sink.clone(),
                filter: self.filter.clone(),
                stop: stop.clone(),
            };
            let join = std::thread::Builder::new()
                .name(format!("vfs-watch-{root_id}"))
                .spawn(move || ctx.run(dir))?;
            self.threads.insert(
                root_id,
                RootThread {
                    stop,
                    join: Some(join),
                },
            );
            Ok(())
        }

        pub(super) fn unwatch(&mut self, root_id: i64) {
            self.threads.remove(&root_id);
        }
    }

    struct Loop {
        root_id: i64,
        root: PathBuf,
        sink: WatchSink,
        filter: Filter,
        stop: std::sync::Arc<Owned>,
    }

    impl Loop {
        fn rescan(&self) {
            (self.sink)(WatchEvent::Rescan(self.root_id));
        }

        /// Arms `ReadDirectoryChangesW` until `stop` is signalled. Overflow (0 bytes or
        /// `ERROR_NOTIFY_ENUM_DIR`) re-arms on the same handle; any other failure closes the
        /// handle and reopens it every `REOPEN_MS`. Both report a re-walk of the root.
        fn run(self, dir: Owned) {
            let io_event = match new_event() {
                Ok(e) => e,
                Err(e) => {
                    log::warn!("watcher event for {}: {e}", self.root.display());
                    self.rescan();
                    return;
                }
            };
            // `u32` elements keep the buffer DWORD-aligned, as the API requires.
            let mut buf = vec![0u32; BUF_BYTES / 4];
            let mut dir = Some(dir);
            loop {
                let Some(handle) = dir.as_ref().map(|d| d.0) else {
                    // SAFETY: `stop` is a valid event handle.
                    match unsafe { WaitForSingleObject(self.stop.0, REOPEN_MS) } {
                        WAIT_TIMEOUT => {}
                        _ => return,
                    }
                    if let Ok(d) = open_dir(&self.root) {
                        dir = Some(d);
                        // Changes made while the handle was closed were not seen.
                        self.rescan();
                    }
                    continue;
                };
                // SAFETY: OVERLAPPED is a plain C struct; zero is its initial state.
                let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
                ov.hEvent = io_event.0;
                // SAFETY: `buf` and `ov` outlive the operation: every path below waits for it
                // to complete (or cancels it and waits) before they are reused or dropped.
                let armed = unsafe {
                    ResetEvent(io_event.0);
                    ReadDirectoryChangesW(
                        handle,
                        buf.as_mut_ptr().cast(),
                        BUF_BYTES as u32,
                        1,
                        FILTER,
                        std::ptr::null_mut(),
                        &mut ov,
                        None,
                    )
                };
                if armed == 0 {
                    log::warn!(
                        "watch {} failed: {}",
                        self.root.display(),
                        std::io::Error::last_os_error()
                    );
                    self.rescan();
                    dir = None;
                    continue;
                }
                let handles = [io_event.0, self.stop.0];
                // SAFETY: both handles are valid events.
                let woke = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
                let mut bytes = 0u32;
                if woke != WAIT_OBJECT_0 {
                    // Stop requested (or the wait failed): cancel and wait for completion.
                    // SAFETY: `handle` and `ov` belong to the pending operation.
                    unsafe {
                        CancelIoEx(handle, &ov);
                        GetOverlappedResult(handle, &ov, &mut bytes, 1);
                    }
                    return;
                }
                // SAFETY: the operation has completed (its event is signalled).
                if unsafe { GetOverlappedResult(handle, &ov, &mut bytes, 0) } == 0 {
                    // SAFETY: reads the calling thread's last error.
                    let err = unsafe { GetLastError() };
                    if err != ERROR_NOTIFY_ENUM_DIR {
                        log::warn!(
                            "watch {} failed: {}",
                            self.root.display(),
                            std::io::Error::from_raw_os_error(err as i32)
                        );
                        dir = None;
                    }
                    self.rescan();
                    continue;
                }
                if bytes == 0 {
                    // The buffer overflowed: the system dropped this batch of changes.
                    self.rescan();
                    continue;
                }
                // SAFETY: the system wrote `bytes` (<= BUF_BYTES) bytes into `buf`.
                let data = unsafe {
                    std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), bytes as usize)
                };
                for_each_notify_name(data, |name| {
                    let path = self.root.join(OsString::from_wide(name));
                    if !self.filter.ignored(&path) {
                        (self.sink)(WatchEvent::Dirty(path));
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod live_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// One `FILE_NOTIFY_INFORMATION` record, padded to 4 bytes, with a next offset of 0.
    fn record(action: u32, name: &str) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut out = Vec::new();
        out.extend(0u32.to_le_bytes());
        out.extend(action.to_le_bytes());
        out.extend(((units.len() * 2) as u32).to_le_bytes());
        for u in units {
            out.extend(u.to_le_bytes());
        }
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    /// VFS-1 AC-7: the Windows notification buffer parser yields every record's name,
    /// including non-ASCII names, and stops at a truncated record.
    #[test]
    fn parses_notify_buffer() {
        let records = [
            record(1, "Notaría.pdf"),
            record(5, "sub\\nuevo.txt"),
            record(2, "borrado"),
        ];
        let mut buf = Vec::new();
        for (i, mut r) in records.into_iter().enumerate() {
            if i < 2 {
                let next = r.len() as u32;
                r[..4].copy_from_slice(&next.to_le_bytes());
            }
            buf.extend(r);
        }
        let mut names = Vec::new();
        for_each_notify_name(&buf, |n| names.push(String::from_utf16_lossy(n)));
        assert_eq!(names, ["Notaría.pdf", "sub\\nuevo.txt", "borrado"]);

        names.clear();
        let mut truncated = record(1, "largo-nombre.txt");
        truncated.truncate(16);
        for_each_notify_name(&truncated, |n| names.push(String::from_utf16_lossy(n)));
        assert!(names.is_empty());
    }

    /// VFS-1 AC-7: events under the app data directory are dropped.
    #[test]
    fn filter_drops_app_data_paths() {
        let f = Filter(Arc::new(vec![PathBuf::from("/datos/app")]));
        assert!(f.ignored(Path::new("/datos/app/index.db-wal")));
        assert!(!f.ignored(Path::new("/datos/apple.txt")));
    }
}
