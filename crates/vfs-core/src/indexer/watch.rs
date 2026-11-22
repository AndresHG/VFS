//! Filesystem watcher: events are hints that mark directories dirty (VFS-1 AC-7).
//! Owned by task T3b. macOS/Linux: `notify` (FSEvents), honouring its rescan flag. Windows:
//! own `ReadDirectoryChangesW` loop with a 64 KB buffer; 0 bytes or an error queues a re-walk
//! of the root and re-arms the watch. Events under the app data directory, and events for
//! names the walker excludes (`walk::is_excluded`, any component below the root), are
//! dropped before they reach the indexer (VFS-1 I3, I8). On Windows a rename's old and new
//! names in one buffer are also reported as a pair (VFS-1 I9).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use super::exclude::{self, Exclusions};
use super::walk::is_excluded;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WatchEvent {
    /// Something changed at this absolute path; its parent (and itself, if a known directory)
    /// becomes pending shallow.
    Dirty(PathBuf),
    /// The watch overflowed and dropped events for this root, but stays armed: the changes
    /// can be read back from the change history (VFS-26 R1).
    Overflow(i64),
    /// The watch of this root failed (closed or never armed): events were lost while
    /// nothing watched (VFS-26 R1).
    Failed(i64),
    /// `from` was renamed or moved to `to` (absolute paths). Sent in addition to `Dirty` for
    /// both paths, so ignoring it is always safe.
    #[cfg_attr(not(windows), allow(dead_code))]
    Renamed { from: PathBuf, to: PathBuf },
}

/// What a lost batch of events means for the watch (VFS-26 R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Loss {
    /// Events dropped, the watch still armed: the history can be replayed.
    Overflow,
    /// The watch is closed: nothing watches until it is opened again.
    Failed,
}

/// `ERROR_NOTIFY_ENUM_DIR`: the system dropped the changes and asks for a re-listing.
const NOTIFY_ENUM_DIR: u32 = 1022;

/// How a completed `ReadDirectoryChangesW` lost events, if it did (VFS-26 R1): `error` is
/// `GetLastError` when `GetOverlappedResult` failed. 0 bytes or `ERROR_NOTIFY_ENUM_DIR` is an
/// overflow; any other error a failed watch. Pure, so it is tested on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn completion_loss(error: Option<u32>, bytes: u32) -> Option<Loss> {
    match error {
        Some(NOTIFY_ENUM_DIR) => Some(Loss::Overflow),
        Some(_) => Some(Loss::Failed),
        None if bytes == 0 => Some(Loss::Overflow),
        None => None,
    }
}

/// Receives watcher events on the watcher's own thread(s); must not block.
pub(crate) type WatchSink = Arc<dyn Fn(WatchEvent) + Send + Sync>;

pub(crate) struct RootWatcher {
    backend: imp::Backend,
}

impl RootWatcher {
    /// Creates a watcher that reports through `sink` and drops the events `filter` drops.
    pub(crate) fn start(sink: WatchSink, filter: Filter) -> Result<RootWatcher> {
        Ok(RootWatcher {
            backend: imp::Backend::new(sink, filter)?,
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

/// Paths whose events are dropped (the app data directory, as given and canonicalized, and
/// the default folder exclusions, VFS-33 R5). Replayed changes go through the same filter
/// (VFS-9 R7). Clones share the exclusion setting.
#[derive(Clone)]
pub(crate) struct Filter {
    ignore: Arc<Vec<PathBuf>>,
    rules: Arc<RwLock<Rules>>,
}

/// The default exclusions per root, built on first use; none while the user includes them.
struct Rules {
    include_all: bool,
    home: Option<PathBuf>,
    by_root: HashMap<PathBuf, Arc<Exclusions>>,
}

impl Filter {
    /// Drops `ignore` (the app data directory) as given and canonicalized.
    pub(crate) fn new(ignore: PathBuf) -> Filter {
        let mut ignore_list = Vec::with_capacity(2);
        if let Ok(canonical) = dunce::canonicalize(&ignore) {
            if canonical != ignore {
                ignore_list.push(canonical);
            }
        }
        ignore_list.push(ignore);
        Filter::from_list(ignore_list)
    }

    fn from_list(ignore: Vec<PathBuf>) -> Filter {
        Filter {
            ignore: Arc::new(ignore),
            rules: Arc::new(RwLock::new(Rules {
                include_all: false,
                home: exclude::home(),
                by_root: HashMap::new(),
            })),
        }
    }

    /// Turns the default exclusions off (`true`) or on, for every clone.
    pub(crate) fn set_include_all(&self, include_all: bool) {
        let mut rules = self.rules.write().unwrap_or_else(|e| e.into_inner());
        if rules.include_all != include_all {
            rules.include_all = include_all;
            rules.by_root.clear();
        }
    }

    /// The exclusions in force below `root`.
    pub(crate) fn exclusions(&self, root: &Path) -> Arc<Exclusions> {
        if let Some(x) = self
            .rules
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .by_root
            .get(root)
        {
            return Arc::clone(x);
        }
        let mut rules = self.rules.write().unwrap_or_else(|e| e.into_inner());
        let x = Arc::new(if rules.include_all {
            Exclusions::none(root)
        } else {
            Exclusions::for_root(root, rules.home.as_deref())
        });
        rules.by_root.insert(root.to_path_buf(), Arc::clone(&x));
        x
    }

    fn ignored(&self, path: &Path) -> bool {
        self.ignore.iter().any(|p| path.starts_with(p))
    }

    /// Whether events for `path`, under the watched `root`, are dropped: under the app data
    /// directory, with a component below the root that the walker excludes, or in a folder
    /// of the default exclusions.
    pub(crate) fn dropped(&self, path: &Path, root: &Path) -> bool {
        self.ignored(path)
            || path.strip_prefix(root).is_ok_and(|rel| {
                rel.components()
                    .any(|c| is_excluded(&c.as_os_str().to_string_lossy()))
            })
            || self.exclusions(root).drops(path)
    }
}

/// `FILE_ACTION_RENAMED_OLD_NAME` and `FILE_ACTION_RENAMED_NEW_NAME` of a
/// `FILE_NOTIFY_INFORMATION` record.
#[cfg(any(windows, test))]
const ACTION_RENAMED_OLD: u32 = 4;
#[cfg(any(windows, test))]
const ACTION_RENAMED_NEW: u32 = 5;

/// Turns one `ReadDirectoryChangesW` buffer of `root` into events: `Dirty` for every name
/// `filter` keeps, plus `Renamed` for each old-name record directly followed by its new-name
/// record when both are kept.
#[cfg(any(windows, test))]
fn notify_events(bytes: &[u8], root: &Path, filter: &Filter) -> Vec<WatchEvent> {
    let mut out = Vec::new();
    let mut old: Option<PathBuf> = None;
    for_each_notify_name(bytes, |action, name| {
        let path = name
            .split(|&u| u == u16::from(b'\\'))
            .fold(root.to_path_buf(), |p, part| p.join(os_from_wide(part)));
        let keep = !filter.dropped(&path, root);
        match action {
            ACTION_RENAMED_OLD => old = keep.then(|| path.clone()),
            ACTION_RENAMED_NEW => {
                if let (Some(from), true) = (old.take(), keep) {
                    out.push(WatchEvent::Renamed {
                        from,
                        to: path.clone(),
                    });
                }
            }
            _ => old = None,
        }
        if keep {
            out.push(WatchEvent::Dirty(path));
        }
    });
    out
}

#[cfg(windows)]
fn os_from_wide(wide: &[u16]) -> std::ffi::OsString {
    std::os::windows::ffi::OsStringExt::from_wide(wide)
}

/// Unit tests on other systems feed UTF-16 test names.
#[cfg(all(test, not(windows)))]
fn os_from_wide(wide: &[u16]) -> std::ffi::OsString {
    String::from_utf16_lossy(wide).into()
}

/// Walks the records of a `ReadDirectoryChangesW` buffer (`FILE_NOTIFY_INFORMATION`:
/// next-entry offset, action, name length in bytes, UTF-16 name) and calls `f` with each
/// action and name.
#[cfg(any(windows, test))]
fn for_each_notify_name(bytes: &[u8], mut f: impl FnMut(u32, &[u16])) {
    let u32_at =
        |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let mut off = 0usize;
    let mut name = Vec::new();
    while off + 12 <= bytes.len() {
        let next = u32_at(off) as usize;
        let action = u32_at(off + 4);
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
        f(action, &name);
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
            Ok(event) if event.need_rescan() => {
                lost(sink, roots, &event.paths, WatchEvent::Overflow)
            }
            Ok(event) => {
                if matches!(event.kind, EventKind::Access(_)) {
                    return;
                }
                let roots = lock(roots).clone();
                for path in event.paths {
                    let dropped = match roots.iter().find(|(_, r)| path.starts_with(r)) {
                        Some((_, root)) => filter.dropped(&path, root),
                        None => filter.ignored(&path),
                    };
                    if !dropped {
                        sink(WatchEvent::Dirty(path));
                    }
                }
            }
            Err(e) => {
                log::warn!("watcher error, queueing a re-walk: {e}");
                lost(sink, roots, &e.paths, WatchEvent::Failed);
            }
        }
    }

    /// Reports lost events (`kind`: overflow or failed watch) for the roots containing
    /// `paths`, or for every root when none does.
    pub(super) fn lost(
        sink: &WatchSink,
        roots: &Roots,
        paths: &[PathBuf],
        kind: fn(i64) -> WatchEvent,
    ) {
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
            sink(kind(id));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// VFS-26 AC-5: a `notify` event that needs a rescan is an overflow of the root
        /// holding its path, or of every root when none does; a `notify` error is a failed
        /// watch.
        #[test]
        fn notify_losses_are_classified() {
            let got = Arc::new(Mutex::new(Vec::new()));
            let seen = Arc::clone(&got);
            let sink: WatchSink = Arc::new(move |e| seen.lock().unwrap().push(e));
            let roots: Roots = Arc::new(Mutex::new(vec![
                (1, PathBuf::from("/r1")),
                (2, PathBuf::from("/r2")),
            ]));
            let filter = Filter::new(PathBuf::from("/data"));
            let rescan = |p: &str| {
                Ok(Event::new(EventKind::Other)
                    .set_flag(notify::event::Flag::Rescan)
                    .add_path(PathBuf::from(p)))
            };
            handle(&sink, &filter, &roots, rescan("/r1/a"));
            assert_eq!(*got.lock().unwrap(), vec![WatchEvent::Overflow(1)]);
            got.lock().unwrap().clear();
            handle(&sink, &filter, &roots, rescan("/elsewhere"));
            assert_eq!(
                *got.lock().unwrap(),
                vec![WatchEvent::Overflow(1), WatchEvent::Overflow(2)]
            );
            got.lock().unwrap().clear();
            let err = notify::Error::generic("gone").add_path(PathBuf::from("/r2/b"));
            handle(&sink, &filter, &roots, Err(err));
            assert_eq!(*got.lock().unwrap(), vec![WatchEvent::Failed(2)]);
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::thread::JoinHandle;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
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

    use super::{completion_loss, notify_events, Filter, Loss, WatchEvent, WatchSink};
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
        fn failed(&self) {
            (self.sink)(WatchEvent::Failed(self.root_id));
        }

        fn overflow(&self) {
            (self.sink)(WatchEvent::Overflow(self.root_id));
        }

        /// Arms `ReadDirectoryChangesW` until `stop` is signalled. Overflow (0 bytes or
        /// `ERROR_NOTIFY_ENUM_DIR`) re-arms on the same handle; any other failure closes the
        /// handle and reopens it every `REOPEN_MS`. Both report a re-walk of the root.
        fn run(self, dir: Owned) {
            let io_event = match new_event() {
                Ok(e) => e,
                Err(e) => {
                    log::warn!("watcher event for {}: {e}", self.root.display());
                    self.failed();
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
                        self.failed();
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
                    self.failed();
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
                    if completion_loss(Some(err), bytes) == Some(Loss::Failed) {
                        log::warn!(
                            "watch {} failed: {}",
                            self.root.display(),
                            std::io::Error::from_raw_os_error(err as i32)
                        );
                        dir = None;
                        self.failed();
                    } else {
                        self.overflow();
                    }
                    continue;
                }
                if completion_loss(None, bytes) == Some(Loss::Overflow) {
                    // The buffer overflowed: the system dropped this batch of changes.
                    self.overflow();
                    continue;
                }
                // SAFETY: the system wrote `bytes` (<= BUF_BYTES) bytes into `buf`.
                let data = unsafe {
                    std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), bytes as usize)
                };
                for event in notify_events(data, &self.root, &self.filter) {
                    (self.sink)(event);
                }
            }
        }
    }
}

#[cfg(test)]
mod live_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-26 AC-5: a completed watch with 0 bytes or `ERROR_NOTIFY_ENUM_DIR` overflowed; any
    /// other error failed; bytes and no error lost nothing.
    #[test]
    fn completions_are_classified() {
        assert_eq!(completion_loss(None, 0), Some(Loss::Overflow));
        assert_eq!(
            completion_loss(Some(NOTIFY_ENUM_DIR), 0),
            Some(Loss::Overflow)
        );
        for err in [5, 6, 64, 87] {
            assert_eq!(completion_loss(Some(err), 0), Some(Loss::Failed), "{err}");
        }
        assert_eq!(completion_loss(None, 120), None);
    }

    /// VFS-26 R1: the overflow code is windows-sys's `ERROR_NOTIFY_ENUM_DIR`.
    #[cfg(windows)]
    #[test]
    fn notify_enum_dir_matches_windows_sys() {
        assert_eq!(
            NOTIFY_ENUM_DIR,
            windows_sys::Win32::Foundation::ERROR_NOTIFY_ENUM_DIR
        );
    }

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
        for_each_notify_name(&buf, |a, n| names.push((a, String::from_utf16_lossy(n))));
        assert_eq!(
            names,
            [
                (1, "Notaría.pdf".to_string()),
                (5, "sub\\nuevo.txt".to_string()),
                (2, "borrado".to_string())
            ]
        );

        names.clear();
        let mut truncated = record(1, "largo-nombre.txt");
        truncated.truncate(16);
        for_each_notify_name(&truncated, |a, n| {
            names.push((a, String::from_utf16_lossy(n)))
        });
        assert!(names.is_empty());
    }

    /// Chains records into one buffer, setting each next-entry offset.
    fn buffer(records: Vec<Vec<u8>>) -> Vec<u8> {
        let n = records.len();
        let mut buf = Vec::new();
        for (i, mut r) in records.into_iter().enumerate() {
            if i + 1 < n {
                let next = r.len() as u32;
                r[..4].copy_from_slice(&next.to_le_bytes());
            }
            buf.extend(r);
        }
        buf
    }

    /// VFS-1 AC-7: events under the app data directory are dropped.
    #[test]
    fn filter_drops_app_data_paths() {
        let f = Filter::from_list(vec![PathBuf::from("/datos/app")]);
        assert!(f.ignored(Path::new("/datos/app/index.db-wal")));
        assert!(!f.ignored(Path::new("/datos/apple.txt")));
    }

    /// VFS-1 AC-7, I8: events for names the walker excludes, at any depth below the root,
    /// are dropped before they can mark a folder dirty; an excluded name above the root is
    /// not a reason to drop.
    #[test]
    fn filter_drops_excluded_names_below_root() {
        let f = Filter::from_list(vec![PathBuf::from("/datos/app")]);
        let root = Path::new("/discos/#recycle/Archivo");
        for p in [
            "Fotos/.SynologyWorkingDirectory/x.db",
            "Fotos/@eaDir/IMG_1.JPG/SYNOFILE_THUMB_M.jpg",
            "Docs/~WRL0003.tmp",
            "Docs/.~lock.cuentas.ods#",
            "#snapshot",
        ] {
            assert!(f.dropped(&root.join(p), root), "{p}");
        }
        assert!(!f.dropped(&root.join("Docs/cuentas.ods"), root));
        assert!(f.dropped(Path::new("/datos/app/index.db"), Path::new("/datos")));
    }

    /// VFS-1 AC-7, I8, I9: a Windows buffer yields `Dirty` for kept names only, and a rename
    /// pair (old name, then new name) also yields `Renamed`; a pair with an excluded side
    /// yields no `Renamed`.
    #[test]
    fn notify_buffer_pairs_renames_and_drops_excluded() {
        let f = Filter::from_list(Vec::new());
        let root = Path::new("/r");
        let buf = buffer(vec![
            record(ACTION_RENAMED_OLD, "Viejo"),
            record(ACTION_RENAMED_NEW, "sub\\Nuevo"),
            record(1, "Docs\\~WRL0001.tmp"),
            record(ACTION_RENAMED_OLD, "Docs\\~WRL0001.tmp"),
            record(ACTION_RENAMED_NEW, "Docs\\informe.docx"),
        ]);
        assert_eq!(
            notify_events(&buf, root, &f),
            vec![
                WatchEvent::Dirty(PathBuf::from("/r/Viejo")),
                WatchEvent::Renamed {
                    from: PathBuf::from("/r/Viejo"),
                    to: PathBuf::from("/r/sub/Nuevo"),
                },
                WatchEvent::Dirty(PathBuf::from("/r/sub/Nuevo")),
                WatchEvent::Dirty(PathBuf::from("/r/Docs/informe.docx")),
            ]
        );
    }
}
