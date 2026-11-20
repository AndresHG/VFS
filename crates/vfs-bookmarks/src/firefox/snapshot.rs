//! A consistent copy of a SQLite database that another process holds open in WAL mode
//! (VFS-6 R3). The original is only ever opened for reading with Rust's default sharing
//! (on Windows read, write and delete sharing), so Firefox can keep writing, checkpointing,
//! renaming and deleting while the copy is made; nothing is created in the profile folder.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use vfs_core::{Error, Result};

/// Copy attempts before giving up on a database that keeps changing.
const COPY_ATTEMPTS: u32 = 3;
/// Wait between attempts.
const RETRY_DELAY: Duration = Duration::from_millis(500);
/// A database larger than this is not copied (the backup is read instead): a places
/// database is a few to tens of MB; Firefox expires history long before this.
const MAX_COPY_BYTES: u64 = 512 * 1024 * 1024;

/// Size and modification time of a file; `None` when it does not exist.
type Stamp = Option<(u64, Option<SystemTime>)>;

fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

fn wal_of(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push("-wal");
    PathBuf::from(s)
}

/// What changes when `places.sqlite` or its WAL changes, or the newest backup appears:
/// compared with the stored value so an unchanged profile is not copied again (AC-9).
pub fn signature(profile_dir: &Path) -> String {
    let db = profile_dir.join("places.sqlite");
    let part = |s: Stamp| match s {
        None => "-".to_string(),
        Some((len, t)) => {
            let ns = t
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            format!("{len}@{ns}")
        }
    };
    let newest_backup = super::backup::backup_files(&profile_dir.join("bookmarkbackups"))
        .into_iter()
        .next()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    format!(
        "{} {} {newest_backup}",
        part(stamp(&db)),
        part(stamp(&wal_of(&db)))
    )
}

/// A copied database; its files are deleted when this is dropped.
pub struct Copy {
    pub path: PathBuf,
}

impl Drop for Copy {
    fn drop(&mut self) {
        remove_copy(&self.path);
    }
}

fn remove_copy(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut s = path.as_os_str().to_owned();
        s.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(s));
    }
}

fn copy_file(from: &Path, to: &Path) -> io::Result<()> {
    let mut src = File::open(from)?;
    let mut dst = File::create(to)?;
    io::copy(&mut src, &mut dst)?;
    Ok(())
}

/// Copies `db` and its `-wal` into `work_dir` and returns the copy once the size and
/// modification time of both were the same before and after copying and SQLite's
/// `quick_check` passes on the copy; retries a database that changed meanwhile.
pub fn copy_consistent(db: &Path, work_dir: &Path) -> Result<Copy> {
    std::fs::create_dir_all(work_dir)?;
    let name = db.file_name().unwrap_or_default();
    let target = work_dir.join(name);
    let wal = wal_of(db);
    for attempt in 0..COPY_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(RETRY_DELAY);
        }
        let before = (stamp(db), stamp(&wal));
        match before.0 {
            None => return Err(Error::NotFound(db.to_string_lossy().into_owned())),
            Some((len, _)) if len > MAX_COPY_BYTES => {
                return Err(Error::Unavailable(format!("{} is too large", db.display())))
            }
            _ => {}
        }
        remove_copy(&target);
        let copy = Copy {
            path: target.clone(),
        };
        copy_file(db, &target)?;
        if before.1.is_some() {
            if let Err(e) = copy_file(&wal, &wal_of(&target)) {
                // The WAL vanished (Firefox closed and checkpointed): copy again.
                log::warn!("copy {}: {e}", wal.display());
                continue;
            }
        }
        if (stamp(db), stamp(&wal)) != before {
            continue;
        }
        if quick_check(&target)? {
            return Ok(copy);
        }
    }
    Err(Error::Unavailable(format!(
        "{} kept changing or is damaged",
        db.display()
    )))
}

fn quick_check(path: &Path) -> Result<bool> {
    let conn = rusqlite::Connection::open(path)?;
    let res: rusqlite::Result<String> = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0));
    match res {
        Ok(v) => Ok(v == "ok"),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if matches!(
                e.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(e.into()),
    }
}
