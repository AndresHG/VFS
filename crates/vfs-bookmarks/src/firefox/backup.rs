//! Firefox's daily bookmark backups, `bookmarkbackups/bookmarks-YYYY-MM-DD[_n_hash].jsonlz4`
//! (VFS-6 R4): written atomically by Firefox, so they are read in place. `jsonlz4` is
//! `mozLz40\0`, a little-endian u32 with the decompressed size, then one LZ4 block of JSON.

use std::path::{Path, PathBuf};

use serde_json::Value;
use vfs_core::{Error, Result};

use crate::types::{Bookmark, BookmarkRoot, Folder, Tree};

const MAGIC: &[u8; 8] = b"mozLz40\0";
/// Largest decompressed backup accepted (a 3,000-bookmark backup is about 1 MB).
const MAX_JSON_BYTES: usize = 256 * 1024 * 1024;
/// Nesting deeper than this is not followed.
const MAX_DEPTH: usize = 128;

/// Backup files of a `bookmarkbackups` folder, newest first (the date leads the name).
pub(super) fn backup_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy();
            n.starts_with("bookmarks-") && (n.ends_with(".jsonlz4") || n.ends_with(".json"))
        })
        .collect();
    files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    files
}

/// The bookmarks of the newest backup in `dir` that can be read.
pub fn read_backup(dir: &Path) -> Result<Tree> {
    let mut last = Error::NotFound(dir.to_string_lossy().into_owned());
    for file in backup_files(dir) {
        match read_one(&file) {
            Ok(list) => return Ok(list),
            Err(e) => {
                log::warn!("bookmark backup {}: {e}", file.display());
                last = e;
            }
        }
    }
    Err(last)
}

fn bad(file: &Path, why: &str) -> Error {
    Error::Unavailable(format!("{}: {why}", file.display()))
}

fn read_one(file: &Path) -> Result<Tree> {
    let bytes = std::fs::read(file)?;
    let json = if file.extension().is_some_and(|e| e == "jsonlz4") {
        decompress(&bytes).ok_or_else(|| bad(file, "not a mozLz4 file"))?
    } else {
        bytes
    };
    let tree: Value =
        serde_json::from_slice(&json).map_err(|e| bad(file, &format!("JSON: {e}")))?;
    let mut out = Tree::default();
    for child in children(&tree) {
        let root = match child.get("root").and_then(Value::as_str) {
            Some("bookmarksMenuFolder") => BookmarkRoot::Menu,
            Some("toolbarFolder") => BookmarkRoot::Toolbar,
            Some("unfiledBookmarksFolder") => BookmarkRoot::Unfiled,
            Some("mobileFolder") => BookmarkRoot::Mobile,
            _ => continue,
        };
        let guid = text(child, "guid");
        out.folders.push(Folder {
            guid: guid.clone(),
            parent_guid: None,
            root,
            path: Vec::new(),
            title: String::new(),
            position: num(child, "index"),
            added: num(child, "dateAdded") / 1_000_000,
        });
        walk(child, &guid, root, &mut Vec::new(), &mut out, 0);
    }
    if out.folders.is_empty() {
        return Err(bad(file, "no bookmark roots"));
    }
    Ok(super::places::normalize(out))
}

/// The LZ4 block of a `mozLz40\0` file.
fn decompress(bytes: &[u8]) -> Option<Vec<u8>> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let size = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?) as usize;
    if size > MAX_JSON_BYTES {
        return None;
    }
    lz4_flex::block::decompress(&rest[4..], size).ok()
}

fn children(node: &Value) -> impl Iterator<Item = &Value> {
    node.get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn text(node: &Value, key: &str) -> String {
    node.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn num(node: &Value, key: &str) -> i64 {
    node.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn walk(
    node: &Value,
    guid: &str,
    root: BookmarkRoot,
    path: &mut Vec<String>,
    out: &mut Tree,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for child in children(node) {
        let title = text(child, "title");
        match child.get("type").and_then(Value::as_str) {
            Some("text/x-moz-place-container") => {
                let own = text(child, "guid");
                out.folders.push(Folder {
                    guid: own.clone(),
                    parent_guid: Some(guid.to_string()),
                    root,
                    path: path.clone(),
                    title: title.clone(),
                    position: num(child, "index"),
                    added: num(child, "dateAdded") / 1_000_000,
                });
                path.push(title);
                walk(child, &own, root, path, out, depth + 1);
                path.pop();
            }
            Some("text/x-moz-place") => {
                out.bookmarks.push(Bookmark {
                    guid: text(child, "guid"),
                    parent_guid: guid.to_string(),
                    position: num(child, "index"),
                    root,
                    folder: path.clone(),
                    title,
                    url: text(child, "uri"),
                    tags: child
                        .get("tags")
                        .and_then(Value::as_str)
                        .map(|t| {
                            t.split(',')
                                .map(str::trim)
                                .filter(|t| !t.is_empty())
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default(),
                    keyword: child
                        .get("keyword")
                        .and_then(Value::as_str)
                        .map(String::from),
                    added: num(child, "dateAdded") / 1_000_000,
                    modified: num(child, "lastModified") / 1_000_000,
                });
            }
            _ => {}
        }
    }
}
