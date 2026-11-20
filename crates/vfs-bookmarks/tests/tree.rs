//! The bookmark folder tree (VFS-6 AC-13) and the read-only invariant (AC-15).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use rusqlite::Connection;
use vfs_bookmarks::firefox;
use vfs_bookmarks::sources::search_sources;
use vfs_bookmarks::testing::{FakeFirefox, Places, MENU, UNFILED};
use vfs_bookmarks::{BookmarkHit, BookmarkRoot, Bookmarks, Source, SourceHit, SourceOptions};
use vfs_core::{SearchQuery, Vfs};

const TOPICS: [&str; 30] = [
    "Recetas",
    "Jardín",
    "Salud",
    "Viajes",
    "Historia",
    "Música",
    "Bancos",
    "Nietos",
    "Costura",
    "Pintura",
    "Poesía",
    "Fotografía",
    "Cine",
    "Teatro",
    "Libros",
    "Deporte",
    "Coches",
    "Casa",
    "Ropa",
    "Regalos",
    "Médicos",
    "Pueblo",
    "Iglesia",
    "Vecinos",
    "Impuestos",
    "Pensión",
    "Seguros",
    "Ordenador",
    "Teléfono",
    "Varios",
];
const DEPTH: usize = 12;

struct World {
    _tmp: tempfile::TempDir,
    vfs: Vfs,
    bm: Bookmarks,
    profile: std::path::PathBuf,
}

/// Menu: 30 topic folders, each with "Antiguo" and "Pendiente" subfolders holding one
/// bookmark each and 8 numbered subfolders (300 folders in all); a 12-level chain
/// "Temas/Nivel 1/…/Nivel 12" with a bookmark at the bottom; "Recétas viejas" (accent) in
/// "Otros marcadores"; and a folder "Orden" whose bookmarks were saved Zeta, Alfa, Medio.
fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let mut ff = FakeFirefox::new(&tmp.path().join("Firefox"));
    let profile = ff.add_profile("default", "p.default");
    {
        let conn = Connection::open(profile.join("places.sqlite")).unwrap();
        conn.execute_batch("BEGIN").unwrap();
        let p = Places { conn: &conn };
        for (i, topic) in TOPICS.iter().enumerate() {
            let t = p.folder(MENU, topic);
            for sub in ["Antiguo", "Pendiente"] {
                let f = p.folder(t, sub);
                p.bookmark(
                    f,
                    Some(&format!("{sub} de {topic}")),
                    &format!("https://t{i}.es/{sub}"),
                    10,
                );
            }
            for n in 0..8 {
                p.folder(t, &format!("Apartado {n}"));
            }
        }
        let mut parent = p.folder(MENU, "Temas");
        for level in 1..=DEPTH {
            parent = p.folder(parent, &format!("Nivel {level}"));
        }
        p.bookmark(parent, Some("Lo más hondo"), "https://hondo.es/", 10);
        p.folder(UNFILED, "Recétas viejas");
        let orden = p.folder(MENU, "Orden");
        for t in ["Zeta", "Alfa", "Medio"] {
            p.bookmark(
                orden,
                Some(t),
                &format!("https://{}.es/", t.to_lowercase()),
                10,
            );
        }
        conn.execute_batch("COMMIT").unwrap();
    }
    let data = tmp.path().join("datos");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("Recetas antiguas.pdf"), "x").unwrap();
    let vfs = Vfs::open(&tmp.path().join("app/index.db")).unwrap();
    vfs.add_root(&data).unwrap();
    vfs.scan_to_completion(&std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    let bm = Bookmarks::open(&tmp.path().join("app/bookmarks.db"), ff.locator()).unwrap();
    bm.set_enabled(true).unwrap();
    bm.refresh().unwrap();
    World {
        _tmp: tmp,
        vfs,
        bm,
        profile,
    }
}

fn q(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        filters: Default::default(),
        limit: 500,
        offset: 0,
        sort: Default::default(),
    }
}

fn marks(w: &World, text: &str, inside: Option<i64>) -> Vec<BookmarkHit> {
    let options = SourceOptions {
        sources: if inside.is_some() {
            Vec::new()
        } else {
            vec![Source::Bookmarks]
        },
        bookmark_folder: inside,
        ..Default::default()
    };
    search_sources(&w.vfs, Some(&w.bm), &q(text), &options)
        .unwrap()
        .hits
        .into_iter()
        .map(|h| match h {
            SourceHit::Bookmarks(b) => b,
            SourceHit::Files(f) => panic!("a file inside a bookmark folder: {f:?}"),
        })
        .collect()
}

/// VFS-6 AC-13: a 12-level tree of over 300 folders is read whole; folder names are results
/// with their own paths, duplicates and accents included; the folder filter keeps only what
/// is inside one of two same-named folders, never files; with no words it lists a folder's
/// contents in Firefox's order and nesting.
#[test]
fn ac13_deep_tree_folders_are_results_and_filters() {
    let w = world();
    let (tree, _) = firefox::read_profile(&w.profile, &w._tmp.path().join("work")).unwrap();
    assert!(tree.folders.len() > 300, "{}", tree.folders.len());
    let deepest = tree.folders.iter().find(|f| f.title == "Nivel 12").unwrap();
    assert_eq!(deepest.path.len(), DEPTH, "{:?}", deepest.path);
    assert_eq!(deepest.path[0], "Temas");
    let st = w.bm.status();
    assert_eq!(st.count, 64);
    assert_eq!(
        st.roots.iter().map(|r| r.root).collect::<Vec<_>>(),
        vec![BookmarkRoot::Menu, BookmarkRoot::Unfiled]
    );

    // "recetas" finds both folders; the accent and case do not matter.
    let hits = marks(&w, "recetas", None);
    let folders: Vec<(&str, &Vec<String>)> = hits
        .iter()
        .filter(|h| h.is_folder)
        .map(|h| (h.title.as_str(), &h.folder))
        .collect();
    assert!(folders.contains(&("Recetas", &Vec::new())), "{folders:?}");
    assert!(
        folders.contains(&("Recétas viejas", &Vec::new())),
        "{folders:?}"
    );
    // A folder path segment finds what is below it, as for files.
    assert!(hits.iter().any(|h| h.title == "Antiguo de Recetas"));

    // 30 folders named "Antiguo", each with its own path.
    let antiguos: Vec<BookmarkHit> = marks(&w, "antiguo", None)
        .into_iter()
        .filter(|h| h.is_folder)
        .collect();
    assert_eq!(antiguos.len(), TOPICS.len());
    let in_jardin = antiguos.iter().find(|h| h.folder == ["Jardín"]).unwrap();
    let inside = marks(&w, "", Some(in_jardin.id));
    assert_eq!(
        inside.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(),
        ["Antiguo de Jardín"]
    );
    // Words and the folder filter together: only inside, and no file even when one matches.
    let inside = marks(&w, "antiguo", Some(in_jardin.id));
    assert_eq!(inside.len(), 1);

    // A typo in a folder name still finds the folder (6+ letters).
    assert!(marks(&w, "pendinte", None)
        .iter()
        .any(|h| h.is_folder && h.title == "Pendiente"));

    // Firefox's order and nesting: "Temas" lists Nivel 1, Nivel 2, … then the bookmark.
    let temas = marks(&w, "temas", None)
        .into_iter()
        .find(|h| h.is_folder && h.title == "Temas")
        .unwrap();
    let listed: Vec<String> = marks(&w, "", Some(temas.id))
        .into_iter()
        .map(|h| h.title)
        .collect();
    let mut expected: Vec<String> = (1..=DEPTH).map(|n| format!("Nivel {n}")).collect();
    expected.push("Lo más hondo".into());
    assert_eq!(listed, expected);
    let orden = marks(&w, "orden", None)
        .into_iter()
        .find(|h| h.is_folder)
        .unwrap();
    let listed: Vec<String> = marks(&w, "", Some(orden.id))
        .into_iter()
        .map(|h| h.title)
        .collect();
    assert_eq!(listed, ["Zeta", "Alfa", "Medio"]);

    // A root as the filter: the unsorted bookmarks, browsed in Firefox's order.
    let unfiled = st
        .roots
        .iter()
        .find(|r| r.root == BookmarkRoot::Unfiled)
        .unwrap();
    let listed: Vec<String> = marks(&w, "", Some(unfiled.id))
        .into_iter()
        .map(|h| h.title)
        .collect();
    assert_eq!(listed, ["Recétas viejas"]);
}

fn listing(dir: &Path) -> BTreeMap<String, (u64, SystemTime, bool)> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let m = e.metadata().unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                (m.len(), m.modified().unwrap(), m.permissions().readonly()),
            )
        })
        .collect()
}

fn set_readonly(path: &Path, on: bool) {
    let mut p = std::fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    p.set_readonly(on);
    std::fs::set_permissions(path, p).unwrap();
}

/// VFS-6 AC-15 (R14): with Firefox's files and the profile folder read-only, the read still
/// succeeds (nothing needs write access) and leaves the folder exactly as it was.
#[test]
fn ac15_reads_a_read_only_profile_and_changes_nothing() {
    let w = world();
    let files: Vec<std::path::PathBuf> = std::fs::read_dir(&w.profile)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    for f in &files {
        set_readonly(f, true);
    }
    set_readonly(&w.profile, true);
    let before = listing(&w.profile);

    let read = firefox::read_profile(&w.profile, &w._tmp.path().join("work2"));
    let after = listing(&w.profile);
    set_readonly(&w.profile, false);
    for f in &files {
        set_readonly(f, false);
    }
    let (tree, origin) = read.unwrap();
    assert_eq!(origin, firefox::Origin::Database);
    assert_eq!(tree.bookmarks.len(), 64);
    assert_eq!(after, before);
}
