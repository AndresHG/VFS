//! Browser bookmarks as a VFS search source: Firefox (VFS-6), Chrome and Brave (VFS-44).
//! Reads Firefox bookmarks from a copy of each profile's database (never the original in
//! place) and Chromium bookmarks from each profile's JSON file, keeps them in `bookmarks.db`
//! next to the file index, and searches them in memory with the file rules. Each browser
//! is off until the user turns its switch on; turning it off deletes what was stored.
//!
//! Privacy: only bookmarks are read (`firefox::ALLOWED_COLUMNS`, the Chromium bookmark
//! files); nothing leaves the computer.

pub mod chromium;
pub mod firefox;
mod search;
pub mod sources;
mod store;
#[cfg(feature = "test-support")]
pub mod testing;
mod types;

pub use search::{filters_exclude_bookmarks, is_openable, MAX_BOOKMARK_HITS};
pub use types::*;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use vfs_core::matching::parse_query;
use vfs_core::{Error, Result, SearchQuery};

use firefox::{Origin, Profile};
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

/// Where every browser keeps its profiles.
#[derive(Debug, Clone, Default)]
pub struct Locators {
    pub firefox: firefox::Locator,
    pub chromium: Vec<chromium::Locator>,
}

impl Locators {
    /// This system's Firefox, Chrome and Brave data folders.
    pub fn system() -> Locators {
        Locators {
            firefox: firefox::Locator::system(),
            chromium: chromium::Locator::system(),
        }
    }

    fn chromium(&self, browser: Browser) -> Option<&chromium::Locator> {
        self.chromium.iter().find(|l| l.browser == browser)
    }
}

/// Firefox alone (tests and `vfs bookmarks --firefox-root`).
impl From<firefox::Locator> for Locators {
    fn from(firefox: firefox::Locator) -> Locators {
        Locators {
            firefox,
            chromium: Vec::new(),
        }
    }
}

struct State {
    store: Store,
    entries: Vec<Entry>,
    /// Each browser's state; a browser never read is `Never`.
    read_state: HashMap<Browser, ReadState>,
    /// How the last read of each profile folder went (Ok, FromBackup or Failed).
    outcomes: HashMap<String, (Browser, ReadState)>,
}

impl State {
    fn reload(&mut self) -> Result<()> {
        self.entries = search::entries(self.store.load()?);
        Ok(())
    }

    fn state_of(&self, browser: Browser) -> ReadState {
        self.read_state
            .get(&browser)
            .copied()
            .unwrap_or(ReadState::Never)
    }

    fn enabled(&self, browser: Browser) -> bool {
        self.store.enabled(browser).unwrap_or(false)
    }

    /// Bookmarks and profiles with bookmarks of `browser`, or of every browser.
    fn counts(&self, browser: Option<Browser>) -> (u32, u32) {
        let mut profiles: Vec<i64> = Vec::new();
        let mut count = 0;
        for e in &self.entries {
            if e.is_folder || browser.is_some_and(|b| b != e.browser) {
                continue;
            }
            count += 1;
            if !profiles.contains(&e.profile_id) {
                profiles.push(e.profile_id);
            }
        }
        (count, profiles.len() as u32)
    }
}

/// The switched-on browsers' states together (`BookmarkStatus::state`).
fn overall(states: &[ReadState]) -> ReadState {
    [
        ReadState::Reading,
        ReadState::Failed,
        ReadState::FromBackup,
        ReadState::Ok,
        ReadState::NotFound,
    ]
    .into_iter()
    .find(|s| states.contains(s))
    .unwrap_or(ReadState::Never)
}

/// The bookmark source: settings, stored bookmarks and their search. Share it with `Arc`.
pub struct Bookmarks {
    locators: Locators,
    /// Where database copies are made and deleted (`<store>.snapshot`).
    work_dir: PathBuf,
    state: Mutex<State>,
    /// Held while a refresh runs; a second refresh meanwhile does nothing.
    refreshing: Mutex<()>,
}

impl Bookmarks {
    /// Opens or creates the store at `db_path` and deletes copies a crash may have left.
    pub fn open(db_path: &Path, locators: impl Into<Locators>) -> Result<Bookmarks> {
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
            read_state: HashMap::new(),
            outcomes: HashMap::new(),
        };
        state.reload()?;
        let stored: Vec<Browser> = state.entries.iter().map(|e| e.browser).collect();
        for b in stored {
            state.read_state.insert(b, ReadState::Ok);
        }
        Ok(Bookmarks {
            locators: locators.into(),
            work_dir,
            state: Mutex::new(state),
            refreshing: Mutex::new(()),
        })
    }

    /// At least one browser's switch is on.
    pub fn is_enabled(&self) -> bool {
        let st = lock(&self.state);
        Browser::ALL.iter().any(|b| st.enabled(*b))
    }

    /// Turns Firefox's switch on or off (VFS-6); see [`Bookmarks::set_browser_enabled`].
    pub fn set_enabled(&self, on: bool) -> Result<BookmarkStatus> {
        self.set_browser_enabled(Browser::Firefox, on)
    }

    /// Turns one browser's switch on or off. Off deletes that browser's stored bookmarks
    /// (VFS-6 R8, VFS-44 R9); on does not read by itself: call [`Bookmarks::refresh`] (the
    /// app does it on a background thread).
    pub fn set_browser_enabled(&self, browser: Browser, on: bool) -> Result<BookmarkStatus> {
        let mut st = lock(&self.state);
        st.store.set_enabled(browser, on)?;
        if !on {
            st.outcomes.retain(|_, (b, _)| *b != browser);
            st.read_state.remove(&browser);
            st.reload()?;
        }
        drop(st);
        Ok(self.status())
    }

    pub fn status(&self) -> BookmarkStatus {
        let st = lock(&self.state);
        let browsers: Vec<BrowserStatus> = Browser::ALL
            .into_iter()
            .filter(|b| {
                *b == Browser::Firefox
                    || st.enabled(*b)
                    || self.locators.chromium(*b).is_some_and(|l| l.installed())
            })
            .map(|b| {
                let enabled = st.enabled(b);
                let (count, profiles) = st.counts(Some(b));
                BrowserStatus {
                    browser: b,
                    enabled,
                    state: if enabled {
                        st.state_of(b)
                    } else {
                        ReadState::Never
                    },
                    count,
                    profiles,
                    last_read: st.store.last_read(Some(b)).ok().flatten(),
                }
            })
            .collect();
        let on: Vec<ReadState> = browsers
            .iter()
            .filter(|b| b.enabled)
            .map(|b| b.state)
            .collect();
        let (count, profiles) = st.counts(None);
        BookmarkStatus {
            enabled: !on.is_empty(),
            state: overall(&on),
            count,
            profiles,
            last_read: st.store.last_read(None).ok().flatten(),
            roots: search::places(&st.entries),
            browsers,
        }
    }

    /// Reads every profile of every switched-on browser whose files changed since its last
    /// read (VFS-6 R2, VFS-44 R4), and forgets profiles no longer found. Does nothing while
    /// every switch is off or another refresh runs.
    pub fn refresh(&self) -> Result<BookmarkStatus> {
        let Ok(_running) = self.refreshing.try_lock() else {
            return Ok(self.status());
        };
        let on: Vec<Browser> = {
            let mut st = lock(&self.state);
            let on: Vec<Browser> = Browser::ALL
                .into_iter()
                .filter(|b| st.enabled(*b))
                .collect();
            for b in &on {
                st.read_state.insert(*b, ReadState::Reading);
            }
            on
        };
        if on.is_empty() {
            return Ok(self.status());
        }
        let result = self.read_changed(&on);
        let _ = std::fs::remove_dir_all(&self.work_dir);
        let mut st = lock(&self.state);
        match &result {
            Ok(states) => st.read_state.extend(states.iter().copied()),
            // An error never leaves a browser at "reading".
            Err(_) => {
                for b in &on {
                    st.read_state.insert(*b, ReadState::Failed);
                }
            }
        }
        drop(st);
        result.map(|_| self.status())
    }

    /// The profiles of a switched-on browser.
    fn profiles_of(&self, browser: Browser) -> Vec<Profile> {
        match browser {
            Browser::Firefox => self.locators.firefox.profiles(&self.work_dir),
            b => self
                .locators
                .chromium(b)
                .map(chromium::Locator::profiles)
                .unwrap_or_default(),
        }
    }

    /// The reading part of [`Bookmarks::refresh`]; returns each browser's state. A profile
    /// skipped as unchanged keeps the outcome of its last read.
    fn read_changed(&self, browsers: &[Browser]) -> Result<Vec<(Browser, ReadState)>> {
        let mut found: Vec<String> = Vec::new();
        let mut states = Vec::new();
        for &browser in browsers {
            let profiles = self.profiles_of(browser);
            for p in &profiles {
                let dir = p.dir.to_string_lossy().into_owned();
                found.push(dir.clone());
                let sig = match browser {
                    Browser::Firefox => firefox::signature(&p.dir),
                    _ => chromium::signature(&p.dir),
                };
                if lock(&self.state).store.signature(&dir)?.as_deref() == Some(sig.as_str()) {
                    continue;
                }
                let read = match browser {
                    Browser::Firefox => {
                        firefox::read_profile(&p.dir, &self.work_dir.join("places"))
                    }
                    _ => chromium::read_profile(&p.dir),
                };
                let outcome = match read {
                    Ok((tree, origin)) => {
                        let mut st = lock(&self.state);
                        if !st.store.enabled(browser)? {
                            break;
                        }
                        st.store
                            .replace_profile(browser, &dir, &p.name, &sig, &tree, now())?;
                        if origin == Origin::Backup {
                            ReadState::FromBackup
                        } else {
                            ReadState::Ok
                        }
                    }
                    Err(e) => {
                        log::warn!("{browser:?} profile {}: {e}", p.dir.display());
                        ReadState::Failed
                    }
                };
                lock(&self.state).outcomes.insert(dir, (browser, outcome));
            }
            let st = lock(&self.state);
            let has = |s: ReadState| {
                st.outcomes
                    .iter()
                    .any(|(dir, (b, o))| *b == browser && *o == s && found.contains(dir))
            };
            states.push((
                browser,
                if profiles.is_empty() {
                    ReadState::NotFound
                } else if has(ReadState::Failed) {
                    ReadState::Failed
                } else if has(ReadState::FromBackup) {
                    ReadState::FromBackup
                } else {
                    ReadState::Ok
                },
            ));
        }
        let mut st = lock(&self.state);
        if browsers.iter().any(|b| st.enabled(*b)) {
            st.store.keep_only_profiles(&found)?;
            st.reload()?;
        }
        st.outcomes.retain(|dir, _| found.contains(dir));
        Ok(states)
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
            query.scope == vfs_core::Scope::Names,
        )
    }

    /// The address to open for a bookmark result, with the browser it came from (VFS-44
    /// R6): `not_found` for an unknown id, `unavailable` for an address that is not
    /// `http`/`https` (VFS-6 R10, AC-12).
    pub fn url_to_open(&self, id: i64) -> Result<(Browser, String)> {
        let st = lock(&self.state);
        let e = st
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| Error::NotFound(format!("bookmark {id}")))?;
        if !e.is_folder && is_openable(&e.url) {
            Ok((e.browser, e.url.clone()))
        } else {
            Err(Error::Unavailable("not a web address".into()))
        }
    }

    /// Refreshes now and then every [`REFRESH_INTERVAL`] on a low-priority thread (the
    /// indexer's walk profile, VFS-5 R1) until the returned sender is dropped; sending `()`
    /// refreshes at once ("Buscar cambios ahora", the setting turned on). `on_change` runs
    /// after every refresh.
    pub fn start_refresher(
        self: &Arc<Self>,
        on_change: impl Fn(&BookmarkStatus) + Send + 'static,
    ) -> Sender<()> {
        let (tx, rx) = mpsc::channel::<()>();
        let me = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("vfs-bookmarks".into())
            .spawn(move || {
                vfs_core::lower_thread_priority();
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
