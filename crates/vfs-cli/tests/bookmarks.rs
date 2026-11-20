//! `vfs search --source` (VFS-6 AC-10).

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
