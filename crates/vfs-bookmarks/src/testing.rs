//! Fake Firefox data folders for tests (feature `test-support`, VFS-6): a `profiles.ini`,
//! profiles whose `places.sqlite` has Firefox's real schema (`places-schema.sql`) in WAL
//! mode, and `jsonlz4` backups.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::firefox::Locator;

/// Firefox's schema (version 88), no data.
pub const PLACES_SCHEMA: &str = include_str!("places-schema.sql");

/// Ids of the root folders in a new profile, as Firefox creates them.
pub const MENU: i64 = 2;
pub const TOOLBAR: i64 = 3;
pub const TAGS: i64 = 4;
pub const UNFILED: i64 = 5;
pub const MOBILE: i64 = 6;

/// A Firefox data folder (the one holding `profiles.ini`).
pub struct FakeFirefox {
    pub root: PathBuf,
    ini: String,
    next_section: usize,
}

impl FakeFirefox {
    pub fn new(root: &Path) -> FakeFirefox {
        std::fs::create_dir_all(root).unwrap();
        FakeFirefox {
            root: root.to_path_buf(),
            ini: String::from("[General]\nStartWithLastProfile=1\nVersion=2\n\n"),
            next_section: 0,
        }
    }

    pub fn locator(&self) -> Locator {
        Locator {
            roots: vec![self.root.clone()],
        }
    }

    /// Lists a profile in `profiles.ini` (`Profiles/<dir_name>`, relative) without creating
    /// anything on disk; `store_id` makes it a profile group's entry.
    pub fn list_profile(&mut self, name: &str, dir_name: &str, store_id: Option<&str>) -> PathBuf {
        let _ = write!(
            self.ini,
            "[Profile{}]\nName={name}\nIsRelative=1\nPath=Profiles/{dir_name}\n",
            self.next_section
        );
        if let Some(id) = store_id {
            let _ = writeln!(self.ini, "StoreID={id}");
        }
        self.ini.push('\n');
        self.next_section += 1;
        std::fs::write(self.root.join("profiles.ini"), &self.ini).unwrap();
        self.root.join("Profiles").join(dir_name)
    }

    /// Lists a profile and creates its `places.sqlite`; returns the profile folder.
    pub fn add_profile(&mut self, name: &str, dir_name: &str) -> PathBuf {
        let dir = self.list_profile(name, dir_name, None);
        create_places(&dir.join("places.sqlite"));
        dir
    }

    /// A profile group database listing `(name, dir_name)` profiles under `Profiles/`.
    pub fn add_group(&self, store_id: &str, profiles: &[(&str, &str)]) {
        let dir = self.root.join("Profile Groups");
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join(format!("{store_id}.sqlite"))).unwrap();
        conn.execute_batch(
            "CREATE TABLE Profiles(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, \
             name TEXT NOT NULL, avatar TEXT, themeId TEXT)",
        )
        .unwrap();
        for (name, d) in profiles {
            conn.execute(
                "INSERT INTO Profiles(path, name) VALUES(?1, ?2)",
                params![format!("Profiles/{d}"), name],
            )
            .unwrap();
            create_places(&self.root.join("Profiles").join(d).join("places.sqlite"));
        }
    }
}

/// A `places.sqlite` with Firefox's schema and root folders, in WAL mode.
pub fn create_places(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA page_size = 32768; PRAGMA journal_mode = WAL;")
        .unwrap();
    conn.execute_batch(PLACES_SCHEMA).unwrap();
    for (id, parent, guid, title) in [
        (1, 0, "root________", ""),
        (MENU, 1, "menu________", "menu"),
        (TOOLBAR, 1, "toolbar_____", "toolbar"),
        (TAGS, 1, "tags________", "tags"),
        (UNFILED, 1, "unfiled_____", "unfiled"),
        (MOBILE, 1, "mobile______", "mobile"),
    ] {
        conn.execute(
            "INSERT INTO moz_bookmarks(id, type, parent, position, title, dateAdded, lastModified, guid) \
             VALUES(?1, 2, ?2, 0, ?3, 0, 0, ?4)",
            params![id, parent, title, guid],
        )
        .unwrap();
    }
}

/// Writes into an open `places.sqlite` the way Firefox stores bookmarks.
pub struct Places<'c> {
    pub conn: &'c Connection,
}

/// Microseconds since 1970 for `secs`.
pub fn prtime(secs: i64) -> i64 {
    secs * 1_000_000
}

fn guid(conn: &Connection) -> String {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM moz_bookmarks", [], |r| r.get(0))
        .unwrap();
    format!("vfstest{n:05}")
}

impl Places<'_> {
    pub fn folder(&self, parent: i64, title: &str) -> i64 {
        let g = guid(self.conn);
        self.conn
            .execute(
                "INSERT INTO moz_bookmarks(type, parent, position, title, dateAdded, lastModified, guid) \
                 VALUES(2, ?1, 0, ?2, 0, 0, ?3)",
                params![parent, title, g],
            )
            .unwrap();
        self.conn.last_insert_rowid()
    }

    fn place(&self, url: &str) -> i64 {
        if let Ok(id) =
            self.conn
                .query_row("SELECT id FROM moz_places WHERE url = ?1", [url], |r| {
                    r.get(0)
                })
        {
            return id;
        }
        self.conn
            .execute(
                "INSERT INTO moz_places(url, title, visit_count, guid, url_hash) \
                 VALUES(?1, 'history title', 7, ?2, 1)",
                params![url, guid(self.conn)],
            )
            .unwrap();
        let id = self.conn.last_insert_rowid();
        // History the reader must never read (VFS-6 R5).
        self.conn
            .execute(
                "INSERT INTO moz_historyvisits(place_id, visit_date, visit_type) VALUES(?1, 1, 1)",
                [id],
            )
            .unwrap();
        id
    }

    /// A bookmark (`title` `None` stores NULL, as Firefox does for an untitled one).
    pub fn bookmark(&self, parent: i64, title: Option<&str>, url: &str, added_secs: i64) -> i64 {
        let place = self.place(url);
        let g = guid(self.conn);
        self.conn
            .execute(
                "INSERT INTO moz_bookmarks(type, fk, parent, position, title, dateAdded, lastModified, guid) \
                 VALUES(1, ?1, ?2, 0, ?3, ?4, ?4, ?5)",
                params![place, parent, title, prtime(added_secs), g],
            )
            .unwrap();
        self.conn.last_insert_rowid()
    }

    pub fn separator(&self, parent: i64) {
        let g = guid(self.conn);
        self.conn
            .execute(
                "INSERT INTO moz_bookmarks(type, parent, position, dateAdded, lastModified, guid) \
                 VALUES(3, ?1, 0, 0, 0, ?2)",
                params![parent, g],
            )
            .unwrap();
    }

    /// Tags `url` the way Firefox does: a bookmark of it inside the tag's folder.
    pub fn tag(&self, url: &str, tag: &str) {
        let folder = self
            .conn
            .query_row(
                "SELECT id FROM moz_bookmarks WHERE parent = ?1 AND type = 2 AND title = ?2",
                params![TAGS, tag],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| self.folder(TAGS, tag));
        self.bookmark(folder, None, url, 0);
    }

    pub fn keyword(&self, url: &str, keyword: &str) {
        let place = self.place(url);
        self.conn
            .execute(
                "INSERT INTO moz_keywords(keyword, place_id) VALUES(?1, ?2)",
                params![keyword, place],
            )
            .unwrap();
    }
}

/// A `jsonlz4` backup file with `json` as content.
pub fn write_jsonlz4(path: &Path, json: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = b"mozLz40\0".to_vec();
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&lz4_flex::block::compress(json.as_bytes()));
    std::fs::write(path, bytes).unwrap();
}
