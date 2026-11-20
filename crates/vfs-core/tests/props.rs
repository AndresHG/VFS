//! Property-based tests that try to break folding, the one-typo matcher, the query parser,
//! the FTS5 query construction, filters and ranking (VFS-1 AC-8, AC-10, AC-11). Indexes are
//! built with `db::ingest`, so they have the production layout, and kept to a handful of
//! rows per case so the suite stays fast. `VFS_STRESS=1` multiplies the case counts.

use std::collections::HashSet;
use std::path::PathBuf;

use proptest::prelude::*;
use proptest::sample::Index;
use rusqlite::Connection;
use tempfile::TempDir;
use vfs_core::db::{self, ingest};
use vfs_core::{
    fold_key, FileKind, MatchKind, Result, SearchConfig, SearchFilters, SearchHit, SearchQuery,
    SearchResponse, TruncatedBy, Vfs,
};

/// The properties are about which rows a query returns, not how fast: searches get a
/// deadline no run reaches, even on a loaded CI machine, so a deadline-cut list never
/// passes for a wrong answer (latency is AC-9's benchmark).
fn patient() -> SearchConfig {
    SearchConfig {
        deadline: std::time::Duration::from_secs(600),
    }
}

/// Opens an index with the [`patient`] deadline.
fn open_patient(path: &std::path::Path) -> Vfs {
    Vfs::open_with(path, patient()).unwrap()
}

/// Proptest's default case count, times 16 with `VFS_STRESS=1`. Failures are not persisted
/// to files: a shrunk failure becomes a plain `#[test]` below.
fn config() -> ProptestConfig {
    let base = ProptestConfig::default();
    let stress = std::env::var("VFS_STRESS").is_ok_and(|v| v == "1");
    ProptestConfig {
        cases: if stress { base.cases * 16 } else { base.cases },
        failure_persistence: None,
        ..base
    }
}

/// A fresh index in a temporary directory plus a writer connection.
/// Fields drop in order: the index closes before its directory is deleted, which Windows
/// refuses for open files (`TempDir` ignores the error, and every case leaked an index
/// until the CI runner's disk was full).
struct Ix {
    vfs: Vfs,
    conn: Connection,
    _dir: TempDir,
}

impl Ix {
    fn new() -> Ix {
        let dir = tempfile::tempdir().unwrap();
        let path: PathBuf = dir.path().join("index.db");
        let vfs = open_patient(&path);
        let conn = db::open_rw(&path).unwrap();
        Ix {
            _dir: dir,
            vfs,
            conn,
        }
    }

    /// Root `path` and the id of its root directory.
    fn root(&mut self, path: &str) -> (i64, i64) {
        let tx = self.conn.transaction().unwrap();
        let root = ingest::insert_root(&tx, "/", path).unwrap();
        let dir: i64 = tx
            .query_row(
                "SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ''",
                [root],
                |r| r.get(0),
            )
            .unwrap();
        tx.commit().unwrap();
        (root, dir)
    }

    fn dir(&mut self, root: i64, parent: i64, rel: &str, mtime: i64) -> i64 {
        let tx = self.conn.transaction().unwrap();
        let id = ingest::insert_dir(&tx, root, parent, rel, mtime).unwrap();
        tx.commit().unwrap();
        id
    }

    fn file(&mut self, dir: i64, name: &str, size: u64, mtime: i64) -> i64 {
        let tx = self.conn.transaction().unwrap();
        let id = ingest::insert_file(
            &tx,
            dir,
            &ingest::NewFile {
                name,
                size,
                mtime,
                lossy: false,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        id
    }

    fn query(
        &self,
        text: &str,
        filters: SearchFilters,
        limit: u32,
        offset: u32,
    ) -> Result<SearchResponse> {
        let r = self.vfs.search(&SearchQuery {
            text: text.to_string(),
            filters,
            limit,
            offset,
        })?;
        assert_ne!(
            r.truncated_by,
            Some(TruncatedBy::Deadline),
            "{text:?} was cut by the deadline"
        );
        Ok(r)
    }

    fn text(&self, text: &str) -> SearchResponse {
        self.query(text, SearchFilters::default(), 500, 0).unwrap()
    }
}

/// Fragments that stress folding: Latin with accents in NFC and NFD, lone combining marks,
/// Greek (final sigma, dialytika with tonos), Turkish dotted/dotless i, compatibility forms
/// (ligatures, `℡`, fullwidth, fractions, squared units, Kelvin and Ångström signs),
/// emoji with modifiers and ZWJ, RTL text and marks, Hangul, halfwidth kana with a
/// combining sound mark, Cherokee, and odd whitespace.
const FRAGMENTS: &[&str] = &[
    "a",
    "b",
    "n",
    "o",
    "t",
    "r",
    "i",
    "x",
    "Á",
    "á",
    "é",
    "í",
    "ó",
    "ú",
    "ñ",
    "Ñ",
    "ü",
    "ç",
    "e\u{301}",
    "n\u{303}",
    "u\u{308}",
    "I\u{307}",
    "\u{301}",
    "\u{308}",
    "\u{327}",
    "\u{345}",
    "Σ",
    "σ",
    "ς",
    "ΐ",
    "Ω",
    "\u{2126}",
    "İ",
    "ı",
    "I",
    "ﬁ",
    "ﬀ",
    "ﬃ",
    "℡",
    "Ａ",
    "ｚ",
    "½",
    "㎒",
    "ǅ",
    "ß",
    "ẞ",
    "\u{212A}",
    "\u{212B}",
    "😀",
    "👍🏽",
    "👨\u{200D}👩\u{200D}👧",
    "\u{200D}",
    "\u{200F}",
    "\u{202E}",
    "שלום",
    "مرحبا",
    "写真",
    "한국",
    "ﾊﾟ",
    "Ꭰ",
    "ꭰ",
    " ",
    "\u{a0}",
    "\u{3000}",
    "\t",
    ".",
    "_",
    "-",
    "0",
    "9",
    "Ǆ",
    "ǈ",
    "ǋ",
    "ȷ",
    "\u{1E9B}",
    "\u{FB06}",
];

fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::sample::select(FRAGMENTS).prop_map(str::to_string),
        1 => any::<char>().prop_map(String::from),
    ]
}

/// Strings biased toward the folding edge cases above, mixed with arbitrary Unicode.
fn unicode_string() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        prop::collection::vec(fragment(), 0..16).prop_map(|v| v.concat()),
    ]
}

proptest! {
    #![proptest_config(config())]

    /// VFS-1 AC-8, AC-10: `fold_key` is idempotent on arbitrary Unicode, so a folded name
    /// and a folded query agree whichever is folded twice.
    #[test]
    fn fold_key_is_idempotent(s in unicode_string()) {
        let once = fold_key(&s);
        prop_assert_eq!(fold_key(&once), once);
    }
}

/// A file name the query side can reproduce: no `/` (a path separator on every platform),
/// no `\` (the query folds `\` to `/` by design, search.rs module doc; also excluded when a
/// compatibility form such as `＼` folds to it) and no NUL (impossible in a file name). It
/// must fold to at least 3 characters with something other than whitespace.
fn name_strategy() -> impl Strategy<Value = String> {
    unicode_string().prop_filter("not a reproducible file name", |s| {
        let k = fold_key(s);
        !s.contains(['/', '\\', '\0'])
            && !k.contains('\\')
            && k.chars().count() >= 3
            && !k.trim().is_empty()
    })
}

/// The chars `[start, start + len)` of `chars`, with `len >= 3`.
fn substring(chars: &[char], len: Index, start: Index) -> String {
    let n = chars.len();
    let len = 3 + len.index(n - 2);
    let start = start.index(n - len + 1);
    chars[start..start + len].iter().collect()
}

proptest! {
    #![proptest_config(config())]

    /// VFS-1 AC-10: an indexed file is found by every substring of its folded name of 3 or
    /// more characters (whitespace inside splits it into terms, each still in the name). A
    /// blank substring is an empty query, which by design returns nothing, so it is skipped.
    #[test]
    fn every_folded_substring_finds_the_file(
        names in prop::collection::hash_set(name_strategy(), 1..4),
        pick in any::<Index>(),
        len in any::<Index>(),
        start in any::<Index>(),
    ) {
        let names: Vec<String> = names.into_iter().collect();
        let target = pick.get(&names);
        let chars: Vec<char> = fold_key(target).chars().collect();
        let sub = substring(&chars, len, start);
        prop_assume!(!sub.trim().is_empty());

        let mut ix = Ix::new();
        let (_, top) = ix.root("/r");
        let mut target_id = 0;
        for n in &names {
            let id = ix.file(top, n, 1, 1);
            if n == target {
                target_id = id;
            }
        }
        let r = ix.text(&sub);
        prop_assert!(
            r.hits.iter().any(|h| !h.is_dir && h.id == target_id),
            "{:?} not found by {:?}; hits {:?}", target, sub, r.hits
        );
    }
}

/// Folded characters (fixed points of `fold_key`) a word and its typo are made of.
const ALPHABET: &[char] = &[
    'a', 'b', 'c', 'd', 'e', 'i', 'l', 'n', 'o', 'r', 's', 't', 'u', 'z', '0', '7', 'ø', 'ß', 'æ',
    'α', 'я', '写',
];

#[derive(Debug, Clone)]
enum Edit {
    Insert(Index, char),
    Delete(Index),
    Substitute(Index, char),
    Transpose(Index),
}

fn edit() -> impl Strategy<Value = Edit> {
    let c = || prop::sample::select(ALPHABET);
    prop_oneof![
        (any::<Index>(), c()).prop_map(|(i, c)| Edit::Insert(i, c)),
        any::<Index>().prop_map(Edit::Delete),
        (any::<Index>(), c()).prop_map(|(i, c)| Edit::Substitute(i, c)),
        any::<Index>().prop_map(Edit::Transpose),
    ]
}

fn apply(word: &[char], e: &Edit) -> String {
    let mut w = word.to_vec();
    match e {
        Edit::Insert(i, c) => w.insert(i.index(w.len() + 1), *c),
        Edit::Delete(i) => {
            w.remove(i.index(w.len()));
        }
        Edit::Substitute(i, c) => {
            let i = i.index(w.len());
            w[i] = *c;
        }
        Edit::Transpose(i) => {
            let i = i.index(w.len() - 1);
            w.swap(i, i + 1);
        }
    }
    w.into_iter().collect()
}

const DECOYS: &[&str] = &["otro.txt", "foto 2019.jpg", "resumen anual.pdf"];

proptest! {
    #![proptest_config(config())]

    /// VFS-1 AC-10: one typo (insertion, deletion, substitution or adjacent transposition)
    /// in a word of the name is tolerated: the typed term, when it has the 6 characters the
    /// typo tiers need, finds the file, as `Fuzzy` unless the name holds it exactly. Decoys
    /// holding the typed term exactly are excluded (an exact hit rightly skips the typo
    /// tiers).
    #[test]
    fn one_typo_finds_the_file(
        word in prop::collection::vec(prop::sample::select(ALPHABET), 6..12),
        e in edit(),
        prefix in prop::sample::select(&["", "informe ", "2019_", "copia de ", "Ácta-"][..]),
        suffix in prop::sample::select(&[".pdf", " final.docx", "", "_v2.txt"][..]),
    ) {
        let typed = apply(&word, &e);
        prop_assume!(typed.chars().count() >= 6);
        prop_assume!(DECOYS.iter().all(|d| !fold_key(d).contains(&typed)));
        let word: String = word.into_iter().collect();
        let name = format!("{prefix}{word}{suffix}");

        let mut ix = Ix::new();
        let (_, top) = ix.root("/r");
        for d in DECOYS {
            ix.file(top, d, 1, 1);
        }
        let id = ix.file(top, &name, 1, 1);
        let r = ix.text(&typed);
        let hit = r.hits.iter().find(|h| !h.is_dir && h.id == id);
        prop_assert!(hit.is_some(), "{:?} not found by {:?}; hits {:?}", name, typed, r.hits);
        let expected = if fold_key(&name).contains(&typed) {
            MatchKind::Name
        } else {
            MatchKind::Fuzzy
        };
        prop_assert_eq!(hit.unwrap().matched, expected);
    }
}

const VOCAB: &[&str] = &[
    "acta", "notaria", "perez", "foto", "2019", "banco", "copia", "final",
];

/// Everything that identifies a hit and its place in the list.
fn keys(hits: &[SearchHit]) -> Vec<String> {
    hits.iter().map(|h| format!("{h:?}")).collect()
}

proptest! {
    #![proptest_config(config())]

    /// VFS-1 AC-10, AC-11: ranking is a total order: the same query twice (on the same
    /// handle with the page cache evicted, and on a second handle) gives identical hits, and
    /// the pages of size k at offsets 0, k, 2k, ... concatenate to the single large page,
    /// without duplicates.
    #[test]
    fn results_are_deterministic_and_page_stably(
        files in prop::collection::vec(
            (prop::collection::vec(prop::sample::select(VOCAB), 1..4),
             prop::sample::select(&[".pdf", ".jpg", ""][..]),
             0..3usize, 0..4i64),
            5..25),
        query in prop_oneof![
            prop::collection::vec(prop::sample::select(VOCAB), 1..3).prop_map(|v| v.join(" ")),
            prop::sample::select(&["notaira", "notria", "fianl copia", "act", "a"][..])
                .prop_map(str::to_string),
        ],
        k in 1..8u32,
    ) {
        let mut ix = Ix::new();
        let (root, top) = ix.root("/r");
        let acta = ix.dir(root, top, "acta", 2);
        let deep = ix.dir(root, acta, "acta/notaria 2019", 3);
        let dirs = [top, acta, deep];
        let mut seen = HashSet::new();
        for (words, ext, d, mtime) in &files {
            let name = format!("{}{ext}", words.join(" "));
            if seen.insert((*d, name.clone())) {
                ix.file(dirs[*d], &name, 1, *mtime);
            }
        }
        let evict = || ix.text("zzzzqqq");
        let all = ix.text(&query);
        evict();
        prop_assert_eq!(keys(&ix.text(&query).hits), keys(&all.hits));
        let other = open_patient(&ix._dir.path().join("index.db"));
        let again = other
            .search(&SearchQuery { text: query.clone(), filters: SearchFilters::default(), limit: 500, offset: 0 })
            .unwrap();
        prop_assert_eq!(keys(&again.hits), keys(&all.hits));

        let mut paged = Vec::new();
        let mut offset = 0;
        loop {
            evict();
            let page = ix.query(&query, SearchFilters::default(), k, offset).unwrap();
            prop_assert_eq!(page.total, all.total);
            prop_assert!(page.hits.len() <= k as usize);
            if page.hits.is_empty() {
                break;
            }
            paged.extend(page.hits);
            offset += k;
        }
        prop_assert_eq!(keys(&paged), keys(&all.hits));
        let unique: HashSet<(bool, i64)> = paged.iter().map(|h| (h.is_dir, h.id)).collect();
        prop_assert_eq!(unique.len(), paged.len());
    }
}

const ALL_KINDS: &[FileKind] = &[
    FileKind::Other,
    FileKind::Document,
    FileKind::Photo,
    FileKind::Video,
    FileKind::Audio,
    FileKind::Folder,
];

fn size() -> impl Strategy<Value = u64> {
    let max = i64::MAX as u64;
    prop_oneof![0..1000u64, Just(max - 1), Just(max), 0..=max,]
}

fn bound_size() -> impl Strategy<Value = u64> {
    let max = i64::MAX as u64;
    prop_oneof![
        0..1000u64,
        Just(max - 1),
        Just(max),
        Just(max + 1),
        Just(u64::MAX),
        any::<u64>(),
    ]
}

fn time() -> impl Strategy<Value = i64> {
    prop_oneof![8 => -4..6i64, 1 => Just(i64::MIN), 1 => Just(i64::MAX)]
}

fn filters() -> impl Strategy<Value = SearchFilters> {
    (
        prop::sample::subsequence(ALL_KINDS, 0..=ALL_KINDS.len()),
        prop::option::of(time()),
        prop::option::of(time()),
        prop::option::of(bound_size()),
        prop::option::of(bound_size()),
        prop::option::of(prop::sample::select(
            &[
                "/r",
                "/r/Sub",
                "/r/Sub/",
                "/R/sub",
                "/r/Sub/Deep",
                "/r/Subway",
                "/s",
                "/",
                "/nope",
                "/r/Sub/Deep/x",
                "",
                "\\r\\Sub",
                "/r/Otra",
            ][..],
        )),
    )
        .prop_map(|(kinds, from, to, smin, smax, folder)| SearchFilters {
            kinds,
            modified_from: from,
            modified_to: to,
            size_min: smin,
            size_max: smax,
            folder: folder.map(str::to_string),
        })
}

/// Folded absolute path of a hit, `/`-separated.
fn hit_path(h: &SearchHit) -> String {
    let dir = h.dir.replace('\\', "/");
    fold_key(&format!("{}/{}", dir.trim_end_matches('/'), h.name))
}

/// Why `h` violates `f`, if it does.
fn violation(h: &SearchHit, f: &SearchFilters) -> Option<String> {
    if !f.kinds.is_empty() && !f.kinds.contains(&h.kind) {
        return Some("kind".into());
    }
    if h.is_dir != (h.kind == FileKind::Folder) {
        return Some("folder kind".into());
    }
    if f.modified_from.is_some_and(|v| h.modified < v) {
        return Some("modified_from".into());
    }
    if f.modified_to.is_some_and(|v| h.modified >= v) {
        return Some("modified_to".into());
    }
    if h.is_dir && (f.size_min.is_some() || f.size_max.is_some()) {
        return Some("folder under a size filter".into());
    }
    if !h.is_dir && f.size_min.is_some_and(|v| h.size < v) {
        return Some("size_min".into());
    }
    if !h.is_dir && f.size_max.is_some_and(|v| h.size > v) {
        return Some("size_max".into());
    }
    if let Some(folder) = &f.folder {
        let key = fold_key(&folder.replace('\\', "/"));
        let prefix = format!("{}/", key.trim_end_matches('/'));
        if !hit_path(h).starts_with(&prefix) {
            return Some("folder".into());
        }
    }
    None
}

proptest! {
    #![proptest_config(config())]

    /// VFS-1 AC-11: no hit violates the filters: kinds (folders as their own kind), date
    /// (from inclusive, to exclusive), size (inclusive, bounds above `i64::MAX` included;
    /// folders have no size) and folder (strictly inside, not a sibling sharing its name
    /// prefix), with or without query text.
    #[test]
    fn filters_never_return_a_violating_row(
        files in prop::collection::vec(
            (0..6usize, prop::sample::select(&["acta", "foto", "sub"][..]),
             prop::sample::select(&[".pdf", ".jpg", ".mp4", ".mp3", ".xyz", ""][..]),
             size(), time()),
            1..12),
        dir_times in prop::collection::vec(time(), 6),
        text in prop::sample::select(&["", "acta", "foto", "sub", "deep", "act", "r", "su"][..]),
        f in filters(),
    ) {
        let mut ix = Ix::new();
        let (r, rtop) = ix.root("/r");
        let (s, stop) = ix.root("/s");
        let sub = ix.dir(r, rtop, "Sub", dir_times[0]);
        let deep = ix.dir(r, sub, "Sub/Deep", dir_times[1]);
        let subway = ix.dir(r, rtop, "Subway", dir_times[2]);
        let otra = ix.dir(r, rtop, "Otra", dir_times[3]);
        let ssub = ix.dir(s, stop, "Sub", dir_times[4]);
        let dirs = [rtop, sub, deep, subway, otra, ssub];
        for (i, (d, word, ext, size, mtime)) in files.iter().enumerate() {
            ix.file(dirs[*d], &format!("{word} {i}{ext}"), *size, *mtime);
        }
        let r = ix.query(text, f.clone(), 500, 0).unwrap();
        for h in &r.hits {
            prop_assert!(violation(h, &f).is_none(), "{:?} violates {:?}: {:?}", h, f, violation(h, &f));
        }
    }
}

/// A small fixed index whose names hold FTS5 operators, quotes and other syntax, for the
/// parser and injection properties.
fn junk_index() -> Ix {
    let mut ix = Ix::new();
    let (root, top) = ix.root("/datos");
    let g = ix.dir(root, top, "Gestoría", 1);
    let a = ix.dir(root, g, "Gestoría/Año 2019", 1);
    let o = ix.dir(root, top, "NEAR (OR)", 1);
    for (d, name) in [
        (top, "Notaría Pérez.pdf"),
        (top, "acta \"firmada\".pdf"),
        (top, "NEAR AND OR NOT.txt"),
        (top, "a*b^c:d.txt"),
        (top, "(copia) - final+1.doc"),
        (top, "it's mine.jpg"),
        (g, "extracto banco.pdf"),
        (a, "名前 写真.png"),
        (o, "name_key: path_key.txt"),
        (o, "100% _real_ ; DROP TABLE files; --.txt"),
    ] {
        ix.file(d, name, 1, 1);
    }
    ix
}

/// Pieces of FTS5 and SQL syntax a user might type or paste.
const JUNK: &[&str] = &[
    "\"",
    "'",
    "*",
    "^",
    ":",
    "(",
    ")",
    "-",
    "+",
    "NEAR",
    "NEAR(",
    "NEAR/2",
    "AND",
    "OR",
    "NOT",
    "and",
    "or",
    "not",
    "near",
    "name_key:",
    "path_key :",
    "name_key",
    "{",
    "}",
    ",",
    "\"\"",
    "\\\"",
    " ",
    " ",
    " ",
    "%",
    "_",
    ";",
    "--",
    "' OR 1=1 --",
    "\0",
    "\u{1}",
    "\u{7f}",
    "\u{301}",
    "\u{308}",
    "\n",
    "\t",
    "/",
    "\\",
    "acta",
    "notaria",
    "firmada",
    "copia",
    "final",
    "banco",
    "gestoria",
    "2019",
    "pdf",
    "txt",
    "near",
    "drop",
    "table",
    "files",
    "real",
    "100%",
    "it's",
    "写真",
    "名前",
    "*)",
    "(\"",
    "\")",
    "^\"",
    "\"*",
    "rowid",
    "MATCH",
    "SELECT",
    "x'00'",
    "0x00",
];

fn junk_query() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(JUNK), 1..12).prop_map(|v| v.concat())
}

/// The terms the search derives from `text`: `\` as `/`, NUL as whitespace, folded, split
/// on whitespace, deduplicated, at most 8.
fn terms_of(text: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for t in fold_key(&text.replace('\\', "/").replace('\0', " ")).split_whitespace() {
        if terms.len() < 8 && !terms.iter().any(|x| x == t) {
            terms.push(t.to_string());
        }
    }
    terms
}

/// Optimal-string-alignment distance at most 1 between `a` and `b`.
fn osa_le1(a: &[char], b: &[char]) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    match long.len() - short.len() {
        0 => {
            let diff: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
            diff.len() <= 1
                || (diff.len() == 2
                    && diff[1] == diff[0] + 1
                    && a[diff[0]] == b[diff[1]]
                    && a[diff[1]] == b[diff[0]])
        }
        1 => (0..long.len()).any(|i| {
            let mut l = long.to_vec();
            l.remove(i);
            l == short
        }),
        _ => false,
    }
}

/// Brute force: some substring of `hay` is within one edit of `needle`.
fn contains_within_one(hay: &str, needle: &str) -> bool {
    let h: Vec<char> = hay.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    (n.len().saturating_sub(1)..=n.len() + 1)
        .any(|len| len <= h.len() && (0..=h.len() - len).any(|s| osa_le1(&h[s..s + len], &n)))
}

/// Checks a hit against the documented semantics (search.rs module doc): every term is a
/// substring of the folded name or folded directory path (root path included, which covers
/// the S1 rule), or, for a `Fuzzy` hit and a term of 6+ characters, within one edit of one.
/// A folder holds every term in its own folded path and one in its name.
fn genuinely_matches(h: &SearchHit, terms: &[String]) -> bool {
    let name = fold_key(&h.name);
    let full = hit_path(h);
    let hay: &[&str] = if h.is_dir { &[&full] } else { &[&name, &full] };
    let ok = |t: &str, s: &str| {
        s.contains(t)
            || (h.matched == MatchKind::Fuzzy
                && t.chars().count() >= 6
                && contains_within_one(s, t))
    };
    terms.iter().all(|t| hay.iter().any(|s| ok(t, s)))
        && (!h.is_dir
            || terms
                .iter()
                .any(|t| ok(t, &name) || fold_key("/datos").contains(t.as_str())))
}

#[test]
/// VFS-1 AC-10: no user input changes the structure of the FTS5 query: a query built from
/// FTS5 operators, quotes, SQL fragments and control characters never errors, and every hit
/// genuinely holds every folded term (verified here, independently of the engine), so
/// operator junk can never widen the result to unrelated rows.
fn fts_operator_junk_cannot_change_the_query() {
    let ix = junk_index();
    proptest!(config(), |(text in junk_query())| {
        let r = ix.query(&text, SearchFilters::default(), 500, 0);
        prop_assert!(r.is_ok(), "{:?} -> {:?}", text, r.err());
        let terms = terms_of(&text);
        for h in &r.unwrap().hits {
            prop_assert!(genuinely_matches(h, &terms), "{:?} returned {:?} (terms {:?})", text, h, terms);
        }
    });
}

/// Arbitrary query text: Unicode, operator junk, very long inputs (10k chars, one term or
/// many) and many terms.
fn any_query() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => unicode_string(),
        3 => junk_query(),
        1 => prop::collection::vec(prop_oneof![fragment(), prop::sample::select(JUNK).prop_map(str::to_string)], 1..6)
            .prop_map(|v| {
                let unit = v.concat();
                let n = 10_000 / unit.chars().count().max(1) + 1;
                unit.repeat(n).chars().take(10_000).collect()
            }),
        1 => prop::collection::vec(prop::sample::select(JUNK), 50..300).prop_map(|v| v.join(" ")),
    ]
}

#[test]
/// VFS-1 AC-10, AC-11: a query never panics and never returns `Err`, whatever the text
/// (NUL and control characters, lone combining marks, quotes and FTS5 operators, 10k-char
/// inputs, hundreds of terms), the paging values or the folder filter.
fn arbitrary_queries_never_fail() {
    let ix = junk_index();
    proptest!(config(), |(
        text in any_query(),
        limit in any::<u32>(),
        offset in prop_oneof![0..20u32, any::<u32>()],
        folder in prop::option::of(prop_oneof![unicode_string(), Just("/datos/Gestoría".to_string())]),
    )| {
        let f = SearchFilters { folder, ..SearchFilters::default() };
        let r = ix.query(&text, f, limit, offset);
        prop_assert!(r.is_ok(), "{:?} -> {:?}", text, r.err());
    });
}

/// VFS-1 AC-10 regression (shrunk from `arbitrary_queries_never_fail`): a NUL inside a term
/// of 3 or more characters ended the FTS5 expression early, so the query failed with
/// "unterminated string". A NUL cannot occur in a file name; the query treats it as
/// whitespace.
#[test]
fn nul_in_a_query_term_is_not_an_error() {
    let ix = junk_index();
    for text in ["abc\0", "NEAR\0", "\"\0\"", "acta\0firmada"] {
        let r = ix.query(text, SearchFilters::default(), 500, 0);
        assert!(r.is_ok(), "{text:?} -> {:?}", r.err());
    }
    let r = ix.text("acta\0firmada");
    assert_eq!(r.hits.len(), 1, "{:?}", r.hits);
    assert_eq!(r.hits[0].name, "acta \"firmada\".pdf");
}

/// VFS-1 AC-11 regression (shrunk from `filters_never_return_a_violating_row`): a minimum
/// size above `i64::MAX` was clamped to `i64::MAX`, so a file of exactly `i64::MAX` bytes
/// passed `size_min = i64::MAX + 1`.
#[test]
fn size_min_above_i64_max_matches_no_file() {
    let mut ix = Ix::new();
    let (_, top) = ix.root("/r");
    ix.file(top, "acta 0.mp4", i64::MAX as u64, 0);
    for size_min in [i64::MAX as u64 + 1, u64::MAX] {
        let f = SearchFilters {
            size_min: Some(size_min),
            ..SearchFilters::default()
        };
        for text in ["", "acta"] {
            let r = ix.query(text, f.clone(), 500, 0).unwrap();
            assert!(r.hits.is_empty(), "{size_min} {text:?}: {:?}", r.hits);
        }
    }
    let f = SearchFilters {
        size_min: Some(i64::MAX as u64),
        ..SearchFilters::default()
    };
    assert_eq!(ix.query("acta", f, 500, 0).unwrap().hits.len(), 1);
}

/// VFS-1 AC-9, AC-10 regression (found by `arbitrary_queries_never_fail` hanging): the typo
/// alternatives of one long term grew faster than quadratically outside the deadline, so a
/// pasted 10k-character term without spaces took minutes. A term too long for typo
/// tolerance now matches exactly and the query returns in well under the bound here.
#[test]
fn a_10k_character_term_returns_promptly() {
    let ix = junk_index();
    for unit in ["abcdefghij", "notaria", "a\u{301}"] {
        let text: String = unit.repeat(10_000).chars().take(10_000).collect();
        let t = std::time::Instant::now();
        let r = ix.query(&text, SearchFilters::default(), 500, 0);
        assert!(r.is_ok(), "{:?}", r.err());
        assert!(
            t.elapsed() < std::time::Duration::from_secs(10),
            "{:?}",
            t.elapsed()
        );
    }
}
