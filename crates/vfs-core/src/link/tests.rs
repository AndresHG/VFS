//! Parser tests on links built here to the [MS-SHLLINK] layout. Real links written by the
//! Windows shell are read by `tests/windows_shortcuts.rs` on the Windows CI runner.

use super::*;

fn utf16z(s: &str) -> Vec<u8> {
    let mut v: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    v.extend([0, 0]);
    v
}

fn ansiz(s: &str) -> Vec<u8> {
    let mut v: Vec<u8> = s.chars().map(|c| c as u8).collect();
    v.push(0);
    v
}

/// Where a built link points.
enum Info<'a> {
    None,
    /// LocalBasePath + CommonPathSuffix; Unicode offsets when `wide`.
    Local {
        base: &'a str,
        suffix: &'a str,
        wide: bool,
    },
    /// CommonNetworkRelativeLink: `\\server\share`, an optional mapped drive letter.
    Net {
        name: &'a str,
        device: Option<&'a str>,
        suffix: &'a str,
        wide: bool,
    },
}

#[derive(Default)]
struct Link<'a> {
    id_list: bool,
    relative: Option<&'a str>,
    env: Option<&'a str>,
    prefer_env: bool,
    ansi_strings: bool,
}

fn link_info(info: &Info) -> Vec<u8> {
    match *info {
        Info::None => Vec::new(),
        Info::Local { base, suffix, wide } => {
            let header = if wide { 0x24 } else { 0x1C };
            let volume: Vec<u8> = [0x10u32, 3, 0x1234, 0x10]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .chain([0, 0, 0, 0])
                .collect();
            let vol_at = header;
            let base_at = vol_at + volume.len();
            let ansi_base = ansiz("IGNORED-IF-WIDE");
            let ansi_base = if wide { ansi_base } else { ansiz(base) };
            let suffix_at = base_at + ansi_base.len();
            let ansi_suffix = if wide { ansiz("") } else { ansiz(suffix) };
            let wbase_at = suffix_at + ansi_suffix.len();
            let wbase = utf16z(base);
            let wsuffix_at = wbase_at + wbase.len();
            let wsuffix = utf16z(suffix);
            let mut body = Vec::new();
            body.extend(volume);
            body.extend(ansi_base);
            body.extend(ansi_suffix);
            if wide {
                body.extend(wbase);
                body.extend(wsuffix);
            }
            let size = header + body.len();
            let mut out: Vec<u8> = [
                size as u32,
                header as u32,
                1,
                vol_at as u32,
                base_at as u32,
                0,
                suffix_at as u32,
            ]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
            if wide {
                out.extend((wbase_at as u32).to_le_bytes());
                out.extend((wsuffix_at as u32).to_le_bytes());
            }
            out.extend(body);
            out
        }
        Info::Net {
            name,
            device,
            suffix,
            wide,
        } => {
            let header = if wide { 0x24 } else { 0x1C };
            // CommonNetworkRelativeLink.
            let cnrl_header = if wide { 0x1C } else { 0x14 };
            let a_name = ansiz(name);
            let a_dev = ansiz(device.unwrap_or(""));
            let w_name = utf16z(name);
            let w_dev = utf16z(device.unwrap_or(""));
            let name_at = cnrl_header;
            let dev_at = name_at + a_name.len();
            let wname_at = dev_at + a_dev.len();
            let wdev_at = wname_at + w_name.len();
            let mut cnrl_body: Vec<u8> = Vec::new();
            cnrl_body.extend(&a_name);
            cnrl_body.extend(&a_dev);
            if wide {
                cnrl_body.extend(&w_name);
                cnrl_body.extend(&w_dev);
            }
            let cnrl_size = cnrl_header + cnrl_body.len();
            let flags = if device.is_some() { 1u32 } else { 0 } | 2;
            let mut cnrl: Vec<u8> = [
                cnrl_size as u32,
                flags,
                name_at as u32,
                dev_at as u32,
                0x0002_0000,
            ]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
            if wide {
                cnrl.extend((wname_at as u32).to_le_bytes());
                cnrl.extend((wdev_at as u32).to_le_bytes());
            }
            cnrl.extend(cnrl_body);
            let cnrl_at = header;
            let a_suffix = if wide { ansiz("") } else { ansiz(suffix) };
            let suffix_at = cnrl_at + cnrl.len();
            let w_suffix = utf16z(suffix);
            let wsuffix_at = suffix_at + a_suffix.len();
            let mut body = cnrl;
            body.extend(a_suffix);
            if wide {
                body.extend(w_suffix);
            }
            let size = header + body.len();
            let mut out: Vec<u8> = [
                size as u32,
                header as u32,
                2,
                0,
                0,
                cnrl_at as u32,
                suffix_at as u32,
            ]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
            if wide {
                out.extend(0u32.to_le_bytes());
                out.extend((wsuffix_at as u32).to_le_bytes());
            }
            out.extend(body);
            out
        }
    }
}

fn build(info: Info, l: Link) -> Vec<u8> {
    let mut flags = 0u32;
    if l.id_list {
        flags |= HAS_ID_LIST;
    }
    let info_bytes = link_info(&info);
    if !info_bytes.is_empty() {
        flags |= HAS_LINK_INFO;
    }
    if !l.ansi_strings {
        flags |= IS_UNICODE;
    }
    // A name (comment) always precedes the relative path, to check the strings are walked.
    flags |= HAS_NAME;
    if l.relative.is_some() {
        flags |= HAS_RELATIVE_PATH;
    }
    if l.prefer_env {
        flags |= PREFER_ENVIRONMENT_PATH;
    }
    let mut out = vec![0u8; HEADER_SIZE];
    out[0..4].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
    out[4..20].copy_from_slice(&LINK_CLSID);
    out[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
    if l.id_list {
        // A two-byte terminator only, preceded by a dummy item.
        let items = [4u8, 0, 0x1F, 0x50, 0, 0];
        out.extend((items.len() as u16).to_le_bytes());
        out.extend(items);
    }
    out.extend(info_bytes);
    let mut string = |s: &str| {
        if l.ansi_strings {
            out.extend((s.len() as u16).to_le_bytes());
            out.extend(s.chars().map(|c| c as u8));
        } else {
            let u: Vec<u16> = s.encode_utf16().collect();
            out.extend((u.len() as u16).to_le_bytes());
            out.extend(u.iter().flat_map(|u| u.to_le_bytes()));
        }
    };
    string("Comentario");
    if let Some(r) = l.relative {
        string(r);
    }
    if let Some(e) = l.env {
        out.extend(0x314u32.to_le_bytes());
        out.extend(ENVIRONMENT_BLOCK.to_le_bytes());
        let mut ansi = ansiz(e);
        ansi.resize(260, 0);
        out.extend(ansi);
        let mut wide = utf16z(e);
        wide.resize(520, 0);
        out.extend(wide);
    }
    out.extend(0u32.to_le_bytes());
    out
}

fn local(base: &str, suffix: &str) -> Info<'static> {
    let base: &'static str = Box::leak(base.to_owned().into_boxed_str());
    let suffix: &'static str = Box::leak(suffix.to_owned().into_boxed_str());
    Info::Local {
        base,
        suffix,
        wide: true,
    }
}

/// VFS-18 AC-2: the target is the LinkInfo local path, in the Unicode and the code-page
/// form, accents and long paths included.
#[test]
fn local_targets() {
    let b = build(local("C:\\Fotos\\Boda 2003\\", ""), Link::default());
    assert_eq!(
        parse_lnk(&b, None).as_deref(),
        Some("C:\\Fotos\\Boda 2003\\")
    );
    let b = build(local("D:\\Ñandú\\Café ☕.txt", ""), Link::default());
    assert_eq!(
        parse_lnk(&b, None).as_deref(),
        Some("D:\\Ñandú\\Café ☕.txt")
    );
    let b = build(
        Info::Local {
            base: "C:\\Documentos\\Notaría Pérez.pdf",
            suffix: "",
            wide: false,
        },
        Link {
            ansi_strings: true,
            id_list: true,
            ..Link::default()
        },
    );
    assert_eq!(
        parse_lnk(&b, None).as_deref(),
        Some("C:\\Documentos\\Notaría Pérez.pdf")
    );
    let long = format!("C:\\{}\\fin.txt", "carpeta larga\\".repeat(40));
    let b = build(local(&long, ""), Link::default());
    assert_eq!(parse_lnk(&b, None), Some(long));
}

/// VFS-18 AC-2: a network target is the mapped drive letter when the link recorded one,
/// else the `\\server\share` name, joined with the common path suffix.
#[test]
fn network_targets() {
    for wide in [true, false] {
        let b = build(
            Info::Net {
                name: "\\\\DISKSTATION\\fotos",
                device: Some("Z:"),
                suffix: "Boda\\album.jpg",
                wide,
            },
            Link::default(),
        );
        assert_eq!(
            parse_lnk(&b, None).as_deref(),
            Some("Z:\\Boda\\album.jpg"),
            "wide {wide}"
        );
        let b = build(
            Info::Net {
                name: "\\\\DISKSTATION\\fotos",
                device: None,
                suffix: "Boda",
                wide,
            },
            Link::default(),
        );
        assert_eq!(
            parse_lnk(&b, None).as_deref(),
            Some("\\\\DISKSTATION\\fotos\\Boda")
        );
    }
}

/// VFS-18 AC-2: without LinkInfo the environment-variable target is expanded from this
/// process's environment, or the relative path is resolved against the link's folder;
/// a link asking for its environment path gets it even when LinkInfo exists.
#[test]
fn environment_and_relative_targets() {
    // SAFETY: tests in this binary that read the environment use their own variable.
    unsafe { std::env::set_var("VFS18_TEST_HOME", "C:\\Users\\Carmen") };
    let b = build(
        Info::None,
        Link {
            env: Some("%VFS18_TEST_HOME%\\Documents\\x.txt"),
            ..Link::default()
        },
    );
    assert_eq!(
        parse_lnk(&b, None).as_deref(),
        Some("C:\\Users\\Carmen\\Documents\\x.txt")
    );
    let b = build(
        Info::None,
        Link {
            env: Some("%VFS18_UNSET_VARIABLE%\\x"),
            ..Link::default()
        },
    );
    assert_eq!(
        parse_lnk(&b, None).as_deref(),
        Some("%VFS18_UNSET_VARIABLE%\\x")
    );
    let b = build(
        local("C:\\old\\x.txt", ""),
        Link {
            env: Some("%VFS18_TEST_HOME%\\x.txt"),
            prefer_env: true,
            ..Link::default()
        },
    );
    assert_eq!(
        parse_lnk(&b, None).as_deref(),
        Some("C:\\Users\\Carmen\\x.txt")
    );
    let b = build(
        Info::None,
        Link {
            relative: Some("..\\Fotos\\boda.jpg"),
            ..Link::default()
        },
    );
    assert_eq!(
        parse_lnk(&b, Some("D:\\Archivo\\Accesos")).as_deref(),
        Some("D:\\Archivo\\Fotos\\boda.jpg")
    );
    assert_eq!(parse_lnk(&b, None), None);
    assert_eq!(
        parse_lnk(&b, Some("/Users/c/Accesos")).as_deref(),
        Some("/Users/c/Fotos/boda.jpg")
    );
}

/// VFS-18 AC-2: a link with no file-system target (only a shell item list, as for a
/// Control Panel item) yields no target.
#[test]
fn no_target() {
    let b = build(
        Info::None,
        Link {
            id_list: true,
            ..Link::default()
        },
    );
    assert_eq!(parse_lnk(&b, None), None);
}

/// Deterministic pseudo-random bytes (xorshift64*).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// VFS-18 AC-9: every prefix of valid links, random bytes behind a valid header, random
/// byte flips and absurd lengths and offsets return a target or `None`, never panic.
#[test]
fn malformed_links_never_panic() {
    let samples = [
        build(local("C:\\Fotos\\Boda 2003", ""), Link::default()),
        build(
            Info::Net {
                name: "\\\\NAS\\x",
                device: Some("Z:"),
                suffix: "a\\b",
                wide: true,
            },
            Link {
                id_list: true,
                relative: Some("..\\a"),
                env: Some("%TEMP%\\a"),
                ..Link::default()
            },
        ),
        build(
            Info::Local {
                base: "C:\\a",
                suffix: "b",
                wide: false,
            },
            Link {
                ansi_strings: true,
                ..Link::default()
            },
        ),
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for s in &samples {
        for n in 0..=s.len() {
            let _ = parse_lnk(&s[..n], Some("C:\\x"));
        }
        for _ in 0..3000 {
            let mut m = s.clone();
            for _ in 0..1 + rng.next() % 6 {
                let i = (rng.next() as usize) % m.len();
                m[i] = match rng.next() % 4 {
                    0 => 0xFF,
                    1 => 0x00,
                    _ => rng.next() as u8,
                };
            }
            let _ = parse_lnk(&m, Some("C:\\x"));
        }
    }
    for _ in 0..3000 {
        let mut m = samples[0][..HEADER_SIZE].to_vec();
        m[0x14..0x18].copy_from_slice(&(rng.next() as u32).to_le_bytes());
        let extra = (rng.next() % 512) as usize;
        m.extend((0..extra).map(|_| rng.next() as u8));
        let _ = parse_lnk(&m, None);
    }
    assert_eq!(parse_lnk(b"", None), None);
    assert_eq!(parse_lnk(&[0xFF; 200], None), None);
}

/// VFS-18 AC-7: the address of an Internet Shortcut, in UTF-8 or the code page, with or
/// without a byte-order mark; other sections and malformed files give none.
#[test]
fn internet_shortcuts() {
    let f = b"[{000214A0-0000-0000-C000-000000000046}]\r\nProp3=19,11\r\n[InternetShortcut]\r\nIDList=\r\nURL=https://www.cocina.es/flan?x=1\r\nIconIndex=0\r\n";
    assert_eq!(
        parse_url(f).as_deref(),
        Some("https://www.cocina.es/flan?x=1")
    );
    let bom = "\u{FEFF}[InternetShortcut]\nurl = https://a.es/ñ\n";
    assert_eq!(parse_url(bom.as_bytes()).as_deref(), Some("https://a.es/ñ"));
    assert_eq!(
        parse_url(b"[internetshortcut]\nURL=http://caf\xe9.es\n").as_deref(),
        Some("http://café.es")
    );
    assert_eq!(parse_url(b"[Other]\nURL=https://a.es\n"), None);
    assert_eq!(parse_url(b"URL=https://a.es\n"), None);
    assert_eq!(parse_url(b"[InternetShortcut]\nURL=\n"), None);
    assert_eq!(parse_url(&[0xFF; 300]), None);
    assert!(is_openable_url("https://a.es/x?y=1"));
    assert!(!is_openable_url("file:///C:/x"));
    assert!(!is_openable_url("javascript:alert(1)"));
    assert!(!is_openable_url("https://a.es/\" --x"));
}

/// VFS-18 AC-1: the extension decides which files are read as shortcuts.
#[test]
fn shortcut_names() {
    assert_eq!(link_file_kind("Fotos boda.lnk"), Some(LinkFileKind::Lnk));
    assert_eq!(link_file_kind("BANCO.URL"), Some(LinkFileKind::Url));
    assert_eq!(link_file_kind(".lnk"), None);
    assert_eq!(link_file_kind("lnk"), None);
    assert_eq!(link_file_kind("informe.lnk.pdf"), None);
}

/// VFS-18 AC-9: at most `MAX_READ` bytes are read from a shortcut file, however large.
#[test]
fn reads_a_bounded_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("grande.lnk");
    let mut b = build(local("C:\\a.txt", ""), Link::default());
    b.resize(10 * MAX_READ, 0xAB);
    std::fs::write(&p, &b).unwrap();
    assert_eq!(
        read_target(&p, LinkFileKind::Lnk).unwrap().as_deref(),
        Some("C:\\a.txt")
    );
    let u = dir.path().join("x.url");
    std::fs::write(&u, b"[InternetShortcut]\nURL=https://a.es\n").unwrap();
    assert_eq!(
        read_target(&u, LinkFileKind::Url).unwrap().as_deref(),
        Some("https://a.es")
    );
}

struct QuietSink;
impl crate::ProgressSink for QuietSink {
    fn on_progress(&self, _: &crate::IndexProgress) {}
}

/// VFS-18 AC-3 (R3): a shortcut to a share on a server that never answers (192.0.2.1, the
/// TEST-NET-1 block) is indexed, read, found by its target and shown with no access to the
/// target: the whole run ends well under the seconds an SMB connection attempt waits on
/// Windows, where the target would be a network path if any call touched it.
#[test]
fn unreachable_network_target_is_never_contacted() {
    let tree = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tree.path()).unwrap();
    std::fs::create_dir(root.join("Familia")).unwrap();
    let target = "\\\\192.0.2.1\\fotos\\Boda 2003";
    std::fs::write(
        root.join("Familia").join("Fotos NAS.lnk"),
        build(
            Info::Net {
                name: "\\\\192.0.2.1\\fotos",
                device: None,
                suffix: "Boda 2003",
                wide: true,
            },
            Link::default(),
        ),
    )
    .unwrap();

    let started = std::time::Instant::now();
    let vfs = crate::Vfs::open(&db.path().join("index.db")).unwrap();
    vfs.add_root(&root).unwrap();
    vfs.start_indexer_with(
        std::sync::Arc::new(QuietSink),
        crate::IndexerOptions {
            boot_grace: std::time::Duration::ZERO,
        },
    )
    .unwrap();
    let query = crate::SearchQuery {
        text: "boda".into(),
        filters: Default::default(),
        limit: 10,
        offset: 0,
        sort: Default::default(),
    };
    let hit = loop {
        let found = vfs.search(&query).unwrap().hits.into_iter().find(|h| {
            h.link
                .as_ref()
                .is_some_and(|l| l.target.as_deref() == Some(target))
        });
        if let Some(hit) = found {
            break hit;
        }
        assert!(
            started.elapsed().as_secs() < 10,
            "never found by its target"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    // Displaying the row reads what the index holds, nothing more.
    let shown = vfs.link_target(hit.id).unwrap();
    assert_eq!(shown.target.as_deref(), Some(target));
    assert_eq!(hit.matched, crate::MatchKind::Target);
    vfs.shutdown();
    assert!(
        started.elapsed().as_secs() < 5,
        "took {:?}: something waited on the network",
        started.elapsed()
    );
}
