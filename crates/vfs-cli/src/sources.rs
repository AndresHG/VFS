//! `vfs search --source` and `vfs bookmarks` (VFS-6): Firefox bookmarks as a second source,
//! stored in `bookmarks.db` next to the index given with `--db`.

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use vfs_bookmarks::firefox::Locator;
use vfs_bookmarks::sources::search_sources;
use vfs_bookmarks::{bookmarks_db_path, Bookmarks, SortOrder, Source, SourceHit, SourceOptions};
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

/// What `vfs bookmarks` does.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BookmarksAction {
    /// Turn the source on and read the bookmarks.
    On,
    /// Turn it off; every stored bookmark is deleted.
    Off,
    /// Read changed profiles again.
    Refresh,
    /// Print the status.
    Status,
}

fn open(db: &Path, firefox_roots: Vec<PathBuf>) -> AnyResult<Bookmarks> {
    let locator = if firefox_roots.is_empty() {
        Locator::system()
    } else {
        Locator {
            roots: firefox_roots,
        }
    };
    Ok(Bookmarks::open(&bookmarks_db_path(db), locator)?)
}

pub fn cmd_bookmarks(
    db: &Path,
    action: BookmarksAction,
    firefox_roots: Vec<PathBuf>,
) -> AnyResult<()> {
    let bm = open(db, firefox_roots)?;
    let status = match action {
        BookmarksAction::On => {
            bm.set_enabled(true)?;
            bm.refresh()?
        }
        BookmarksAction::Off => bm.set_enabled(false)?,
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
                    "{what}  {y:04}-{m:02}-{d:02}  {:?}/{}  {}  <{}>",
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
