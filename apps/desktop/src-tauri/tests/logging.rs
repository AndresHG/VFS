//! The desktop log file (VFS-1 C7, in the local app data directory of C6). Its own test
//! binary, because the logger is process-wide and can be installed once.

use tauri::test::{mock_builder, mock_context, noop_assets};
use vfs_desktop_lib::feedback::{
    install_panic_hook, parse_line, read_logs, redact_line, OMITTED, PANIC_TARGET,
};
use vfs_desktop_lib::{install_logger, LOG_KEEP, LOG_MAX_BYTES};

/// VFS-1 C7: warnings and errors reach `<local data>/logs/vfs.log`; info-level messages,
/// which could name the user's files, are never written.
/// VFS-5 AC-9: info records of the diagnostic target are written; info from any other
/// target is not.
/// VFS-20 AC-6: a panic is written as an error with its location; the file rotates at
/// `LOG_MAX_BYTES`, keeps `LOG_KEEP` old files, and they hold at least 20 MB (several weeks).
/// VFS-20 AC-2: the report's parser reads the lines this logger writes (target, level,
/// message), so a diagnostic record is kept and a panic loses its message.
#[test]
fn warnings_go_to_the_log_file_and_info_does_not() {
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app");
    let dir = tempfile::tempdir().unwrap();
    install_logger(app.handle(), dir.path()).unwrap();
    log::warn!("cannot list a folder: permission denied");
    log::info!("Notaría Pérez escritura 2019.pdf");
    log::info!(target: vfs_core::DIAG_TARGET, "pass mode=walk dirs=12");
    log::info!(target: "vfs_core::indexer", "Herencia Gómez.docx");
    log::logger().flush();
    let text = std::fs::read_to_string(dir.path().join("logs").join("vfs.log")).unwrap();
    assert!(text.contains("cannot list a folder"), "{text}");
    assert!(text.contains("pass mode=walk dirs=12"), "{text}");
    assert!(!text.contains("Notaría"), "{text}");
    assert!(!text.contains("Herencia"), "{text}");

    let warn = text
        .lines()
        .find(|l| l.contains("cannot list a folder"))
        .unwrap();
    let (target, level, start) = parse_line(warn).unwrap_or_else(|| panic!("unparsed {warn}"));
    assert_eq!((target, level), ("logging", "WARN"), "{warn}");
    assert_eq!(&warn[start..], "cannot list a folder: permission denied");
    let diag = text.lines().find(|l| l.contains("pass mode=walk")).unwrap();
    assert_eq!(parse_line(diag).map(|p| p.0), Some(vfs_core::DIAG_TARGET));
    assert_eq!(redact_line(diag, &[], false), diag);

    install_panic_hook();
    let panicked = std::thread::spawn(|| panic!("simulated failure 42")).join();
    assert!(panicked.is_err());
    let text = std::fs::read_to_string(dir.path().join("logs").join("vfs.log")).unwrap();
    let line = text
        .lines()
        .find(|l| l.contains(PANIC_TARGET))
        .unwrap_or_else(|| panic!("no panic record in {text}"));
    assert!(line.contains("[ERROR]"), "{line}");
    assert!(line.contains("simulated failure 42"), "{line}");
    assert!(line.contains("logging.rs:"), "{line}");
    let redacted = redact_line(line, &[], false);
    assert!(redacted.ends_with(&format!(": {OMITTED}")), "{redacted}");
    assert!(redacted.contains("[ERROR] panic at "), "{redacted}");
    assert!(redacted.contains("logging.rs:"), "{redacted}");
    assert!(!redacted.contains("simulated failure"), "{redacted}");

    // Enough warnings for two more rotations than are kept: exactly LOG_KEEP old files plus
    // the active one remain, together holding at least 20 MB of history.
    let filler = "x".repeat(1000);
    for _ in 0..((LOG_KEEP as u128 + 2) * LOG_MAX_BYTES / 1000) {
        log::warn!("{filler}");
    }
    log::logger().flush();
    let files = read_logs(&dir.path().join("logs"));
    assert_eq!(files.len(), LOG_KEEP + 1);
    assert!(files[0].name.starts_with("vfs_"), "{}", files[0].name);
    assert!(files
        .iter()
        .all(|f| f.text.len() as u128 <= LOG_MAX_BYTES + 2048));
    let kept: usize = files.iter().map(|f| f.text.len()).sum();
    assert!(kept >= 20 * 1024 * 1024, "{kept}");
}
