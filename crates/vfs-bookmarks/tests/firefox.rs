//! Reading Firefox profiles (VFS-6 AC-1 to AC-5, AC-9).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use rusqlite::Connection;
use vfs_bookmarks::firefox::{self, Locator, Origin};
use vfs_bookmarks::testing::{self, FakeFirefox, Places, MENU, MOBILE, TOOLBAR, UNFILED};
use vfs_bookmarks::{BookmarkRoot, Bookmarks, ReadState};

fn work(tmp: &Path) -> PathBuf {
    tmp.join("work")
}

fn by_title(list: &[vfs_bookmarks::Bookmark], title: &str) -> Vec<vfs_bookmarks::Bookmark> {
    list.iter().filter(|b| b.title == title).cloned().collect()
}

/// VFS-6 AC-1: nested folders, unsorted bookmarks, tags, a keyword, duplicates, very long
/// titles and addresses and non-Latin text are read with their folder path, root, tags,
/// keyword and date; separators, `place:` and `javascript:` entries and the tags tree are
/// not bookmarks; identical rows appear once and an untitled one is titled by its address.
#[test]
fn ac1_reads_a_profile_with_every_kind_of_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ff = FakeFirefox::new(&tmp.path().join("Firefox"));
    let dir = ff.add_profile("default-release", "a.default-release");
    let conn = Connection::open(dir.join("places.sqlite")).unwrap();
    let p = Places { conn: &conn };
    let recetas = p.folder(MENU, "Recetas");
    let postres = p.folder(recetas, "Postres de la abuela");
    p.bookmark(
        postres,
        Some("Flan de huevo"),
        "https://cocina.es/flan",
        1_600_000_000,
    );
    p.bookmark(
        UNFILED,
        Some("Sin ordenar"),
        "https://www.ejemplo.es/x",
        1_500_000_000,
    );
    p.bookmark(TOOLBAR, Some("Banco"), "https://banco.es/", 10);
    p.bookmark(MOBILE, Some("Desde el móvil"), "https://movil.es/", 10);
    // The same page in two folders, and one exact duplicate.
    p.bookmark(MENU, Some("Doble"), "https://doble.es/", 10);
    p.bookmark(recetas, Some("Doble"), "https://doble.es/", 10);
    p.bookmark(recetas, Some("Doble"), "https://doble.es/", 10);
    let long_title = "Título muy largo ".repeat(120);
    let long_url = format!("https://largo.es/{}", "a".repeat(2000));
    p.bookmark(UNFILED, Some(&long_title), &long_url, 10);
    for t in [
        "Медицина и здоровье",
        "日本料理のレシピ",
        "Ελληνική κουζίνα",
    ] {
        p.bookmark(UNFILED, Some(t), "https://intl.example/", 10);
    }
    p.bookmark(UNFILED, None, "https://sin-titulo.es/", 10);
    p.separator(MENU);
    p.bookmark(
        MENU,
        Some("Más visitados"),
        "place:sort=8&maxResults=10",
        10,
    );
    p.bookmark(MENU, Some("Bookmarklet"), "javascript:alert(1)", 10);
    p.tag("https://cocina.es/flan", "receta");
    p.tag("https://cocina.es/flan", "para Begoña");
    p.keyword("https://banco.es/", "bco");
    drop(conn);

    let (tree, origin) = firefox::read_profile(&dir, &work(tmp.path())).unwrap();
    let list = tree.bookmarks;
    assert_eq!(origin, Origin::Database);

    let flan = &by_title(&list, "Flan de huevo")[0];
    assert_eq!(flan.root, BookmarkRoot::Menu);
    assert_eq!(flan.folder, vec!["Recetas", "Postres de la abuela"]);
    assert_eq!(flan.tags, vec!["para Begoña", "receta"]);
    assert_eq!(flan.added, 1_600_000_000);
    assert_eq!(
        by_title(&list, "Sin ordenar")[0].root,
        BookmarkRoot::Unfiled
    );
    assert!(by_title(&list, "Sin ordenar")[0].folder.is_empty());
    assert_eq!(by_title(&list, "Banco")[0].root, BookmarkRoot::Toolbar);
    assert_eq!(by_title(&list, "Banco")[0].keyword.as_deref(), Some("bco"));
    assert_eq!(
        by_title(&list, "Desde el móvil")[0].root,
        BookmarkRoot::Mobile
    );
    let doble = by_title(&list, "Doble");
    assert_eq!(doble.len(), 2, "once per folder: {doble:?}");
    assert_eq!(by_title(&list, &long_title)[0].url, long_url);
    for t in [
        "Медицина и здоровье",
        "日本料理のレシピ",
        "Ελληνική κουζίνα",
    ] {
        assert_eq!(by_title(&list, t).len(), 1, "{t}");
    }
    assert_eq!(by_title(&list, "https://sin-titulo.es/").len(), 1);
    for gone in ["Más visitados", "Bookmarklet", "receta", "para Begoña"] {
        assert!(by_title(&list, gone).is_empty(), "{gone} is not a bookmark");
    }
    // Tag entries (untitled bookmarks inside tag folders) are not bookmarks either.
    assert_eq!(
        list.iter()
            .filter(|b| b.url == "https://cocina.es/flan")
            .count(),
        1
    );
    assert_eq!(list.len(), 11);
}

/// The profile folder's file names with size and modification time.
fn listing(dir: &Path) -> BTreeMap<String, (u64, SystemTime)> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let m = e.metadata().unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                (m.len(), m.modified().unwrap()),
            )
        })
        .collect()
}

const HOLDER_DB: &str = "VFS_TEST_HOLDER_DB";

/// Helper for `ac2_reads_while_another_process_holds_a_write_lock`, run in a child process:
/// with `VFS_TEST_HOLDER_DB` set it holds that database the way Firefox does (WAL,
/// exclusive locking, a committed change not yet checkpointed, a write transaction open),
/// signals `<db>.ready`, waits for `<db>.go`, commits, and writes `<db>.done`. Without the
/// variable (a normal test run) it returns at once.
#[test]
fn child_holder_process() {
    let Some(db) = std::env::var_os(HOLDER_DB) else {
        return;
    };
    let db = PathBuf::from(db);
    let side = |ext: &str| {
        let mut s = db.as_os_str().to_owned();
        s.push(ext);
        PathBuf::from(s)
    };
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "PRAGMA locking_mode = EXCLUSIVE; PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;",
    )
    .unwrap();
    Places { conn: &conn }.bookmark(MENU, Some("En el WAL"), "https://wal.es/", 10);
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    Places { conn: &conn }.bookmark(MENU, Some("Sin confirmar"), "https://nc.es/", 10);
    std::fs::write(side(".ready"), "").unwrap();
    let start = Instant::now();
    while !side(".go").exists() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "parent never answered"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    conn.execute_batch("COMMIT").unwrap();
    Places { conn: &conn }.bookmark(MENU, Some("Después"), "https://despues.es/", 10);
    std::fs::write(side(".done"), "").unwrap();
}

/// VFS-6 AC-2 (R3): while another process holds `places.sqlite` with an exclusive write lock
/// and has committed changes only in its WAL and an uncommitted one open, the read succeeds
/// with the committed bookmarks, the holder can still commit and write, and the reader
/// created or modified no file in the profile folder.
#[test]
fn ac2_reads_while_another_process_holds_a_write_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ff = FakeFirefox::new(&tmp.path().join("Firefox"));
    let dir = ff.add_profile("default", "p.default");
    let db = dir.join("places.sqlite");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "child_holder_process",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HOLDER_DB, &db)
        .spawn()
        .unwrap();
    let ready = {
        let mut s = db.as_os_str().to_owned();
        s.push(".ready");
        PathBuf::from(s)
    };
    let start = Instant::now();
    while !ready.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "holder did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Taken once the holder is ready; the reader must leave it exactly as it is.
    let before = listing(&dir);

    let (tree, _) = firefox::read_profile(&dir, &work(tmp.path())).unwrap();
    let list = tree.bookmarks;
    let titles: Vec<&str> = list.iter().map(|b| b.title.as_str()).collect();
    assert!(titles.contains(&"En el WAL"), "{titles:?}");
    assert!(!titles.contains(&"Sin confirmar"), "{titles:?}");
    assert_eq!(
        listing(&dir),
        before,
        "the reader changed the profile folder"
    );

    let go = {
        let mut s = db.as_os_str().to_owned();
        s.push(".go");
        PathBuf::from(s)
    };
    std::fs::write(&go, "").unwrap();
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "the holder could not commit after the read"
    );
    let done = {
        let mut s = db.as_os_str().to_owned();
        s.push(".done");
        PathBuf::from(s)
    };
    assert!(done.exists());
}

fn backup_json(title: &str) -> String {
    format!(
        r#"{{"guid":"root________","title":"","typeCode":2,"type":"text/x-moz-place-container","root":"placesRoot","children":[
          {{"guid":"menu________","title":"menu","type":"text/x-moz-place-container","root":"bookmarksMenuFolder","children":[
            {{"guid":"f1","title":"Recetas","type":"text/x-moz-place-container","children":[
              {{"guid":"b1","title":"{title}","type":"text/x-moz-place","uri":"https://copia.es/","tags":"receta,para Begoña","keyword":"cp","dateAdded":1600000000000000,"lastModified":1600000000000000}},
              {{"guid":"s1","type":"text/x-moz-place-separator"}}]}}]}},
          {{"guid":"unfiled_____","title":"unfiled","type":"text/x-moz-place-container","root":"unfiledBookmarksFolder","children":[
            {{"guid":"b2","title":"Suelto","type":"text/x-moz-place","uri":"https://suelto.es/","dateAdded":1,"lastModified":1}}]}}]}}"#
    )
}

/// VFS-6 AC-3 (R4): a corrupt or missing `places.sqlite` falls back to the newest backup that
/// can be read (an older valid one when the newest is damaged); with neither, the bookmarks
/// stored before stay and the status says the read failed.
#[test]
fn ac3_damaged_database_falls_back_to_the_newest_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ff = FakeFirefox::new(&tmp.path().join("Firefox"));
    let dir = ff.add_profile("default", "p.default");
    let backups = dir.join("bookmarkbackups");
    testing::write_jsonlz4(
        &backups.join("bookmarks-2026-11-01_2_aaaaaaaaaaaaaaaaaaaaaaaa.jsonlz4"),
        &backup_json("Antigua"),
    );
    testing::write_jsonlz4(
        &backups.join("bookmarks-2026-11-19_2_bbbbbbbbbbbbbbbbbbbbbbbb.jsonlz4"),
        &backup_json("Nueva"),
    );
    std::fs::write(
        backups.join("bookmarks-2026-11-20_2_cccccccccccccccccccccccc.jsonlz4"),
        b"mozLz40\0garbage",
    )
    .unwrap();
    std::fs::write(
        dir.join("places.sqlite"),
        b"not a database at all, just bytes",
    )
    .unwrap();

    let (tree, origin) = firefox::read_profile(&dir, &work(tmp.path())).unwrap();
    let list = tree.bookmarks;
    assert_eq!(origin, Origin::Backup);
    let nueva = &by_title(&list, "Nueva")[0];
    assert_eq!(nueva.folder, vec!["Recetas"]);
    assert_eq!(nueva.tags, vec!["para Begoña", "receta"]);
    assert_eq!(nueva.keyword.as_deref(), Some("cp"));
    assert_eq!(nueva.added, 1_600_000_000);
    assert_eq!(by_title(&list, "Suelto")[0].root, BookmarkRoot::Unfiled);
    assert_eq!(list.len(), 2);

    std::fs::remove_file(dir.join("places.sqlite")).unwrap();
    let (tree, origin) = firefox::read_profile(&dir, &work(tmp.path())).unwrap();
    let list = tree.bookmarks;
    assert_eq!((origin, list.len()), (Origin::Backup, 2));

    // Through the store: read once, then make everything unreadable.
    let bm = Bookmarks::open(&tmp.path().join("app/bookmarks.db"), ff.locator()).unwrap();
    bm.set_enabled(true).unwrap();
    let st = bm.refresh().unwrap();
    assert_eq!((st.state, st.count), (ReadState::FromBackup, 2));
    // Nothing changed: the profile is skipped and still reads as from the backup.
    assert_eq!(bm.refresh().unwrap().state, ReadState::FromBackup);
    std::fs::remove_dir_all(&backups).unwrap();
    std::fs::write(dir.join("places.sqlite"), b"still not a database").unwrap();
    let st = bm.refresh().unwrap();
    assert_eq!((st.state, st.count), (ReadState::Failed, 2));
}

/// VFS-6 AC-4: no Firefox, or a `profiles.ini` listing a folder that does not exist, finds no
/// profile and fails nothing; several profiles, one only in a profile group database, are
/// all read and their hits carry the profile name.
#[test]
fn ac4_no_firefox_missing_profiles_and_several_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let nothing = Locator {
        roots: vec![tmp.path().join("no-firefox-here")],
    };
    assert!(nothing.profiles(&work(tmp.path())).is_empty());
    let bm = Bookmarks::open(&tmp.path().join("a/bookmarks.db"), nothing).unwrap();
    bm.set_enabled(true).unwrap();
    let st = bm.refresh().unwrap();
    assert_eq!((st.state, st.count), (ReadState::NoFirefox, 0));

    let mut ff = FakeFirefox::new(&tmp.path().join("Firefox"));
    ff.list_profile("fantasma", "missing.default", None);
    assert!(ff.locator().profiles(&work(tmp.path())).is_empty());

    let casa = ff.add_profile("Casa", "c.default");
    ff.list_profile("Grupo", "c.default", Some("store1"));
    ff.add_group("store1", &[("Trabajo", "t.selectable")]);
    for (dir, title) in [
        (casa, "Receta de casa"),
        (ff.root.join("Profiles/t.selectable"), "Receta del trabajo"),
    ] {
        let conn = Connection::open(dir.join("places.sqlite")).unwrap();
        Places { conn: &conn }.bookmark(MENU, Some(title), "https://r.es/", 10);
    }
    let names: Vec<String> = ff
        .locator()
        .profiles(&work(tmp.path()))
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, vec!["Casa", "Trabajo"]);

    let bm = Bookmarks::open(&tmp.path().join("b/bookmarks.db"), ff.locator()).unwrap();
    bm.set_enabled(true).unwrap();
    let st = bm.refresh().unwrap();
    assert_eq!((st.state, st.count, st.profiles), (ReadState::Ok, 2, 2));
}

/// VFS-6 AC-5 (R5): the reader's connection may read only the allowed tables and columns;
/// history (`moz_historyvisits`, `moz_places.title`, visit counts) cannot be read.
#[test]
fn ac5_only_bookmark_tables_can_be_read() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("places.sqlite");
    testing::create_places(&db);
    {
        let conn = Connection::open(&db).unwrap();
        Places { conn: &conn }.bookmark(MENU, Some("Uno"), "https://uno.es/", 10);
    }
    let conn = firefox::open_guarded(&db).unwrap();
    for sql in [
        "SELECT place_id FROM moz_historyvisits",
        "SELECT title FROM moz_places",
        "SELECT visit_count FROM moz_places",
        "SELECT url FROM moz_places JOIN moz_historyvisits h ON h.place_id = moz_places.id",
        "SELECT * FROM moz_places",
        "SELECT input FROM moz_inputhistory",
        "SELECT host FROM moz_origins",
        "SELECT content FROM moz_annos",
        "INSERT INTO moz_bookmarks(type) VALUES(1)",
    ] {
        assert!(conn.prepare(sql).is_err(), "allowed: {sql}");
    }
    let url: String = conn
        .query_row("SELECT url FROM moz_places", [], |r| r.get(0))
        .unwrap();
    assert_eq!(url, "https://uno.es/");
    drop(conn);
    // The whole reader runs under that authorizer, so succeeding means it read nothing else.
    assert_eq!(firefox::read_places(&db).unwrap().bookmarks.len(), 1);
}

/// VFS-6 AC-9: a refresh with `places.sqlite` and its WAL unchanged since the last read makes
/// no copy and reads nothing (the read time stays); a change is read at the next refresh.
#[test]
fn ac9_unchanged_profile_is_not_read_again() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ff = FakeFirefox::new(&tmp.path().join("Firefox"));
    let dir = ff.add_profile("default", "p.default");
    let bm = Bookmarks::open(&tmp.path().join("app/bookmarks.db"), ff.locator()).unwrap();
    bm.set_enabled(true).unwrap();
    {
        let conn = Connection::open(dir.join("places.sqlite")).unwrap();
        Places { conn: &conn }.bookmark(MENU, Some("Uno"), "https://uno.es/", 10);
    }
    let first = bm.refresh().unwrap();
    assert_eq!(first.count, 1);
    std::thread::sleep(Duration::from_millis(1100));
    let again = bm.refresh().unwrap();
    assert_eq!(
        again.last_read, first.last_read,
        "an unchanged profile was read again"
    );
    {
        let conn = Connection::open(dir.join("places.sqlite")).unwrap();
        Places { conn: &conn }.bookmark(MENU, Some("Dos"), "https://dos.es/", 10);
    }
    let changed = bm.refresh().unwrap();
    assert_eq!(changed.count, 2);
    assert!(changed.last_read > first.last_read);
}
