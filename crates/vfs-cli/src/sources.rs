//! `vfs search --source` and `vfs bookmarks` (VFS-6, VFS-44): browser bookmarks as a second
//! source, stored in `bookmarks.db` next to the index given with `--db`.

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use vfs_bookmarks::firefox::Locator;
use vfs_bookmarks::sources::search_sources;
use vfs_bookmarks::{
    bookmarks_db_path, Bookmarks, Browser, Locators, SortOrder, Source, SourceHit, SourceOptions,
};
use vfs_core::{SearchQuery, Vfs};

use crate::gen::{civil_from_unix, AnyResult};

/// `--source` of `vfs search`.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SourceArg {
    Files,
    Bookmarks,
    All,
}

impl SourceArg {
    /// The sources searched; an empty list is every source.
    fn sources(self) -> Vec<Source> {
        match self {
            SourceArg::Files => vec![Source::Files],
            SourceArg::Bookmarks => vec![Source::Bookmarks],
            SourceArg::All => Vec::new(),
        }
    }
}

/// `--sort` of `vfs search` and `vfs bench` (VFS-12).
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SortArg {
    Relevance,
    Newest,
    Oldest,
    Name,
    Size,
    Source,
}

impl From<SortArg> for SortOrder {
    fn from(s: SortArg) -> SortOrder {
        match s {
            SortArg::Relevance => SortOrder::Relevance,
            SortArg::Newest => SortOrder::Newest,
            SortArg::Oldest => SortOrder::Oldest,
            SortArg::Name => SortOrder::Name,
            SortArg::Size => SortOrder::Size,
            SortArg::Source => SortOrder::Source,
        }
    }
}

/// Whose switch `vfs bookmarks on|off` turns (VFS-44 R1).
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrowserArg {
    Firefox,
    Chrome,
    Brave,
}

impl From<BrowserArg> for Browser {
    fn from(b: BrowserArg) -> Browser {
        match b {
            BrowserArg::Firefox => Browser::Firefox,
            BrowserArg::Chrome => Browser::Chrome,
            BrowserArg::Brave => Browser::Brave,
        }
    }
}

/// What `vfs bookmarks` does.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BookmarksAction {
    /// Turn the browser on and read the bookmarks.
    On,
    /// Turn it off; its stored bookmarks are deleted.
    Off,
    /// Read changed profiles again.
    Refresh,
    /// Print the status.
    Status,
}

fn open(db: &Path, firefox_roots: Vec<PathBuf>) -> AnyResult<Bookmarks> {
    let mut locators = Locators::system();
    if !firefox_roots.is_empty() {
        locators.firefox = Locator {
            roots: firefox_roots,
        };
    }
    Ok(Bookmarks::open(&bookmarks_db_path(db), locators)?)
}

pub fn cmd_bookmarks(
    db: &Path,
    action: BookmarksAction,
    browser: BrowserArg,
    firefox_roots: Vec<PathBuf>,
) -> AnyResult<()> {
    let bm = open(db, firefox_roots)?;
    let browser = Browser::from(browser);
    let status = match action {
        BookmarksAction::On => {
            bm.set_browser_enabled(browser, true)?;
            bm.refresh()?
        }
        BookmarksAction::Off => bm.set_browser_enabled(browser, false)?,
        BookmarksAction::Refresh => bm.refresh()?,
        BookmarksAction::Status => bm.status(),
    };
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}

pub fn cmd_search(
    db: &Path,
    query: &SearchQuery,
    source: SourceArg,
    sort: SortArg,
    json: bool,
) -> AnyResult<()> {
    let vfs = Vfs::open(db)?;
    // A search creates no bookmark store; only `vfs bookmarks on` does.
    let bm = if bookmarks_db_path(db).exists() {
        Some(open(db, Vec::new())?)
    } else {
        None
    };
    let options = SourceOptions {
        sources: source.sources(),
        bookmark_folder: None,
        sort: sort.into(),
    };
    let r = search_sources(&vfs, bm.as_ref(), query, &options)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    for h in &r.hits {
        match h {
            SourceHit::Files(f) => {
                let (y, m, d) = civil_from_unix(f.modified);
                let sep = std::path::MAIN_SEPARATOR;
                println!("archivo   {y:04}-{m:02}-{d:02}  {}{sep}{}", f.dir, f.name);
            }
            SourceHit::Bookmarks(b) => {
                let (y, m, d) = civil_from_unix(b.added);
                let what = if b.is_folder { "carpeta " } else { "marcador" };
                println!(
                    "{what}  {y:04}-{m:02}-{d:02}  {:?} {:?}/{}  {}  <{}>",
                    b.browser,
                    b.root,
                    b.folder.join("/"),
                    b.title,
                    b.url
                );
            }
        }
    }
    println!("{} of {} hits; {} ms", r.hits.len(), r.total, r.elapsed_ms);
    Ok(())
}
