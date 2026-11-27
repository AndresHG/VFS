//! Chrome and Brave bookmarks (VFS-44): finding the profiles, reading `Bookmarks`,
//! `AccountBookmarks` and `Bookmarks.bak`, one switch per browser, and the results naming
//! their browser.

use std::path::Path;
use std::time::SystemTime;

use rusqlite::Connection;
use vfs_bookmarks::sources::search_sources;
use vfs_bookmarks::testing::{
    chromium_file, chromium_folder, chromium_url, FakeChromium, FakeFirefox, Places, MENU,
};
use vfs_bookmarks::{
    BookmarkHit, BookmarkRoot, Bookmarks, Browser, Locators, ReadState, Source, SourceHit,
    SourceOptions,
};
use vfs_core::{Error, SearchFilters, SearchQuery, Vfs};

struct World {
    tmp: tempfile::TempDir,
    vfs: Vfs,
    chrome: FakeChromium,
    brave: FakeChromium,
    firefox: FakeFirefox,
}

/// An empty index, a Firefox data folder with no profile and a Chrome one with no profile;
/// Brave's folder is not created.
fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let vfs = Vfs::open(&tmp.path().join("app/index.db")).unwrap();
    let chrome = FakeChromium::new(&tmp.path().join("Chrome"), Browser::Chrome);
    let brave = FakeChromium::new(&tmp.path().join("Brave"), Browser::Brave);
    std::fs::remove_dir(&brave.root).unwrap();
    let firefox = FakeFirefox::new(&tmp.path().join("Firefox"));
    World {
        tmp,
        vfs,
        chrome,
        brave,
        firefox,
    }
}

impl World {
    fn open(&self) -> Bookmarks {
        let locators = Locators {
            firefox: self.firefox.locator(),
            chromium: vec![self.chrome.locator(), self.brave.locator()],
        };
        Bookmarks::open(
            &vfs_bookmarks::bookmarks_db_path(&self.tmp.path().join("app/index.db")),
            locators,
        )
        .unwrap()
    }

    fn find(&self, bm: &Bookmarks, text: &str, folder: Option<i64>) -> Vec<BookmarkHit> {
        let query = SearchQuery {
            text: text.into(),
            filters: SearchFilters::default(),
            limit: 100,
            offset: 0,
            sort: Default::default(),
            scope: Default::default(),
        };
        let options = SourceOptions {
            sources: vec![Source::Bookmarks],
            bookmark_folder: folder,
            ..Default::default()
        };
        search_sources(&self.vfs, Some(bm), &query, &options)
            .unwrap()
            .hits
            .into_iter()
            .map(|h| match h {
                SourceHit::Bookmarks(b) => b,
                SourceHit::Files(f) => panic!("file {f:?}"),
            })
            .collect()
    }
}

fn titles(hits: &[BookmarkHit]) -> Vec<&str> {
    hits.iter().map(|h| h.title.as_str()).collect()
}

fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>, SystemTime)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            let m = std::fs::metadata(&p).unwrap();
            (
                p.display().to_string(),
                std::fs::read(&p).unwrap_or_default(),
                m.modified().unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

/// VFS-44 AC-1: Firefox is always listed; Chrome only once its data folder exists, Brave
/// not without one; every switch starts off.
#[test]
fn lists_installed_browsers_switched_off() {
    let w = world();
    let bm = w.open();
    let st = bm.status();
    let listed: Vec<(Browser, bool)> = st.browsers.iter().map(|b| (b.browser, b.enabled)).collect();
    assert_eq!(
        listed,
        vec![(Browser::Firefox, false), (Browser::Chrome, false)]
    );
    assert!(!st.enabled);
}

/// VFS-44 AC-2, AC-6: two Chrome profiles, one also keeping account bookmarks; each word is
/// found under its root, folders are results and filters, profiles are named, and nothing
/// in the browser's folders changes.
#[test]
fn reads_every_profile_and_the_account_file_read_only() {
    let mut w = world();
    let personal = w.chrome.add_profile("Default", "Personal");
    let work = w.chrome.add_profile("Profile 1", "Trabajo");
    std::fs::write(
        personal.join("Bookmarks"),
        chromium_file(
            vec![chromium_folder(
                "Recetas",
                vec![chromium_url(
                    "Croquetas caseras",
                    "https://cocina.es/croquetas",
                    1_650_000_000,
                )],
            )],
            vec![chromium_url("Banco", "https://banco.es/", 1_600_000_000)],
            vec![],
        ),
    )
    .unwrap();
    std::fs::write(
        personal.join("AccountBookmarks"),
        chromium_file(
            vec![chromium_url(
                "Rosales en invierno",
                "https://jardin.es/rosales",
                1_700_000_000,
            )],
            vec![],
            vec![],
        ),
    )
    .unwrap();
    std::fs::write(
        work.join("Bookmarks"),
        chromium_file(
            vec![],
            vec![],
            vec![chromium_url(
                "Nóminas del móvil",
                "https://empresa.es/",
                1_600_000_000,
            )],
        ),
    )
    .unwrap();
    let before = (
        snapshot(&personal),
        snapshot(&work),
        snapshot(&w.chrome.root),
    );

    let bm = w.open();
    bm.set_browser_enabled(Browser::Chrome, true).unwrap();
    let st = bm.refresh().unwrap();
    assert_eq!(st.state, ReadState::Ok);
    let chrome = st
        .browsers
        .iter()
        .find(|b| b.browser == Browser::Chrome)
        .unwrap();
    assert_eq!((chrome.count, chrome.profiles), (4, 2));

    let croquetas = w.find(&bm, "croquetas", None);
    assert_eq!(titles(&croquetas), ["Croquetas caseras"]);
    assert_eq!(croquetas[0].root, BookmarkRoot::Toolbar);
    assert_eq!(croquetas[0].folder, ["Recetas"]);
    assert_eq!(croquetas[0].browser, Browser::Chrome);
    assert_eq!(croquetas[0].profile.as_deref(), Some("Personal"));
    assert_eq!(w.find(&bm, "banco", None)[0].root, BookmarkRoot::Unfiled);
    let rosales = w.find(&bm, "rosales", None);
    assert_eq!(rosales[0].root, BookmarkRoot::Toolbar);
    let nominas = w.find(&bm, "nominas", None);
    assert_eq!(nominas[0].root, BookmarkRoot::Mobile);
    assert_eq!(nominas[0].profile.as_deref(), Some("Trabajo"));

    // The folder is a result, and its contents are listed inside it.
    let folder = w.find(&bm, "recetas", None);
    let folder = folder.iter().find(|h| h.is_folder).unwrap();
    assert_eq!(
        titles(&w.find(&bm, "", Some(folder.id))),
        ["Croquetas caseras"]
    );
    // The account's bar is the same «Barra de marcadores»: one place per root and profile.
    let bars = st
        .roots
        .iter()
        .filter(|r| r.root == BookmarkRoot::Toolbar && r.profile.as_deref() == Some("Personal"))
        .count();
    assert_eq!(bars, 1);

    assert_eq!(
        (
            snapshot(&personal),
            snapshot(&work),
            snapshot(&w.chrome.root)
        ),
        before
    );
}

/// VFS-44 AC-3: a truncated `Bookmarks` falls back to `Bookmarks.bak`; with neither
/// readable, or a damaged `AccountBookmarks`, the bookmarks read earlier stay and the state
/// says the read failed.
#[test]
fn falls_back_to_the_backup_then_keeps_what_it_had() {
    let mut w = world();
    let dir = w.chrome.add_profile("Default", "Persona 1");
    let good = chromium_file(
        vec![chromium_url(
            "Flan de huevo",
            "https://cocina.es/flan",
            1_650_000_000,
        )],
        vec![],
        vec![],
    );
    std::fs::write(dir.join("Bookmarks"), &good[..good.len() / 2]).unwrap();
    std::fs::write(dir.join("Bookmarks.bak"), &good).unwrap();
    let bm = w.open();
    bm.set_browser_enabled(Browser::Chrome, true).unwrap();
    assert_eq!(bm.refresh().unwrap().state, ReadState::FromBackup);
    assert_eq!(titles(&w.find(&bm, "flan", None)), ["Flan de huevo"]);

    std::fs::write(dir.join("Bookmarks"), "{").unwrap();
    std::fs::write(dir.join("Bookmarks.bak"), "{").unwrap();
    assert_eq!(bm.refresh().unwrap().state, ReadState::Failed);
    assert_eq!(titles(&w.find(&bm, "flan", None)), ["Flan de huevo"]);

    // A damaged account file fails the read too, and keeps what was read.
    std::fs::write(dir.join("Bookmarks"), &good).unwrap();
    std::fs::write(dir.join("AccountBookmarks"), "{").unwrap();
    assert_eq!(bm.refresh().unwrap().state, ReadState::Failed);
    assert_eq!(titles(&w.find(&bm, "flan", None)), ["Flan de huevo"]);
}

/// VFS-44 AC-4: an unchanged file (same size and time) is not read again; a changed one is.
#[test]
fn reads_again_only_when_the_file_changes() {
    let mut w = world();
    let dir = w.chrome.add_profile("Default", "Persona 1");
    let file = dir.join("Bookmarks");
    let body = |title: &str| {
        chromium_file(
            vec![chromium_url(title, "https://a.es/", 1_650_000_000)],
            vec![],
            vec![],
        )
    };
    std::fs::write(&file, body("Primero")).unwrap();
    let bm = w.open();
    bm.set_browser_enabled(Browser::Chrome, true).unwrap();
    bm.refresh().unwrap();
    let stamp = std::fs::metadata(&file).unwrap().modified().unwrap();

    // Same length, same time: the refresh does not open it.
    std::fs::write(&file, body("Segundo")).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    bm.refresh().unwrap();
    assert_eq!(titles(&w.find(&bm, "primero", None)), ["Primero"]);

    std::fs::write(&file, body("Tercero más largo")).unwrap();
    bm.refresh().unwrap();
    assert!(w.find(&bm, "primero", None).is_empty());
    assert_eq!(titles(&w.find(&bm, "tercero", None)), ["Tercero más largo"]);
}

/// VFS-44 AC-5, AC-7: a bookmark opens in its own browser, a `chrome://` address in none;
/// one address in Chrome and Brave is one result naming both.
#[test]
fn opens_in_its_browser_and_folds_one_address_across_browsers() {
    let mut w = world();
    std::fs::create_dir_all(&w.brave.root).unwrap();
    let c = w.chrome.add_profile("Default", "Persona 1");
    let b = w.brave.add_profile("Default", "Persona 1");
    std::fs::write(
        c.join("Bookmarks"),
        chromium_file(
            vec![
                chromium_url("Periódico", "https://diario.es/", 1_650_000_000),
                chromium_url("Ajustes", "chrome://settings", 1_650_000_000),
            ],
            vec![],
            vec![],
        ),
    )
    .unwrap();
    std::fs::write(
        b.join("Bookmarks"),
        chromium_file(
            vec![
                chromium_url("Periódico de hoy", "https://diario.es/", 1_650_000_000),
                chromium_url("Mapas", "https://mapas.es/", 1_650_000_000),
            ],
            vec![],
            vec![],
        ),
    )
    .unwrap();
    let bm = w.open();
    bm.set_browser_enabled(Browser::Chrome, true).unwrap();
    bm.set_browser_enabled(Browser::Brave, true).unwrap();
    bm.refresh().unwrap();

    let diario = w.find(&bm, "periodico", None);
    assert_eq!(titles(&diario), ["Periódico"]);
    assert_eq!(diario[0].browser, Browser::Chrome);
    assert_eq!(diario[0].also_in, [Browser::Brave]);

    let mapas = w.find(&bm, "mapas", None);
    assert_eq!(
        bm.url_to_open(mapas[0].id).unwrap(),
        (Browser::Brave, "https://mapas.es/".to_string())
    );
    let ajustes = w.find(&bm, "ajustes", None);
    assert!(!ajustes[0].openable);
    assert!(matches!(
        bm.url_to_open(ajustes[0].id),
        Err(Error::Unavailable(_))
    ));
}

/// VFS-44 AC-6, AC-8, AC-9: with Firefox and Chrome on, each place names its browser and
/// one profile per browser shows no profile name; turning Chrome off removes only Chrome's
/// rows, and no Chromium file is read while its switch is off.
#[test]
fn one_switch_per_browser() {
    let mut w = world();
    let ff = w.firefox.add_profile("default-release", "p.default");
    {
        let conn = Connection::open(ff.join("places.sqlite")).unwrap();
        Places { conn: &conn }.bookmark(
            MENU,
            Some("Receta de Firefox"),
            "https://ff.es/",
            1_650_000_000,
        );
    }
    let c = w.chrome.add_profile("Default", "Persona 1");
    std::fs::write(
        c.join("Bookmarks"),
        chromium_file(
            vec![chromium_url(
                "Receta de Chrome",
                "https://cr.es/",
                1_650_000_000,
            )],
            vec![],
            vec![],
        ),
    )
    .unwrap();
    let bm = w.open();
    bm.set_enabled(true).unwrap();
    let st = bm.refresh().unwrap();
    // Chrome's switch is off: its file was not read and it has nothing stored.
    let chrome = st
        .browsers
        .iter()
        .find(|b| b.browser == Browser::Chrome)
        .unwrap();
    assert_eq!((chrome.state, chrome.count), (ReadState::Never, 0));
    assert_eq!(titles(&w.find(&bm, "receta", None)), ["Receta de Firefox"]);

    bm.set_browser_enabled(Browser::Chrome, true).unwrap();
    let st = bm.refresh().unwrap();
    let browsers: Vec<Browser> = st.roots.iter().map(|r| r.browser).collect();
    assert_eq!(browsers, [Browser::Firefox, Browser::Chrome]);
    assert!(st.roots.iter().all(|r| r.profile.is_none()));
    let hits = w.find(&bm, "receta", None);
    let mut found: Vec<(Browser, Option<String>)> = hits
        .iter()
        .map(|h| (h.browser, h.profile.clone()))
        .collect();
    found.sort();
    assert_eq!(found, [(Browser::Firefox, None), (Browser::Chrome, None)]);

    let st = bm.set_browser_enabled(Browser::Chrome, false).unwrap();
    assert!(st.enabled);
    assert_eq!(titles(&w.find(&bm, "receta", None)), ["Receta de Firefox"]);
    let chrome = st
        .browsers
        .iter()
        .find(|b| b.browser == Browser::Chrome)
        .unwrap();
    assert_eq!((chrome.enabled, chrome.count), (false, 0));
}
