//! Bookmarks from a copy of `places.sqlite` (VFS-6). Every statement runs under a SQLite
//! authorizer that allows reading only the tables and columns in [`ALLOWED_COLUMNS`]; the
//! history (`moz_historyvisits`, `moz_places.title`, visit counts…) cannot be read (R5).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, OpenFlags};
use vfs_core::Result;

use crate::types::{Bookmark, BookmarkRoot};

/// The only `(table, columns)` the reader may read (VFS-6 "Privacy").
pub const ALLOWED_COLUMNS: &[(&str, &[&str])] = &[
    (
        "moz_bookmarks",
        &[
            "id",
            "type",
            "fk",
            "parent",
            "title",
            "dateAdded",
            "lastModified",
            "guid",
        ],
    ),
    ("moz_places", &["id", "url"]),
    ("moz_keywords", &["keyword", "place_id"]),
];

/// `moz_bookmarks.type` of a bookmark and of a folder.
const TYPE_BOOKMARK: i64 = 1;
const TYPE_FOLDER: i64 = 2;
const TAGS_ROOT_GUID: &str = "tags________";
/// Folders deeper than this are treated as a broken tree (a cycle) and skipped.
const MAX_DEPTH: usize = 128;

fn root_of_guid(guid: &str) -> Option<BookmarkRoot> {
    match guid {
        "menu________" => Some(BookmarkRoot::Menu),
        "toolbar_____" => Some(BookmarkRoot::Toolbar),
        "unfiled_____" => Some(BookmarkRoot::Unfiled),
        "mobile______" => Some(BookmarkRoot::Mobile),
        _ => None,
    }
}

fn authorize(ctx: AuthContext<'_>) -> Authorization {
    match ctx.action {
        AuthAction::Select => Authorization::Allow,
        AuthAction::Read {
            table_name,
            column_name,
        } => {
            let allowed = ALLOWED_COLUMNS
                .iter()
                .any(|(t, cols)| *t == table_name && cols.contains(&column_name));
            if allowed {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }
        _ => Authorization::Deny,
    }
}

/// Opens a copied places database with the authorizer installed. The copy is the app's own
/// file, so it is opened read-write: SQLite needs that to apply the copied WAL.
pub fn open_guarded(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // Load the schema before the authorizer: parsing it is not a read of user data.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
    conn.authorizer(Some(authorize))?;
    Ok(conn)
}

struct Folder {
    parent: i64,
    title: String,
    guid: String,
}

/// Every bookmark of a copied `places.sqlite`, with its folder path, tags and keyword.
pub fn read_places(path: &Path) -> Result<Vec<Bookmark>> {
    let conn = open_guarded(path)?;
    let mut folders: HashMap<i64, Folder> = HashMap::new();
    let mut stmt = conn
        .prepare("SELECT id, parent, title, guid FROM moz_bookmarks WHERE type = ?1 ORDER BY id")?;
    let rows = stmt.query_map([TYPE_FOLDER], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            Folder {
                parent: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                title: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                guid: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            },
        ))
    })?;
    for row in rows {
        let (id, f) = row?;
        folders.insert(id, f);
    }
    let tags_root = folders
        .iter()
        .find(|(_, f)| f.guid == TAGS_ROOT_GUID)
        .map(|(id, _)| *id);

    let mut keywords: HashMap<i64, String> = HashMap::new();
    let mut stmt = conn.prepare("SELECT keyword, place_id FROM moz_keywords")?;
    for row in stmt.query_map([], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i64>>(1)?))
    })? {
        if let (Some(k), Some(p)) = row? {
            keywords.insert(p, k);
        }
    }

    struct Item {
        parent: i64,
        place: i64,
        title: Option<String>,
        added: i64,
        modified: i64,
        guid: String,
        url: String,
    }
    let mut stmt = conn.prepare(
        "SELECT b.parent, b.fk, b.title, b.dateAdded, b.lastModified, b.guid, p.url \
         FROM moz_bookmarks b JOIN moz_places p ON p.id = b.fk \
         WHERE b.type = ?1 ORDER BY b.parent, b.id",
    )?;
    let items = stmt
        .query_map([TYPE_BOOKMARK], |r| {
            Ok(Item {
                parent: r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                place: r.get(1)?,
                title: r.get(2)?,
                added: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                modified: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                guid: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                url: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            })
        })?
        .collect::<rusqlite::Result<Vec<Item>>>()?;

    // A tag is a folder under the tags root; its children name the tagged places.
    let mut tags: HashMap<i64, Vec<String>> = HashMap::new();
    let is_tag_folder =
        |id: i64| tags_root.is_some_and(|t| folders.get(&id).is_some_and(|f| f.parent == t));
    for it in &items {
        if is_tag_folder(it.parent) {
            let name = folders[&it.parent].title.clone();
            let list = tags.entry(it.place).or_default();
            if !name.is_empty() && !list.contains(&name) {
                list.push(name);
            }
        }
    }

    let mut out = Vec::new();
    for it in items {
        if is_tag_folder(it.parent) || Some(it.parent) == tags_root {
            continue;
        }
        let Some((root, folder)) = folder_path(&folders, it.parent) else {
            continue;
        };
        out.push(Bookmark {
            guid: it.guid,
            root,
            folder,
            title: it.title.unwrap_or_default(),
            url: it.url,
            tags: tags.get(&it.place).cloned().unwrap_or_default(),
            keyword: keywords.get(&it.place).cloned(),
            added: it.added / 1_000_000,
            modified: it.modified / 1_000_000,
        });
    }
    Ok(normalize(out))
}

/// The root and the folder names from it down to `parent`; `None` when the chain does not
/// end at one of the four bookmark roots (the tags tree, an orphan, a cycle).
fn folder_path(folders: &HashMap<i64, Folder>, parent: i64) -> Option<(BookmarkRoot, Vec<String>)> {
    let mut names = Vec::new();
    let mut id = parent;
    for _ in 0..MAX_DEPTH {
        let f = folders.get(&id)?;
        if let Some(root) = root_of_guid(&f.guid) {
            names.reverse();
            return Some((root, names));
        }
        names.push(f.title.clone());
        id = f.parent;
    }
    None
}

/// True for addresses that are bookmarks to show: not `place:` queries (smart folders)
/// and not `javascript:` bookmarklets (VFS-6 AC-1).
fn is_shown(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    !url.trim().is_empty() && !lower.starts_with("place:") && !lower.starts_with("javascript:")
}

/// Shared by both readers: drops what is not shown, gives an untitled bookmark its address
/// as title, sorts tags, and keeps one of identical (address, root, folder, title) rows.
pub(super) fn normalize(list: Vec<Bookmark>) -> Vec<Bookmark> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(list.len());
    for mut b in list {
        if !is_shown(&b.url) {
            continue;
        }
        if b.title.trim().is_empty() {
            b.title = b.url.clone();
        }
        b.tags.sort();
        let key = (b.url.clone(), b.root, b.folder.clone(), b.title.clone());
        if b.guid.is_empty() {
            // The store keys rows by guid; a backup entry without one gets a stable stand-in.
            b.guid = format!("{:?}", key);
        }
        if seen.insert(key) {
            out.push(b);
        }
    }
    out
}
