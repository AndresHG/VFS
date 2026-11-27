//! `bookmarks.db`: the setting and what was read, in the app's local data directory next to
//! `index.db` (VFS-6 D3). It holds nothing that cannot be read again, so a file that cannot
//! be opened is deleted and created anew.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use vfs_core::Result;

use crate::types::{BookmarkRoot, Browser, Tree};

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
CREATE TABLE IF NOT EXISTS items(
  id INTEGER PRIMARY KEY,
  profile_id INTEGER NOT NULL,
  guid TEXT NOT NULL,
  is_folder INTEGER NOT NULL,
  parent_guid TEXT,
  position INTEGER NOT NULL,
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
const SEP: char = '\u{1f}';
/// Firefox's switch keeps the key it had before other browsers (VFS-6).
const KEY_ENABLED: &str = "enabled";
const ON: &str = "1";
const OFF: &str = "0";

/// A stored bookmark or folder.
pub(crate) struct Row {
    pub id: i64,
    pub profile_id: i64,
    pub browser: Browser,
    pub profile: String,
    pub guid: String,
    pub is_folder: bool,
    pub parent_guid: Option<String>,
    pub position: i64,
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

/// The `settings` key of a browser's switch.
fn enabled_key(browser: Browser) -> &'static str {
    match browser {
        Browser::Firefox => KEY_ENABLED,
        Browser::Chrome => "enabled.chrome",
        Browser::Brave => "enabled.brave",
    }
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

    pub fn enabled(&self, browser: Browser) -> Result<bool> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [enabled_key(browser)],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.as_deref() == Some(ON))
    }

    /// Turning a browser off deletes its stored profiles, bookmarks and folders (VFS-6 R8,
    /// VFS-44 R9).
    pub fn set_enabled(&mut self, browser: Browser, on: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![enabled_key(browser), if on { ON } else { OFF }],
        )?;
        if !on {
            tx.execute(
                "DELETE FROM items WHERE profile_id IN \
                 (SELECT id FROM profiles WHERE browser = ?1)",
                [browser.code()],
            )?;
            tx.execute("DELETE FROM profiles WHERE browser = ?1", [browser.code()])?;
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

    /// Replaces one profile's bookmarks and folders in one transaction. A row keeps its id
    /// while Firefox's guid stays the same, so a result on screen, or a folder chosen as a
    /// filter, still names the same item after a refresh.
    pub fn replace_profile(
        &mut self,
        browser: Browser,
        dir: &str,
        name: &str,
        signature: &str,
        tree: &Tree,
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
                "INSERT INTO items(profile_id, guid, is_folder, parent_guid, position, root, folder, \
                 title, url, tags, keyword, added, modified) \
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
                 ON CONFLICT(profile_id, guid) DO UPDATE SET is_folder = excluded.is_folder, \
                 parent_guid = excluded.parent_guid, position = excluded.position, \
                 root = excluded.root, folder = excluded.folder, title = excluded.title, \
                 url = excluded.url, tags = excluded.tags, keyword = excluded.keyword, \
                 added = excluded.added, modified = excluded.modified",
            )?;
            for f in &tree.folders {
                up.execute(params![
                    profile_id,
                    f.guid,
                    true,
                    f.parent_guid,
                    f.position,
                    f.root.code(),
                    join(&f.path),
                    f.title,
                    "",
                    "",
                    None::<String>,
                    f.added,
                    f.added
                ])?;
            }
            for b in &tree.bookmarks {
                up.execute(params![
                    profile_id,
                    b.guid,
                    false,
                    b.parent_guid,
                    b.position,
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
            let seen: HashSet<&str> = tree
                .folders
                .iter()
                .map(|f| f.guid.as_str())
                .chain(tree.bookmarks.iter().map(|b| b.guid.as_str()))
                .collect();
            let mut stmt = tx.prepare("SELECT id, guid FROM items WHERE profile_id = ?1")?;
            let gone: Vec<i64> = stmt
                .query_map([profile_id], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .filter(|(_, g)| !seen.contains(g.as_str()))
                .map(|(id, _)| id)
                .collect();
            let mut del = tx.prepare("DELETE FROM items WHERE id = ?1")?;
            for id in gone {
                del.execute([id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Forgets profiles that are no longer found, with what was read from them.
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
                tx.execute("DELETE FROM items WHERE profile_id = ?1", [id])?;
                tx.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The newest successful read of any profile, or of `browser`'s.
    pub fn last_read(&self, browser: Option<Browser>) -> Result<Option<i64>> {
        Ok(self.conn.query_row(
            "SELECT max(read_at) FROM profiles WHERE ?1 IS NULL OR browser = ?1",
            [browser.map(Browser::code)],
            |r| r.get(0),
        )?)
    }

    pub fn load(&self) -> Result<Vec<Row>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.profile_id, p.browser, p.name, i.guid, i.is_folder, i.parent_guid, i.position, \
             i.root, i.folder, i.title, i.url, i.tags, i.keyword, i.added \
             FROM items i JOIN profiles p ON p.id = i.profile_id ORDER BY i.id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    profile_id: r.get(1)?,
                    browser: Browser::from_code(r.get(2)?),
                    profile: r.get(3)?,
                    guid: r.get(4)?,
                    is_folder: r.get(5)?,
                    parent_guid: r.get(6)?,
                    position: r.get(7)?,
                    root: BookmarkRoot::from_code(r.get(8)?),
                    folder: split(&r.get::<_, String>(9)?),
                    title: r.get(10)?,
                    url: r.get(11)?,
                    tags: split(&r.get::<_, String>(12)?),
                    keyword: r.get(13)?,
                    added: r.get(14)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<Row>>>()?;
        Ok(rows)
    }

    /// Number of stored bookmarks and folders.
    pub fn count(&self) -> Result<u32> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM items", [], |r| r.get(0))?)
    }
}
