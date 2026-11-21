//! Hostile real-world trees thrown at the indexer (VFS-1 AC-4, AC-6, AC-7, AC-8, AC-10).
//!
//! Each test builds a nasty tree with a generator that records what the filesystem really
//! accepted, scans it, and checks the core invariant (`assert_index_matches_disk`): the
//! index equals an independent `std::fs` walk, every path rebuilds to an existing entry,
//! and every name is found by searching it. The big cases run at full size only with
//! `VFS_STRESS=1` and at a reduced size otherwise, so their code paths always run.

#[path = "common/hostile_tree.rs"]
mod hostile_tree;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use hostile_tree::*;
use unicode_normalization::UnicodeNormalization;
use vfs_core::{Error, OverlapRelation, RootStatus};

fn nfc(s: &str) -> String {
    s.nfc().collect()
}

fn nfd(s: &str) -> String {
    s.nfd().collect()
}

/// VFS-1 AC-8, AC-10: names in NFC, NFD and mixed forms, emoji (ZWJ sequences, flags),
/// right-to-left text with bidi overrides, invisible characters, compatibility forms and
/// FTS/SQL syntax characters are indexed as they are on disk, rebuild to existing paths,
/// and are found by searching their NFC and their NFD spelling.
#[test]
fn unicode_and_syntax_names() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let names = [
        "Notar\u{00ED}a escritura.pdf",   // NFC
        "Notari\u{0301}a escritura.pdf",  // NFD of the same name
        "Pe\u{0301}rez G\u{00F3}mez.doc", // mixed forms in one name
        "an\u{0303}o nuevo \u{00F1}and\u{00FA}.txt",
        "\u{1F4C1} fotos \u{1F600}.jpg", // emoji
        "familia \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}.png", // ZWJ sequence
        "bandera \u{1F1EA}\u{1F1F8}.gif", // regional indicators
        "\u{05E9}\u{05DC}\u{05D5}\u{05DD} \u{05E2}\u{05D5}\u{05DC}\u{05DD}.txt", // Hebrew
        "\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}.pdf", // Arabic
        "factura\u{202E}fdp.exe",        // right-to-left override
        "mezcla abc \u{05D0}\u{05D1}\u{05D2} 123.txt", // mixed direction
        "\u{0301}\u{0301}\u{0301}acentos sueltos.txt", // leading combining marks
        "\u{1100}\u{1161}\u{11A8} hangul.txt", // conjoining jamo
        "\u{FB01}chero ligadura.txt",    // ligature fi
        "\u{FF21}\u{FF22}\u{FF23} ancho.txt", // fullwidth ABC
        "\u{0130}stanbul.txt",
        "Stra\u{00DF}e.txt",
        "\u{039F}\u{0394}\u{03A5}\u{03A3}\u{03A3}\u{0395}\u{03A5}\u{03A3}.txt",
        "\u{65E5}\u{672C}\u{8A9E}\u{306E}\u{30D5}\u{30A1}\u{30A4}\u{30EB}.txt",
        "cero\u{200B}ancho.txt", // zero-width space
        "\u{FEFF}bom inicial.txt",
        "espacio\u{00A0}duro.txt",
        "\"comillas\".txt",
        "a\"b c.txt",
        "NOT AND OR.txt",
        "NEAR(uno dos).txt",
        "col:umna.txt",
        "estrella*.txt",
        "^circunflejo.txt",
        "-menos.txt",
        "(parentesis).txt",
        "{llaves} [corchetes].txt",
        "100% _like_.txt",
        "'simples'.txt",
        "barra\\invertida.txt",
        "...",
        "..puntos",
        "tab\tdentro.txt",
        "salto\nde linea.txt",
        " espacio inicial.txt",
    ];
    let mut probe = Probe::default();
    probe.dir(&root.join("carpetas"));
    for name in names {
        probe.file(&root.join(name), 3);
        let dir = root.join("carpetas").join(name);
        if probe.dir(&dir) {
            probe.file(&dir.join("dentro.txt"), 1);
        }
    }
    probe.report("unicode");
    assert!(
        probe.created.len() > names.len(),
        "the generator created almost nothing"
    );

    let summary = env.scan_ok();
    assert_eq!(summary.errors, 0);
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());

    // Both normalization forms of each name find it (VFS-1 AC-8).
    let (files, _) = index_snapshot(&env.reader(), root_id);
    for rel in disk.files.keys().filter(|r| !r.contains('/')) {
        let f = &files[rel];
        if !searchable(&f.name) {
            continue;
        }
        for q in [nfc(&f.name), nfd(&f.name)] {
            assert!(
                found_by_name(&env.vfs, &q, f.id, false),
                "{:?} not found by {q:?}",
                f.name
            );
        }
    }
}

/// VFS-23 AC-2 (issue 57): on macOS a name may hold backslashes; searching it exactly finds
/// it as an exact name match, with one backslash or several.
#[cfg(unix)]
#[test]
fn backslash_names_are_found_exactly() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let names = [
        "una\\barra.txt",
        "dos\\barras\\invertidas.txt",
        "tres\\\\\\juntas.txt",
    ];
    for name in names {
        fs::write(root.join(name), b"x").unwrap();
    }
    env.scan_ok();
    let (files, _) = index_snapshot(&env.reader(), root_id);
    for name in names {
        let id = files[name].id;
        let r = env
            .vfs
            .search(&vfs_core::SearchQuery {
                text: name.to_string(),
                filters: Default::default(),
                limit: 50,
                offset: 0,
            })
            .unwrap();
        let hit = r.hits.iter().find(|h| h.id == id);
        assert!(
            matches!(hit, Some(h) if h.matched == vfs_core::MatchKind::Name),
            "{name:?}: {hit:?}"
        );
    }
}

/// VFS-1 AC-4, AC-8: Windows-reserved device names, trailing dots and spaces, reserved
/// characters and control characters. NTFS refuses or rewrites some of them (and APFS
/// accepts them all); whatever exists afterwards is what the index must hold.
#[test]
fn windows_hostile_names() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let names = [
        "CON",
        "nul",
        "COM1",
        "LPT9.txt",
        "con.txt",
        "AUX.tar.gz",
        "PRN.",
        "fichero.",
        "fichero..",
        "espacio final ",
        "mezcla. .",
        "menor<mayor>.txt",
        "pregunta?.txt",
        "tubo|tubo.txt",
        "comilla\"doble.txt",
        "dos:puntos.txt",
        "control\u{1}x.txt",
        "control\u{1f}y.txt",
    ];
    let mut probe = Probe::default();
    for name in names {
        probe.file(&root.join(name), 2);
        let dir = root.join(format!("dir {name}"));
        if probe.dir(&dir) {
            probe.file(&dir.join(name), 1);
        }
    }
    probe.report("windows names");
    assert!(!probe.created.is_empty(), "the generator created nothing");
    let summary = env.scan_ok();
    assert_eq!(summary.errors, 0);
    assert_index_matches_disk(&env, root_id, &root, Check::default());
}

/// VFS-1 AC-8, AC-10: names that differ only by case or only by accent or normalization.
/// On a case- or normalization-insensitive filesystem only one of each group exists; the
/// index holds exactly what exists, and a folded search finds every variant that does.
#[test]
fn case_and_accent_variants() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let mut probe = Probe::default();
    let files = ["Informe.txt", "informe.txt", "INFORME.TXT", "inForme.txt"];
    for f in files {
        probe.file(&root.join(f), 1);
    }
    let dirs = [
        "A\u{00F1}o",
        "Ano",
        "an\u{0303}o",
        "A\u{00D1}O",
        "a\u{00F1}o",
    ];
    for (i, d) in dirs.iter().enumerate() {
        if probe.dir(&root.join(d)) {
            probe.file(&root.join(d).join(format!("contenido {i}.txt")), 1);
        }
    }
    probe.report("variants");
    let summary = env.scan_ok();
    assert_eq!(summary.errors, 0);
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());

    let informes: Vec<&String> = disk
        .files
        .keys()
        .filter(|k| k.eq_ignore_ascii_case("informe.txt"))
        .collect();
    if case_insensitive(&root) {
        assert_eq!(informes.len(), 1, "case-insensitive FS: {informes:?}");
    } else {
        assert_eq!(
            informes.len(),
            files.len(),
            "case-sensitive FS: {informes:?}"
        );
    }
    let nfc_nfd: Vec<&String> = disk
        .dirs
        .iter()
        .filter(|d| nfc(d).to_lowercase() == "a\u{00F1}o")
        .collect();
    if normalization_insensitive(&root) {
        assert_eq!(
            nfc_nfd.len(),
            1,
            "normalization-insensitive FS: {nfc_nfd:?}"
        );
    }
    let (indexed, _) = index_snapshot(&env.reader(), root_id);
    for rel in &informes {
        assert!(found_by_name(&env.vfs, "informe", indexed[*rel].id, false));
    }
}

/// VFS-1 AC-8: names at the 255-unit limit (ASCII, 2-, 3- and 4-byte UTF-8) and just past
/// it. APFS limits names to 255 UTF-8 bytes and NTFS to 255 UTF-16 units, so each
/// filesystem refuses a different subset; the index holds whatever was accepted.
#[test]
fn names_at_255_bytes() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let names = [
        format!("{}.txt", "a".repeat(251)),        // 255 bytes, 255 units
        format!("{}.txt", "\u{00E9}".repeat(125)), // 254 bytes
        format!("{}.txt", "\u{65E5}".repeat(83)),  // 253 bytes
        format!("{}.txt", "\u{1F600}".repeat(62)), // 252 bytes, 128 units
        format!("{}.txt", "b".repeat(252)),        // 256: refused everywhere
        format!("{}.txt", "\u{65E5}".repeat(251)), // 757 bytes, 255 units: NTFS only
    ];
    let mut probe = Probe::default();
    let created: Vec<bool> = names.iter().map(|n| probe.file(&root.join(n), 1)).collect();
    let long_dir = root.join("d".repeat(255));
    if probe.dir(&long_dir) {
        probe.file(&long_dir.join("f".repeat(255)), 1);
    }
    probe.report("255");
    assert!(
        created[0],
        "a 255-byte ASCII name is valid on every target filesystem"
    );
    let summary = env.scan_ok();
    assert_eq!(summary.errors, 0);
    assert_index_matches_disk(&env, root_id, &root, Check::default());
}

/// Result of building a chain of nested directories.
struct Chain {
    /// Levels created through ordinary paths.
    levels: usize,
    /// Levels created through directory descriptors past the OS path limit (Unix).
    hidden_levels: usize,
    /// Length in characters of the deepest file path created.
    deepest_chars: usize,
}

/// Nests `levels` directories named by `name(level)` under `root`, each holding one file,
/// stopping early once the path reaches `max_chars` or the filesystem refuses. Where
/// ordinary paths stop (macOS PATH_MAX), Unix continues through directory descriptors.
fn build_chain(
    root: &Path,
    levels: usize,
    max_chars: usize,
    name: impl Fn(usize) -> String,
) -> Chain {
    let mut chain = Chain {
        levels: 0,
        hidden_levels: 0,
        deepest_chars: 0,
    };
    let mut cur = root.to_path_buf();
    let mut chars = root.to_string_lossy().chars().count();
    let mut level = 0;
    while level < levels && chars < max_chars {
        let n = name(level);
        let next = cur.join(&n);
        let file = next.join(format!("archivo nivel {level:03}.txt"));
        let made = fs::create_dir(&next).and_then(|_| fs::write(&file, b"x"));
        if made.is_err() {
            let _ = fs::remove_dir(&next);
            break;
        }
        chars += 1 + n.chars().count();
        chain.deepest_chars =
            chars + 1 + file.file_name().unwrap().to_string_lossy().chars().count();
        chain.levels += 1;
        level += 1;
        cur = next;
    }
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        let Ok(mut fd) = deep::DirFd::open(&cur) else {
            return chain;
        };
        while level < levels && chars < max_chars {
            let n = name(level);
            let Ok(next) = fd.mkdir_at(OsStr::new(&n)) else {
                break;
            };
            let file = format!("archivo nivel {level:03}.txt");
            if next.file_at(OsStr::new(&file), 1).is_err() {
                break;
            }
            chars += 1 + n.chars().count();
            chain.deepest_chars = chars + 1 + file.chars().count();
            chain.hidden_levels += 1;
            level += 1;
            fd = next;
        }
    }
    chain
}

/// A long directory name of about 120 characters (well under 255 UTF-8 bytes).
fn long_name(level: usize) -> String {
    let mut n = format!("{level:03} carpeta con un nombre largo de verdad \u{00F1}and\u{00FA} ");
    while n.chars().count() < 120 {
        n.push_str("relleno ");
    }
    n.trim_end().to_string()
}

/// VFS-1 AC-8: paths far past 260 characters (past 32,000 under `VFS_STRESS=1`). The scan
/// finishes, indexes every level the OS lets a path reach (past 260, the point of AC-8),
/// rebuilds those paths, and leaves directories beyond the OS limit (macOS PATH_MAX)
/// pending a retry instead of failing or looping.
#[test]
fn paths_past_260_and_32000_chars() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let max_chars = sized(33_000, 1_600);
    let chain = build_chain(&root, usize::MAX, max_chars, long_name);
    eprintln!(
        "[long paths] {} levels by path, {} past the path limit, deepest file {} chars",
        chain.levels, chain.hidden_levels, chain.deepest_chars
    );
    assert!(
        chain.deepest_chars > 260,
        "the tree must reach past 260 characters"
    );

    let summary = env.scan_ok();
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    let deepest_indexed = disk
        .files
        .keys()
        .map(|k| root.join(k).to_string_lossy().chars().count())
        .max()
        .unwrap();
    assert!(
        deepest_indexed > 260,
        "indexed paths reach {deepest_indexed} chars"
    );
    assert_eq!(
        summary.errors as usize,
        disk.unreadable.len(),
        "every directory beyond the OS limit counts as one listing error"
    );
    // A second scan retries those directories and still terminates without error.
    env.scan_ok();
    assert_index_matches_disk(&env, root_id, &root, Check::default());
}

/// VFS-1 AC-4, AC-8: 60 levels of nesting (long names under `VFS_STRESS=1`, short names
/// otherwise); every level the OS can address is indexed and found.
#[test]
fn sixty_levels_of_nesting() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let chain = if stress() {
        build_chain(&root, 60, usize::MAX, long_name)
    } else {
        build_chain(&root, 60, usize::MAX, |l| format!("nivel{l:02}"))
    };
    assert_eq!(
        chain.levels + chain.hidden_levels,
        60,
        "60 levels must be created"
    );
    env.scan_ok();
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    if chain.hidden_levels == 0 {
        assert_eq!(disk.files.len(), 60, "one file per level, all reachable");
    } else {
        assert!(
            !disk.unreadable.is_empty(),
            "levels past the OS limit stay pending"
        );
    }
}

/// VFS-1 AC-4, AC-9 scale: one directory with 300,000 entries (3,000 without
/// `VFS_STRESS=1`) is indexed in full and a sample of its names is found.
#[test]
fn directory_with_300k_entries() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let n = sized(300_000, 3_000);
    let dir = root.join("enorme");
    fs::create_dir(&dir).unwrap();
    for i in 0..n {
        fs::write(dir.join(format!("archivo {i:06}.dat")), b"").unwrap();
    }
    let summary = env.scan_ok();
    assert_eq!(summary.files_added as usize, n);
    let disk = assert_index_matches_disk(&env, root_id, &root, Check { search_sample: 200 });
    assert_eq!(disk.files.len(), n);
}

/// VFS-1 AC-4, VFS-18 AC-8: symlink loops (to `.`, to `..`, to the root, a two-link cycle),
/// dangling links, links to files and to folders outside the root, and on Windows junction
/// loops, are never followed: the scan terminates, nothing outside the root is indexed, the
/// one real file is listed once and each link is a row of its own.
#[test]
fn symlink_and_junction_loops() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let outside = env.base().join("fuera");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("no indexar.txt"), b"x").unwrap();
    fs::create_dir_all(root.join("a/b")).unwrap();
    fs::write(root.join("a/b/real.txt"), b"x").unwrap();
    let mut links = 0;
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let made = [
            symlink(".", root.join("a/self")),
            symlink("..", root.join("a/b/up")),
            symlink(&root, root.join("a/b/root")),
            symlink("ciclo2", root.join("ciclo1")),
            symlink("ciclo1", root.join("ciclo2")),
            symlink("no existe", root.join("colgante")),
            symlink("a/b/real.txt", root.join("enlace a fichero.txt")),
            symlink(&outside, root.join("enlace fuera")),
        ];
        links += made.iter().filter(|r| r.is_ok()).count();
        assert_eq!(links, made.len());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{symlink_dir, symlink_file};
        // Symlinks need developer mode or elevation; junctions need neither.
        links += [
            symlink_dir(&root, root.join("a\\b\\root")),
            symlink_dir(&outside, root.join("enlace fuera")),
            symlink_file(
                root.join("a\\b\\real.txt"),
                root.join("enlace a fichero.txt"),
            ),
        ]
        .iter()
        .filter(|r| r.is_ok())
        .count();
        for (link, target) in [
            (root.join("junction raiz"), root.clone()),
            (root.join("a\\b\\junction arriba"), root.join("a")),
            (root.join("junction fuera"), outside.clone()),
        ] {
            let ok = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&target)
                .output()
                .is_ok_and(|o| o.status.success());
            links += ok as usize;
        }
        assert!(links >= 3, "junctions must be creatable without elevation");
    }
    eprintln!("[links] {links} links created");
    let summary = env.scan_ok();
    assert_eq!(summary.errors, 0);
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    let real: Vec<_> = disk
        .files
        .iter()
        .filter(|(_, &(size, _))| size > 0)
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(real, vec!["a/b/real.txt"]);
    assert_eq!(disk.files.len(), 1 + links);
}

/// VFS-1 AC-4, AC-7: hard links are separate names of one file and are all indexed; when
/// the original name goes, a re-walk drops only that name.
#[test]
fn hard_links() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    fs::create_dir_all(root.join("x/y")).unwrap();
    let original = root.join("original.bin");
    fs::write(&original, vec![0u8; 1234]).unwrap();
    let links = [
        root.join("enlace duro.bin"),
        root.join("x/otro.bin"),
        root.join("x/y/tercero.bin"),
    ];
    for l in &links {
        fs::hard_link(&original, l).unwrap();
    }
    env.scan_ok();
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    assert_eq!(disk.files.len(), 4);
    fs::remove_file(&original).unwrap();
    env.force_rewalk(root_id);
    env.scan_ok();
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    assert_eq!(disk.files.len(), 3);
}

/// VFS-1 AC-4: a named pipe and a socket in the tree are neither indexed nor opened (a
/// FIFO would block a reader forever), and the scan finishes.
#[cfg(unix)]
#[test]
fn fifo_and_socket_are_skipped() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    fs::write(root.join("normal.txt"), b"x").unwrap();
    let fifo = std::ffi::CString::new(root.join("tuberia").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    let _sock = std::os::unix::net::UnixListener::bind(root.join("enchufe.sock")).unwrap();
    let summary = env.scan_ok();
    assert_eq!(summary.errors, 0);
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    assert_eq!(disk.files.keys().collect::<Vec<_>>(), vec!["normal.txt"]);
}

/// Tiny deterministic PRNG (xorshift64) for the churn thread.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Creates, deletes and renames files and folders under `root` until `stop` is set;
/// returns the number of operations attempted.
fn churn(root: PathBuf, stop: Arc<AtomicBool>, started: Arc<AtomicBool>) -> u64 {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut ops = 0u64;
    while !stop.load(Ordering::SeqCst) {
        let d = root.join(format!("dir {:02}", rng.below(20)));
        ops += 1;
        if ops == 200 {
            started.store(true, Ordering::SeqCst);
        }
        let names: Vec<PathBuf> = fs::read_dir(&d)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        let pick = (!names.is_empty()).then(|| names[rng.below(names.len())].clone());
        let _ = match rng.below(6) {
            0 | 1 => fs::write(d.join(format!("nuevo {ops}.txt")), b"n"),
            2 => pick.map_or(Ok(()), |p| {
                if p.is_dir() {
                    fs::remove_dir_all(p)
                } else {
                    fs::remove_file(p)
                }
            }),
            3 => pick.map_or(Ok(()), |p| {
                fs::rename(&p, d.join(format!("renombrado {ops}.txt")))
            }),
            4 => {
                let sub = d.join(format!("sub {ops}"));
                fs::create_dir(&sub).and_then(|_| fs::write(sub.join("hijo.txt"), b"h"))
            }
            _ => fs::write(
                d.join(format!("archivo {:03}.txt", rng.below(50))),
                vec![b'c'; rng.below(9)],
            ),
        };
    }
    ops
}

/// VFS-1 AC-7: files and folders that vanish, appear and are renamed while the scan lists
/// them never abort it; once the churn stops, one full re-walk converges to the disk.
#[test]
fn churn_during_scan_converges() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    for d in 0..20 {
        let dir = root.join(format!("dir {d:02}"));
        fs::create_dir(&dir).unwrap();
        for f in 0..50 {
            fs::write(dir.join(format!("archivo {f:03}.txt")), b"x").unwrap();
        }
    }
    let stop = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicBool::new(false));
    let worker = {
        let (root, stop, started) = (root.clone(), stop.clone(), started.clone());
        std::thread::spawn(move || churn(root, stop, started))
    };
    while !started.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    let first = env.scan();
    stop.store(true, Ordering::SeqCst);
    let ops = worker.join().unwrap();
    eprintln!("[churn] {ops} operations during the scan");
    first.expect("churn must not abort the scan");

    env.force_rewalk(root_id);
    env.scan_ok();
    assert_index_matches_disk(&env, root_id, &root, Check { search_sample: 100 });
}

/// VFS-1 AC-6: a scan cancelled part-way through a hostile tree resumes from where it
/// stopped and converges to the disk.
#[test]
fn cancelled_scan_resumes_and_converges() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    for d in 0..40 {
        let dir = root.join(format!("Gestor\u{00ED}a {d:02} \u{1F4C1}"));
        fs::create_dir(&dir).unwrap();
        for f in 0..25 {
            fs::write(dir.join(format!("Notari\u{0301}a {f:02}.pdf")), b"x").unwrap();
        }
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(5));
        flag.store(true, Ordering::SeqCst);
    });
    env.vfs
        .scan_to_completion(&cancel)
        .expect("a cancelled scan returns Ok");
    canceller.join().unwrap();
    env.scan_ok();
    assert_index_matches_disk(&env, root_id, &root, Check { search_sample: 50 });
}

/// Restores a directory's permissions on drop, so a failed test still cleans up.
#[cfg(unix)]
struct Restore(Vec<PathBuf>);
#[cfg(unix)]
impl Drop for Restore {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        for p in &self.0 {
            let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o755));
        }
    }
}

/// True when the current user can list `dir` despite its permissions (running as root).
#[cfg(unix)]
fn can_list(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut rd| rd.all(|e| e.is_ok_and(|e| e.metadata().is_ok())))
}

/// VFS-1 AC-4, AC-7: a directory the user cannot list (`chmod 000`), one that can be
/// listed but whose entries cannot be stat'ed (`chmod 444`), and an unreadable file do not
/// abort the scan; the rest is indexed, and after the permissions are restored the next
/// scan picks up what was hidden.
#[cfg(unix)]
#[test]
fn permission_denied_directory() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    for d in ["secreto", "solo lectura", "abierto"] {
        fs::create_dir(root.join(d)).unwrap();
        for f in 0..3 {
            fs::write(root.join(d).join(format!("{d} {f}.txt")), b"x").unwrap();
        }
    }
    let locked_file = root.join("abierto/abierto 0.txt");
    fs::set_permissions(&locked_file, fs::Permissions::from_mode(0o000)).unwrap();
    let secret = root.join("secreto");
    let read_only = root.join("solo lectura");
    let _restore = Restore(vec![secret.clone(), read_only.clone(), locked_file.clone()]);
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
    fs::set_permissions(&read_only, fs::Permissions::from_mode(0o444)).unwrap();
    let enforced = !can_list(&secret);

    let summary = env.scan_ok();
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    assert!(
        disk.files.contains_key("abierto/abierto 0.txt"),
        "an unreadable file is indexed"
    );
    if enforced {
        assert!(
            summary.errors >= 1,
            "the unlistable directory counts as an error"
        );
        assert!(env.pending_dirs(root_id).contains(&"secreto".to_string()));
    }

    for p in [&secret, &read_only] {
        fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    env.scan_ok();
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    for d in ["secreto", "solo lectura", "abierto"] {
        for f in 0..3 {
            let rel = format!("{d}/{d} {f}.txt");
            assert!(
                disk.files.contains_key(&rel),
                "{rel} indexed after chmod back"
            );
        }
    }
}

/// VFS-1 AC-4, AC-6: an unlistable root does not abort the scan and is listed in full by
/// the next scan once its permissions are restored.
#[cfg(unix)]
#[test]
fn permission_denied_root() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    fs::write(root.join("dentro.txt"), b"x").unwrap();
    let _restore = Restore(vec![root.clone()]);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();
    env.scan_ok();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    env.scan_ok();
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    assert_eq!(disk.files.keys().collect::<Vec<_>>(), vec!["dentro.txt"]);
}

/// VFS-1 AC-4: zero-byte files and a 4 TiB sparse file are indexed with their exact
/// sizes, without the indexer reading them.
#[test]
fn zero_byte_and_4tib_sparse_files() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    for i in 0..5 {
        fs::write(root.join(format!("vacio {i}.txt")), b"").unwrap();
    }
    let sparse = root.join("disco enorme.img");
    let f = fs::File::create(&sparse).unwrap();
    let wanted = 4u64 << 40;
    let set = f.set_len(wanted);
    drop(f);
    let actual = fs::metadata(&sparse).unwrap().len();
    env.scan_ok();
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    let (indexed, _) = index_snapshot(&env.reader(), root_id);
    let size = indexed["disco enorme.img"].size;
    match set {
        Ok(()) => assert_eq!(size, wanted, "4 TiB sparse file indexed with its size"),
        Err(e) => assert_eq!(
            size, actual,
            "the filesystem refused set_len(4 TiB) ({e}); size checked at the {actual} bytes it kept"
        ),
    }
    assert_eq!(
        disk.files.values().filter(|(s, _)| *s == 0).count(),
        5 + (actual == 0) as usize
    );
}

/// VFS-1 AC-4, AC-11 dates: modification times at 1601-01-01, just before and at
/// 1970-01-01, at the 2038 32-bit rollover and far in the future are indexed as the
/// filesystem stored them (each filesystem clamps or refuses a different subset).
#[test]
fn extreme_mtimes() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let cases: [(&str, i64); 7] = [
        ("1601-01-01", -11_644_473_600),
        ("1969-12-31 23.59.59", -1),
        ("1970-01-01", 0),
        ("2038-01-19 03.14.07", i32::MAX as i64),
        ("2038-01-19 03.14.08", i32::MAX as i64 + 1),
        ("3000-01-01", 32_503_680_000),
        ("9999-12-31", 253_402_214_400),
    ];
    for (name, secs) in cases {
        let path = root.join(format!("fecha {name}.txt"));
        let f = fs::File::create(&path).unwrap();
        let t = if secs >= 0 {
            UNIX_EPOCH + Duration::from_secs(secs as u64)
        } else {
            UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
        };
        let r = f.set_modified(t);
        drop(f);
        let stored = fs::metadata(&path).unwrap().modified().map(floor_secs);
        eprintln!("[mtime] {name}: asked {secs}, set {r:?}, stored {stored:?}");
    }
    env.scan_ok();
    // The invariant compares each indexed mtime with the one the filesystem reports.
    let disk = assert_index_matches_disk(&env, root_id, &root, Check::default());
    assert_eq!(disk.files.len(), cases.len());
}

/// Builds `dirs` directories of `files` files each under `root`.
fn fill(root: &Path, dirs: usize, files: usize) {
    for d in 0..dirs {
        let dir = root.join(format!("carpeta {d:03}"));
        fs::create_dir_all(&dir).unwrap();
        for f in 0..files {
            fs::write(dir.join(format!("fichero {f:02}.txt")), b"x").unwrap();
        }
    }
}

/// VFS-1 AC-4, AC-7: a root deleted before a re-walk becomes unavailable without error;
/// recreated with other content, the next scan brings it back and converges.
#[test]
fn root_deleted_before_rewalk() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    fill(&root, 5, 5);
    env.scan_ok();
    env.force_rewalk(root_id);
    fs::remove_dir_all(&root).unwrap();
    env.scan_ok();
    assert_eq!(env.root_status(root_id), RootStatus::Unavailable);
    fs::create_dir(&root).unwrap();
    fill(&root, 2, 3);
    env.scan_ok();
    assert_eq!(env.root_status(root_id), RootStatus::Ready);
    assert_index_matches_disk(&env, root_id, &root, Check::default());
}

/// Runs a first scan of a fresh tree while `mutate` runs on another thread after a short
/// delay; the scan must return Ok whatever point it reached. `mutate` is retried for up to
/// 30 s, since Windows refuses to move a folder while the scan holds a handle inside it.
fn scan_while(env: &Env, mutate: impl Fn() -> std::io::Result<()> + Send + 'static) {
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(3));
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while let Err(e) = mutate() {
            assert!(
                std::time::Instant::now() < deadline,
                "mutation kept failing: {e}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let r = env.scan();
    worker.join().unwrap();
    r.expect("a root vanishing mid-scan must not abort the scan");
}

/// VFS-1 AC-4, AC-7: a root deleted while it is being scanned ends unavailable, not in an
/// error; recreated, it is re-walked and converges to the disk.
#[test]
fn root_deleted_while_scanning() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    fill(&root, sized(2_000, 300), 3);
    let target = root.clone();
    scan_while(&env, move || match fs::remove_dir_all(&target) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => r,
    });
    env.scan_ok(); // notices the root is gone if the first scan finished before
    assert_eq!(env.root_status(root_id), RootStatus::Unavailable);
    fs::create_dir(&root).unwrap();
    fill(&root, 3, 3);
    env.scan_ok();
    assert_eq!(env.root_status(root_id), RootStatus::Ready);
    assert_index_matches_disk(&env, root_id, &root, Check::default());
}

/// VFS-1 AC-4, AC-7: a root renamed away while it is being scanned ends unavailable;
/// renamed back, it is re-walked and converges to the disk.
#[test]
fn root_renamed_while_scanning() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    fill(&root, sized(2_000, 300), 3);
    let moved = env.base().join("raiz movida");
    let (from, to) = (root.clone(), moved.clone());
    scan_while(&env, move || fs::rename(&from, &to));
    env.scan_ok();
    assert_eq!(env.root_status(root_id), RootStatus::Unavailable);
    fs::rename(&moved, &root).unwrap();
    env.scan_ok();
    assert_eq!(env.root_status(root_id), RootStatus::Ready);
    assert_index_matches_disk(&env, root_id, &root, Check { search_sample: 100 });
}

fn overlap(r: vfs_core::Result<vfs_core::Root>) -> OverlapRelation {
    match r {
        Err(Error::OverlappingRoot { relation, .. }) => relation,
        other => panic!("expected OverlappingRoot, got {other:?}"),
    }
}

/// VFS-1 AC-4: `add_root` refuses a folder inside, containing, or equal to an existing
/// root, including spellings that differ by case, trailing separator or normalization.
#[test]
fn nested_roots_are_refused() {
    let env = Env::new();
    let (_, root) = env.add_root("A\u{00F1}o");
    fs::create_dir_all(root.join("dentro/mas")).unwrap();
    assert_eq!(
        overlap(env.vfs.add_root(&root.join("dentro/mas"))),
        OverlapRelation::Inside
    );
    assert_eq!(
        overlap(env.vfs.add_root(&env.base())),
        OverlapRelation::Contains
    );
    let mut trailing = root.clone().into_os_string();
    trailing.push(std::path::MAIN_SEPARATOR_STR);
    assert_eq!(
        overlap(env.vfs.add_root(Path::new(&trailing))),
        OverlapRelation::Same
    );
    let base = env.base();
    for variant in ["A\u{00D1}O", "an\u{0303}o"] {
        let p = base.join(variant);
        if p.is_dir() {
            // Same folder spelled differently (case- or normalization-insensitive FS).
            assert!(
                overlap(env.vfs.add_root(&p)) == OverlapRelation::Same,
                "{variant:?}"
            );
        }
    }
    assert_eq!(env.vfs.list_roots().unwrap().len(), 1);
}

/// VFS-1 AC-4, AC-7: two roots that become nested when one is moved inside the other: the
/// moved root turns unavailable, the outer root indexes the moved folder as its own
/// content, and moving it back out restores both.
#[test]
fn roots_nested_after_rename() {
    let env = Env::new();
    let (a_id, a) = env.add_root("a");
    let (b_id, b) = env.add_root("b");
    fill(&a, 2, 2);
    fill(&b, 3, 2);
    env.scan_ok();
    let inner = a.join("b");
    fs::rename(&b, &inner).unwrap();
    env.force_rewalk(a_id);
    env.scan_ok();
    assert_eq!(env.root_status(b_id), RootStatus::Unavailable);
    let disk = assert_index_matches_disk(&env, a_id, &a, Check::default());
    assert!(disk.files.contains_key("b/carpeta 000/fichero 00.txt"));
    assert_eq!(overlap(env.vfs.add_root(&inner)), OverlapRelation::Inside);

    fs::rename(&inner, &b).unwrap();
    env.force_rewalk(a_id);
    env.scan_ok();
    assert_eq!(env.root_status(b_id), RootStatus::Ready);
    assert_index_matches_disk(&env, a_id, &a, Check::default());
    assert_index_matches_disk(&env, b_id, &b, Check::default());
}

/// VFS-1 AC-4, AC-11 dates. Defect (fixed in `walk::unix_secs`): a modification time
/// before 1970 with a fractional second was truncated toward zero, so 1969-12-31
/// 23:59:59.5 was indexed as 1970-01-01 00:00:00 and APFS's clamped 1601 date was one
/// second late. Times are floored.
#[test]
fn pre_1970_fractional_mtime_is_floored() {
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let path = root.join("medio segundo antes.txt");
    let f = fs::File::create(&path).unwrap();
    f.set_modified(UNIX_EPOCH - Duration::from_millis(500))
        .unwrap();
    drop(f);
    let stored = floor_secs(fs::metadata(&path).unwrap().modified().unwrap());
    env.scan_ok();
    let (indexed, _) = index_snapshot(&env.reader(), root_id);
    assert_eq!(indexed["medio segundo antes.txt"].mtime, stored);
    if stored != 0 {
        assert_eq!(stored, -1, "the filesystem kept the half second");
    }
}

/// VFS-1 AC-4, AC-7. Defect (fixed in `walk::list_dir`): in a folder that can be listed but
/// whose entries cannot be stat'ed (`chmod 444`), every entry was silently skipped and the
/// folder marked clean, so restoring its permissions never brought its files back until
/// the next weekly re-walk. The listing now fails and the folder is retried.
#[cfg(unix)]
#[test]
fn unstattable_entries_are_retried() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let (root_id, root) = env.add_root("raiz");
    let dir = root.join("solo lectura");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("oculto.txt"), b"x").unwrap();
    let _restore = Restore(vec![dir.clone()]);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o444)).unwrap();
    let enforced = !can_list(&dir);
    let summary = env.scan_ok();
    if enforced {
        assert_eq!(summary.errors, 1);
        assert_eq!(env.pending_dirs(root_id), vec!["solo lectura".to_string()]);
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    env.scan_ok();
    let (indexed, _) = index_snapshot(&env.reader(), root_id);
    assert!(indexed.contains_key("solo lectura/oculto.txt"));
}
