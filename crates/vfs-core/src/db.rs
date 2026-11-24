//! Connections, pragmas, migrations and the write helpers shared by the indexer and the
//! synthetic database generator (`ingest`), so both produce exactly the same layout.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::error::{Error, Result};
use crate::types::{LinkKind, LinkTarget};

/// `PRAGMA user_version` written by this build. Later versions (such as document content
/// search, VFS-1 AC-15) add tables without touching existing ones.
pub const SCHEMA_VERSION: u32 = 7;

/// Migrations: entry `i` takes an index from version `i` to `i + 1`.
const MIGRATIONS: [&str; 7] = [
    include_str!("schema.sql"),
    include_str!("schema_v2.sql"),
    include_str!("schema_v3.sql"),
    include_str!("schema_v4.sql"),
    include_str!("schema_v5.sql"),
    include_str!("schema_v6.sql"),
    include_str!("schema_v7.sql"),
];

/// An index migrated from a version below this one is compacted afterwards: the migrations
/// to versions 3 and 4 free about a fifth of the file. Later ones (5: `links`, 6:
/// `dirs.error`, 7: VFS-37 counts) free nothing, and a `VACUUM` rewrites the whole index:
/// minutes on the target HDD right after an update, for nothing.
const COMPACT_FROM_BELOW: u32 = 4;

/// Page size of a new index, and of an existing index compacted after a migration.
/// A cold spinning disk pays about one seek per page read: larger pages read fewer pages
/// per query (docs/research/index-footprint.md, VFS-4).
pub(crate) const PAGE_SIZE: u32 = 16384;

/// Readers wait this long for a lock.
const READER_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// A writer waits this long for the other writer (indexer vs facade root changes).
const WRITER_BUSY_TIMEOUT: Duration = Duration::from_secs(30);
/// Page cache of a writer and of a reader, KiB (negative `cache_size`). With `mmap_size=0`
/// these bound the index's resident memory (VFS-1 AC-9).
const WRITER_CACHE_KIB: i64 = 32 * 1024;
const READER_CACHE_KIB: i64 = 64 * 1024;
/// WAL pages before an automatic checkpoint.
pub(crate) const WAL_AUTOCHECKPOINT_PAGES: i64 = 4000;
/// The WAL is truncated to this size after a checkpoint.
const JOURNAL_SIZE_LIMIT_BYTES: i64 = 64 * 1024 * 1024;
/// Windows: a writer grows the index file in steps of this size, so a file that reaches
/// 1–2 GB while Synology writes to the same disk ends up in fewer, larger extents
/// (`SQLITE_FCNTL_CHUNK_SIZE`, VFS-5 R10). SQLite extends with `SetEndOfFile` there, which
/// allocates without writing; on macOS it would write a byte per block, so it stays off.
#[cfg(windows)]
const WRITER_CHUNK_BYTES: i32 = 32 * 1024 * 1024;
/// Prepared statements kept per writer connection: every `prepare_cached` statement of the
/// ingest helpers and the indexer loop stays cached.
const WRITER_STATEMENT_CACHE: usize = 32;

/// Soft limit on SQLite's heap for the whole process (VFS-41 R3): above it, page caches
/// recycle their own pages instead of growing. Every SQLite database of the app (index,
/// content store, bookmarks) shares it.
pub(crate) const SOFT_HEAP_LIMIT_BYTES: i64 = 96 << 20;

/// Sets [`SOFT_HEAP_LIMIT_BYTES`] for this process (VFS-41 R3).
pub(crate) fn limit_sqlite_heap() {
    // SAFETY: a process-wide setting with no pointer arguments, safe to call at any time.
    unsafe {
        rusqlite::ffi::sqlite3_soft_heap_limit64(SOFT_HEAP_LIMIT_BYTES);
    }
}

/// SQLite's heap in bytes for this process: (in use now, highest since start).
pub(crate) fn sqlite_heap() -> (u64, u64) {
    // SAFETY: read-only counters; `0` leaves the high-water mark as it is.
    unsafe {
        (
            rusqlite::ffi::sqlite3_memory_used().max(0) as u64,
            rusqlite::ffi::sqlite3_memory_highwater(0).max(0) as u64,
        )
    }
}

/// Suffixes of SQLite's companion files next to the index.
pub(crate) const WAL_SUFFIX: &str = "-wal";
const SHM_SUFFIX: &str = "-shm";

/// Pragmas shared by every connection. `mmap_size=0` keeps resident memory bounded by the
/// page caches (VFS-1 AC-9).
fn apply_common_pragmas(conn: &Connection) -> Result<()> {
    conn.busy_timeout(READER_BUSY_TIMEOUT)?;
    // Two processes creating the index at once race to switch it to WAL, and SQLite reports
    // that race as busy without calling the busy handler: retry it ourselves (VFS-3 AC-9).
    let started = std::time::Instant::now();
    loop {
        match conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0)) {
            Ok(_) => break,
            Err(e)
                if e.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy)
                    && started.elapsed() < READER_BUSY_TIMEOUT =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e.into()),
        }
    }
    conn.execute_batch(
        "PRAGMA synchronous=NORMAL;
         PRAGMA temp_store=MEMORY;
         PRAGMA foreign_keys=OFF;",
    )?;
    let _mmap: Option<i64> = conn.query_row("PRAGMA mmap_size=0", [], |r| r.get(0)).ok();
    Ok(())
}

fn schema_version(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Opens (creating if needed) the index for writing: writer pragmas, then creates or
/// migrates the schema. Fails with `SchemaTooNew` for an index from a newer build.
pub fn open_rw(db_path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(db_path)?;
    // Takes effect only on a new, empty database (before WAL writes its header).
    conn.execute_batch(&format!("PRAGMA page_size={PAGE_SIZE}"))?;
    apply_common_pragmas(&conn)?;
    conn.busy_timeout(WRITER_BUSY_TIMEOUT)?;
    conn.set_prepared_statement_cache_capacity(WRITER_STATEMENT_CACHE);
    conn.execute_batch(&format!(
        "PRAGMA cache_size=-{WRITER_CACHE_KIB};
         PRAGMA wal_autocheckpoint={WAL_AUTOCHECKPOINT_PAGES};"
    ))?;
    let _limit: i64 = conn.query_row(
        &format!("PRAGMA journal_size_limit={JOURNAL_SIZE_LIMIT_BYTES}"),
        [],
        |r| r.get(0),
    )?;
    #[cfg(windows)]
    set_chunk_size(&conn, WRITER_CHUNK_BYTES)?;
    let version = schema_version(&conn)?;
    if version > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew);
    }
    if version < SCHEMA_VERSION {
        // Another process may be migrating the same index: take the write lock first and
        // read the version again under it, so each migration runs once (VFS-3 AC-9).
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version = schema_version(&tx)?;
        if version > SCHEMA_VERSION {
            return Err(Error::SchemaTooNew);
        }
        for sql in &MIGRATIONS[version as usize..] {
            tx.execute_batch(sql)?;
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        // A migrated index (not a new one) gets back the pages its migrations freed, when
        // one of them freed any.
        if version > 0 && version < COMPACT_FROM_BELOW {
            if let Err(e) = compact(&conn) {
                // Still a valid index: the freed pages are reused as it grows. WAL is
                // required, so a failure to return to it is an error.
                log::warn!("index not compacted after migration: {e}");
                let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
            }
        }
    }
    Ok(conn)
}

/// Rewrites the index without the pages its migrations freed (version 3: the old FTS
/// tables, about a fifth of the file; version 4: the old `files` table and its name index)
/// and at `PAGE_SIZE`. SQLite changes the page size only
/// outside WAL mode, so the journal mode is switched around the `VACUUM`; this fails
/// harmlessly while another connection has the index open.
fn compact(conn: &Connection) -> Result<()> {
    let _mode: String = conn.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
    let vacuum = conn.execute_batch(&format!("PRAGMA page_size={PAGE_SIZE}; VACUUM;"));
    let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
    Ok(vacuum?)
}

/// Makes the main database file of `conn` grow and shrink in multiples of `bytes`.
#[cfg_attr(not(windows), allow(dead_code))]
fn set_chunk_size(conn: &Connection, bytes: i32) -> Result<()> {
    let mut size: std::ffi::c_int = bytes;
    // SAFETY: `conn.handle()` is the open connection's handle; `size` outlives the call, and
    // SQLITE_FCNTL_CHUNK_SIZE reads one int through the pointer.
    let rc = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            conn.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_CHUNK_SIZE,
            (&mut size as *mut std::ffi::c_int).cast(),
        )
    };
    if rc != rusqlite::ffi::SQLITE_OK {
        return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rc), None).into());
    }
    Ok(())
}

/// Opens an existing index for reading only (`query_only=1`).
pub fn open_ro(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    apply_common_pragmas(&conn)?;
    conn.execute_batch(&format!(
        "PRAGMA cache_size=-{READER_CACHE_KIB};
         PRAGMA query_only=1;"
    ))?;
    if schema_version(&conn)? > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew);
    }
    Ok(conn)
}

/// `<db_path>.bad`, where [`open_or_recover`] moves an unreadable index.
pub fn bad_path(db_path: &Path) -> PathBuf {
    let mut p = db_path.as_os_str().to_owned();
    p.push(".bad");
    PathBuf::from(p)
}

/// Suffix of the session directory next to the index: `<db_path>.session/`.
pub(crate) const SESSION_DIR_SUFFIX: &str = ".session";
/// Extension of a session marker file, `<pid>-<n>.lock`.
pub(crate) const SESSION_MARKER_EXT: &str = "lock";
/// Extension of a marker while it is being created and locked.
const SESSION_TMP_EXT: &str = "tmp";
/// How long `begin_session` retries Windows' transient access-denied errors on markers.
const SESSION_RETRY: Duration = Duration::from_secs(5);

/// `<db_path>.session/`: one marker file per open session, held under an OS file lock for
/// the session's lifetime, so a CLI and the app can share an index.
pub(crate) fn session_dir(db_path: &Path) -> PathBuf {
    let mut p = db_path.as_os_str().to_owned();
    p.push(SESSION_DIR_SUFFIX);
    PathBuf::from(p)
}

/// An open session on an index. Dropping it is the clean shutdown: its marker file is
/// unlocked and removed (and the session directory too, once empty).
pub(crate) struct Session {
    file: Option<File>,
    path: PathBuf,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.unlock();
        }
        let _ = std::fs::remove_file(&self.path);
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::remove_dir(dir); // fails while other sessions are open
        }
    }
}

/// Starts a session and reports whether an earlier one ended uncleanly: a marker file that
/// no live session holds locked (its process died) is stale; it is removed. A marker is
/// created exclusively and locked under a temporary name, then renamed, so no other opener
/// can mistake it for stale; only the holder of `<pid>-<n>.tmp` may create `<pid>-<n>.lock`.
pub(crate) fn begin_session(db_path: &Path) -> Result<(Session, bool)> {
    let dir = session_dir(db_path);
    let mut unclean = false;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for path in entries.flatten().map(|e| e.path()) {
            if path.extension().is_none_or(|e| e != SESSION_MARKER_EXT) {
                continue;
            }
            let Ok(file) = File::options().read(true).write(true).open(&path) else {
                continue;
            };
            if file.try_lock().is_ok() {
                unclean = true;
                let _ = file.unlock();
                drop(file);
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let pid = std::process::id();
    let mut n = 0u32;
    // On Windows a marker or session directory that a closing session just deleted stays
    // "delete pending" while another opener still holds a handle on it, and creating a file
    // in it or renaming onto its name fails with access denied until that handle closes.
    // Such a failure is retried, with the next `n`, for up to `SESSION_RETRY` (VFS-3 AC-9).
    let started = std::time::Instant::now();
    let transient = |e: &std::io::Error| {
        e.kind() == std::io::ErrorKind::PermissionDenied && started.elapsed() < SESSION_RETRY
    };
    let pause = || std::thread::sleep(Duration::from_millis(10));
    loop {
        let tmp = dir.join(format!("{pid}-{n}.{SESSION_TMP_EXT}"));
        let path = dir.join(format!("{pid}-{n}.{SESSION_MARKER_EXT}"));
        match std::fs::create_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if transient(&e) => {
                pause();
                continue;
            }
            Err(e) => return Err(e.into()),
        }
        match File::options().write(true).create_new(true).open(&tmp) {
            Ok(file) if path.exists() => {
                // Another session of this process owns `n`.
                drop(file);
                let _ = std::fs::remove_file(&tmp);
                n += 1;
            }
            Ok(file) => {
                file.lock()?;
                match std::fs::rename(&tmp, &path) {
                    Ok(()) => {
                        let session = Session {
                            file: Some(file),
                            path,
                        };
                        return Ok((session, unclean));
                    }
                    Err(e) if transient(&e) => {
                        drop(file);
                        let _ = std::fs::remove_file(&tmp);
                        n += 1;
                        pause();
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            // A closing session removed the empty directory in between: create it again.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) if transient(&e) => {
                n += 1;
                pause();
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// [`open_rw`] with integrity handling, for a caller that ends its session itself.
/// `PRAGMA quick_check` reads the whole index, so it runs only when an earlier session did
/// not end cleanly. Only damage recovers the index ([`is_damage`]: SQLite's corrupt or
/// not-a-database, or a failed check): [`recover`] builds a replacement holding every root
/// path still readable (pending deep, so the next scan rebuilds it, VFS-1 AC-6) at
/// `<db_path>.new`, moves the damaged index to `<db_path>.bad` and swaps the new one in; a
/// recovery killed between those steps is finished here first. Any other error (busy,
/// locked, I/O, and `SchemaTooNew`: a newer build's index) is returned and nothing is moved
/// (VFS-3 AC-3, AC-9).
pub(crate) fn open_session(db_path: &Path) -> Result<(Connection, bool, Session)> {
    let started = std::time::Instant::now();
    let (session, unclean) = begin_session(db_path)?;
    if !db_path.exists() && fresh_path(db_path).exists() && bad_path(db_path).exists() {
        // A recovery was killed after moving the damaged index aside.
        finish_recovery(db_path)?;
    }
    let opened = if emptied(db_path) {
        Err(corrupt("index file truncated to nothing".into()))
    } else {
        open_rw(db_path)
    };
    let mut checked = false;
    let opened = opened.and_then(|c| {
        if unclean {
            checked = true;
            quick_check(&c)?;
        }
        Ok(c)
    });
    let (conn, recovered) = match opened {
        Ok(conn) => (conn, false),
        // A busy, locked or unreadable index is not a damaged one: moving it aside would
        // discard a healthy index another process is using (VFS-3 AC-9).
        Err(e) if !is_damage(&e) => return Err(e),
        Err(e) => {
            log::warn!("index {} unusable ({e}); recreating it", db_path.display());
            (recover(db_path)?, true)
        }
    };
    if let Err(e) = sync_roots_backup(db_path, &conn) {
        log::warn!("root list backup not written: {e}");
    }
    log::info!(target: crate::indexer::DIAG_TARGET, "{}", open_record(
        started.elapsed(),
        checked,
        recovered,
        std::fs::metadata(db_path).ok().map(|m| m.len()),
    ));
    Ok((conn, recovered, session))
}

/// The diagnostic record of an index open (VFS-41 R4): its time, whether the integrity
/// check ran (after an unclean exit) and whether the index was rebuilt.
fn open_record(took: Duration, checked: bool, recovered: bool, bytes: Option<u64>) -> String {
    format!(
        "open ms={} checked={} recovered={} index_mb={}",
        took.as_millis(),
        u8::from(checked),
        u8::from(recovered),
        crate::indexer::sched::mb(bytes)
    )
}

/// [`open_session`] whose session ends when this returns (one-shot tools). Returns the
/// connection and whether recovery ran.
pub fn open_or_recover(db_path: &Path) -> Result<(Connection, bool)> {
    let (conn, recovered, _session) = open_session(db_path)?;
    Ok((conn, recovered))
}

/// True for the errors that prove the index file itself is damaged: SQLite's "corrupt" and
/// "not a database", and a failed `quick_check`.
fn is_damage(e: &Error) -> bool {
    matches!(
        e,
        Error::Db(err) if matches!(
            err.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
        )
    )
}

fn quick_check(conn: &Connection) -> Result<()> {
    let verdict: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
    if verdict == "ok" {
        Ok(())
    } else {
        Err(corrupt(verdict))
    }
}

fn corrupt(detail: String) -> Error {
    Error::Db(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
        Some(detail),
    ))
}

/// True when the index file exists but is empty with no WAL content to replay, while the
/// backup lists roots: SQLite would open it as a new, empty index and drop those roots,
/// so it is treated as damaged and rebuilt from the backup (VFS-23 AC-1). A new index in
/// WAL mode can have an empty main file, but then its WAL holds its pages.
fn emptied(db_path: &Path) -> bool {
    let len = |p: &Path| std::fs::metadata(p).map(|m| m.len()).ok();
    len(db_path) == Some(0)
        && len(&with_suffix(db_path, WAL_SUFFIX)).unwrap_or(0) == 0
        && read_roots_backup(db_path).is_some_and(|r| !r.is_empty())
}

/// Roots for a recovery (VFS-23 AC-1, D1): those still readable from the damaged index (none
/// when `roots` cannot be read: a destroyed page, an overwritten header, a truncated file),
/// joined with the backup list. Empty when neither yields a root.
fn salvage_root_paths(db_path: &Path) -> Vec<String> {
    let read = || -> rusqlite::Result<Vec<String>> {
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        root_paths(&conn)
    };
    // A torn page can leave `roots` partly readable: the union with the backup never loses
    // a kept root. A root it may bring back (a stale backup) only costs a scan.
    let mut paths = read().unwrap_or_else(|e| {
        log::warn!("roots unreadable ({e}); taking them from the backup");
        Vec::new()
    });
    for p in read_roots_backup(db_path).unwrap_or_default() {
        if !paths.contains(&p) {
            paths.push(p);
        }
    }
    paths
}

/// Paths of the roots the user keeps: one being removed is not brought back by a recovery.
fn root_paths(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT path FROM roots WHERE status != ?1 ORDER BY id")?;
    let paths = stmt
        .query_map([crate::types::RootStatus::Removing.code()], |r| r.get(0))?
        .collect();
    paths
}

/// `<db_path>.roots`: a copy of the root list outside the database, so a recovery that
/// cannot read `roots` still keeps the user's folders (VFS-23 AC-1, D1). Each path is UTF-8
/// ended by a NUL, which no path holds (a newline it may).
fn roots_backup_path(db_path: &Path) -> PathBuf {
    with_suffix(db_path, ".roots")
}

/// The backup root list, or `None` when it is missing or unreadable.
pub(crate) fn read_roots_backup(db_path: &Path) -> Option<Vec<String>> {
    let bytes = std::fs::read(roots_backup_path(db_path)).ok()?;
    // Only entries ended by their NUL are whole: a cut-off tail, and any entry that is not
    // UTF-8 (a torn write), are dropped rather than becoming a folder that never existed.
    let whole = bytes.rsplitn(2, |b| *b == 0).nth(1).unwrap_or_default();
    Some(
        whole
            .split(|b| *b == 0)
            .filter_map(|p| std::str::from_utf8(p).ok())
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// Makes the backup equal `conn`'s root list. Written to a per-process temporary file,
/// flushed and renamed over the old one, so a kill, a power cut or a second process never
/// leaves half a list; left
/// alone when already equal, so a normal open writes nothing.
pub(crate) fn sync_roots_backup(db_path: &Path, conn: &Connection) -> Result<()> {
    let paths = root_paths(conn)?;
    if read_roots_backup(db_path).as_ref() == Some(&paths) {
        return Ok(());
    }
    let mut bytes = Vec::new();
    for p in &paths {
        bytes.extend_from_slice(p.as_bytes());
        bytes.push(0);
    }
    let target = roots_backup_path(db_path);
    let tmp = with_suffix(&target, &format!(".tmp{}", std::process::id()));
    let write = || -> std::io::Result<()> {
        let mut f = File::create(&tmp)?;
        std::io::Write::write_all(&mut f, &bytes)?;
        f.sync_all() // on disk before the rename, so a power cut cannot leave it empty
    };
    write()?;
    std::fs::rename(&tmp, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// `<db_path>.new`, where [`recover`] builds the replacement index before swapping it in.
fn fresh_path(db_path: &Path) -> PathBuf {
    let mut p = db_path.as_os_str().to_owned();
    p.push(".new");
    PathBuf::from(p)
}

/// `path` with a SQLite companion suffix appended.
pub(crate) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(suffix);
    PathBuf::from(p)
}

/// Moves the index to `<db_path>.bad` and puts a fresh one holding the salvaged roots
/// (pending deep) in its place. A kill at any moment leaves either the damaged index in
/// place (the next open recovers again) or a complete replacement at `<db_path>.new` that
/// the next open adopts ([`finish_recovery`]): the root list is never lost to a crash
/// (VFS-3 AC-3).
pub(crate) fn recover(db_path: &Path) -> Result<Connection> {
    build_fresh(db_path)?;
    move_aside(db_path)?;
    finish_recovery(db_path)?;
    open_rw(db_path)
}

/// Step 1: writes the replacement index at `<db_path>.new` (replacing what an interrupted
/// recovery left there) with every root path readable from the damaged index, checkpointed
/// so the single file is complete.
fn build_fresh(db_path: &Path) -> Result<()> {
    let roots = salvage_root_paths(db_path);
    let fresh = fresh_path(db_path);
    for suffix in ["", WAL_SUFFIX, SHM_SUFFIX] {
        let p = with_suffix(&fresh, suffix);
        if p.exists() {
            std::fs::remove_file(&p)?;
        }
    }
    let mut conn = open_rw(&fresh)?;
    let tx = conn.transaction()?;
    for path in &roots {
        let volume = crate::platform::volume_key(Path::new(path));
        ingest::insert_root(&tx, &volume, path)?;
    }
    tx.commit()?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(())
}

/// Step 2: moves the damaged index file to `<db_path>.bad`.
fn move_aside(db_path: &Path) -> Result<()> {
    if db_path.exists() {
        std::fs::rename(db_path, bad_path(db_path))?;
    }
    Ok(())
}

/// Step 3, also run by the next open after a kill between steps 2 and 3: moves the damaged
/// index's `-wal`/`-shm` next to `<db_path>.bad` (dropping older ones there), so they are
/// never applied to the new index, then renames `<db_path>.new` into place.
fn finish_recovery(db_path: &Path) -> Result<()> {
    let bad = bad_path(db_path);
    for suffix in [WAL_SUFFIX, SHM_SUFFIX] {
        let from = with_suffix(db_path, suffix);
        let to = with_suffix(&bad, suffix);
        if from.exists() {
            std::fs::rename(&from, &to)?;
        } else if to.exists() {
            std::fs::remove_file(&to)?;
        }
    }
    std::fs::rename(fresh_path(db_path), db_path)?;
    Ok(())
}

/// Absolute path of a directory: `roots.path` joined with the components of `rel_path`.
pub fn dir_abs_path(conn: &Connection, dir_id: i64) -> Result<PathBuf> {
    let (root, rel): (String, String) = conn
        .query_row(
            "SELECT r.path, d.rel_path FROM dirs d JOIN roots r ON r.id = d.root_id WHERE d.id = ?1",
            [dir_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("directory {dir_id}")))?;
    Ok(join_rel(&root, &rel))
}

/// Ids of the roots the user paused (VFS-32 R1), for other readers of the index such as the
/// content indexer, which skips their documents (VFS-27 R8).
pub fn paused_root_ids(conn: &Connection) -> Result<Vec<i64>> {
    let mut st = conn.prepare(concat!(
        "SELECT id FROM roots WHERE EXISTS ",
        crate::roots::paused_key_sql!("roots.id")
    ))?;
    let ids = st.query_map([], |r| r.get(0))?;
    Ok(ids.collect::<rusqlite::Result<_>>()?)
}

/// Absolute path of a file, built from the original names.
pub fn file_abs_path(conn: &Connection, file_id: i64) -> Result<PathBuf> {
    let (root, rel, name): (String, String, String) = conn
        .query_row(
            "SELECT r.path, d.rel_path, f.name FROM files f JOIN dirs d ON d.id = f.dir_id \
             JOIN roots r ON r.id = d.root_id WHERE f.id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("file {file_id}")))?;
    Ok(join_rel(&root, &rel).join(name))
}

/// What the index holds for the link `file_id` (VFS-18): its kind and target, as read from
/// the link itself. `NotFound` when the file is not a link.
pub fn link_of(conn: &Connection, file_id: i64) -> Result<LinkTarget> {
    let (kind, target): (i64, Option<String>) = conn
        .query_row(
            "SELECT kind, target FROM links WHERE file_id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("link {file_id}")))?;
    let kind = LinkKind::from_code(kind);
    let openable =
        kind == LinkKind::Internet && target.as_deref().is_some_and(crate::link::is_openable_url);
    Ok(LinkTarget {
        kind,
        target,
        openable,
    })
}

/// `root` (absolute, native separators) joined with the '/'-separated `rel_path`.
pub fn join_rel(root: &str, rel_path: &str) -> PathBuf {
    let mut path = PathBuf::from(root);
    path.extend(rel_path.split('/').filter(|c| !c.is_empty()));
    path
}

/// Write helpers. Every FTS row is written here, in the caller's transaction, next to its
/// base row (`files_fts.rowid = file_paths_fts.rowid = files.id`, `dirs_fts.rowid = dirs.id`).
pub mod ingest {
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::{params, Connection, OptionalExtension, Transaction};

    use crate::error::{Error, Result};
    use crate::fold::fold_key;
    use crate::kind::file_kind;
    use crate::link::{link_file_kind, LinkFileKind};
    use crate::search::NAME_START;
    use crate::types::{FileKind, LinkKind, RootStatus, UnreadableReason};

    /// A file's `file_paths_fts` value from its row `f` and its directory `d`: the folder's
    /// folded path and the folded name, or the name alone in the root directory.
    pub(crate) const FILE_PATH_SQL: &str =
        "CASE d.path_key WHEN '' THEN f.name_key ELSE d.path_key || '/' || f.name_key END";

    /// Value of `dirs.scan_state`; the persistent work queue of the indexer.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ScanState {
        Clean = 0,
        PendingDeep = 1,
        PendingShallow = 2,
    }

    /// `files.flags` bit 1: the entry is a symbolic link or junction (VFS-18).
    pub const FLAG_LINK: i64 = 2;

    /// `files.flags` bit 2: the file's data is only in the cloud, a placeholder that reading
    /// would download (VFS-25). Content extraction must skip such files.
    pub const FLAG_CLOUD_ONLY: i64 = 4;

    /// Value of `links.state` (VFS-18). Written into SQL as a literal so the partial index
    /// `links_pending` (`WHERE state = 0`) serves the pending query.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum LinkState {
        Pending = 0,
        Read = 1,
        Unreadable = 2,
    }

    pub struct NewFile<'a> {
        pub name: &'a str,
        pub size: u64,
        pub mtime: i64,
        /// The on-disk name was not valid Unicode and `name` is its lossy form (`flags` bit 0).
        pub lossy: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ChildFile {
        pub id: i64,
        pub name: String,
        pub size: u64,
        pub mtime: i64,
        /// A symbolic link or junction entry (`flags` bit 1, VFS-18).
        pub link: bool,
        /// A cloud-only file (`flags` bit 2, VFS-25).
        pub cloud_only: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ChildDir {
        pub id: i64,
        /// Last component of `rel_path`.
        pub name: String,
        pub mtime: i64,
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Inserts a root (status pending) and its root directory row (`rel_path = ''`,
    /// pending deep); creates the volume row if needed. `path` is absolute with native
    /// separators and no trailing separator. Returns the root id.
    pub fn insert_root(tx: &Transaction, volume_key: &str, path: &str) -> Result<i64> {
        tx.execute(
            "INSERT INTO volumes(volume_key) VALUES (?1) ON CONFLICT(volume_key) DO NOTHING",
            [volume_key],
        )?;
        let volume_id: i64 = tx.query_row(
            "SELECT id FROM volumes WHERE volume_key = ?1",
            [volume_key],
            |r| r.get(0),
        )?;
        let path_key = fold_key(&path.replace('\\', "/"));
        tx.execute(
            "INSERT INTO roots(volume_id, path, path_key, status, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![volume_id, path, path_key, RootStatus::Pending.code(), now()],
        )?;
        let root_id = tx.last_insert_rowid();
        let root_name = path
            .rsplit(['/', '\\'])
            .find(|c| !c.is_empty())
            .unwrap_or_default();
        insert_dir_row(tx, root_id, None, "", root_name, 0)?;
        Ok(root_id)
    }

    fn insert_dir_row(
        tx: &Transaction,
        root_id: i64,
        parent_id: Option<i64>,
        rel_path: &str,
        name: &str,
        mtime: i64,
    ) -> Result<i64> {
        let path_key = fold_key(rel_path);
        let name_key = fold_key(name);
        tx.prepare_cached(
            "INSERT INTO dirs(root_id, parent_id, rel_path, path_key, name_key, mtime, scan_state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .execute(params![
                root_id,
                parent_id,
                rel_path,
                path_key,
                name_key,
                mtime,
                ScanState::PendingDeep as i64
            ])?;
        let id = tx.last_insert_rowid();
        tx.prepare_cached("INSERT INTO dirs_fts(rowid, path_key, name_key) VALUES (?1, ?2, ?3)")?
            .execute(params![id, path_key, name_key])?;
        Ok(id)
    }

    /// Inserts a directory (pending deep) and its `dirs_fts` row. `rel_path` is '/'-separated
    /// original names relative to the root; `mtime` is the directory's own (unix seconds).
    pub fn insert_dir(
        tx: &Transaction,
        root_id: i64,
        parent_id: i64,
        rel_path: &str,
        mtime: i64,
    ) -> Result<i64> {
        let name = rel_path.rsplit('/').next().unwrap_or_default();
        insert_dir_row(tx, root_id, Some(parent_id), rel_path, name, mtime)
    }

    /// Updates the stored modification time of a directory (shown on folder hits).
    pub fn set_dir_mtime(tx: &Transaction, dir_id: i64, mtime: i64) -> Result<()> {
        tx.prepare_cached("UPDATE dirs SET mtime = ?2 WHERE id = ?1")?
            .execute(params![dir_id, mtime])?;
        Ok(())
    }

    /// Inserts a file with its `files_fts` and `file_paths_fts` rows; computes `name_key`
    /// and `kind`.
    pub fn insert_file(tx: &Transaction, dir_id: i64, file: &NewFile) -> Result<i64> {
        let name_key = fold_key(file.name);
        tx.prepare_cached(
            "INSERT INTO files(dir_id, name, name_key, kind, size, mtime, flags) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .execute(params![
                dir_id,
                file.name,
                name_key,
                file_kind(file.name).code(),
                file.size as i64,
                file.mtime,
                i64::from(file.lossy),
            ])?;
        let id = tx.last_insert_rowid();
        tx.prepare_cached("INSERT INTO files_fts(rowid, name) VALUES (?1, ?2)")?
            .execute(params![id, format!("{NAME_START}{name_key}")])?;
        tx.prepare_cached(&format!(
            "INSERT INTO file_paths_fts(rowid, path) SELECT f.id, {FILE_PATH_SQL} \
             FROM files f JOIN dirs d ON d.id = f.dir_id WHERE f.id = ?1"
        ))?
        .execute([id])?;
        let link = match link_file_kind(file.name) {
            Some(LinkFileKind::Lnk) => Some(LinkKind::Shortcut),
            Some(LinkFileKind::Url) => Some(LinkKind::Internet),
            None => None,
        };
        if let Some(kind) = link {
            tx.prepare_cached("INSERT INTO links(file_id, kind) VALUES (?1, ?2)")?
                .execute(params![id, kind.code()])?;
        }
        Ok(id)
    }

    /// Sets or clears a file's cloud-only bit (VFS-25 R2). A link file that is local again
    /// goes back to the link pass, which skipped it while it was cloud-only (VFS-25 R3).
    pub fn set_cloud_only(tx: &Transaction, file_id: i64, cloud_only: bool) -> Result<()> {
        if cloud_only {
            tx.prepare_cached("UPDATE files SET flags = flags | ?2 WHERE id = ?1")?
                .execute(params![file_id, FLAG_CLOUD_ONLY])?;
        } else {
            tx.prepare_cached("UPDATE files SET flags = flags & ~?2 WHERE id = ?1")?
                .execute(params![file_id, FLAG_CLOUD_ONLY])?;
            tx.prepare_cached(&format!(
                "UPDATE links SET state = {}, target = NULL, target_key = NULL WHERE file_id = ?1",
                LinkState::Pending as i64
            ))?
            .execute([file_id])?;
        }
        Ok(())
    }

    /// Inserts a symbolic link or junction entry as a file row (`flags` bit 1) with a pending
    /// link row of kind symlink, whatever its name (VFS-18 R1, D5).
    pub fn insert_link_entry(tx: &Transaction, dir_id: i64, file: &NewFile) -> Result<i64> {
        let id = insert_file(tx, dir_id, file)?;
        // Its kind is "other" whatever its name: a type filter keeps links only under "Otros"
        // (D10).
        tx.prepare_cached("UPDATE files SET flags = flags | ?2, kind = ?3 WHERE id = ?1")?
            .execute(params![id, FLAG_LINK, FileKind::Other.code()])?;
        tx.prepare_cached("INSERT OR REPLACE INTO links(file_id, kind) VALUES (?1, ?2)")?
            .execute(params![id, LinkKind::Symlink.code()])?;
        Ok(id)
    }

    /// Updates a file's size and modification time; a link whose file changed is read again
    /// by the deferred link pass (VFS-18 R2).
    pub fn update_file(tx: &Transaction, file_id: i64, size: u64, mtime: i64) -> Result<()> {
        tx.prepare_cached("UPDATE files SET size = ?2, mtime = ?3 WHERE id = ?1")?
            .execute(params![file_id, size as i64, mtime])?;
        tx.prepare_cached(&format!(
            "UPDATE links SET state = {}, target = NULL, target_key = NULL WHERE file_id = ?1",
            LinkState::Pending as i64
        ))?
        .execute([file_id])?;
        Ok(())
    }

    /// Up to `limit` links waiting for the deferred pass, in id order: (file id, kind,
    /// cloud-only). Links of a paused root wait (VFS-32 R2).
    pub fn pending_links(conn: &Connection, limit: usize) -> Result<Vec<(i64, LinkKind, bool)>> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT l.file_id, l.kind, COALESCE(f.flags, 0) FROM links l \
             LEFT JOIN files f ON f.id = l.file_id \
             WHERE l.state = {} AND NOT EXISTS (SELECT 1 FROM dirs d \
               WHERE d.id = f.dir_id AND EXISTS {}) \
             ORDER BY l.file_id LIMIT ?1",
            LinkState::Pending as i64,
            crate::roots::paused_key_sql!("d.root_id")
        ))?;
        let rows = stmt
            .query_map([limit as i64], |r| {
                let flags: i64 = r.get(2)?;
                Ok((
                    r.get(0)?,
                    LinkKind::from_code(r.get(1)?),
                    flags & FLAG_CLOUD_ONLY != 0,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Stores what the deferred pass read for a link: its target (state read), no target
    /// (state read, `None`), or that the link file could not be read (state unreadable).
    pub fn set_link_target(
        tx: &Transaction,
        file_id: i64,
        read: std::result::Result<Option<&str>, ()>,
    ) -> Result<()> {
        let (state, target) = match read {
            Ok(t) => (LinkState::Read, t),
            Err(()) => (LinkState::Unreadable, None),
        };
        let key = target.map(|t| fold_key(&t.replace('\\', "/")));
        tx.prepare_cached(
            "UPDATE links SET state = ?2, target = ?3, target_key = ?4 WHERE file_id = ?1",
        )?
        .execute(params![file_id, state as i64, target, key])?;
        Ok(())
    }

    /// Queues every unreadable link of a root for another read (after a full walk).
    pub fn retry_unreadable_links(tx: &Transaction, root_id: i64) -> Result<()> {
        tx.prepare_cached(&format!(
            "UPDATE links SET state = {} WHERE state = {} AND file_id IN \
             (SELECT f.id FROM files f JOIN dirs d ON d.id = f.dir_id WHERE d.root_id = ?1)",
            LinkState::Pending as i64,
            LinkState::Unreadable as i64
        ))?
        .execute([root_id])?;
        Ok(())
    }

    /// Deletes the `files_fts` and `file_paths_fts` rows of the files `f` that `files_where`
    /// selects (a condition on `f`, joined to its directory `d`), by the values they were
    /// indexed under: both tables are contentless without `contentless_delete` (schema v3),
    /// so the files and their directories must still hold those values. Returns the number
    /// of files.
    pub(crate) fn delete_files_fts<P: rusqlite::Params + Copy>(
        tx: &Connection,
        files_where: &str,
        params: P,
    ) -> Result<usize> {
        let n = tx
            .prepare_cached(&format!(
                "INSERT INTO files_fts(files_fts, rowid, name) \
                 SELECT 'delete', f.id, char({}) || f.name_key FROM files f WHERE {files_where}",
                u32::from(NAME_START)
            ))?
            .execute(params)?;
        delete_file_paths_fts(tx, files_where, params)?;
        Ok(n)
    }

    /// Deletes the `file_paths_fts` rows of the files `f` that `files_where` selects, by
    /// their indexed value (see `delete_files_fts`); a folder rename calls it before the
    /// folder's path changes. Every file FTS deletion passes here, so it counts them for
    /// compaction (VFS-40 R1).
    pub(crate) fn delete_file_paths_fts<P: rusqlite::Params>(
        tx: &Connection,
        files_where: &str,
        params: P,
    ) -> Result<()> {
        let n = tx
            .prepare_cached(&format!(
                "INSERT INTO file_paths_fts(file_paths_fts, rowid, path) \
                 SELECT 'delete', f.id, {FILE_PATH_SQL} FROM files f \
                 JOIN dirs d ON d.id = f.dir_id WHERE {files_where}"
            ))?
            .execute(params)?;
        crate::indexer::compact::record_deleted(tx, n)
    }

    /// Deletes a file and its FTS rows.
    pub fn delete_file(tx: &Transaction, file_id: i64) -> Result<()> {
        delete_files_fts(tx, "f.id = ?1", [file_id])?;
        tx.prepare_cached("DELETE FROM links WHERE file_id = ?1")?
            .execute([file_id])?;
        tx.prepare_cached("DELETE FROM files WHERE id = ?1")?
            .execute([file_id])?;
        Ok(())
    }

    /// Deletes a directory, every directory below it and all their files, with their FTS
    /// rows. Returns the number of files removed.
    pub fn delete_dir_subtree(tx: &Transaction, dir_id: i64) -> Result<u64> {
        let (root_id, rel_path): (i64, String) = tx
            .query_row(
                "SELECT root_id, rel_path FROM dirs WHERE id = ?1",
                [dir_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("directory {dir_id}")))?;
        // Descendants of `p` are exactly the rel_paths in [p + '/', p + '0'): '0' follows '/'.
        let subtree = if rel_path.is_empty() {
            "SELECT id FROM dirs WHERE root_id = ?1"
        } else {
            "SELECT id FROM dirs WHERE root_id = ?1 AND (rel_path = ?2 \
             OR (rel_path >= ?2 || '/' AND rel_path < ?2 || '0'))"
        };
        let all = params![root_id, rel_path];
        let p = if rel_path.is_empty() { &all[..1] } else { all };
        let removed: i64 = tx.query_row(
            &format!("SELECT COUNT(*) FROM files WHERE dir_id IN ({subtree})"),
            p,
            |r| r.get(0),
        )?;
        delete_files_fts(tx, &format!("f.dir_id IN ({subtree})"), p)?;
        tx.execute(
            &format!(
                "DELETE FROM links WHERE file_id IN \
                 (SELECT id FROM files WHERE dir_id IN ({subtree}))"
            ),
            p,
        )?;
        tx.execute(&format!("DELETE FROM files WHERE dir_id IN ({subtree})"), p)?;
        tx.execute(
            &format!("DELETE FROM dirs_fts WHERE rowid IN ({subtree})"),
            p,
        )?;
        tx.execute(&format!("DELETE FROM dirs WHERE id IN ({subtree})"), p)?;
        Ok(removed as u64)
    }

    /// Deletes a root with all its directories, files and FTS rows at once.
    pub fn delete_root(tx: &Transaction, root_id: i64) -> Result<()> {
        let subtree = "SELECT id FROM dirs WHERE root_id = ?1";
        delete_files_fts(tx, &format!("f.dir_id IN ({subtree})"), [root_id])?;
        tx.execute(
            &format!(
                "DELETE FROM links WHERE file_id IN \
                 (SELECT id FROM files WHERE dir_id IN ({subtree}))"
            ),
            [root_id],
        )?;
        tx.execute(
            &format!("DELETE FROM files WHERE dir_id IN ({subtree})"),
            [root_id],
        )?;
        tx.execute(
            &format!("DELETE FROM dirs_fts WHERE rowid IN ({subtree})"),
            [root_id],
        )?;
        tx.execute("DELETE FROM dirs WHERE root_id = ?1", [root_id])?;
        tx.execute("DELETE FROM roots WHERE id = ?1", [root_id])?;
        Ok(())
    }

    /// Marks a directory listed: clean, its unreadable reason forgotten (VFS-10 R2) and its
    /// unpopulated mark cleared (VFS-37 R2).
    pub fn mark_listed(tx: &Transaction, dir_id: i64) -> Result<()> {
        tx.prepare_cached(
            "UPDATE dirs SET scan_state = 0, error = NULL, unpopulated = NULL WHERE id = ?1",
        )?
        .execute([dir_id])?;
        Ok(())
    }

    /// Marks a directory just listed as an unpopulated cloud placeholder: empty, and still
    /// not all present locally (VFS-37 R2).
    pub fn mark_unpopulated(tx: &Transaction, dir_id: i64) -> Result<()> {
        tx.prepare_cached("UPDATE dirs SET unpopulated = 1 WHERE id = ?1")?
            .execute([dir_id])?;
        Ok(())
    }

    /// Records why a directory failed to list (VFS-10 R1); with `settled` (its retries are
    /// exhausted) it is also marked clean until the next full walk (VFS-1 I11).
    /// A folder that fails to list is never unpopulated (VFS-37 R2).
    pub fn record_unreadable(
        tx: &Transaction,
        dir_id: i64,
        reason: UnreadableReason,
        settled: bool,
    ) -> Result<()> {
        let sql = if settled {
            "UPDATE dirs SET error = ?2, scan_state = 0, unpopulated = NULL WHERE id = ?1"
        } else {
            "UPDATE dirs SET error = ?2, unpopulated = NULL WHERE id = ?1"
        };
        tx.prepare_cached(sql)?
            .execute(params![dir_id, reason.code()])?;
        Ok(())
    }

    pub fn set_dir_state(tx: &Transaction, dir_id: i64, state: ScanState) -> Result<()> {
        tx.prepare_cached("UPDATE dirs SET scan_state = ?2 WHERE id = ?1")?
            .execute(params![dir_id, state as i64])?;
        Ok(())
    }

    /// The files and the child directories of one directory, as stored.
    pub fn list_children(
        conn: &Connection,
        dir_id: i64,
    ) -> Result<(Vec<ChildFile>, Vec<ChildDir>)> {
        let mut stmt = conn
            .prepare_cached("SELECT id, name, size, mtime, flags FROM files WHERE dir_id = ?1")?;
        let files = stmt
            .query_map([dir_id], |r| {
                Ok(ChildFile {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    size: r.get::<_, i64>(2)? as u64,
                    mtime: r.get(3)?,
                    link: r.get::<_, i64>(4)? & FLAG_LINK != 0,
                    cloud_only: r.get::<_, i64>(4)? & FLAG_CLOUD_ONLY != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt =
            conn.prepare_cached("SELECT id, rel_path, mtime FROM dirs WHERE parent_id = ?1")?;
        let dirs = stmt
            .query_map([dir_id], |r| {
                let rel: String = r.get(1)?;
                Ok(ChildDir {
                    id: r.get(0)?,
                    name: rel.rsplit('/').next().unwrap_or_default().to_string(),
                    mtime: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((files, dirs))
    }
}

#[cfg(test)]
mod tests {
    use super::ingest::*;
    use super::*;
    use crate::search::NAME_START;
    use crate::types::{FileKind, RootStatus};

    /// Rowids of an FTS table holding `term`: a phrase, or for `files_fts` (detail=none,
    /// schema v3) every trigram of the term.
    fn fts_ids(conn: &Connection, table: &str, term: &str) -> Vec<i64> {
        let sql = format!("SELECT rowid FROM {table} WHERE {table} MATCH ?1 ORDER BY rowid");
        let mut stmt = conn.prepare(&sql).unwrap();
        let expr = if table == "files_fts" {
            let c: Vec<char> = term.chars().collect();
            c.windows(3)
                .map(|w| format!("\"{}\"", w.iter().collect::<String>()))
                .collect::<Vec<_>>()
                .join(" AND ")
        } else {
            format!("\"{term}\"")
        };
        stmt.query_map([expr], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<i64>>>()
            .unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// VFS-1 AC-10: the bundled SQLite supports contentless-delete FTS5 trigram tables
    /// (>= 3.43) and the schema is created once, at version 1, in WAL mode.
    #[test]
    fn schema_creates_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let conn = open_rw(&path).unwrap();
        let version: String = conn
            .query_row("SELECT sqlite_version()", [], |r| r.get(0))
            .unwrap();
        let parts: Vec<u32> = version.split('.').map(|p| p.parse().unwrap()).collect();
        assert!(
            (parts[0], parts[1]) >= (3, 43),
            "SQLite {version} lacks contentless_delete"
        );
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        drop(conn);
        let again = open_rw(&path).unwrap();
        assert_eq!(count(&again, "SELECT COUNT(*) FROM roots"), 0);
        let ro = open_ro(&path).unwrap();
        assert!(
            ro.execute("DELETE FROM roots", []).is_err(),
            "reader is query-only"
        );
    }

    fn path_ids(conn: &Connection, grams: &str) -> Vec<i64> {
        path_ids_in(conn, "file_paths_fts", grams)
    }

    fn path_ids_in(conn: &Connection, table: &str, expr: &str) -> Vec<i64> {
        let sql = format!("SELECT rowid FROM {table} WHERE {table} MATCH ?1 ORDER BY rowid");
        let mut stmt = conn.prepare(&sql).unwrap();
        stmt.query_map([expr], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<i64>>>()
            .unwrap()
    }

    /// VFS-37: the README and the architecture doc name the current schema version, so a
    /// schema change that leaves them behind fails here.
    #[test]
    fn docs_name_the_schema_version() {
        let readme = include_str!("../../../README.md");
        assert!(readme.contains(&format!("schema version {SCHEMA_VERSION} ")));
        let arch = include_str!("../../../docs/architecture.md");
        assert!(arch.contains(&format!("`core/schema_v{SCHEMA_VERSION}.sql`")));
    }

    /// A version-2 index at 4 KB pages: file 3 `Año 2019/Banco.pdf` and 2,901 more files,
    /// with their version-2 FTS rows.
    fn version_2_index(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA page_size=4096; PRAGMA journal_mode=WAL;")
            .unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.execute_batch(MIGRATIONS[1]).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.execute_batch(
            "INSERT INTO dirs(id, root_id, parent_id, rel_path, path_key, name_key) \
                   VALUES (7, 1, NULL, 'Año 2019', 'ano 2019', 'ano 2019');
                 INSERT INTO files(id, dir_id, name, name_key, kind, size, mtime) \
                   VALUES (3, 7, 'Banco.pdf', 'banco.pdf', 1, 1, 1);
                 INSERT INTO files_fts(rowid, name_key) VALUES (3, 'banco.pdf');
                 INSERT INTO file_paths_fts(rowid, path) VALUES (3, 'ano 2019/banco.pdf');",
        )
        .unwrap();
        // Enough rows that the old FTS tables leave free pages behind.
        conn.execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 100 UNION ALL SELECT i + 1 FROM n WHERE i < 3000) \
                 INSERT INTO files(id, dir_id, name, name_key, kind, size, mtime) \
                   SELECT i, 7, 'archivo ' || i || '.txt', 'archivo ' || i || '.txt', 0, 1, 1 \
                   FROM n;
                 INSERT INTO files_fts(rowid, name_key) SELECT id, name_key FROM files \
                   WHERE id >= 100;
                 INSERT INTO file_paths_fts(rowid, path) SELECT id, 'ano 2019/' || name_key \
                   FROM files WHERE id >= 100;",
        )
        .unwrap();
    }

    /// VFS-4 AC-1, AC-2, VFS-7 AC-2: opening a version-2 index migrates it to the current
    /// version (3, then 4) from the existing rows (no rescan), compacts it at `PAGE_SIZE`
    /// and leaves it in WAL mode, and the migrated rows are found and deleted like new ones.
    #[test]
    fn version_2_index_is_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        version_2_index(&path);
        let conn = open_rw(&path).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        // Names keep their start marker: a prefix is found without positions.
        let prefix = format!("\"{NAME_START}ba\" AND \"ban\"");
        assert_eq!(path_ids_in(&conn, "files_fts", &prefix), vec![3]);
        assert_eq!(path_ids(&conn, "\"201\" AND \"ban\""), vec![3]);
        // Version 4 (VFS-7): the files keep their ids, without the name index.
        let names: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'sqlite_autoindex_files_1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(names, 0);
        assert_eq!(list_children(&conn, 7).unwrap().0.len(), 2902);
        // Compacted at the new page size, with no per-row size tables left.
        assert_eq!(pragma(&conn, "freelist_count"), 0);
        assert_eq!(pragma(&conn, "page_size"), i64::from(PAGE_SIZE));
        let docsize: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name LIKE 'file%_docsize'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(docsize, 0);
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        // Rows migrated from version 2 are deleted by the values they were indexed under.
        let tx = Connection::unchecked_transaction(&conn).unwrap();
        delete_file(&tx, 3).unwrap();
        tx.commit().unwrap();
        assert!(path_ids_in(&conn, "files_fts", &prefix).is_empty());
        assert!(path_ids(&conn, "\"201\" AND \"ban\"").is_empty());
    }

    /// VFS-7 AC-1: files are found by folder through `files_dir`; no index stores their
    /// names a second time.
    #[test]
    fn files_are_listed_by_folder_without_a_name_index() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_rw(&dir.path().join("index.db")).unwrap();
        let indexes: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'index' AND tbl_name = 'files'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(indexes, vec!["files_dir".to_string()]);
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT id, name, size, mtime FROM files WHERE dir_id = 1",
                [],
                |r| r.get(3),
            )
            .unwrap();
        assert!(plan.contains("files_dir"), "{plan}");
    }

    /// VFS-10 R6, VFS-25: an index at version 4 (as 0.1.1 wrote it) is migrated to the
    /// current version without a `VACUUM`: its free pages stay, nothing is rewritten.
    #[test]
    fn version_4_index_is_migrated_without_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        {
            let conn = open_rw(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE junk(x);
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000)
                   INSERT INTO junk SELECT randomblob(1000) FROM n;
                 DROP TABLE junk;
                 DROP INDEX dirs_unpopulated;
                 ALTER TABLE dirs DROP COLUMN unpopulated;
                 ALTER TABLE roots DROP COLUMN cloud_only_count;
                 ALTER TABLE roots DROP COLUMN links_count;
                 DROP TABLE links;
                 DROP INDEX dirs_unreadable;
                 ALTER TABLE dirs DROP COLUMN error;
                 PRAGMA user_version = 4;
                 PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .unwrap();
            assert!(pragma(&conn, "freelist_count") > 0);
        }
        let conn = open_rw(&path).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        assert!(
            pragma(&conn, "freelist_count") > 0,
            "the index was compacted"
        );
    }

    /// VFS-4 AC-2 (R2): when the compaction after migrating cannot run, because another
    /// connection holds the index open, the index is still migrated, open, in WAL mode and
    /// searchable; only the page size stays as it was.
    #[test]
    fn migration_survives_a_failed_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        version_2_index(&path);
        let other = Connection::open(&path).unwrap();
        other.execute_batch("BEGIN").unwrap();
        let _n: i64 = other
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        let conn = open_rw(&path).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(pragma(&conn, "page_size"), 4096);
        assert!(pragma(&conn, "freelist_count") > 0, "not compacted");
        let prefix = format!("\"{NAME_START}ba\" AND \"ban\"");
        assert_eq!(path_ids_in(&conn, "files_fts", &prefix), vec![3]);
        drop(other);
        let tx = Connection::unchecked_transaction(&conn).unwrap();
        delete_file(&tx, 3).unwrap();
        tx.commit().unwrap();
        assert!(path_ids_in(&conn, "files_fts", &prefix).is_empty());
        assert!(path_ids(&conn, "\"201\" AND \"ban\"").is_empty());
    }

    /// VFS-1 S2, AC-9: opening a version-1 index migrates it to version 2 and fills the
    /// per-file path index from the existing rows, so path searches work without a rescan.
    #[test]
    fn version_1_index_is_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute_batch(
                "INSERT INTO dirs(id, root_id, parent_id, rel_path, path_key, name_key) \
                   VALUES (7, 1, NULL, 'Año 2019', 'ano 2019', 'ano 2019');
                 INSERT INTO files(id, dir_id, name, name_key, kind, size, mtime) \
                   VALUES (3, 7, 'Banco.pdf', 'banco.pdf', 1, 1, 1);",
            )
            .unwrap();
        }
        let conn = open_rw(&path).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(path_ids(&conn, "\"201\" AND \"ban\""), vec![3]);
    }

    /// VFS-1 S2: every file write keeps its row in the per-file path index ("folder path /
    /// name"), and deleting the file or its folder removes it.
    #[test]
    fn path_index_follows_file_writes() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open_rw(&dir.path().join("index.db")).unwrap();
        let tx = conn.transaction().unwrap();
        let root = insert_root(&tx, "/", "/datos").unwrap();
        let top: i64 = tx
            .query_row("SELECT id FROM dirs WHERE root_id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        let sub = insert_dir(&tx, root, top, "Viaje Cádiz", 1).unwrap();
        let file = |name| NewFile {
            name,
            size: 1,
            mtime: 1,
            lossy: false,
        };
        let a = insert_file(&tx, sub, &file("playa 2008.jpg")).unwrap();
        let b = insert_file(&tx, top, &file("cadiz.txt")).unwrap();
        tx.commit().unwrap();
        assert_eq!(path_ids(&conn, "\"cad\" AND \"008\""), vec![a]);
        assert_eq!(path_ids(&conn, "\"adi\""), vec![a, b]);
        let tx = conn.transaction().unwrap();
        delete_file(&tx, b).unwrap();
        delete_dir_subtree(&tx, sub).unwrap();
        tx.commit().unwrap();
        assert!(path_ids(&conn, "\"adi\"").is_empty());
    }

    /// An index written by a newer build is refused, not overwritten.
    #[test]
    fn newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        open_rw(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(matches!(open_rw(&path), Err(Error::SchemaTooNew)));
        assert!(matches!(open_ro(&path), Err(Error::SchemaTooNew)));
    }

    /// open_ro does not create a missing index.
    #[test]
    fn open_ro_requires_existing_index() {
        let dir = tempfile::tempdir().unwrap();
        assert!(open_ro(&dir.path().join("missing.db")).is_err());
    }

    /// VFS-1 AC-4, AC-8, AC-10, VFS-4 AC-4: ingest round trip — roots, directories and
    /// files are stored with folded keys and FTS rows, listed back, updated, and a subtree
    /// delete removes exactly the subtree (siblings sharing a name prefix survive).
    #[test]
    fn ingest_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open_rw(&dir.path().join("index.db")).unwrap();
        let tx = conn.transaction().unwrap();
        let root = insert_root(&tx, "/", "/Users/ana/Archivo").unwrap();
        let root_dir: i64 = tx
            .query_row(
                "SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ''",
                [root],
                |r| r.get(0),
            )
            .unwrap();
        let gestoria = insert_dir(&tx, root, root_dir, "Gestoría", 50).unwrap();
        let ano = insert_dir(&tx, root, gestoria, "Gestoría/Año 2019", 60).unwrap();
        let sibling = insert_dir(&tx, root, root_dir, "Gestoría2", 70).unwrap();
        let nfd = "Notari\u{0301}a Pe\u{0301}rez escritura 2019.pdf";
        let deep = insert_file(
            &tx,
            ano,
            &NewFile {
                name: nfd,
                size: 10,
                mtime: 100,
                lossy: false,
            },
        )
        .unwrap();
        let mid = insert_file(
            &tx,
            gestoria,
            &NewFile {
                name: "IMG_0001.JPG",
                size: 20,
                mtime: 200,
                lossy: true,
            },
        )
        .unwrap();
        let keep = insert_file(
            &tx,
            sibling,
            &NewFile {
                name: "notaria copia.pdf",
                size: 30,
                mtime: 300,
                lossy: false,
            },
        )
        .unwrap();
        tx.commit().unwrap();

        let (name, name_key, kind, flags): (String, String, i64, i64) = conn
            .query_row(
                "SELECT name, name_key, kind, flags FROM files WHERE id = ?1",
                [deep],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(name, nfd, "original name is stored untouched");
        assert_eq!(name_key, "notaria perez escritura 2019.pdf");
        assert_eq!(FileKind::from_code(kind), FileKind::Document);
        assert_eq!(flags, 0);
        let flags: i64 = conn
            .query_row("SELECT flags FROM files WHERE id = ?1", [mid], |r| r.get(0))
            .unwrap();
        assert_eq!(flags, 1);
        let root_key: String = conn
            .query_row("SELECT path_key FROM roots WHERE id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(root_key, "/users/ana/archivo");

        assert_eq!(fts_ids(&conn, "files_fts", "notaria"), vec![deep, keep]);
        assert_eq!(fts_ids(&conn, "dirs_fts", "ano 2019"), vec![ano]);
        assert_eq!(
            fts_ids(&conn, "dirs_fts", "gestoria"),
            vec![gestoria, ano, sibling]
        );

        // Folder hits (VFS-1 decision 13): the name_key column matches the folder's own name only;
        // the root directory is named after the last component of the root path.
        let by_name = |t: &str| {
            let mut stmt = conn
                .prepare("SELECT rowid FROM dirs_fts WHERE dirs_fts MATCH ?1 ORDER BY rowid")
                .unwrap();
            stmt.query_map([format!("name_key : \"{t}\"")], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<i64>>>()
                .unwrap()
        };
        assert_eq!(by_name("gestoria"), vec![gestoria, sibling]);
        assert_eq!(by_name("ano 2019"), vec![ano]);
        assert_eq!(by_name("archivo"), vec![root_dir]);
        assert_eq!(
            dir_abs_path(&conn, ano).unwrap(),
            Path::new("/Users/ana/Archivo")
                .join("Gestoría")
                .join("Año 2019")
        );
        assert_eq!(
            file_abs_path(&conn, deep).unwrap(),
            Path::new("/Users/ana/Archivo")
                .join("Gestoría")
                .join("Año 2019")
                .join(nfd)
        );
        assert_eq!(
            dir_abs_path(&conn, root_dir).unwrap(),
            Path::new("/Users/ana/Archivo")
        );
        assert!(matches!(
            file_abs_path(&conn, 9999),
            Err(Error::NotFound(_))
        ));

        let (files, dirs) = list_children(&conn, gestoria).unwrap();
        assert_eq!(
            files,
            vec![ChildFile {
                id: mid,
                name: "IMG_0001.JPG".into(),
                size: 20,
                mtime: 200,
                link: false,
                cloud_only: false
            }]
        );
        assert_eq!(
            dirs,
            vec![ChildDir {
                id: ano,
                name: "Año 2019".into(),
                mtime: 60
            }]
        );

        let tx = conn.transaction().unwrap();
        update_file(&tx, mid, 21, 201).unwrap();
        set_dir_state(&tx, gestoria, ScanState::Clean).unwrap();
        set_dir_mtime(&tx, gestoria, 99).unwrap();
        tx.commit().unwrap();
        assert_eq!(list_children(&conn, gestoria).unwrap().0[0].mtime, 201);
        assert_eq!(list_children(&conn, gestoria).unwrap().0[0].size, 21);
        let top_dirs = list_children(&conn, root_dir).unwrap().1;
        let g = top_dirs.iter().find(|d| d.id == gestoria).unwrap();
        assert_eq!(g.mtime, 99);
        assert_eq!(
            count(
                &conn,
                &format!("SELECT scan_state FROM dirs WHERE id = {gestoria}")
            ),
            ScanState::Clean as i64
        );

        let tx = conn.transaction().unwrap();
        delete_file(&tx, keep).unwrap();
        tx.commit().unwrap();
        assert_eq!(fts_ids(&conn, "files_fts", "notaria"), vec![deep]);

        let tx = conn.transaction().unwrap();
        let tail = insert_file(
            &tx,
            sibling,
            &NewFile {
                name: "otro.txt",
                size: 1,
                mtime: 1,
                lossy: false,
            },
        )
        .unwrap();
        assert_eq!(delete_dir_subtree(&tx, gestoria).unwrap(), 2);
        tx.commit().unwrap();
        assert!(fts_ids(&conn, "files_fts", "notaria").is_empty());
        assert!(fts_ids(&conn, "files_fts", "img_").is_empty());
        assert_eq!(fts_ids(&conn, "dirs_fts", "gestoria"), vec![sibling]);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM dirs"), 2);
        assert_eq!(fts_ids(&conn, "files_fts", "otro"), vec![tail]);

        let tx = conn.transaction().unwrap();
        assert_eq!(delete_dir_subtree(&tx, root_dir).unwrap(), 1);
        tx.commit().unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM dirs"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM files"), 0);
        assert!(fts_ids(&conn, "files_fts", "otro").is_empty());
    }

    /// VFS-1 AC-6: a garbage file in place of the index is moved to `index.db.bad` and a
    /// fresh, usable index is created.
    #[test]
    fn garbage_index_is_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let garbage: Vec<u8> = (0..8192u32).map(|i| (i * 31 % 251) as u8).collect();
        std::fs::write(&path, &garbage).unwrap();
        let (conn, recovered) = open_or_recover(&path).unwrap();
        assert!(recovered);
        assert_eq!(std::fs::read(bad_path(&path)).unwrap(), garbage);
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM roots"), 0);
        drop(conn);
        let (_, again) = open_or_recover(&path).unwrap();
        assert!(!again, "a healthy index is opened as is");
    }

    /// VFS-1 AC-6: recovery re-adds every readable root pending deep, so the next scan
    /// rebuilds the index without the user adding folders again.
    #[test]
    fn recovery_keeps_roots_for_a_full_rescan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let mut conn = open_rw(&path).unwrap();
        let tx = conn.transaction().unwrap();
        let root = insert_root(&tx, "/", "/datos/Fotos").unwrap();
        tx.execute(
            "UPDATE roots SET status = ?2 WHERE id = ?1",
            [root, RootStatus::Ready.code()],
        )
        .unwrap();
        tx.execute("UPDATE dirs SET scan_state = ?1", [ScanState::Clean as i64])
            .unwrap();
        tx.commit().unwrap();
        drop(conn);
        let conn = recover(&path).unwrap();
        assert!(bad_path(&path).exists());
        let (rpath, status): (String, i64) = conn
            .query_row("SELECT path, status FROM roots", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(
            (rpath.as_str(), status),
            ("/datos/Fotos", RootStatus::Pending.code())
        );
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM dirs WHERE rel_path = '' AND scan_state = ?1",
                [ScanState::PendingDeep as i64],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pending, 1);
    }

    /// VFS-23 AC-1: when a torn page leaves `roots` partly readable, recovery joins what is
    /// left with the backup; a backup cut short or holding a torn entry keeps its whole ones.
    #[test]
    fn recovery_joins_partial_roots_with_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let mut conn = open_rw(&path).unwrap();
        let tx = conn.transaction().unwrap();
        insert_root(&tx, "/", "/datos/Fotos").unwrap();
        tx.commit().unwrap();
        drop(conn);
        // The backup knows a root the damaged table lost, then a torn entry and a cut tail.
        let mut backup = b"/datos/Fotos\0/datos/Cartas\0".to_vec();
        backup.extend_from_slice(b"\xff\xfe\0/datos/Cort");
        std::fs::write(roots_backup_path(&path), &backup).unwrap();
        assert_eq!(
            read_roots_backup(&path).unwrap(),
            vec!["/datos/Fotos", "/datos/Cartas"]
        );
        let conn = recover(&path).unwrap();
        let paths: Vec<String> = conn
            .prepare("SELECT path FROM roots ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(paths, vec!["/datos/Fotos", "/datos/Cartas"]);
    }

    /// VFS-23 AC-1: a root the user is removing is not brought back by a recovery.
    #[test]
    fn recovery_leaves_out_roots_being_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let mut conn = open_rw(&path).unwrap();
        let tx = conn.transaction().unwrap();
        insert_root(&tx, "/", "/datos/Fotos").unwrap();
        let gone = insert_root(&tx, "/", "/datos/Viejo").unwrap();
        tx.execute(
            "UPDATE roots SET status = ?2 WHERE id = ?1",
            [gone, RootStatus::Removing.code()],
        )
        .unwrap();
        tx.commit().unwrap();
        drop(conn);
        let conn = recover(&path).unwrap();
        let paths: Vec<String> = conn
            .prepare("SELECT path FROM roots")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(paths, vec!["/datos/Fotos"]);
    }

    /// VFS-3 AC-3, VFS-1 AC-6: recovery killed after any of its steps (replacement built,
    /// damaged index moved aside, replacement swapped in), with the damaged index's WAL
    /// still next to it, loses neither the root list nor applies the old WAL to the new
    /// index: the next open (an unclean one, the marker of the killed session left behind)
    /// yields a sound index holding the root, pending a full scan. Before this, a kill
    /// after the new empty index was created but before its roots were committed, or
    /// between moving the index and moving its WAL, lost every root.
    #[test]
    fn recovery_killed_after_any_step_keeps_roots() {
        type Step = fn(&Path) -> Result<()>;
        let steps: [Step; 3] = [build_fresh, move_aside, finish_recovery];
        for done in 0..=steps.len() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("index.db");
            let mut conn = open_rw(&path).unwrap();
            conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
            let tx = conn.transaction().unwrap();
            insert_root(&tx, "/", "/datos/Fotos").unwrap();
            tx.commit().unwrap();
            // Keep the root only in the WAL, as after a kill: copy both files while open.
            let wal = with_suffix(&path, WAL_SUFFIX);
            let (db_bytes, wal_bytes) =
                (std::fs::read(&path).unwrap(), std::fs::read(&wal).unwrap());
            drop(conn);
            std::fs::write(&path, db_bytes).unwrap();
            std::fs::write(&wal, wal_bytes).unwrap();
            for step in &steps[..done] {
                step(&path).unwrap();
            }
            let session = session_dir(&path);
            std::fs::create_dir_all(&session).unwrap();
            std::fs::write(session.join("999999-0.lock"), b"").unwrap();

            let (conn, _recovered) = open_or_recover(&path).unwrap();
            let roots: Vec<String> = conn
                .prepare("SELECT path FROM roots")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            assert_eq!(roots, vec!["/datos/Fotos"], "killed after {done} steps");
            assert_eq!(
                count(&conn, "SELECT COUNT(*) FROM dirs WHERE rel_path = ''"),
                1
            );
            quick_check(&conn).unwrap();
        }
    }

    /// VFS-3 AC-9, VFS-1 AC-6: the app and the command line creating the same new index at
    /// the same moment both get it, and neither mistakes the other's half-created schema
    /// for damage. Before this, the loser re-ran the version-1 schema ("table meta already
    /// exists"), took that for corruption and moved the winner's live index to `.bad`.
    #[test]
    fn concurrent_first_opens_never_recover() {
        for round in 0..20 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("index.db");
            let barrier = std::sync::Barrier::new(4);
            std::thread::scope(|s| {
                for _ in 0..4 {
                    s.spawn(|| {
                        barrier.wait();
                        let (conn, recovered) = open_or_recover(&path).unwrap();
                        assert!(
                            !recovered,
                            "round {round}: a healthy new index was recovered"
                        );
                        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
                    });
                }
            });
            assert!(!bad_path(&path).exists(), "round {round}");
        }
    }

    /// An index from a newer build is never moved aside.
    #[test]
    fn newer_schema_is_not_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        open_rw(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(matches!(open_or_recover(&path), Err(Error::SchemaTooNew)));
        assert!(!bad_path(&path).exists());
    }

    /// Relative paths are joined component by component with native separators.
    #[test]
    fn join_rel_builds_native_paths() {
        assert_eq!(join_rel("/r", ""), Path::new("/r"));
        assert_eq!(
            join_rel("/r", "a/b c"),
            Path::new("/r").join("a").join("b c")
        );
    }

    /// VFS-41 AC-4: the open record gives the time, whether the check ran, whether the
    /// index was rebuilt and its size, and nothing else.
    #[test]
    fn open_record_carries_time_check_and_size() {
        let line = open_record(Duration::from_millis(43_210), true, false, Some(1874 << 20));
        assert_eq!(line, "open ms=43210 checked=1 recovered=0 index_mb=1874");
        assert_eq!(
            open_record(Duration::ZERO, false, true, None),
            "open ms=0 checked=0 recovered=1 index_mb=?"
        );
    }

    /// VFS-41 AC-3: opening the index sets the process's SQLite soft heap limit.
    #[test]
    fn open_sets_the_soft_heap_limit() {
        let dir = tempfile::tempdir().unwrap();
        let _vfs = crate::Vfs::open(&dir.path().join("index.db")).unwrap();
        // SAFETY: a negative argument only reads the current limit.
        let limit = unsafe { rusqlite::ffi::sqlite3_soft_heap_limit64(-1) };
        assert_eq!(limit, SOFT_HEAP_LIMIT_BYTES);
    }

    /// Builds an index whose last page is overwritten: it opens, but fails quick_check.
    fn corrupt_tail_index(path: &Path) {
        let mut conn = open_rw(path).unwrap();
        let tx = conn.transaction().unwrap();
        let root = insert_root(&tx, "/", "/datos").unwrap();
        let top: i64 = tx
            .query_row("SELECT id FROM dirs WHERE root_id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        for i in 0..3000 {
            let name = format!("archivo número {i:05}.pdf");
            let file = NewFile {
                name: &name,
                size: i,
                mtime: 0,
                lossy: false,
            };
            insert_file(&tx, top, &file).unwrap();
        }
        tx.commit().unwrap();
        drop(conn); // the last connection checkpoints the WAL into the main file
        let mut bytes = std::fs::read(path).unwrap();
        // The last page in use, from the header (page size at 16, page count at 28): with a
        // chunk size (Windows, VFS-5 R10) the file ends in unused padding.
        let page = match u16::from_be_bytes([bytes[16], bytes[17]]) {
            1 => 65536,
            n => usize::from(n),
        };
        let pages = u32::from_be_bytes(bytes[28..32].try_into().unwrap()) as usize;
        let len = pages * page;
        assert!(pages > 4 && len <= bytes.len());
        bytes[len - page..len].fill(0xA5);
        std::fs::write(path, bytes).unwrap();
    }

    /// VFS-1 AC-6: after a clean shutdown the next open skips the full integrity check (it
    /// would read the whole index); after an unclean one the check runs and a damaged index
    /// is moved to `index.db.bad` and rebuilt from its roots.
    #[test]
    fn integrity_check_runs_only_after_unclean_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        corrupt_tail_index(&path);

        let (conn, recovered) = open_or_recover(&path).unwrap();
        assert!(
            !recovered,
            "no earlier session: the damaged tail is not read at open"
        );
        drop(conn);
        let (conn, recovered) = open_or_recover(&path).unwrap();
        assert!(!recovered, "clean reopen does not run quick_check");
        drop(conn);
        assert!(!session_dir(&path).exists());

        // A marker nobody holds locked is what a crashed session leaves behind.
        std::fs::create_dir_all(session_dir(&path)).unwrap();
        let stale = session_dir(&path).join(format!("99999-0.{SESSION_MARKER_EXT}"));
        std::fs::write(stale, b"").unwrap();
        let (conn, recovered) = open_or_recover(&path).unwrap();
        assert!(recovered);
        assert!(bad_path(&path).exists());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM roots"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM files"), 0);
        assert!(!session_dir(&path).exists(), "the stale marker was removed");
    }

    /// Two sessions on one index (CLI and app): the first to close leaves the other's marker
    /// alone, and a live session is never taken for a crashed one.
    #[test]
    fn concurrent_sessions_keep_each_others_markers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let (a, unclean_a) = begin_session(&path).unwrap();
        let (b, unclean_b) = begin_session(&path).unwrap();
        assert!(!unclean_a && !unclean_b, "a live session is not stale");
        let b_path = b.path.clone();
        drop(a);
        assert!(b_path.exists());
        let (c, unclean_c) = begin_session(&path).unwrap();
        assert!(!unclean_c);
        drop(c);
        assert!(b_path.exists());
        drop(b);
        assert!(!session_dir(&path).exists());
    }

    /// File size of the index after a full checkpoint.
    fn checkpointed_size(conn: &Connection, path: &Path) -> u64 {
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        std::fs::metadata(path).unwrap().len()
    }

    /// VFS-5 AC-11: with a chunk size the index file grows in whole chunks; on Windows every
    /// writer opened by `open_rw` uses one.
    #[test]
    fn writer_file_grows_in_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let conn = open_rw(&path).unwrap();
        #[cfg(windows)]
        let chunk = WRITER_CHUNK_BYTES as u64;
        #[cfg(not(windows))]
        let chunk = {
            let c: u64 = 1 << 20;
            set_chunk_size(&conn, c as i32).unwrap();
            c
        };
        conn.execute_batch(
            "CREATE TABLE t(x); \
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5000) \
             INSERT INTO t SELECT randomblob(300) FROM n;",
        )
        .unwrap();
        let size = checkpointed_size(&conn, &path);
        assert!(size >= chunk, "{size}");
        assert_eq!(size % chunk, 0, "{size} is not a whole number of chunks");
    }

    fn pragma(conn: &Connection, name: &str) -> i64 {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
            .unwrap()
    }

    /// VFS-1 AC-9: no connection memory-maps the index (resident memory stays bounded by
    /// the page caches), every connection is in WAL mode so readers never block the
    /// indexer, and a reader refuses writes.
    #[test]
    fn connection_behaviour() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let w = open_rw(&path).unwrap();
        let r = open_ro(&path).unwrap();
        for c in [&w, &r] {
            assert_eq!(pragma(c, "mmap_size"), 0);
            let mode: String = c
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
        }
        assert!(
            r.execute("DELETE FROM roots", []).is_err(),
            "reader is query-only"
        );
        assert!(w.execute("DELETE FROM roots", []).is_ok());
    }

    /// VFS-1 AC-4: roots on one volume share its row; a Windows root is named after its last
    /// component; unknown ids are `NotFound`.
    #[test]
    fn roots_volumes_and_missing_ids() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open_rw(&dir.path().join("index.db")).unwrap();
        let tx = conn.transaction().unwrap();
        let a = insert_root(&tx, "C:", r"C:\Users\Ana\Fotos").unwrap();
        let b = insert_root(&tx, "C:", r"C:\Datos").unwrap();
        tx.commit().unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM volumes"), 1);
        let (key, name): (String, String) = conn
            .query_row(
                "SELECT r.path_key, d.name_key FROM roots r JOIN dirs d ON d.root_id = r.id \
                 WHERE r.id = ?1",
                [a],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (key.as_str(), name.as_str()),
            ("c:/users/ana/fotos", "fotos")
        );
        assert_ne!(a, b);
        assert!(matches!(dir_abs_path(&conn, 9999), Err(Error::NotFound(_))));
        let tx = conn.transaction().unwrap();
        assert!(matches!(
            delete_dir_subtree(&tx, 9999),
            Err(Error::NotFound(_))
        ));
    }

    /// VFS-1 AC-6: recovery moves the index's `-wal` with it and drops companions of an
    /// older `.bad` copy that no longer match.
    #[test]
    fn recovery_moves_companion_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let side = |base: &Path, suffix: &str| {
            let mut p = base.as_os_str().to_owned();
            p.push(suffix);
            PathBuf::from(p)
        };
        std::fs::write(&path, b"garbage, not a database file at all").unwrap();
        std::fs::write(side(&path, "-wal"), b"old wal").unwrap();
        std::fs::write(side(&bad_path(&path), "-shm"), b"stale").unwrap();
        // Called directly: a failed open_rw would already have discarded the invalid WAL.
        drop(recover(&path).unwrap());
        assert_eq!(
            std::fs::read(side(&bad_path(&path), "-wal")).unwrap(),
            b"old wal"
        );
        // The stale companion is gone (removed, or replaced by this index's own `-shm`).
        let shm = std::fs::read(side(&bad_path(&path), "-shm")).unwrap_or_default();
        assert_ne!(shm, b"stale");
    }
}
