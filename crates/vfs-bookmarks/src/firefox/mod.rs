//! Firefox bookmarks (VFS-6): where the profiles are, and reading one profile's bookmarks
//! without ever opening, locking or writing a Firefox file in place (spec "Comparing the ways
//! to read bookmarks while Firefox runs").

mod backup;
mod places;
mod snapshot;

use std::path::{Path, PathBuf};

pub use backup::read_backup;
pub use places::{open_guarded, read_places, ALLOWED_COLUMNS};
pub use snapshot::{copy_consistent, signature};

use crate::types::Bookmark;

/// One Firefox profile folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The name Firefox shows (`profiles.ini` `Name`, or the profile group's `name`).
    pub name: String,
    pub dir: PathBuf,
}

/// Where Firefox keeps its profile list: the folders holding `profiles.ini`.
#[derive(Debug, Clone, Default)]
pub struct Locator {
    pub roots: Vec<PathBuf>,
}

impl Locator {
    /// The Firefox data folders of this system: Windows `%APPDATA%\Mozilla\Firefox` and the
    /// Microsoft Store build's redirected copy of it; macOS
    /// `~/Library/Application Support/Firefox`; elsewhere `~/.mozilla/firefox`.
    pub fn system() -> Locator {
        let mut roots = Vec::new();
        if cfg!(windows) {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                roots.push(Path::new(&appdata).join("Mozilla").join("Firefox"));
            }
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                roots.push(
                    Path::new(&local)
                        .join("Packages")
                        .join("Mozilla.Firefox_n80bbvh6b1yt2")
                        .join("LocalCache")
                        .join("Roaming")
                        .join("Mozilla")
                        .join("Firefox"),
                );
            }
        } else if let Some(home) = std::env::var_os("HOME") {
            let home = Path::new(&home);
            if cfg!(target_os = "macos") {
                roots.push(home.join("Library/Application Support/Firefox"));
            } else {
                roots.push(home.join(".mozilla/firefox"));
            }
        }
        Locator { roots }
    }

    /// Every profile of every root that has a `places.sqlite` or a `bookmarkbackups` folder,
    /// each once: those in `profiles.ini` and, for a profile group (`StoreID`), those in
    /// `Profile Groups/<StoreID>.sqlite`, read from a copy. A root without `profiles.ini`
    /// yields nothing.
    pub fn profiles(&self, work_dir: &Path) -> Vec<Profile> {
        let mut out: Vec<Profile> = Vec::new();
        for root in &self.roots {
            let Ok(ini) = std::fs::read_to_string(root.join("profiles.ini")) else {
                continue;
            };
            for entry in parse_profiles_ini(&ini) {
                let dir = resolve(root, &entry.path, entry.relative);
                push_profile(&mut out, entry.name, dir);
                if let Some(store) = entry.store_id {
                    let db = root.join("Profile Groups").join(format!("{store}.sqlite"));
                    match group_profiles(&db, work_dir) {
                        Ok(list) => {
                            for (name, path) in list {
                                push_profile(&mut out, name, resolve(root, &path, true));
                            }
                        }
                        Err(e) => log::warn!("profile group {}: {e}", db.display()),
                    }
                }
            }
        }
        out
    }
}

fn push_profile(out: &mut Vec<Profile>, name: String, dir: PathBuf) {
    let has_data = dir.join("places.sqlite").is_file() || dir.join("bookmarkbackups").is_dir();
    if has_data && !out.iter().any(|p| same_dir(&p.dir, &dir)) {
        out.push(Profile { name, dir });
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// A relative profile path uses `/` in `profiles.ini` on every system.
fn resolve(root: &Path, path: &str, relative: bool) -> PathBuf {
    if relative {
        path.split(['/', '\\'])
            .filter(|c| !c.is_empty())
            .fold(root.to_path_buf(), |p, c| p.join(c))
    } else {
        PathBuf::from(path)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct IniProfile {
    name: String,
    path: String,
    relative: bool,
    store_id: Option<String>,
}

/// The `[ProfileN]` sections of `profiles.ini` that have a `Path`.
fn parse_profiles_ini(text: &str) -> Vec<IniProfile> {
    let mut out = Vec::new();
    let mut current: Option<IniProfile> = None;
    let flush = |cur: &mut Option<IniProfile>, out: &mut Vec<IniProfile>| {
        if let Some(p) = cur.take().filter(|p| !p.path.is_empty()) {
            out.push(p);
        }
    };
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            flush(&mut current, &mut out);
            if section.starts_with("Profile") {
                current = Some(IniProfile {
                    name: String::new(),
                    path: String::new(),
                    relative: true,
                    store_id: None,
                });
            }
        } else if let (Some(p), Some((k, v))) = (current.as_mut(), line.split_once('=')) {
            let v = v.trim().to_string();
            match k.trim() {
                "Name" => p.name = v,
                "Path" => p.path = v,
                "IsRelative" => p.relative = v != "0",
                "StoreID" if !v.is_empty() => p.store_id = Some(v),
                _ => {}
            }
        }
    }
    flush(&mut current, &mut out);
    out
}

/// `(name, relative path)` of every profile in a profile group database, read from a copy.
fn group_profiles(db: &Path, work_dir: &Path) -> vfs_core::Result<Vec<(String, String)>> {
    if !db.is_file() {
        return Ok(Vec::new());
    }
    let copy = copy_consistent(db, &work_dir.join("group"))?;
    let conn = rusqlite::Connection::open(&copy.path)?;
    let mut stmt = conn.prepare("SELECT name, path FROM Profiles")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<(String, String)>, _>>()?;
    Ok(rows)
}

/// How a profile was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Database,
    Backup,
}

/// Reads one profile: a consistent copy of `places.sqlite` and its WAL (R3), or else the
/// newest readable bookmark backup (R4).
pub fn read_profile(dir: &Path, work_dir: &Path) -> vfs_core::Result<(Vec<Bookmark>, Origin)> {
    let db = dir.join("places.sqlite");
    let live = if db.is_file() {
        copy_consistent(&db, work_dir).and_then(|copy| read_places(&copy.path))
    } else {
        Err(vfs_core::Error::NotFound(db.to_string_lossy().into_owned()))
    };
    match live {
        Ok(list) => Ok((list, Origin::Database)),
        Err(e) => {
            log::warn!("places.sqlite of {}: {e}", dir.display());
            read_backup(&dir.join("bookmarkbackups")).map(|list| (list, Origin::Backup))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-6 AC-4: `profiles.ini` sections with their keys; install and general sections and
    /// sections without a path are not profiles.
    #[test]
    fn parses_profiles_ini() {
        let ini = "\u{feff}[Profile1]\nName=default\nIsRelative=1\nPath=Profiles/a.default\nDefault=1\n\n\
                   [Profile0]\nName=Trabajo\nIsRelative=0\nPath=D:\\Perfiles\\t\nStoreID=abc\n\n\
                   [General]\nStartWithLastProfile=1\n\n[Install2656FF1E876E9973]\nDefault=Profiles/a.default\n\n\
                   [Profile2]\nName=broken\n";
        let got = parse_profiles_ini(ini);
        assert_eq!(
            got,
            vec![
                IniProfile {
                    name: "default".into(),
                    path: "Profiles/a.default".into(),
                    relative: true,
                    store_id: None
                },
                IniProfile {
                    name: "Trabajo".into(),
                    path: "D:\\Perfiles\\t".into(),
                    relative: false,
                    store_id: Some("abc".into())
                },
            ]
        );
        assert_eq!(
            resolve(Path::new("/r"), "Profiles/a.default", true),
            Path::new("/r").join("Profiles").join("a.default")
        );
    }
}
