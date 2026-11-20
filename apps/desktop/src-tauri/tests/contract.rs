//! Keeps the frontend's closed sets (`apps/desktop/ui/contract.js`) in step with the Rust
//! enums whose serde values cross the IPC boundary (VFS-1 X1, AC-12): a variant added,
//! renamed or removed on either side fails here.

use std::collections::BTreeMap;

use serde::Serialize;
use vfs_bookmarks::{BookmarkRoot, ReadState, SortOrder, Source, SourceHit};
use vfs_core::api::RevealOutcome;
use vfs_core::{
    DriveKind, Error, FileKind, IndexerState, LinkKind, MatchKind, RootStatus, TruncatedBy,
};

/// `export const Name = Object.freeze({ KEY: 'value', ... });` of contract.js, as
/// name -> values in source order.
fn js_sets(src: &str) -> BTreeMap<String, Vec<String>> {
    let mut sets = BTreeMap::new();
    for chunk in src.split("export const ").skip(1) {
        let Some((name, rest)) = chunk.split_once(" = Object.freeze({") else {
            continue;
        };
        let body = &rest[..rest.find("})").expect("unclosed set")];
        let values = body
            .split('\'')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect();
        sets.insert(name.trim().to_string(), values);
    }
    sets
}

fn serde_names<T: Serialize>(all: &[T]) -> Vec<String> {
    all.iter()
        .map(|v| match serde_json::to_value(v).unwrap() {
            serde_json::Value::String(s) => s,
            other => panic!("not a string enum: {other}"),
        })
        .collect()
}

/// The match arms have no wildcard, so a new variant stops this from compiling until it is
/// listed here (and then in contract.js).
fn all_file_kinds() -> Vec<FileKind> {
    use FileKind::*;
    let all = vec![Other, Document, Photo, Video, Audio, Folder];
    for k in &all {
        match k {
            Other | Document | Photo | Video | Audio | Folder => {}
        }
    }
    all
}

fn all_root_statuses() -> Vec<RootStatus> {
    use RootStatus::*;
    let all = vec![Pending, Scanning, Ready, Unavailable, Removing];
    for s in &all {
        match s {
            Pending | Scanning | Ready | Unavailable | Removing => {}
        }
    }
    all
}

fn all_indexer_states() -> Vec<IndexerState> {
    use IndexerState::*;
    let all = vec![Idle, Scanning, Paused];
    for s in &all {
        match s {
            Idle | Scanning | Paused => {}
        }
    }
    all
}

fn all_truncations() -> Vec<TruncatedBy> {
    use TruncatedBy::*;
    let all = vec![Cap, Deadline];
    for t in &all {
        match t {
            Cap | Deadline => {}
        }
    }
    all
}

fn all_match_kinds() -> Vec<MatchKind> {
    use MatchKind::*;
    let all = vec![Name, Path, Fuzzy, Target];
    for m in &all {
        match m {
            Name | Path | Fuzzy | Target => {}
        }
    }
    all
}

fn all_link_kinds() -> Vec<LinkKind> {
    use LinkKind::*;
    let all = vec![Shortcut, Internet, Symlink];
    for k in &all {
        match k {
            Shortcut | Internet | Symlink => {}
        }
    }
    all
}

fn all_drive_kinds() -> Vec<DriveKind> {
    use DriveKind::*;
    let all = vec![Local, Network, Removable, Unknown];
    for d in &all {
        match d {
            Local | Network | Removable | Unknown => {}
        }
    }
    all
}

fn all_reveal_outcomes() -> Vec<RevealOutcome> {
    use RevealOutcome::*;
    let all = vec![Revealed, OpenedParent];
    for r in &all {
        match r {
            Revealed | OpenedParent => {}
        }
    }
    all
}

fn all_sources() -> Vec<Source> {
    use Source::*;
    let all = vec![Files, Bookmarks];
    for s in &all {
        match s {
            Files | Bookmarks => {}
        }
    }
    all
}

fn all_sort_orders() -> Vec<SortOrder> {
    use SortOrder::*;
    let all = vec![Relevance, Newest, Oldest, Name, Size, Source];
    for s in &all {
        match s {
            Relevance | Newest | Oldest | Name | Size | Source => {}
        }
    }
    all
}

fn all_bookmark_roots() -> Vec<BookmarkRoot> {
    use BookmarkRoot::*;
    let all = vec![Menu, Toolbar, Unfiled, Mobile];
    for r in &all {
        match r {
            Menu | Toolbar | Unfiled | Mobile => {}
        }
    }
    all
}

fn all_read_states() -> Vec<ReadState> {
    use ReadState::*;
    let all = vec![Never, Reading, Ok, FromBackup, NoFirefox, Failed];
    for r in &all {
        match r {
            Never | Reading | Ok | FromBackup | NoFirefox | Failed => {}
        }
    }
    all
}

fn all_error_codes() -> Vec<String> {
    let all = vec![
        Error::NotFound(String::new()),
        Error::NotADirectory(String::new()),
        Error::OverlappingRoot {
            relation: vfs_core::OverlapRelation::Same,
            existing: String::new(),
        },
        Error::Unavailable(String::new()),
        Error::SchemaTooNew,
        Error::Db(rusqlite::Error::InvalidQuery),
        Error::Io(std::io::Error::other("x")),
    ];
    for e in &all {
        match e {
            Error::NotFound(_)
            | Error::NotADirectory(_)
            | Error::OverlappingRoot { .. }
            | Error::Unavailable(_)
            | Error::SchemaTooNew
            | Error::Db(_)
            | Error::Io(_) => {}
        }
    }
    all.iter().map(|e| e.code().to_string()).collect()
}

/// VFS-1 X1, AC-12: every set of contract.js equals the serde values of its Rust enum, and
/// contract.js has no set this test does not check (besides the UI's own `UiErrorCode`).
#[test]
fn contract_js_matches_the_rust_enums() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/contract.js");
    let sets = js_sets(&std::fs::read_to_string(path).unwrap());
    let expected: BTreeMap<String, Vec<String>> = [
        ("FileKind", serde_names(&all_file_kinds())),
        ("RootStatus", serde_names(&all_root_statuses())),
        ("IndexerState", serde_names(&all_indexer_states())),
        ("TruncatedBy", serde_names(&all_truncations())),
        ("MatchKind", serde_names(&all_match_kinds())),
        ("LinkKind", serde_names(&all_link_kinds())),
        ("DriveKind", serde_names(&all_drive_kinds())),
        ("RevealOutcome", serde_names(&all_reveal_outcomes())),
        ("ErrorCode", all_error_codes()),
        ("Source", serde_names(&all_sources())),
        ("BookmarkRoot", serde_names(&all_bookmark_roots())),
        ("ReadState", serde_names(&all_read_states())),
        ("SortOrder", serde_names(&all_sort_orders())),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    for (name, values) in &expected {
        let mut js = sets
            .get(name)
            .unwrap_or_else(|| panic!("{name} missing"))
            .clone();
        let mut rust = values.clone();
        js.sort();
        rust.sort();
        assert_eq!(js, rust, "{name}");
    }
    let extra: Vec<&String> = sets
        .keys()
        .filter(|k| !expected.contains_key(*k) && *k != "UiErrorCode")
        .collect();
    assert!(extra.is_empty(), "unchecked sets in contract.js: {extra:?}");
}

/// VFS-6 AC-11: the `source` tag of a result is spelled like the `Source` it comes from, so
/// the UI tells results apart with the one `Source` set.
#[test]
fn source_hit_tags_are_the_source_values() {
    let file = SourceHit::Files(vfs_core::SearchHit {
        id: 1,
        name: "a".into(),
        dir: "/d".into(),
        kind: FileKind::Other,
        size: 0,
        modified: 0,
        matched: MatchKind::Name,
        is_dir: false,
        copies: Vec::new(),
        link: None,
    });
    let mark = SourceHit::Bookmarks(vfs_bookmarks::BookmarkHit {
        id: 2,
        is_folder: false,
        title: "t".into(),
        url: "https://x.es/".into(),
        root: BookmarkRoot::Menu,
        folder: vec!["Recetas".into()],
        profile: None,
        added: 3,
        matched: MatchKind::Path,
        openable: true,
    });
    for hit in [file, mark] {
        let v = serde_json::to_value(&hit).unwrap();
        assert_eq!(v["source"], serde_json::to_value(hit.source()).unwrap());
    }
}
