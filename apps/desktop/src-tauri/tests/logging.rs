//! The desktop log file (VFS-1 C7, in the local app data directory of C6). Its own test
//! binary, because the logger is process-wide and can be installed once.

use tauri::test::{mock_builder, mock_context, noop_assets};
use vfs_desktop_lib::install_logger;

/// VFS-1 C7: warnings and errors reach `<local data>/logs/vfs.log`; info-level messages,
/// which could name the user's files, are never written.
/// VFS-5 AC-9: info records of the diagnostic target are written; info from any other
/// target is not.
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
}
