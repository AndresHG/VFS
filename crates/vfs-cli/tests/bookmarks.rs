//! `vfs search --source` (VFS-6 AC-10) and `vfs search --names-only` (VFS-38 AC-3).

use std::path::Path;
use std::process::Command;

use rusqlite::Connection;
use vfs_bookmarks::testing::{FakeFirefox, Places, MENU};

fn vfs(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_vfs"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "vfs {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// VFS-6 AC-10: `--source bookmarks` prints bookmarks only, `--source files` files only and
/// `--source all` both, once `vfs bookmarks on` has read the profile.
#[test]
fn ac10_cli_searches_the_chosen_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("datos");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("Recetas.pdf"), "x").unwrap();
    let db = tmp.path().join("index.db");
    vfs(&["index", "--db", s(&db), s(&data)]);

    let mut ff = FakeFirefox::new(&tmp.path().join("Firefox"));
    let profile = ff.add_profile("default", "p.default");
    {
        let conn = Connection::open(profile.join("places.sqlite")).unwrap();
        Places { conn: &conn }.bookmark(MENU, Some("Recetas de la abuela"), "https://r.es/", 10);
    }
    let status = vfs(&[
        "bookmarks",
        "--db",
        s(&db),
        "on",
        "--firefox-root",
        s(&ff.root),
    ]);
    assert!(status.contains("\"count\": 1"), "{status}");

    let only_marks = vfs(&["search", "--db", s(&db), "--source", "bookmarks", "recetas"]);
    assert!(
        only_marks.contains("marcador") && !only_marks.contains("archivo"),
        "{only_marks}"
    );
    let only_files = vfs(&["search", "--db", s(&db), "--source", "files", "recetas"]);
    assert!(
        only_files.contains("Recetas.pdf") && !only_files.contains("abuela"),
        "{only_files}"
    );
    let all = vfs(&["search", "--db", s(&db), "--source", "all", "recetas"]);
    assert!(all.contains("archivo") && all.contains("marcador"), "{all}");

    vfs(&["bookmarks", "--db", s(&db), "off"]);
    let off = vfs(&["search", "--db", s(&db), "--source", "bookmarks", "recetas"]);
    assert!(!off.contains("marcador"), "{off}");
}

/// VFS-38 AC-3 (R14): `vfs search --names-only` gives the same hits as the core call with
/// `scope: "names"`, and fewer than without the flag (the photo found by its folder).
#[test]
fn names_only_flag_matches_the_core_call() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("D");
    std::fs::create_dir_all(data.join("Peñalara julio")).unwrap();
    std::fs::write(data.join("Peñalara julio/IMG_0412.jpg"), "x").unwrap();
    std::fs::write(data.join("Excursión Peñalara.docx"), "x").unwrap();
    let db = tmp.path().join("index.db");
    vfs(&["index", "--db", s(&db), s(&data)]);

    let hits = |out: &str| -> Vec<(bool, i64)> {
        let v: serde_json::Value = serde_json::from_str(out).unwrap();
        v["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| (h["isDir"].as_bool().unwrap(), h["id"].as_i64().unwrap()))
            .collect()
    };
    let cli = hits(&vfs(&[
        "search",
        "--db",
        s(&db),
        "--json",
        "--names-only",
        "peñalara",
    ]));
    let core = vfs_core::Vfs::open(&db)
        .unwrap()
        .search(&vfs_core::SearchQuery {
            text: "peñalara".into(),
            filters: Default::default(),
            limit: 20,
            offset: 0,
            sort: Default::default(),
            scope: vfs_core::Scope::Names,
        })
        .unwrap();
    let core: Vec<(bool, i64)> = core.hits.iter().map(|h| (h.is_dir, h.id)).collect();
    assert_eq!(cli, core);
    assert_eq!(cli.len(), 2, "the document and the folder: {cli:?}");
    let all = hits(&vfs(&["search", "--db", s(&db), "--json", "peñalara"]));
    assert_eq!(all.len(), 3, "{all:?}");
}
