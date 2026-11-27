//! Chrome and Brave bookmarks (VFS-44): where the profiles are, and reading one profile's
//! `Bookmarks` and `AccountBookmarks` JSON files. The browser replaces these files by
//! writing a temporary file and renaming it, never in place, so they are read directly,
//! with Rust's default sharing (on Windows read, write and delete), and nothing is written
//! or created in the browser's folders (R2, D4).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;
use vfs_core::{Error, Result};

use crate::firefox::{Origin, Profile};
use crate::types::{Bookmark, BookmarkRoot, Browser, Folder, Tree};

/// The local bookmarks, the account's (signed in without sync), and the browser's previous
/// version of the first.
const BOOKMARKS: &str = "Bookmarks";
const ACCOUNT: &str = "AccountBookmarks";
const BACKUP: &str = "Bookmarks.bak";
const LOCAL_STATE: &str = "Local State";
/// Read attempts of a file that does not parse or changes while read (R3).
const READ_ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(500);
/// A file larger than this is not read: 1,000 bookmarks take about 1 MB.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Nesting deeper than this is not followed.
const MAX_DEPTH: usize = 128;
/// Seconds from 1601-01-01 (Chromium's epoch) to 1970-01-01.
const EPOCH_GAP_SECS: i64 = 11_644_473_600;
/// Chromium's roots in the order it shows them, with the root each maps to (R5).
const ROOTS: [(&str, BookmarkRoot); 3] = [
    ("bookmark_bar", BookmarkRoot::Toolbar),
    ("other", BookmarkRoot::Unfiled),
    ("synced", BookmarkRoot::Mobile),
];

/// One Chromium browser's data folder (the one holding `Local State`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locator {
    pub browser: Browser,
    pub root: PathBuf,
}

impl Locator {
    /// Chrome's and Brave's data folders on this system: Windows under `%LOCALAPPDATA%`,
    /// macOS under `~/Library/Application Support`, elsewhere under `~/.config`.
    pub fn system() -> Vec<Locator> {
        let base = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("HOME").map(|h| {
                let home = PathBuf::from(h);
                if cfg!(target_os = "macos") {
                    home.join("Library").join("Application Support")
                } else {
                    home.join(".config")
                }
            })
        };
        let Some(base) = base else {
            return Vec::new();
        };
        let (chrome, brave): (&[&str], &[&str]) = if cfg!(windows) {
            (
                &["Google", "Chrome", "User Data"],
                &["BraveSoftware", "Brave-Browser", "User Data"],
            )
        } else if cfg!(target_os = "macos") {
            (&["Google", "Chrome"], &["BraveSoftware", "Brave-Browser"])
        } else {
            (&["google-chrome"], &["BraveSoftware", "Brave-Browser"])
        };
        let join = |parts: &[&str]| parts.iter().fold(base.clone(), |p, c| p.join(c));
        vec![
            Locator {
                browser: Browser::Chrome,
                root: join(chrome),
            },
            Locator {
                browser: Browser::Brave,
                root: join(brave),
            },
        ]
    }

    /// The browser is installed (or was): its data folder exists (R1).
    pub fn installed(&self) -> bool {
        self.root.is_dir()
    }

    /// Every profile `Local State` lists that has a bookmarks file, with the name the
    /// browser shows; without a readable `Local State`, `Default` and `Profile *` (R2).
    pub fn profiles(&self) -> Vec<Profile> {
        let listed = std::fs::read(self.root.join(LOCAL_STATE))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| {
                let cache = v.get("profile")?.get("info_cache")?.as_object()?;
                Some(
                    cache
                        .iter()
                        .map(|(dir, info)| {
                            let name = info.get("name").and_then(Value::as_str).unwrap_or(dir);
                            (dir.clone(), name.to_string())
                        })
                        .collect::<Vec<_>>(),
                )
            });
        let candidates = listed.unwrap_or_else(|| {
            std::fs::read_dir(&self.root)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|n| n == "Default" || n.starts_with("Profile "))
                .map(|n| (n.clone(), n))
                .collect()
        });
        let mut out: Vec<Profile> = candidates
            .into_iter()
            // A key naming a folder outside the data folder is not followed.
            .filter(|(dir, _)| !dir.contains(['/', '\\']) && dir != ".." && dir != ".")
            .map(|(dir, name)| Profile {
                name,
                dir: self.root.join(dir),
            })
            .filter(|p| {
                [BOOKMARKS, ACCOUNT, BACKUP]
                    .iter()
                    .any(|f| p.dir.join(f).is_file())
            })
            .collect();
        out.sort_by(|a, b| a.dir.cmp(&b.dir));
        out
    }
}

/// Size and modification time of a file; `None` when it does not exist.
fn stamp(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

/// What changes when `Bookmarks` or `AccountBookmarks` changes: compared with the stored
/// value so an unchanged profile is not read again (R4).
pub fn signature(profile_dir: &Path) -> String {
    [BOOKMARKS, ACCOUNT]
        .iter()
        .map(|f| match stamp(&profile_dir.join(f)) {
            None => "-".to_string(),
            Some((len, t)) => {
                let ns = t
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                format!("{len}@{ns}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reads one profile: `Bookmarks` (retried while it does not parse or changes) and
/// `AccountBookmarks`, or else `Bookmarks.bak` (R3).
pub fn read_profile(dir: &Path) -> Result<(Tree, Origin)> {
    let main = dir.join(BOOKMARKS);
    let mut tree = Tree::default();
    let origin = match read_stable(&main) {
        Ok(json) => {
            add_file(&json, "", &mut tree);
            Origin::Database
        }
        Err(e) => {
            log::warn!("{}: {e}", main.display());
            let backup = read_stable(&dir.join(BACKUP));
            match backup {
                Ok(json) => {
                    add_file(&json, "", &mut tree);
                    Origin::Backup
                }
                // A profile kept only in the account still has its account bookmarks.
                Err(_) if !main.exists() && dir.join(ACCOUNT).is_file() => Origin::Database,
                Err(_) => return Err(e),
            }
        }
    };
    let account = dir.join(ACCOUNT);
    if account.is_file() {
        match read_stable(&account) {
            Ok(json) => add_file(&json, "account:", &mut tree),
            Err(e) if tree.folders.is_empty() => return Err(e),
            Err(e) => log::warn!("{}: {e}", account.display()),
        }
    }
    Ok((crate::firefox::normalize(tree), origin))
}

/// The parsed file, once a read gave JSON with `roots` and the file's size and time were the
/// same before and after it.
fn read_stable(path: &Path) -> Result<Value> {
    let mut last = Error::NotFound(path.to_string_lossy().into_owned());
    for attempt in 0..READ_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(RETRY_DELAY);
        }
        let before = stamp(path);
        match before {
            None => return Err(Error::NotFound(path.to_string_lossy().into_owned())),
            Some((len, _)) if len > MAX_FILE_BYTES => {
                return Err(Error::Unavailable(format!(
                    "{} is too large",
                    path.display()
                )))
            }
            _ => {}
        }
        let parsed = std::fs::read(path)
            .map_err(Error::from)
            .and_then(|b| {
                serde_json::from_slice::<Value>(&b)
                    .map_err(|e| Error::Unavailable(format!("{}: JSON: {e}", path.display())))
            })
            .and_then(|v| {
                if v.get("roots").is_some_and(Value::is_object) {
                    Ok(v)
                } else {
                    Err(Error::Unavailable(format!("{}: no roots", path.display())))
                }
            });
        match parsed {
            Ok(v) if stamp(path) == before => return Ok(v),
            Ok(_) => last = Error::Unavailable(format!("{} kept changing", path.display())),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Chromium's `date_added`: microseconds since 1601-01-01, as a string; 0 when missing.
fn date(node: &Value) -> i64 {
    node.get("date_added")
        .and_then(|v| {
            v.as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or(v.as_i64())
        })
        .filter(|us| *us > 0)
        .map_or(0, |us| us / 1_000_000 - EPOCH_GAP_SECS)
}

fn text(node: &Value, key: &str) -> String {
    node.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn children(node: &Value) -> impl Iterator<Item = &Value> {
    node.get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// The roots of one file into `tree`. The account file's roots are the same folders on
/// screen: its items go under the local root of the same kind, after the local ones.
fn add_file(json: &Value, guid_prefix: &str, tree: &mut Tree) {
    for (pos, (key, root)) in ROOTS.iter().enumerate() {
        let Some(node) = json.get("roots").and_then(|r| r.get(key)) else {
            continue;
        };
        let root_guid = format!("root:{key}");
        let first = if let Some(r) = tree.folders.iter().find(|f| f.guid == root_guid) {
            let parent = r.guid.clone();
            tree.folders
                .iter()
                .filter(|f| f.parent_guid.as_ref() == Some(&parent))
                .count()
                + tree
                    .bookmarks
                    .iter()
                    .filter(|b| b.parent_guid == parent)
                    .count()
        } else {
            tree.folders.push(Folder {
                guid: root_guid.clone(),
                parent_guid: None,
                root: *root,
                path: Vec::new(),
                title: String::new(),
                position: pos as i64,
                added: date(node),
            });
            0
        };
        let mut walker = Walker {
            prefix: guid_prefix,
            root: *root,
            out: tree,
        };
        walker.walk(node, &root_guid, &mut Vec::new(), first as i64, 0);
    }
}

struct Walker<'a> {
    prefix: &'a str,
    root: BookmarkRoot,
    out: &'a mut Tree,
}

impl Walker<'_> {
    fn guid(&self, node: &Value) -> String {
        let g = text(node, "guid");
        if g.is_empty() {
            String::new()
        } else {
            format!("{}{g}", self.prefix)
        }
    }

    fn walk(
        &mut self,
        node: &Value,
        parent: &str,
        path: &mut Vec<String>,
        first: i64,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        for (i, child) in children(node).enumerate() {
            let position = first + i as i64;
            let title = text(child, "name");
            match child.get("type").and_then(Value::as_str) {
                Some("folder") => {
                    let mut own = self.guid(child);
                    if own.is_empty() {
                        own = format!("{parent}/{position}");
                    }
                    self.out.folders.push(Folder {
                        guid: own.clone(),
                        parent_guid: Some(parent.to_string()),
                        root: self.root,
                        path: path.clone(),
                        title: title.clone(),
                        position,
                        added: date(child),
                    });
                    path.push(title);
                    self.walk(child, &own, path, 0, depth + 1);
                    path.pop();
                }
                Some("url") => {
                    self.out.bookmarks.push(Bookmark {
                        guid: self.guid(child),
                        parent_guid: parent.to_string(),
                        position,
                        root: self.root,
                        folder: path.clone(),
                        title,
                        url: text(child, "url"),
                        tags: Vec::new(),
                        keyword: None,
                        added: date(child),
                        modified: date(child),
                    });
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-44 R5: `date_added` is microseconds since 1601; 2021-01-01 is 1609459200.
    #[test]
    fn converts_chromium_dates() {
        let node = serde_json::json!({ "date_added": "13253932800000000" });
        assert_eq!(date(&node), 1_609_459_200);
        assert_eq!(date(&serde_json::json!({})), 0);
    }
}
