//! Firefox's daily bookmark backups, `bookmarkbackups/bookmarks-YYYY-MM-DD[_n_hash].jsonlz4`
//! (VFS-6 R4): written atomically by Firefox, so they are read in place. `jsonlz4` is
//! `mozLz40\0`, a little-endian u32 with the decompressed size, then one LZ4 block of JSON.

use std::path::{Path, PathBuf};

use serde_json::Value;
use vfs_core::{Error, Result};

use crate::types::{Bookmark, BookmarkRoot};

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
pub fn read_backup(dir: &Path) -> Result<Vec<Bookmark>> {
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

fn read_one(file: &Path) -> Result<Vec<Bookmark>> {
    let bytes = std::fs::read(file)?;
    let json = if file.extension().is_some_and(|e| e == "jsonlz4") {
        decompress(&bytes).ok_or_else(|| bad(file, "not a mozLz4 file"))?
    } else {
        bytes
    };
    let tree: Value =
        serde_json::from_slice(&json).map_err(|e| bad(file, &format!("JSON: {e}")))?;
    let mut out = Vec::new();
    for child in children(&tree) {
        let root = match child.get("root").and_then(Value::as_str) {
            Some("bookmarksMenuFolder") => BookmarkRoot::Menu,
            Some("toolbarFolder") => BookmarkRoot::Toolbar,
            Some("unfiledBookmarksFolder") => BookmarkRoot::Unfiled,
            Some("mobileFolder") => BookmarkRoot::Mobile,
            _ => continue,
        };
        walk(child, root, &mut Vec::new(), &mut out, 0);
    }
    if out.is_empty() && children(&tree).next().is_none() {
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

fn walk(
    node: &Value,
    root: BookmarkRoot,
    path: &mut Vec<String>,
    out: &mut Vec<Bookmark>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for child in children(node) {
        let title = child.get("title").and_then(Value::as_str).unwrap_or("");
        match child.get("type").and_then(Value::as_str) {
            Some("text/x-moz-place-container") => {
                path.push(title.to_string());
                walk(child, root, path, out, depth + 1);
                path.pop();
            }
            Some("text/x-moz-place") => {
                let micros = |k: &str| child.get(k).and_then(Value::as_i64).unwrap_or(0);
                out.push(Bookmark {
                    guid: child
                        .get("guid")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    root,
                    folder: path.clone(),
                    title: title.to_string(),
                    url: child
                        .get("uri")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
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
                    added: micros("dateAdded") / 1_000_000,
                    modified: micros("lastModified") / 1_000_000,
                });
            }
            _ => {}
        }
    }
}
