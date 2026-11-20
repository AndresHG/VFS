//! VFS-2 AC-11: the content commands exist only with the `content` feature, and with it the
//! CLI indexes a folder of documents and finds a phrase with a highlighted snippet.

use std::process::{Command, Output};

fn vfs(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vfs"))
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// VFS-2 AC-11: index a folder, content-index it, find a quoted phrase with «» highlights.
#[cfg(feature = "content")]
#[test]
fn content_commands_find_a_phrase() {
    let tmp = tempfile::tempdir().unwrap();
    let docs = tmp.path().join("Documentos");
    std::fs::create_dir(&docs).unwrap();
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../vfs-content/tests/fixtures");
    std::fs::copy(
        fixtures.join("muestra_docx.docx"),
        docs.join("escaneo0034.docx"),
    )
    .unwrap();
    std::fs::write(
        docs.join("otra.txt"),
        "El contrato fue firmado. Del alquiler, nada.",
    )
    .unwrap();
    let db = tmp.path().join("index.db");
    let db = db.to_str().unwrap();

    let o = vfs(&["index", "--db", db, docs.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    // Generous limit: the worker runs at background priority, which a busy machine starves.
    let o = vfs(&["content-index", "--db", db, "--timeout", "300"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("2 indexed"), "{}", text(&o));

    let o = vfs(&["content-search", "--db", db, "\"contrato de alquiler\""]);
    let out = text(&o);
    assert!(o.status.success(), "{out}");
    assert!(out.contains("escaneo0034.docx"), "{out}");
    assert!(!out.contains("otra.txt"), "{out}");
    assert!(out.contains("«contrato» «de» «alquiler»"), "{out}");

    let o = vfs(&[
        "content-search",
        "--db",
        db,
        "--layout",
        "sections",
        "contrato",
    ]);
    assert!(
        text(&o).contains("En el contenido de los documentos (2)"),
        "{}",
        text(&o)
    );

    let o = vfs(&["content-off", "--db", db]);
    assert!(o.status.success(), "{}", text(&o));
    let o = vfs(&["content-search", "--db", db, "contrato"]);
    assert!(
        !o.status.success() && text(&o).contains("off"),
        "{}",
        text(&o)
    );
}

/// VFS-2 AC-11: the default build has no content commands.
#[cfg(not(feature = "content"))]
#[test]
fn default_build_has_no_content_commands() {
    let o = vfs(&["content-index", "--db", "x.db"]);
    assert!(!o.status.success());
    assert!(text(&o).contains("unrecognized subcommand"), "{}", text(&o));
}
