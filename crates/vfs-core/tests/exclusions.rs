//! The switch for the default folder exclusions, through the public API (VFS-33).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vfs_core::{IndexProgress, ProgressSink, Vfs};

struct NullSink;
impl ProgressSink for NullSink {
    fn on_progress(&self, _: &IndexProgress) {}
}

/// VFS-33 AC-8, R8: the switch is off by default and stored when set; with the indexer
/// running, turning it on walks every root again and indexes the excluded folders.
#[test]
fn the_switch_is_stored_and_walks_again() {
    let dir = tempfile::tempdir().unwrap();
    let tree = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tree.path().join("web/node_modules/pad")).unwrap();
    std::fs::write(tree.path().join("web/node_modules/pad/index.js"), b"x").unwrap();
    let vfs = Vfs::open(&dir.path().join("index.db")).unwrap();
    assert!(!vfs.include_all_folders().unwrap());
    vfs.add_root(tree.path()).unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    let hits = |vfs: &Vfs| {
        vfs.search(&vfs_core::SearchQuery {
            text: "index.js".into(),
            filters: Default::default(),
            limit: 10,
            offset: 0,
            sort: Default::default(),
            scope: Default::default(),
        })
        .unwrap()
        .hits
        .len()
    };
    assert_eq!(hits(&vfs), 0, "excluded by default");

    // Without a running indexer the setting is still stored.
    vfs.set_include_all_folders(false).unwrap();
    assert!(!vfs.include_all_folders().unwrap());

    vfs.start_indexer(Arc::new(NullSink)).unwrap();
    vfs.set_include_all_folders(true).unwrap();
    assert!(vfs.include_all_folders().unwrap());
    let deadline = Instant::now() + Duration::from_secs(30);
    while hits(&vfs) == 0 {
        assert!(Instant::now() < deadline, "never walked again");
        std::thread::sleep(Duration::from_millis(100));
    }
    vfs.shutdown();
}
