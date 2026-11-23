//! The user's file filter and the content size limit, through the public API (VFS-34).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vfs_core::{Error, IndexProgress, PatternError, ProgressSink, SearchQuery, Vfs};

struct NullSink;
impl ProgressSink for NullSink {
    fn on_progress(&self, _: &IndexProgress) {}
}

fn hits(vfs: &Vfs, text: &str) -> usize {
    vfs.search(&SearchQuery {
        text: text.into(),
        filters: Default::default(),
        limit: 100,
        offset: 0,
        sort: Default::default(),
        scope: Default::default(),
    })
    .unwrap()
    .hits
    .len()
}

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cond() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// VFS-34 AC-3, AC-4, AC-6, R7: a fresh index lists the defaults; with the indexer running,
/// adding `*.bak` takes the indexed copies out of search in the background, and removing it
/// walks again and brings them back. Refused patterns carry their reason.
#[test]
fn adding_and_removing_a_pattern_with_the_indexer_running() {
    let dir = tempfile::tempdir().unwrap();
    let tree = tempfile::tempdir().unwrap();
    for i in 0..20 {
        std::fs::write(tree.path().join(format!("copia{i}.bak")), b"x").unwrap();
    }
    std::fs::write(tree.path().join("informe.pdf"), b"x").unwrap();
    let vfs = Vfs::open(&dir.path().join("index.db")).unwrap();
    let defaults = vfs.file_exclusions().unwrap();
    assert!(!defaults.is_empty());
    vfs.add_root(tree.path()).unwrap();
    vfs.scan_to_completion(&AtomicBool::new(false)).unwrap();
    assert_eq!(hits(&vfs, "copia"), 20);

    vfs.start_indexer(Arc::new(NullSink)).unwrap();
    assert_eq!(vfs.count_file_exclusion("*.bak").unwrap(), 20);
    let list = vfs.add_file_exclusion("*.bak").unwrap();
    assert_eq!(list.last().map(String::as_str), Some("*.bak"));
    wait_for("the purge", || hits(&vfs, "copia") == 0);
    assert_eq!(hits(&vfs, "informe"), 1);

    assert!(matches!(
        vfs.add_file_exclusion("*.BAK"),
        Err(Error::InvalidPattern(PatternError::Duplicate))
    ));
    assert!(matches!(
        vfs.count_file_exclusion("*.*"),
        Err(Error::InvalidPattern(PatternError::NoLiteral))
    ));

    let list = vfs.remove_file_exclusion("*.bak").unwrap();
    assert!(!list.contains(&"*.bak".to_string()));
    wait_for("the walk after removing", || hits(&vfs, "copia") == 20);

    vfs.add_file_exclusion("*.pdf").unwrap();
    wait_for("the purge of *.pdf", || hits(&vfs, "informe") == 0);
    assert_eq!(vfs.reset_file_exclusions().unwrap(), defaults);
    wait_for("the walk after restoring", || hits(&vfs, "informe") == 1);
    vfs.shutdown();
}

/// VFS-34 R6: the content size limit has its default and is stored when set; 0 is kept
/// as "no limit".
#[test]
fn the_content_size_limit_is_stored() {
    let dir = tempfile::tempdir().unwrap();
    let vfs = Vfs::open(&dir.path().join("index.db")).unwrap();
    assert_eq!(
        vfs.content_max_mb().unwrap(),
        vfs_core::content::DEFAULT_CONTENT_MAX_MB
    );
    vfs.set_content_max_mb(200).unwrap();
    assert_eq!(vfs.content_max_mb().unwrap(), 200);
    vfs.set_content_max_mb(0).unwrap();
    assert_eq!(vfs.content_max_mb().unwrap(), 0);
    assert_eq!(vfs_core::content::content_max_bytes(0), i64::MAX as u64);
    assert_eq!(vfs_core::content::content_max_bytes(2), 2 << 20);
}
