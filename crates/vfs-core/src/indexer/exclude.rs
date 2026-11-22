//! Default folder exclusions (VFS-33 part 2): system, app-data, development and program junk
//! folders are not indexed below a root unless the user turns on «Incluir carpetas del
//! sistema y de programas». The list is data, [`LIST`], with its sources in
//! `specs/vfs-33-bigroot/spec.md`. A folder the user adds itself as a root is indexed in
//! full: no rule applies to the root or to a tree that contains it.
//!
//! Matching is cheap: per listing, one map lookup for the listed folder, and per subfolder,
//! a scan of a short list of names. Markers (a file such as `CACHEDIR.TAG` inside a folder)
//! cost nothing: the listing already iterates the folder's entries.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension};

use super::patterns::FilePatterns;
use crate::error::Result;

/// Version of [`LIST`]. Each root records the version and setting of its last full walk; a
/// root walked under others owes a full walk, so newly excluded folders leave the index and
/// newly included ones enter it (VFS-33 R8).
pub(crate) const VERSION: i64 = 1;
/// `meta` key of the setting «Incluir carpetas del sistema y de programas» ("1" when on).
const INCLUDE_KEY: &str = "include_all_folders";

/// Whether the user includes the excluded folders (off unless set).
pub(crate) fn include_all(conn: &Connection) -> Result<bool> {
    Ok(meta(conn, INCLUDE_KEY)?.as_deref() == Some("1"))
}

pub(crate) fn set_include_all(conn: &Connection, on: bool) -> Result<()> {
    set_meta(conn, INCLUDE_KEY, if on { "1" } else { "0" })
}

fn stamp_key(root_id: i64) -> String {
    format!("exclusions/{root_id}")
}

/// `VERSION:include:gen`: the list version, the switch, and the generation of the user's
/// file filter (VFS-34 R5).
fn stamp_value(conn: &Connection) -> Result<String> {
    Ok(format!(
        "{VERSION}:{}:{}",
        u8::from(include_all(conn)?),
        super::patterns::generation(conn)?
    ))
}

/// Whether a stored stamp is `want`. A stamp from before VFS-34, `VERSION:include`, reads as
/// generation 0, so upgrading leaves every root current (VFS-34 R5).
fn current(stored: &str, want: &str) -> bool {
    stored == want || want.strip_suffix(":0") == Some(stored)
}

fn walk_key(root_id: i64) -> String {
    format!("exclusions/{root_id}/walk")
}

/// Records the list version and setting a full walk of `root_id` starts under.
pub(crate) fn begin_walk(conn: &Connection, root_id: i64) -> Result<()> {
    let value = stamp_value(conn)?;
    set_meta(conn, &walk_key(root_id), &value)
}

/// Records that `root_id`'s full walk just ended, under the version and setting it started
/// with: a walk the switch changed halfway is stale, and owes another (VFS-33 R8).
pub(crate) fn stamp(conn: &Connection, root_id: i64) -> Result<()> {
    let value = match meta(conn, &walk_key(root_id))? {
        Some(v) => v,
        None => stamp_value(conn)?,
    };
    set_meta(conn, &stamp_key(root_id), &value)
}

/// Forgets `root_id`'s records (its root is removed).
pub(crate) fn forget(conn: &Connection, root_id: i64) -> Result<()> {
    let mut stmt = conn.prepare_cached("DELETE FROM meta WHERE key = ?1")?;
    stmt.execute([stamp_key(root_id)])?;
    stmt.execute([walk_key(root_id)])?;
    Ok(())
}

/// Roots that finished a full walk under another list version, setting or file-filter
/// generation than now.
pub(crate) fn stale_roots(conn: &Connection) -> Result<Vec<i64>> {
    let want = stamp_value(conn)?;
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM roots WHERE last_full_scan_at IS NOT NULL ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::new();
    for id in ids {
        if !meta(conn, &stamp_key(id))?.is_some_and(|s| current(&s, &want)) {
            out.push(id);
        }
    }
    Ok(out)
}

fn meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT value FROM meta WHERE key = ?1")?
        .query_row([key], |r| r.get(0))
        .optional()?)
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.prepare_cached("INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)")?
        .execute([key, value])?;
    Ok(())
}

/// The user's home folder in canonical form, as root paths are stored.
pub(crate) fn home() -> Option<PathBuf> {
    let home = std::env::home_dir()?;
    Some(dunce::canonicalize(&home).unwrap_or(home))
}

/// What an entry is about, for the spec and the settings text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Category {
    System,
    AppData,
    Development,
    Programs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Os {
    Both,
    Mac,
    Windows,
}

impl Os {
    fn here(self) -> bool {
        match self {
            Os::Both => true,
            Os::Mac => cfg!(target_os = "macos"),
            Os::Windows => cfg!(windows),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Rule {
    /// A folder with this name anywhere below a root (compared ignoring ASCII case).
    Name(&'static str),
    /// A folder at this `/`-separated path below the user's home folder.
    Home(&'static str),
    /// Every child folder of this home path except the named ones (`~/Library` keeps iCloud
    /// Drive and the cloud-storage folders).
    HomeChildrenExcept(&'static str, &'static [&'static str]),
    /// A folder at this `/`-separated path below the root of its volume (`C:\`, `/`).
    Volume(&'static str),
    /// A folder holding an entry with this name: the folder is indexed, its contents are not.
    /// Only the name counts; the file is never opened (VFS-1: metadata only).
    Marker(&'static str),
    /// A folder holding an entry with this name: these subfolders of it are not indexed (a
    /// Python virtual environment made in a project folder keeps the project's own files).
    MarkerChildren(&'static str, &'static [&'static str]),
}

pub(crate) struct Entry {
    /// Groups the table for the spec and the settings text; only tests read it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub category: Category,
    pub os: Os,
    pub rule: Rule,
}

const fn e(category: Category, os: Os, rule: Rule) -> Entry {
    Entry { category, os, rule }
}

use Category::{AppData, Development, Programs, System};
use Os::{Both, Mac, Windows};
use Rule::{Home, HomeChildrenExcept, Marker, MarkerChildren, Name, Volume};

/// The default exclusions. Sources per entry: `specs/vfs-33-bigroot/spec.md`, part 2.
pub(crate) static LIST: &[Entry] = &[
    // System: what a volume root holds besides the users' folders.
    e(System, Windows, Volume("Windows")),
    e(System, Windows, Volume("Windows.old")),
    e(System, Windows, Volume("Program Files")),
    e(System, Windows, Volume("Program Files (x86)")),
    e(System, Windows, Volume("ProgramData")),
    e(System, Windows, Volume("Users/Default")),
    e(System, Windows, Volume("Recovery")),
    e(System, Windows, Volume("PerfLogs")),
    e(System, Windows, Volume("MSOCache")),
    e(System, Windows, Volume("$WinREAgent")),
    e(System, Windows, Volume("$Windows.~BT")),
    e(System, Windows, Volume("$Windows.~WS")),
    e(System, Mac, Volume("System")),
    e(System, Mac, Volume("Library")),
    e(System, Mac, Volume("private")),
    e(System, Mac, Volume("usr")),
    e(System, Mac, Volume("bin")),
    e(System, Mac, Volume("sbin")),
    e(System, Mac, Volume("cores")),
    e(System, Mac, Volume("Network")),
    e(System, Mac, Name(".MobileBackups")),
    // App data: settings, caches and state of programs in the user's home.
    e(
        AppData,
        Mac,
        HomeChildrenExcept("Library", &["Mobile Documents", "CloudStorage"]),
    ),
    e(AppData, Windows, Home("AppData")),
    e(AppData, Both, Home(".cache")),
    e(AppData, Both, Home(".local/share/Trash")),
    // Development: dependencies, virtual environments, build output, repository data.
    e(Development, Both, Marker("CACHEDIR.TAG")),
    e(
        Development,
        Both,
        MarkerChildren(
            "pyvenv.cfg",
            &["lib", "Lib", "bin", "Scripts", "include", "Include"],
        ),
    ),
    e(Development, Both, Name("node_modules")),
    e(Development, Both, Name("bower_components")),
    e(Development, Both, Name("__pycache__")),
    e(Development, Both, Name(".git")),
    e(Development, Both, Name(".hg")),
    e(Development, Both, Name(".svn")),
    e(Development, Both, Name(".tox")),
    e(Development, Both, Home(".npm")),
    e(Development, Both, Home(".cargo")),
    e(Development, Both, Home(".rustup")),
    e(Development, Both, Home(".gradle")),
    e(Development, Both, Home(".m2")),
    e(Development, Both, Home(".nuget")),
    e(Development, Both, Home(".conda")),
    e(Development, Both, Home("go/pkg/mod")),
    e(Development, Mac, Volume("opt/homebrew")),
    // Other programs: game libraries and sync clients' internal state.
    e(Programs, Both, Name("steamapps")),
    e(Programs, Both, Name(".dropbox.cache")),
];

/// One rule for the subfolders of a folder, by that folder's (folded) path.
enum ChildRule {
    /// These child names are skipped.
    Skip(Vec<&'static str>),
    /// Every child but these names is skipped.
    KeepOnly(&'static [&'static str]),
}

/// The exclusions in force below one root.
pub(crate) struct Exclusions {
    names: Vec<&'static str>,
    markers: Vec<&'static str>,
    marker_children: Vec<(&'static str, &'static [&'static str])>,
    by_parent: HashMap<String, ChildRule>,
    root: PathBuf,
    /// The user's file filter (VFS-34 R3), which applies whether or not the folder rules do.
    files: Arc<FilePatterns>,
}

/// The key a folder path is compared by: as is, or on Windows lowercased with `/` read as
/// `\\`, without a trailing separator.
fn key(path: &Path) -> String {
    let s = path.to_string_lossy();
    let s = s.trim_end_matches(['/', '\\']);
    if cfg!(windows) {
        s.to_lowercase().replace('/', "\\")
    } else {
        s.to_owned()
    }
}

fn join(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    p.extend(rel.split('/'));
    p
}

/// The root of `path`'s volume: `C:\` or `\\server\share\` on Windows, `/` elsewhere.
fn volume_of(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
            _ => break,
        }
    }
    out
}

impl Exclusions {
    /// No exclusion: the user asked to include these folders.
    pub(crate) fn none(root: &Path) -> Exclusions {
        Exclusions {
            names: Vec::new(),
            markers: Vec::new(),
            marker_children: Vec::new(),
            by_parent: HashMap::new(),
            root: root.to_path_buf(),
            files: Arc::default(),
        }
    }

    /// The same rules, with the user's file filter (VFS-34 R3).
    pub(crate) fn with_files(mut self, files: Arc<FilePatterns>) -> Exclusions {
        self.files = files;
        self
    }

    /// The rules of [`LIST`] for this system that apply below `root`. A path rule whose folder
    /// is the root or contains it is left out, so a folder added as a root is indexed in full.
    pub(crate) fn for_root(root: &Path, home: Option<&Path>) -> Exclusions {
        Self::from_list(LIST.iter().filter(|e| e.os.here()), root, home)
    }

    fn from_list<'a>(
        list: impl Iterator<Item = &'a Entry>,
        root: &Path,
        home: Option<&Path>,
    ) -> Exclusions {
        let mut out = Exclusions::none(root);
        let root_key = key(root);
        // Below the root, and not the root nor one of its ancestors.
        let below_root = |p: &Path| {
            let k = key(p);
            k.len() > root_key.len() && Path::new(&k).starts_with(&root_key)
        };
        let volume = volume_of(root);
        for entry in list {
            let (base, rel, keep) = match entry.rule {
                Rule::Name(n) => {
                    out.names.push(n);
                    continue;
                }
                Rule::Marker(m) => {
                    out.markers.push(m);
                    continue;
                }
                Rule::MarkerChildren(m, children) => {
                    out.marker_children.push((m, children));
                    continue;
                }
                Rule::Home(rel) => match home {
                    Some(h) => (h.to_path_buf(), rel, None),
                    None => continue,
                },
                Rule::HomeChildrenExcept(rel, keep) => match home {
                    Some(h) => (h.to_path_buf(), rel, Some(keep)),
                    None => continue,
                },
                Rule::Volume(rel) => (volume.clone(), rel, None),
            };
            let target = join(&base, rel);
            if !below_root(&target) {
                continue;
            }
            if let Some(keep) = keep {
                out.by_parent
                    .insert(key(&target), ChildRule::KeepOnly(keep));
                continue;
            }
            let (parent, name) = match rel.rsplit_once('/') {
                Some((up, name)) => (join(&base, up), name),
                None => (base, rel),
            };
            if let ChildRule::Skip(names) = out
                .by_parent
                .entry(key(&parent))
                .or_insert_with(|| ChildRule::Skip(Vec::new()))
            {
                names.push(name);
            }
        }
        out
    }

    /// The rule for the subfolders of `dir`, looked up once per listing.
    pub(crate) fn children_of(&self, dir: &Path) -> Children<'_> {
        Children {
            excl: self,
            rule: if self.by_parent.is_empty() {
                None
            } else {
                self.by_parent.get(&key(dir))
            },
            markers: dir != self.root,
        }
    }

    /// Whether a watcher event or replayed change at `path` (below the root) lies in an
    /// excluded folder.
    pub(crate) fn drops(&self, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return false;
        };
        let mut dir = self.root.clone();
        for c in rel.components() {
            let name = c.as_os_str().to_string_lossy();
            if self.children_of(&dir).skips_dir(&name) {
                return true;
            }
            dir.push(c.as_os_str());
        }
        false
    }
}

/// The exclusions for the entries of one folder being listed.
pub(crate) struct Children<'a> {
    excl: &'a Exclusions,
    rule: Option<&'a ChildRule>,
    /// Markers apply below the root only: a marked folder added as a root is indexed.
    markers: bool,
}

impl Children<'_> {
    /// Whether the subfolder `name` is skipped.
    pub(crate) fn skips_dir(&self, name: &str) -> bool {
        let eq = |n: &&str| n.eq_ignore_ascii_case(name);
        self.excl.names.iter().any(eq)
            || match self.rule {
                Some(ChildRule::Skip(names)) => names.iter().any(eq),
                Some(ChildRule::KeepOnly(keep)) => !keep.iter().any(eq),
                None => false,
            }
    }

    /// Whether the file `name` matches the user's file filter (VFS-34 R3). Called for files
    /// only: a pattern never matches a folder.
    pub(crate) fn skips_file(&self, name: &str) -> bool {
        self.excl.files.matches(name)
    }

    /// Whether the entry `name` marks the folder's contents as not indexed.
    pub(crate) fn is_marker(&self, name: &str) -> bool {
        self.markers && self.excl.markers.contains(&name)
    }

    /// The subfolders that the entry `name` marks as not indexed, if it is such a marker.
    pub(crate) fn marked_children(&self, name: &str) -> Option<&'static [&'static str]> {
        if !self.markers {
            return None;
        }
        self.excl
            .marker_children
            .iter()
            .find(|(m, _)| *m == name)
            .map(|(_, c)| *c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> impl Iterator<Item = &'static Entry> {
        LIST.iter()
    }

    /// VFS-33 AC-6: names, home paths with kept children, volume paths and markers, on any
    /// platform's list.
    #[test]
    fn rules_below_a_home_root() {
        let home = Path::new("/Users/ana");
        let x = Exclusions::from_list(all(), home, Some(home));
        let at = |dir: &str| x.children_of(Path::new(dir));
        assert!(at("/Users/ana/Fotos/web").skips_dir("node_modules"));
        assert!(at("/Users/ana/Fotos/web").skips_dir("Node_Modules"));
        assert!(!at("/Users/ana/Fotos/web").skips_dir("node_modules_old"));
        assert!(at("/Users/ana").skips_dir("AppData"));
        assert!(at("/Users/ana").skips_dir(".cache"));
        assert!(
            !at("/Users/ana").skips_dir("Library"),
            "listed, children filtered"
        );
        assert!(at("/Users/ana/Library").skips_dir("Caches"));
        assert!(at("/Users/ana/Library").skips_dir("Application Support"));
        assert!(!at("/Users/ana/Library").skips_dir("Mobile Documents"));
        assert!(!at("/Users/ana/Library").skips_dir("CloudStorage"));
        assert!(at("/Users/ana/go/pkg").skips_dir("mod"));
        assert!(!at("/Users/ana/go").skips_dir("src"));
        assert!(!at("/Users/ana/go").skips_dir("pkg"));
        assert!(!at("/Users/ana/Documentos").skips_dir("Library"));
        assert!(at("/Users/ana/web").is_marker("CACHEDIR.TAG"));
        assert!(
            !at("/Users/ana").is_marker("CACHEDIR.TAG"),
            "the root itself"
        );
        assert!(
            !at("/Users/ana/web").is_marker("cachedir.tag"),
            "exact name"
        );
        let venv = at("/Users/ana/proyecto")
            .marked_children("pyvenv.cfg")
            .unwrap();
        assert!(venv.contains(&"lib") && venv.contains(&"Scripts"));
        assert!(
            at("/Users/ana").marked_children("pyvenv.cfg").is_none(),
            "the root"
        );
        // Volume rules do not reach a root below the volume's own folders.
        assert!(!at("/Users/ana").skips_dir("System"));
    }

    /// VFS-33 AC-7: a folder added as a root, or a root inside an excluded folder, is indexed
    /// in full; name rules and markers still apply below it.
    #[test]
    fn a_root_opts_in() {
        let home = Path::new("/Users/ana");
        let lib = Path::new("/Users/ana/Library");
        let x = Exclusions::from_list(all(), lib, Some(home));
        assert!(
            !x.children_of(lib).skips_dir("Caches"),
            "the root is ~/Library"
        );
        let caches = Path::new("/Users/ana/Library/Caches");
        let x = Exclusions::from_list(all(), caches, Some(home));
        assert!(!x.children_of(caches).skips_dir("com.apple.Safari"));
        let appdata = Path::new("/Users/ana/AppData");
        let x = Exclusions::from_list(all(), appdata, Some(home));
        assert!(!x.children_of(appdata).skips_dir("Local"));
        assert!(x.children_of(appdata).skips_dir("node_modules"));
        let nm = Path::new("/Users/ana/web/node_modules");
        let x = Exclusions::from_list(all(), nm, Some(home));
        assert!(!x.children_of(nm).skips_dir("left-pad"));
        assert!(!x.children_of(nm).is_marker("CACHEDIR.TAG"));
    }

    /// VFS-33 AC-6: a volume root gets the system folders of the volume.
    #[test]
    fn rules_below_a_volume_root() {
        let x = Exclusions::from_list(all(), Path::new("/"), None);
        assert!(x.children_of(Path::new("/")).skips_dir("System"));
        assert!(x.children_of(Path::new("/")).skips_dir("private"));
        assert!(!x.children_of(Path::new("/")).skips_dir("Users"));
        assert!(!x.children_of(Path::new("/")).skips_dir("Volumes"));
        assert!(!x.children_of(Path::new("/Users")).skips_dir("System"));
    }

    /// VFS-33 AC-6: events and replayed changes under an excluded folder are dropped.
    #[test]
    fn drops_paths_in_excluded_folders() {
        let home = Path::new("/Users/ana");
        let x = Exclusions::from_list(all(), home, Some(home));
        assert!(x.drops(Path::new("/Users/ana/Library/Caches/x/y.db")));
        assert!(x.drops(Path::new("/Users/ana/web/node_modules/a/b.js")));
        assert!(x.drops(Path::new("/Users/ana/AppData/Local/Temp/z.tmp")));
        assert!(!x.drops(Path::new("/Users/ana/Library/Mobile Documents/doc.pages")));
        assert!(!x.drops(Path::new("/Users/ana/Documentos/carta.docx")));
        assert!(!x.drops(Path::new("/Users/ana/Library")));
        assert!(!Exclusions::none(home).drops(Path::new("/Users/ana/web/node_modules/a")));
    }

    /// VFS-33 AC-6 (Windows): paths compare ignoring case, and a drive root gets its system
    /// folders.
    #[cfg(windows)]
    #[test]
    fn windows_paths_ignore_case() {
        let home = Path::new(r"C:\Users\Ana");
        let x = Exclusions::for_root(Path::new(r"c:\users\ana"), Some(home));
        assert!(x
            .children_of(Path::new(r"C:\USERS\ANA"))
            .skips_dir("appdata"));
        assert!(x.drops(Path::new(r"c:\users\ana\AppData\Local\Temp\a.tmp")));
        let c = Exclusions::for_root(Path::new(r"C:\"), Some(home));
        assert!(c.children_of(Path::new(r"C:\")).skips_dir("Windows"));
        assert!(c
            .children_of(Path::new(r"C:\"))
            .skips_dir("program files (x86)"));
        assert!(!c.children_of(Path::new(r"C:\")).skips_dir("Users"));
        assert!(c
            .children_of(Path::new(r"C:\Users\Ana"))
            .skips_dir("AppData"));
    }

    /// VFS-33 AC-6: every entry is well formed: no separators in names or markers, no empty
    /// or absolute path, and each category is used.
    #[test]
    fn list_is_well_formed() {
        for entry in LIST {
            match entry.rule {
                Rule::Name(s) | Rule::Marker(s) | Rule::MarkerChildren(s, _) => {
                    assert!(!s.is_empty() && !s.contains(['/', '\\']), "{s}")
                }
                Rule::Home(s) | Rule::Volume(s) | Rule::HomeChildrenExcept(s, _) => {
                    assert!(
                        !s.is_empty() && !s.starts_with('/') && !s.contains('\\'),
                        "{s}"
                    )
                }
            }
        }
        for c in [System, AppData, Development, Programs] {
            assert!(LIST.iter().any(|e| e.category == c), "{c:?}");
        }
    }
}
