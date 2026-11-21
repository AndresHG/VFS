//! Real shortcuts written by the Windows shell (`WScript.Shell`, the same `IShellLink`
//! Explorer uses), read back by `vfs_core::link` (VFS-18 AC-2, AC-9). Runs only on Windows;
//! CI runs it on the `windows-latest` runner, where it also measures the cost per shortcut.
#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use vfs_core::link::{read_target, LinkFileKind};

/// Writes one shortcut per `(link, target)` pair with one PowerShell process. The script
/// goes in a UTF-8 file with a byte-order mark, the only way Windows PowerShell 5 reads
/// non-ASCII text intact (a `-Command` argument passes through the console code page).
fn make_links(pairs: &[(PathBuf, String)]) {
    let mut script = String::from(
        "\u{FEFF}$ErrorActionPreference='Stop'\r\n$s=New-Object -ComObject WScript.Shell\r\n",
    );
    for (link, target) in pairs {
        let q = |s: &str| s.replace('\'', "''");
        script.push_str(&format!(
            "$l=$s.CreateShortcut('{}'); $l.TargetPath='{}'; $l.Save()\r\n",
            q(link.to_str().unwrap()),
            q(target)
        ));
    }
    run_script(&script);
}

/// Runs a PowerShell script from a UTF-8 file and fails the test with its error output.
fn run_script(script: &str) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("links.ps1");
    fs::write(&file, script).unwrap();
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&file)
        .output()
        .expect("powershell");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Points the existing shortcut `link` at `target` through `Shell.Application`.
fn set_link_path(link: &Path, target: &str) {
    let q = |s: &str| s.replace('\'', "''");
    let script = format!(
        "\u{FEFF}$ErrorActionPreference='Stop'\r\n$sh=New-Object -ComObject Shell.Application\r\n\
         $i=$sh.NameSpace('{}').ParseName('{}')\r\n$k=$i.GetLink\r\n$k.Path='{}'\r\n$k.Save()\r\n",
        q(link.parent().unwrap().to_str().unwrap()),
        q(link.file_name().unwrap().to_str().unwrap()),
        q(target)
    );
    run_script(&script);
}

/// Runs `subst` (no administrator rights needed) with `args`.
fn subst(args: &[&str]) {
    let st = Command::new("subst").args(args).status().expect("subst");
    assert!(st.success(), "subst {args:?}");
}

fn target(link: &Path) -> Option<String> {
    read_target(link, LinkFileKind::Lnk).unwrap()
}

fn same_path(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\')
        .eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

/// One line in the CI log, as a GitHub Actions notice (the PR's measured numbers and what
/// the shell wrote are read there). Written to the stderr handle directly: libtest captures
/// only the `print!` macros.
fn report(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "::notice::{line}");
}

#[track_caller]
fn assert_target(link: &Path, expected: &str) {
    let got = target(link);
    report(&format!(
        "- VFS-18 `{}` ({} bytes) -> `{got:?}`",
        link.file_name().unwrap().to_string_lossy(),
        fs::metadata(link).map_or(0, |m| m.len())
    ));
    assert!(
        got.as_deref().is_some_and(|g| same_path(g, expected)),
        "{}: got {got:?}, expected {expected}",
        link.display()
    );
}

/// `p` without the 8.3 short names `%TEMP%` can hold on the runner (`RUNNER~1`), which the
/// shell expands when it records a target.
fn long(p: &Path) -> String {
    let c = fs::canonicalize(p).unwrap();
    let s = c.to_str().unwrap();
    s.strip_prefix(r"\\?\").unwrap_or(s).to_owned()
}

/// VFS-18 AC-2: local file, local folder, non-ASCII target, long path, missing target
/// (deleted after the link was made), and a shortcut made to a shortcut.
#[test]
fn shell_links_to_local_targets() {
    let t = tempfile::tempdir().unwrap();
    let root = PathBuf::from(long(t.path()));
    let file = root.join("informe.txt");
    fs::write(&file, b"x").unwrap();
    let folder = root.join("Fotos boda 2003");
    fs::create_dir(&folder).unwrap();
    // Fits the ANSI code page (stored as Windows-1252 by the shell).
    let accents = root.join("Ñandú").join("Café niño.txt");
    fs::create_dir(accents.parent().unwrap()).unwrap();
    fs::write(&accents, b"x").unwrap();
    // Does not fit it (stored as UTF-16 by the shell).
    let wide = root.join("Φωτογραφίες ☕");
    fs::create_dir(&wide).unwrap();
    let mut deep = root.join("larga");
    while deep.as_os_str().len() < 240 {
        deep.push("carpeta con nombre largo");
    }
    fs::create_dir_all(&deep).unwrap();
    let gone = root.join("borrado.txt");
    fs::write(&gone, b"x").unwrap();

    let links = root.join("accesos");
    fs::create_dir(&links).unwrap();
    let l = |n: &str| links.join(n);
    make_links(&[
        (l("archivo.lnk"), file.to_str().unwrap().into()),
        (l("carpeta.lnk"), folder.to_str().unwrap().into()),
        (l("acentos.lnk"), accents.to_str().unwrap().into()),
        (l("largo.lnk"), deep.to_str().unwrap().into()),
        (l("borrado.lnk"), gone.to_str().unwrap().into()),
    ]);
    make_links(&[(
        l("a un acceso.lnk"),
        l("archivo.lnk").to_str().unwrap().into(),
    )]);
    fs::remove_file(&gone).unwrap();

    assert_target(&l("archivo.lnk"), file.to_str().unwrap());
    assert_target(&l("carpeta.lnk"), folder.to_str().unwrap());
    assert_target(&l("acentos.lnk"), accents.to_str().unwrap());
    // `WScript.Shell` refuses targets outside the ANSI code page; the Shell.Application
    // link object (`IShellLinkW`) sets one on an existing link.
    make_links(&[(l("unicode.lnk"), file.to_str().unwrap().into())]);
    set_link_path(&l("unicode.lnk"), wide.to_str().unwrap());
    assert_target(&l("unicode.lnk"), wide.to_str().unwrap());
    assert_target(&l("largo.lnk"), deep.to_str().unwrap());
    assert_target(&l("borrado.lnk"), gone.to_str().unwrap());
    // The shell itself resolves a shortcut given as a target when it writes the new link
    // (unless the link allows link-to-link): its recorded target is the final file. VFS only
    // reads what was recorded and never opens the first shortcut to follow it.
    assert_target(&l("a un acceso.lnk"), file.to_str().unwrap());
}

/// VFS-18 AC-2: a target on a drive letter that is no longer present (an unplugged USB
/// disk: here a `subst` drive removed after the link was made) keeps its path. A link made
/// while its target did not exist holds no LinkInfo: its target comes from the relative
/// path the shell records next to the item list.
#[test]
fn shell_links_to_absent_drives() {
    let t = tempfile::tempdir().unwrap();
    let root = PathBuf::from(long(t.path()));
    let media = root.join("usb");
    fs::create_dir(&media).unwrap();
    fs::write(media.join("viaje.jpg"), b"x").unwrap();
    let letter = "Q:";
    subst(&[letter, media.to_str().unwrap()]);
    let made = std::panic::catch_unwind(|| {
        make_links(&[(root.join("usb.lnk"), format!(r"{letter}\viaje.jpg"))]);
    });
    subst(&[letter, "/D"]);
    made.unwrap();
    assert_target(&root.join("usb.lnk"), r"Q:\viaje.jpg");

    let never = root.join("nunca existió").join("x.txt");
    make_links(&[(root.join("nunca.lnk"), never.to_str().unwrap().into())]);
    assert_target(&root.join("nunca.lnk"), never.to_str().unwrap());
}

/// VFS-18 AC-2, AC-3: a target on a network share (the runner's administrative share of
/// its own system drive) is read as its UNC path; reading it makes no network access.
#[test]
fn shell_links_to_network_targets() {
    let t = tempfile::tempdir().unwrap();
    let root = PathBuf::from(long(t.path()));
    let file = root.join("en red.txt");
    fs::write(&file, b"x").unwrap();
    let local = file.to_str().unwrap();
    let unc = format!(r"\\localhost\{}${}", &local[..1], &local[2..]);
    make_links(&[(root.join("red.lnk"), unc.clone())]);
    assert_target(&root.join("red.lnk"), &unc);
}

/// VFS-18 AC-9 and the cost per shortcut on the runner (Defender scanning on): 2,000 real
/// links are read cold-ish (written once, read once) and the mean is printed for the PR.
#[test]
fn cost_per_shortcut() {
    let t = tempfile::tempdir().unwrap();
    let root = PathBuf::from(long(t.path()));
    let target = root.join("destino.txt");
    fs::write(&target, b"x").unwrap();
    let pairs: Vec<_> = (0..2000)
        .map(|i| {
            (
                root.join(format!("acceso {i:04}.lnk")),
                target.to_str().unwrap().to_owned(),
            )
        })
        .collect();
    for chunk in pairs.chunks(250) {
        make_links(chunk);
    }
    let bytes: u64 = pairs
        .iter()
        .map(|(l, _)| fs::metadata(l).unwrap().len())
        .sum();
    let start = Instant::now();
    for (l, _) in &pairs {
        assert!(target_is(l, &target));
    }
    let per = start.elapsed().as_secs_f64() * 1e6 / pairs.len() as f64;
    report(&format!(
        "- VFS-18 cost per shortcut: {per:.1} µs mean over {} links, {} bytes each on average",
        pairs.len(),
        bytes / pairs.len() as u64
    ));
}

fn target_is(link: &Path, target_path: &Path) -> bool {
    target(link).is_some_and(|t| same_path(&t, target_path.to_str().unwrap()))
}
