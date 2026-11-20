//! Browser bookmarks as a VFS search source (VFS-6). Reads Firefox bookmarks from a copy of
//! each profile's database (never the original in place), keeps them in `bookmarks.db` next
//! to the file index, and searches them in memory with the file rules. Off until the user
//! turns "Incluir marcadores de Firefox" on; turning it off deletes what was stored.
//!
//! Privacy: only bookmark tables are read (`firefox::ALLOWED_COLUMNS`); nothing leaves the
//! computer.

pub mod firefox;
mod search;
pub mod sources;
mod store;
#[cfg(feature = "test-support")]
pub mod testing;
mod types;

pub use search::{filters_exclude_bookmarks, is_openable, MAX_BOOKMARK_HITS};
pub use types::*;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use vfs_core::matching::parse_query;
use vfs_core::{Error, Result, SearchQuery};

use firefox::{Locator, Origin};
use search::{Entry, Ranked};
use store::Store;

/// File name of the bookmark store, next to the file index (`index.db`).
pub const BOOKMARKS_FILE: &str = "bookmarks.db";
/// How often a running app looks for changed profiles (VFS-6 D6).
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// The bookmark store that goes with the index at `index_db` (same folder).
pub fn bookmarks_db_path(index_db: &Path) -> PathBuf {
    index_db.with_file_name(BOOKMARKS_FILE)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

struct State {
    store: Store,
    entries: Vec<Entry>,
    /// Profiles with at least one bookmark.
    profiles: u32,
    multi_profile: bool,
    read_state: ReadState,
}

impl State {
    fn reload(&mut self) -> Result<()> {
        let rows = self.store.load()?;
        let mut with_bookmarks: Vec<i64> = rows
            .iter()
            .filter(|r| !r.is_folder)
            .map(|r| r.profile_id)
            .collect();
        with_bookmarks.sort_unstable();
        with_bookmarks.dedup();
        self.profiles = with_bookmarks.len() as u32;
        self.multi_profile = self.profiles > 1;
        self.entries = search::entries(rows);
        Ok(())
    }
}

/// The bookmark source: setting, stored bookmarks and their search. Share it with `Arc`.
pub struct Bookmarks {
    locator: Locator,
    /// Where database copies are made and deleted (`<store>.snapshot`).
    work_dir: PathBuf,
    state: Mutex<State>,
    /// Held while a refresh runs; a second refresh meanwhile does nothing.
    refreshing: Mutex<()>,
}

impl Bookmarks {
    /// Opens or creates the store at `db_path` and deletes copies a crash may have left.
    pub fn open(db_path: &Path, locator: Locator) -> Result<Bookmarks> {
        if let Some(dir) = db_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut work = db_path.as_os_str().to_owned();
        work.push(".snapshot");
        let work_dir = PathBuf::from(work);
        let _ = std::fs::remove_dir_all(&work_dir);
        let mut state = State {
            store: Store::open(db_path)?,
            entries: Vec::new(),
            profiles: 0,
            multi_profile: false,
            read_state: ReadState::Never,
        };
        state.reload()?;
        if !state.entries.is_empty() {
            state.read_state = ReadState::Ok;
        }
        Ok(Bookmarks {
            locator,
            work_dir,
            state: Mutex::new(state),
            refreshing: Mutex::new(()),
        })
    }

    pub fn is_enabled(&self) -> bool {
        lock(&self.state).store.enabled().unwrap_or(false)
    }

    /// Turns the source on or off. Off deletes every stored bookmark (R8); on does not read
    /// by itself: call [`Bookmarks::refresh`] (the app does it on a background thread).
    pub fn set_enabled(&self, on: bool) -> Result<BookmarkStatus> {
        let mut st = lock(&self.state);
        st.store.set_enabled(on)?;
        if !on {
            st.entries.clear();
            st.profiles = 0;
            st.multi_profile = false;
            st.read_state = ReadState::Never;
        }
        drop(st);
        Ok(self.status())
    }

    pub fn status(&self) -> BookmarkStatus {
        let st = lock(&self.state);
        BookmarkStatus {
            enabled: st.store.enabled().unwrap_or(false),
            state: st.read_state,
            count: st.entries.iter().filter(|e| !e.is_folder).count() as u32,
            profiles: st.profiles,
            last_read: st.store.last_read().ok().flatten(),
            roots: search::places(&st.entries, st.multi_profile),
        }
    }

    /// Reads every Firefox profile whose database or backups changed since its last read
    /// (R2, AC-9), from a copy (R3) or its backup (R4), and forgets profiles no longer found.
    /// Does nothing while the setting is off or another refresh runs.
    pub fn refresh(&self) -> Result<BookmarkStatus> {
        let Ok(_running) = self.refreshing.try_lock() else {
            return Ok(self.status());
        };
        if !self.is_enabled() {
            return Ok(self.status());
        }
        lock(&self.state).read_state = ReadState::Reading;
        let profiles = self.locator.profiles(&self.work_dir);
        let (mut any_backup, mut any_failed) = (false, false);
        for p in &profiles {
            let dir = p.dir.to_string_lossy().into_owned();
            let sig = firefox::signature(&p.dir);
            if lock(&self.state).store.signature(&dir)?.as_deref() == Some(sig.as_str()) {
                continue;
            }
            match firefox::read_profile(&p.dir, &self.work_dir.join("places")) {
                Ok((tree, origin)) => {
                    any_backup |= origin == Origin::Backup;
                    let mut st = lock(&self.state);
                    if !st.store.enabled()? {
                        break;
                    }
                    st.store.replace_profile(
                        Browser::Firefox,
                        &dir,
                        &p.name,
                        &sig,
                        &tree,
                        now(),
                    )?;
                }
                Err(e) => {
                    log::warn!("Firefox profile {}: {e}", p.dir.display());
                    any_failed = true;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.work_dir);
        let mut st = lock(&self.state);
        if st.store.enabled()? {
            let found: Vec<String> = profiles
                .iter()
                .map(|p| p.dir.to_string_lossy().into_owned())
                .collect();
            st.store.keep_only_profiles(&found)?;
            st.reload()?;
        }
        st.read_state = if profiles.is_empty() {
            ReadState::NoFirefox
        } else if any_failed {
            ReadState::Failed
        } else if any_backup {
            ReadState::FromBackup
        } else {
            ReadState::Ok
        };
        drop(st);
        Ok(self.status())
    }

    /// Ranked bookmark and folder hits for `query`, only inside the folder `inside` when
    /// given (at most [`MAX_BOOKMARK_HITS`]; the flag says the list was cut).
    pub(crate) fn search(&self, query: &SearchQuery, inside: Option<i64>) -> (Vec<Ranked>, bool) {
        let parsed = parse_query(&query.text);
        let st = lock(&self.state);
        search::search(
            &st.entries,
            &parsed,
            &query.filters,
            inside,
            st.multi_profile,
        )
    }

    /// The address to open for a bookmark result: `not_found` for an unknown id,
    /// `unavailable` for an address that is not `http`/`https` (R10, AC-12).
    pub fn url_to_open(&self, id: i64) -> Result<String> {
        let st = lock(&self.state);
        let e = st
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| Error::NotFound(format!("bookmark {id}")))?;
        if !e.is_folder && is_openable(&e.url) {
            Ok(e.url.clone())
        } else {
            Err(Error::Unavailable("not a web address".into()))
        }
    }

    /// Refreshes now and then every [`REFRESH_INTERVAL`] on a background-priority thread
    /// until the returned sender is dropped; sending `()` refreshes at once ("Buscar cambios
    /// ahora", the setting turned on). `on_change` runs after every refresh.
    pub fn start_refresher(
        self: &Arc<Self>,
        on_change: impl Fn(&BookmarkStatus) + Send + 'static,
    ) -> Sender<()> {
        let (tx, rx) = mpsc::channel::<()>();
        let me = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("vfs-bookmarks".into())
            .spawn(move || {
                vfs_core::enter_background_priority();
                loop {
                    if me.is_enabled() {
                        match me.refresh() {
                            Ok(status) => on_change(&status),
                            Err(e) => log::warn!("bookmark refresh: {e}"),
                        }
                    }
                    match rx.recv_timeout(REFRESH_INTERVAL) {
                        Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("bookmark thread not started: {e}");
        }
        tx
    }

    /// Bookmarks and folders in the store (tests).
    pub fn stored_count(&self) -> Result<u32> {
        lock(&self.state).store.count()
    }
}
