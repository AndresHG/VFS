//! `bookmarks.db`: the setting and the bookmarks read, in the app's local data directory
//! next to `index.db` (VFS-6 D3). It holds nothing that cannot be read again, so a file
//! that cannot be opened is deleted and created anew.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use vfs_core::Result;

use crate::types::{Bookmark, BookmarkRoot, Browser};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS profiles(
  id INTEGER PRIMARY KEY,
  browser INTEGER NOT NULL,
  dir TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  signature TEXT,
  read_at INTEGER
);
CREATE TABLE IF NOT EXISTS bookmarks(
  id INTEGER PRIMARY KEY,
  profile_id INTEGER NOT NULL,
  guid TEXT NOT NULL,
  root INTEGER NOT NULL,
  folder TEXT NOT NULL,
  title TEXT NOT NULL,
  url TEXT NOT NULL,
  tags TEXT NOT NULL,
  keyword TEXT,
  added INTEGER NOT NULL,
  modified INTEGER NOT NULL,
  UNIQUE(profile_id, guid)
);
PRAGMA user_version = 1;
";

/// Joins folder names and tags in one column; never part of a name Firefox shows.
pub(crate) const SEP: char = '\u{1f}';
const KEY_ENABLED: &str = "enabled";

/// A stored bookmark with what the search needs.
pub(crate) struct Row {
    pub id: i64,
    pub profile: String,
    pub root: BookmarkRoot,
    pub folder: Vec<String>,
    pub title: String,
    pub url: String,
    pub tags: Vec<String>,
    pub keyword: Option<String>,
    pub added: i64,
}

pub(crate) struct Store {
    conn: Connection,
}

fn split(s: &str) -> Vec<String> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.split(SEP).map(String::from).collect()
    }
}

fn join(v: &[String]) -> String {
    v.iter()
        .map(|s| s.replace(SEP, " "))
        .collect::<Vec<_>>()
        .join(&SEP.to_string())
}

fn open_conn(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    // Deleted bookmarks are overwritten in the file, not left in free pages (R8).
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA secure_delete=ON;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        match open_conn(path) {
            Ok(conn) => Ok(Store { conn }),
            Err(e) => {
                log::warn!("{}: {e}; starting a new one", path.display());
                for suffix in ["", "-wal", "-shm"] {
                    let mut p = path.as_os_str().to_owned();
                    p.push(suffix);
                    let _ = std::fs::remove_file(std::path::PathBuf::from(p));
                }
                Ok(Store {
                    conn: open_conn(path)?,
                })
            }
        }
    }

    pub fn enabled(&self) -> Result<bool> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [KEY_ENABLED],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.as_deref() == Some("1"))
    }

    /// Turning it off deletes every stored profile and bookmark (R8).
    pub fn set_enabled(&mut self, on: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEY_ENABLED, if on { "1" } else { "0" }],
        )?;
        if !on {
            tx.execute("DELETE FROM bookmarks", [])?;
            tx.execute("DELETE FROM profiles", [])?;
        }
        tx.commit()?;
        if !on {
            self.conn
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
        }
        Ok(())
    }

    pub fn signature(&self, dir: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT signature FROM profiles WHERE dir = ?1",
                [dir],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Replaces one profile's bookmarks in one transaction. Rows keep their id while the
    /// browser's guid stays the same, so a result on screen still names the same bookmark
    /// after a refresh.
    pub fn replace_profile(
        &mut self,
        browser: Browser,
        dir: &str,
        name: &str,
        signature: &str,
        list: &[Bookmark],
        now: i64,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO profiles(browser, dir, name, signature, read_at) VALUES(?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(dir) DO UPDATE SET name = excluded.name, \
             signature = excluded.signature, read_at = excluded.read_at",
            params![browser.code(), dir, name, signature, now],
        )?;
        let profile_id: i64 =
            tx.query_row("SELECT id FROM profiles WHERE dir = ?1", [dir], |r| {
                r.get(0)
            })?;
        {
            let mut up = tx.prepare(
                "INSERT INTO bookmarks(profile_id, guid, root, folder, title, url, tags, keyword, added, modified) \
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                 ON CONFLICT(profile_id, guid) DO UPDATE SET root = excluded.root, \
                 folder = excluded.folder, title = excluded.title, url = excluded.url, \
                 tags = excluded.tags, keyword = excluded.keyword, added = excluded.added, \
                 modified = excluded.modified",
            )?;
            for b in list {
                up.execute(params![
                    profile_id,
                    b.guid,
                    b.root.code(),
                    join(&b.folder),
                    b.title,
                    b.url,
                    join(&b.tags),
                    b.keyword,
                    b.added,
                    b.modified
                ])?;
            }
            let seen: std::collections::HashSet<&str> =
                list.iter().map(|b| b.guid.as_str()).collect();
            let mut stmt = tx.prepare("SELECT id, guid FROM bookmarks WHERE profile_id = ?1")?;
            let gone: Vec<i64> = stmt
                .query_map([profile_id], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .filter(|(_, g)| !seen.contains(g.as_str()))
                .map(|(id, _)| id)
                .collect();
            let mut del = tx.prepare("DELETE FROM bookmarks WHERE id = ?1")?;
            for id in gone {
                del.execute([id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Forgets profiles that are no longer found, with their bookmarks.
    pub fn keep_only_profiles(&mut self, dirs: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        let stored: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, dir FROM profiles")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        for (id, dir) in stored {
            if !dirs.contains(&dir) {
                tx.execute("DELETE FROM bookmarks WHERE profile_id = ?1", [id])?;
                tx.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The newest successful read of any profile.
    pub fn last_read(&self) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT max(read_at) FROM profiles", [], |r| r.get(0))?)
    }

    pub fn load(&self) -> Result<Vec<Row>> {
        let mut stmt = self.conn.prepare(
            "SELECT b.id, p.name, b.root, b.folder, b.title, b.url, b.tags, b.keyword, b.added \
             FROM bookmarks b JOIN profiles p ON p.id = b.profile_id ORDER BY b.id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    profile: r.get(1)?,
                    root: BookmarkRoot::from_code(r.get(2)?),
                    folder: split(&r.get::<_, String>(3)?),
                    title: r.get(4)?,
                    url: r.get(5)?,
                    tags: split(&r.get::<_, String>(6)?),
                    keyword: r.get(7)?,
                    added: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<Row>>>()?;
        Ok(rows)
    }

    /// Number of bookmarks in the store (tests and status).
    pub fn count(&self) -> Result<u32> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM bookmarks", [], |r| r.get(0))?)
    }
}
