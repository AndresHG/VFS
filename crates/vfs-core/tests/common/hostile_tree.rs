//! Hostile-tree helpers for `tests/hostile.rs` (VFS-1 AC-4, AC-6, AC-7, AC-8, AC-10).
//!
//! The generator never trusts that an entry it asked for exists: every `try_*` call reports
//! what the filesystem accepted, and the invariant compares the index against an
//! independent `std::fs` walk of what is really on disk. So the same test runs on APFS
//! (case- and normalization-insensitive, 255-byte names, PATH_MAX 1024) and on NTFS
//! (reserved names, stripped trailing dots, MAX_PATH unless long-path prefixed).
//!
//! The generator never creates a name the indexer excludes (`walk::is_excluded`), so the
//! disk walk needs no copy of that list.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use vfs_core::db::{self, ingest::ScanState};
use vfs_core::{fold_key, RootStatus, ScanSummary, SearchConfig, SearchQuery, Vfs};

/// True when the full-size stress variants run (`VFS_STRESS=1`).
pub fn stress() -> bool {
    std::env::var("VFS_STRESS").is_ok_and(|v| v == "1")
}

/// `full` under `VFS_STRESS=1`, else `reduced`.
pub fn sized<T>(full: T, reduced: T) -> T {
    if stress() {
        full
    } else {
        reduced
    }
}

/// A scan that takes longer than this is treated as a loop.
pub fn scan_limit() -> Duration {
    sized(Duration::from_secs(3600), Duration::from_secs(120))
}

/// One index with one or more roots under a private temporary directory.
/// Fields drop in order: the index closes before the directories are deleted, which
/// Windows refuses for open files (`TempDir` ignores the error and leaks them).
pub struct Env {
    pub vfs: Vfs,
    pub db_path: PathBuf,
    pub tmp: tempfile::TempDir,
    _db_dir: tempfile::TempDir,
}

impl Env {
    /// An empty index; roots are added with [`Env::add_root`].
    pub fn new() -> Env {
        let db_dir = tempfile::tempdir().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("index.db");
        // A generous deadline: a loaded test machine must not truncate the name checks.
        let config = SearchConfig {
            deadline: Duration::from_secs(30),
        };
        let vfs = Vfs::open_with(&db_path, config).unwrap();
        Env {
            _db_dir: db_dir,
            tmp,
            db_path,
            vfs,
        }
    }

    /// Canonical path of the temporary directory.
    pub fn base(&self) -> PathBuf {
        dunce::canonicalize(self.tmp.path()).unwrap()
    }

    /// Creates `<base>/<name>` and registers it as a root; returns `(root id, root path)`.
    pub fn add_root(&self, name: &str) -> (i64, PathBuf) {
        let path = self.base().join(name);
        fs::create_dir_all(&path).unwrap();
        let root = self.vfs.add_root(&path).unwrap();
        (root.id, PathBuf::from(root.path))
    }

    pub fn reader(&self) -> Connection {
        db::open_ro(&self.db_path).unwrap()
    }

    /// Scans to completion on another thread; panics when it outlives [`scan_limit`] (a
    /// loop) after asking it to stop.
    pub fn scan(&self) -> vfs_core::Result<ScanSummary> {
        let (tx, rx) = mpsc::channel();
        let vfs = self.vfs.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            let _ = tx.send(vfs.scan_to_completion(&flag));
        });
        let limit = scan_limit();
        match rx.recv_timeout(limit) {
            Ok(r) => r,
            Err(_) => {
                cancel.store(true, Ordering::SeqCst);
                panic!("scan_to_completion did not finish within {limit:?}: a loop?");
            }
        }
    }

    /// Scans and unwraps: hostile trees must never abort a scan.
    pub fn scan_ok(&self) -> ScanSummary {
        self.scan().expect("a hostile tree must not abort the scan")
    }

    /// Queues a full re-walk of `root_id`: its root directory pending deep, as the indexer's
    /// periodic and manual re-walks do (VFS-1 AC-7).
    pub fn force_rewalk(&self, root_id: i64) {
        db::open_rw(&self.db_path)
            .unwrap()
            .execute(
                "UPDATE dirs SET scan_state = ?2 WHERE root_id = ?1 AND rel_path = ''",
                params![root_id, ScanState::PendingDeep as i64],
            )
            .unwrap();
    }

    pub fn root_status(&self, root_id: i64) -> RootStatus {
        let code: i64 = self
            .reader()
            .query_row("SELECT status FROM roots WHERE id = ?1", [root_id], |r| {
                r.get(0)
            })
            .unwrap();
        RootStatus::from_code(code)
    }

    /// Directories of `root_id` still waiting to be listed.
    pub fn pending_dirs(&self, root_id: i64) -> Vec<String> {
        let conn = self.reader();
        let mut stmt = conn
            .prepare("SELECT rel_path FROM dirs WHERE root_id = ?1 AND scan_state <> ?2")
            .unwrap();
        let rows = stmt
            .query_map(params![root_id, ScanState::Clean as i64], |r| r.get(0))
            .unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    }
}

/// Records what the filesystem accepted while building a tree.
#[derive(Default)]
pub struct Probe {
    pub created: Vec<PathBuf>,
    pub refused: Vec<(PathBuf, String)>,
}

impl Probe {
    fn record(&mut self, path: &Path, r: io::Result<()>) -> bool {
        match r {
            Ok(()) => {
                self.created.push(path.to_path_buf());
                true
            }
            Err(e) => {
                self.refused.push((path.to_path_buf(), e.to_string()));
                false
            }
        }
    }

    /// Creates a new file of `bytes` bytes; false when the filesystem refused or an entry
    /// of that name (or a case or normalization variant of it) exists. `create_new` also
    /// keeps Windows device names (`CON`, `COM1`) from being opened and written.
    pub fn file(&mut self, path: &Path, bytes: usize) -> bool {
        let r = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .and_then(|mut f| f.write_all(&vec![b'x'; bytes]));
        self.record(path, r)
    }

    /// Creates a directory; true when it exists afterwards (created now or already there,
    /// as when a case or normalization variant names an existing one).
    pub fn dir(&mut self, path: &Path) -> bool {
        let r = match fs::create_dir(path) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
            r => r,
        };
        self.record(path, r)
    }

    /// Prints what was refused, for the test log.
    pub fn report(&self, what: &str) {
        eprintln!(
            "[{what}] created {} entries, refused {}",
            self.created.len(),
            self.refused.len()
        );
        for (p, e) in &self.refused {
            eprintln!("  refused {:?}: {e}", p.file_name().unwrap_or_default());
        }
    }
}

/// True when the filesystem under `dir` treats names differing only by ASCII case as one.
pub fn case_insensitive(dir: &Path) -> bool {
    let probe = dir.join("CaseProbe.tmpx");
    fs::write(&probe, b"x").unwrap();
    let r = dir.join("caseprobe.TMPX").exists();
    fs::remove_file(&probe).unwrap();
    r
}

/// True when the filesystem under `dir` treats NFC and NFD spellings of a name as one.
pub fn normalization_insensitive(dir: &Path) -> bool {
    let probe = dir.join("norm\u{00E9}.probe");
    fs::write(&probe, b"x").unwrap();
    let r = dir.join("norme\u{0301}.probe").exists();
    fs::remove_file(&probe).unwrap();
    r
}

/// Unix seconds, floored (negative before 1970).
pub fn floor_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => {
            let d = e.duration();
            let s = d.as_secs() as i64;
            if d.subsec_nanos() > 0 {
                -s - 1
            } else {
                -s
            }
        }
    }
}

/// What a `std::fs` walk of a root sees: files as `rel/name -> (size, mtime)`, directories
/// as `rel`, and the directories it could not list. Symlinks, junctions and anything that
/// is neither a file nor a directory are skipped, as the indexer does.
#[derive(Default, Debug)]
pub struct Snapshot {
    pub files: BTreeMap<String, (u64, i64)>,
    pub dirs: BTreeSet<String>,
    pub unreadable: BTreeSet<String>,
    /// Names that are not valid Unicode, by their lossy relative path.
    pub lossy: BTreeSet<String>,
}

fn join(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_string()
    } else {
        format!("{rel}/{name}")
    }
}

pub fn disk_snapshot(root: &Path) -> Snapshot {
    let mut snap = Snapshot::default();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                snap.unreadable.insert(rel);
                continue;
            }
        };
        for entry in rd {
            let Ok(entry) = entry else { continue };
            let os = entry.file_name();
            let name = os.to_string_lossy().into_owned();
            let Ok(ft) = entry.file_type() else { continue };
            let meta = match entry.metadata() {
                Ok(meta) => meta,
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    // Listed but not stat-able: the folder's content is unknown.
                    snap.unreadable.insert(rel.clone());
                    continue;
                }
                Err(_) => continue,
            };
            let child = join(&rel, &name);
            if os.to_str().is_none() {
                snap.lossy.insert(child.clone());
            }
            if ft.is_symlink() {
                // A link is a row of size 0 with its own time, never entered (VFS-18 R1).
                let mtime = meta.modified().map(floor_secs).unwrap_or(0);
                snap.files.insert(child, (0, mtime));
            } else if ft.is_dir() {
                snap.dirs.insert(child.clone());
                stack.push((entry.path(), child));
            } else if ft.is_file() {
                let mtime = meta.modified().map(floor_secs).unwrap_or(0);
                snap.files.insert(child, (meta.len(), mtime));
            }
        }
    }
    snap
}

/// One indexed file of a root.
pub struct IndexedFile {
    pub id: i64,
    pub rel: String,
    pub name: String,
    pub size: u64,
    pub mtime: i64,
}

/// Every indexed file of `root_id` by relative path, and every directory but the root.
pub fn index_snapshot(
    conn: &Connection,
    root_id: i64,
) -> (BTreeMap<String, IndexedFile>, BTreeMap<String, i64>) {
    let mut stmt = conn
        .prepare(
            "SELECT f.id, d.rel_path, f.name, f.size, f.mtime FROM files f \
             JOIN dirs d ON d.id = f.dir_id WHERE d.root_id = ?1",
        )
        .unwrap();
    let files = stmt
        .query_map([root_id], |r| {
            let rel: String = r.get(1)?;
            let name: String = r.get(2)?;
            Ok(IndexedFile {
                id: r.get(0)?,
                rel: join(&rel, &name),
                name,
                size: r.get::<_, i64>(3)? as u64,
                mtime: r.get(4)?,
            })
        })
        .unwrap()
        .map(|f| {
            let f = f.unwrap();
            (f.rel.clone(), f)
        })
        .collect();
    let mut stmt = conn
        .prepare("SELECT id, rel_path FROM dirs WHERE root_id = ?1 AND rel_path <> ''")
        .unwrap();
    let dirs = stmt
        .query_map([root_id], |r| Ok((r.get(1)?, r.get(0)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    (files, dirs)
}

/// Readable diff of two key sets, at most 20 lines each way.
fn diff<K: Ord + std::fmt::Debug, A, B>(
    what: &str,
    index: &BTreeMap<K, A>,
    disk: &BTreeMap<K, B>,
) -> Option<String> {
    let only_index: Vec<_> = index.keys().filter(|k| !disk.contains_key(*k)).collect();
    let only_disk: Vec<_> = disk.keys().filter(|k| !index.contains_key(*k)).collect();
    if only_index.is_empty() && only_disk.is_empty() {
        return None;
    }
    Some(format!(
        "{what}: {} only in the index {:?}; {} only on disk {:?}",
        only_index.len(),
        only_index.iter().take(20).collect::<Vec<_>>(),
        only_disk.len(),
        only_disk.iter().take(20).collect::<Vec<_>>()
    ))
}

/// True when `path` names an existing entry. Paths longer than the OS allows (macOS
/// PATH_MAX) are checked component by component through directory descriptors.
pub fn path_exists(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        #[cfg(unix)]
        Err(e) if e.raw_os_error() == Some(libc::ENAMETOOLONG) => deep::exists(path),
        Err(_) => false,
    }
}

/// Options of the invariant check.
pub struct Check {
    /// Search for at most this many files and folders by exact name (every one when
    /// `usize::MAX`).
    pub search_sample: usize,
}

impl Default for Check {
    fn default() -> Check {
        Check {
            search_sample: usize::MAX,
        }
    }
}

/// True when a search for `name` returns the file or folder `id`, or lists the file as a
/// copy of another hit.
pub fn found_by_name(vfs: &Vfs, name: &str, id: i64, is_dir: bool) -> bool {
    let mut offset = 0u32;
    loop {
        let r = vfs
            .search(&SearchQuery {
                text: name.to_string(),
                filters: Default::default(),
                limit: 500,
                offset,
            })
            .unwrap();
        // A file may be listed as a copy of another hit with the same name and size
        // (VFS-19): it is found there too.
        if r.hits.iter().any(|h| {
            h.is_dir == is_dir && (h.id == id || (!is_dir && h.copies.iter().any(|c| c.id == id)))
        }) {
            return true;
        }
        offset += 500;
        if offset >= r.total {
            return false;
        }
    }
}

/// A name is searched for when its folded, trimmed form has at least 3 characters (the
/// trigram minimum, VFS-1 AC-10).
pub fn searchable(name: &str) -> bool {
    fold_key(name).trim().chars().count() >= 3
}

/// The core invariant (VFS-1 AC-4, AC-7, AC-8, AC-10): the index of `root_id` equals what a
/// `std::fs` walk of `root` sees, file sizes and mtimes included; every indexed file and
/// folder path rebuilds to an existing entry; and searching a name finds it. Directories
/// the walk cannot list must still be waiting in the indexer's queue. Returns the snapshot.
pub fn assert_index_matches_disk(env: &Env, root_id: i64, root: &Path, check: Check) -> Snapshot {
    let disk = disk_snapshot(root);
    let conn = env.reader();
    let (files, dirs) = index_snapshot(&conn, root_id);

    let disk_dirs: BTreeMap<String, ()> = disk.dirs.iter().map(|d| (d.clone(), ())).collect();
    let mut problems: Vec<String> = Vec::new();
    problems.extend(diff("files", &files, &disk.files));
    problems.extend(diff("dirs", &dirs, &disk_dirs));
    for (rel, f) in &files {
        if let Some(&(size, mtime)) = disk.files.get(rel) {
            if (f.size, f.mtime) != (size, mtime) {
                problems.push(format!(
                    "{rel:?}: index (size {}, mtime {}) != disk (size {size}, mtime {mtime})",
                    f.size, f.mtime
                ));
            }
        }
    }
    let pending: BTreeSet<String> = env.pending_dirs(root_id).into_iter().collect();
    for rel in &disk.unreadable {
        if !pending.contains(rel) {
            problems.push(format!("unreadable dir {rel:?} is not pending a retry"));
        }
    }
    assert!(
        problems.is_empty(),
        "index != disk under {}:\n{}",
        root.display(),
        problems.join("\n")
    );

    for (rel, f) in &files {
        if disk.lossy.contains(rel) {
            continue; // rebuilt from a lossy name: cannot round-trip by design
        }
        let p = env.vfs.file_path(f.id).unwrap();
        assert_eq!(
            p,
            db::join_rel(&root.to_string_lossy(), rel),
            "file_path of {rel:?}"
        );
        assert!(
            path_exists(&p),
            "file_path of {rel:?} does not exist: {p:?}"
        );
    }
    for (rel, &id) in &dirs {
        if disk.lossy.contains(rel) {
            continue;
        }
        let p = env.vfs.dir_path(id).unwrap();
        assert!(path_exists(&p), "dir_path of {rel:?} does not exist: {p:?}");
    }

    let step = |n: usize| (n / check.search_sample.max(1)).max(1);
    let mut missed = Vec::new();
    let file_step = step(files.len());
    for f in files.values().step_by(file_step) {
        if !disk.lossy.contains(&f.rel)
            && searchable(&f.name)
            && !found_by_name(&env.vfs, &f.name, f.id, false)
        {
            missed.push(f.rel.clone());
        }
    }
    let dir_step = step(dirs.len());
    for (rel, &id) in dirs.iter().step_by(dir_step) {
        let name = rel.rsplit('/').next().unwrap();
        if !disk.lossy.contains(rel) && searchable(name) && !found_by_name(&env.vfs, name, id, true)
        {
            missed.push(format!("{rel}/"));
        }
    }
    assert!(
        missed.is_empty(),
        "{} indexed entries not found by their exact name: {:?}",
        missed.len(),
        missed.iter().take(20).collect::<Vec<_>>()
    );
    disk
}

/// Directory-descriptor helpers for building and checking trees deeper than PATH_MAX.
#[cfg(unix)]
pub mod deep {
    use std::ffi::{CString, OsStr};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path};

    pub struct DirFd(libc::c_int);

    impl Drop for DirFd {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }

    fn cstr(name: &OsStr) -> CString {
        CString::new(name.as_bytes()).unwrap()
    }

    fn check(rc: libc::c_int) -> io::Result<libc::c_int> {
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(rc)
        }
    }

    impl DirFd {
        pub fn open(path: &Path) -> io::Result<DirFd> {
            let c = cstr(path.as_os_str());
            let fd = check(unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY) })?;
            Ok(DirFd(fd))
        }

        pub fn open_at(&self, name: &OsStr) -> io::Result<DirFd> {
            let c = cstr(name);
            let fd = check(unsafe {
                libc::openat(
                    self.0,
                    c.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
                )
            })?;
            Ok(DirFd(fd))
        }

        /// Creates the subdirectory `name` (or reuses it) and opens it.
        pub fn mkdir_at(&self, name: &OsStr) -> io::Result<DirFd> {
            let c = cstr(name);
            let rc = unsafe { libc::mkdirat(self.0, c.as_ptr(), 0o755) };
            if rc < 0 {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::AlreadyExists {
                    return Err(e);
                }
            }
            self.open_at(name)
        }

        /// Creates the file `name` holding `bytes` bytes.
        pub fn file_at(&self, name: &OsStr, bytes: usize) -> io::Result<()> {
            let c = cstr(name);
            let fd = check(unsafe {
                libc::openat(
                    self.0,
                    c.as_ptr(),
                    libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                    0o644 as libc::c_uint,
                )
            })?;
            let fd = DirFd(fd);
            let buf = vec![b'x'; bytes];
            let n = unsafe { libc::write(fd.0, buf.as_ptr().cast(), buf.len()) };
            if n < 0 || n as usize != bytes {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn stat_at(&self, name: &OsStr) -> bool {
            let c = cstr(name);
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            unsafe { libc::fstatat(self.0, c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) == 0 }
        }
    }

    /// True when every component of the absolute `path` exists, walked through descriptors.
    pub fn exists(path: &Path) -> bool {
        let mut comps: Vec<&OsStr> = path
            .components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n),
                _ => None,
            })
            .collect();
        let Some(last) = comps.pop() else {
            return false;
        };
        let Ok(mut fd) = DirFd::open(Path::new("/")) else {
            return false;
        };
        for c in comps {
            match fd.open_at(c) {
                Ok(next) => fd = next,
                Err(_) => return false,
            }
        }
        fd.stat_at(last)
    }
}
