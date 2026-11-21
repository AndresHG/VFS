//! Shortcuts and links in search (VFS-18 AC-4, AC-5, AC-7, AC-10): link rows written with
//! `db::ingest`, targets stored as the deferred link pass stores them.

use rusqlite::Connection;
use tempfile::TempDir;
use vfs_core::db::{self, ingest};
use vfs_core::{
    FileKind, LinkKind, MatchKind, SearchFilters, SearchHit, SearchQuery, SearchResponse, Vfs,
};

struct Fixture {
    _dir: TempDir,
    vfs: Vfs,
    conn: Connection,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    let vfs = Vfs::open(&path).unwrap();
    let conn = db::open_rw(&path).unwrap();
    Fixture {
        _dir: dir,
        vfs,
        conn,
    }
}

impl Fixture {
    fn root(&mut self, path: &str) -> (i64, i64) {
        let tx = self.conn.transaction().unwrap();
        let root = ingest::insert_root(&tx, "/", path).unwrap();
        let dir = tx
            .query_row(
                "SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ''",
                [root],
                |r| r.get(0),
            )
            .unwrap();
        tx.commit().unwrap();
        (root, dir)
    }

    fn dir(&mut self, root: i64, parent: i64, rel: &str) -> i64 {
        let tx = self.conn.transaction().unwrap();
        let id = ingest::insert_dir(&tx, root, parent, rel, 1).unwrap();
        tx.commit().unwrap();
        id
    }

    fn file(&mut self, dir: i64, name: &str) -> i64 {
        let tx = self.conn.transaction().unwrap();
        let id = ingest::insert_file(
            &tx,
            dir,
            &ingest::NewFile {
                name,
                size: 1,
                mtime: 1,
                lossy: false,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        id
    }

    /// Stores `target` for the link `file_id` as the deferred pass does.
    fn target(&mut self, file_id: i64, target: Option<&str>) {
        let tx = self.conn.transaction().unwrap();
        ingest::set_link_target(&tx, file_id, Ok(target)).unwrap();
        tx.commit().unwrap();
    }

    fn search(&self, text: &str, filters: SearchFilters) -> SearchResponse {
        self.vfs
            .search(&SearchQuery {
                text: text.into(),
                filters,
                limit: 500,
                offset: 0,
            })
            .unwrap()
    }

    fn hit(&self, text: &str, id: i64) -> Option<SearchHit> {
        self.search(text, SearchFilters::default())
            .hits
            .into_iter()
            .find(|h| !h.is_dir && h.id == id)
    }
}

/// VFS-18 AC-4: a shortcut is found by its target, by its own folder and its target
/// together, and by its own name; every hit carries its kind and target.
#[test]
fn found_by_target_folder_and_name() {
    let mut f = fixture();
    let (root, top) = f.root("/archivo");
    let familia = f.dir(root, top, "Familia");
    let lnk = f.file(familia, "Fotos.lnk");
    f.target(lnk, Some(r"D:\Boda 2003"));

    let by_target = f.hit("boda", lnk).expect("found by target");
    assert_eq!(by_target.matched, MatchKind::Target);
    let link = by_target.link.expect("link info");
    assert_eq!(link.kind, LinkKind::Shortcut);
    assert_eq!(link.target.as_deref(), Some(r"D:\Boda 2003"));
    assert!(!link.openable);

    let mixed = f
        .hit("familia boda", lnk)
        .expect("found by folder + target");
    assert_eq!(mixed.matched, MatchKind::Target);
    let by_name = f.hit("fotos", lnk).expect("found by name");
    assert_eq!(by_name.matched, MatchKind::Name);
    assert!(by_name.link.is_some());
    assert!(f.hit("viaje", lnk).is_none());
    // Accents and case fold in the target too.
    f.target(lnk, Some(r"D:\Álbum NIÑOS"));
    assert!(f.hit("album ninos", lnk).is_some());
}

/// VFS-18 AC-5: a shortcut is left out when the same search returns its target (an indexed
/// file or folder), and kept when the search finds only the shortcut.
#[test]
fn hidden_when_the_target_is_a_result() {
    let mut f = fixture();
    let (root, top) = f.root("/archivo");
    let fotos = f.dir(root, top, "Fotos");
    let boda = f.dir(root, fotos, "Fotos/Boda 2003");
    let album = f.file(boda, "album.jpg");
    let accesos = f.dir(root, top, "Accesos");
    let to_file = f.file(accesos, "mi album.lnk");
    f.target(to_file, Some("/archivo/Fotos/Boda 2003/album.jpg"));
    let to_dir = f.file(accesos, "la boda.lnk");
    f.target(to_dir, Some("/archivo/Fotos/Boda 2003"));

    // "album" finds the file itself and the shortcut by its name: the file wins.
    let r = f.search("album", SearchFilters::default());
    assert!(r.hits.iter().any(|h| !h.is_dir && h.id == album));
    assert!(!r.hits.iter().any(|h| !h.is_dir && h.id == to_file));
    // "boda" finds the folder: the shortcut to it is left out.
    let r = f.search("boda", SearchFilters::default());
    assert!(r.hits.iter().any(|h| h.is_dir && h.id == boda));
    assert!(!r.hits.iter().any(|h| !h.is_dir && h.id == to_dir));
    // "accesos" finds the shortcuts by their own folder, not their targets: both stay.
    let r = f.search("accesos", SearchFilters::default());
    for id in [to_file, to_dir] {
        assert!(r.hits.iter().any(|h| !h.is_dir && h.id == id), "{id}");
    }
    // A target outside every chosen folder never hides a shortcut.
    let usb = f.file(accesos, "viaje.lnk");
    f.target(usb, Some(r"Q:\viaje\album.jpg"));
    assert!(f.hit("album", usb).is_some());
}

/// VFS-18 AC-7: Internet Shortcuts are found by a word of their address; only `http` and
/// `https` addresses are openable.
#[test]
fn internet_shortcuts() {
    let mut f = fixture();
    let (_, top) = f.root("/archivo");
    let banco = f.file(top, "Banco.url");
    f.target(banco, Some("https://www.mibanco.es/acceso"));
    let local = f.file(top, "x.url");
    f.target(local, Some("file:///C:/mibanco/notas.txt"));
    let r = f.search("mibanco", SearchFilters::default());
    let get = |id| r.hits.iter().find(|h| h.id == id).expect("hit");
    let b = get(banco).link.clone().unwrap();
    assert_eq!(b.kind, LinkKind::Internet);
    assert!(b.openable);
    let l = get(local).link.clone().unwrap();
    assert!(!l.openable);
}

/// VFS-18 D10: a type filter other than "Otros" leaves links out; date and size filters
/// apply to the link file itself. A link with no target is still a link hit by its name.
#[test]
fn filters_and_links_without_target() {
    let mut f = fixture();
    let (_, top) = f.root("/archivo");
    let lnk = f.file(top, "Panel.lnk");
    f.target(lnk, Some(r"C:\Fotos\boda.jpg"));
    let photos = SearchFilters {
        kinds: vec![FileKind::Photo],
        ..SearchFilters::default()
    };
    assert!(f.search("boda", photos).hits.is_empty());
    let other = SearchFilters {
        kinds: vec![FileKind::Other],
        ..SearchFilters::default()
    };
    assert_eq!(f.search("boda", other).hits.len(), 1);
    f.target(lnk, None);
    let h = f.hit("panel", lnk).unwrap();
    assert_eq!(h.link.unwrap().target, None);
    assert!(f.hit("boda", lnk).is_none());
}

/// VFS-18 AC-10: an index at the previous schema version gets the `links` table with its
/// existing shortcuts pending, without a rescan.
#[test]
fn migration_queues_existing_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    {
        let mut conn = db::open_rw(&path).unwrap();
        let tx = conn.transaction().unwrap();
        let root = ingest::insert_root(&tx, "/", "/a").unwrap();
        let top: i64 = tx
            .query_row("SELECT id FROM dirs WHERE root_id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        for name in ["x.LNK", "y.url", "z.txt", ".lnk"] {
            ingest::insert_file(
                &tx,
                top,
                &ingest::NewFile {
                    name,
                    size: 1,
                    mtime: 1,
                    lossy: false,
                },
            )
            .unwrap();
        }
        tx.execute_batch("DROP TABLE links; UPDATE dirs SET scan_state = 0;")
            .unwrap();
        tx.pragma_update(None, "user_version", 4).unwrap();
        tx.commit().unwrap();
    }
    let conn = db::open_rw(&path).unwrap();
    let pending = ingest::pending_links(&conn, 10).unwrap();
    let kinds: Vec<LinkKind> = pending.iter().map(|p| p.1).collect();
    assert_eq!(kinds, [LinkKind::Shortcut, LinkKind::Internet]);
    let queued: i64 = conn
        .query_row("SELECT COUNT(*) FROM dirs WHERE scan_state <> 0", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(queued, 0);
}
