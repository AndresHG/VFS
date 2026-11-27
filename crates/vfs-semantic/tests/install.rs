//! Downloading the local model (VFS-29 R1a, R16, D18) through a fake fetcher: every file is
//! checked before anything counts as installed, and a bad or stopped download installs
//! nothing. The signature fixture was made with a throwaway minisign key for these tests.

use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::AtomicBool;

use vfs_semantic::install::{self, Check, Download, Fetch, InstallError};

const WORKER: &[u8] = b"fake worker bytes for the VFS-29 install test\n";
/// Base64 of a throwaway minisign public key (test only).
const TEST_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXkgODg0NzUyODE2ODBERDNGOApSV1Q0MHcxb2dWSkhpRnNQKzVPbGtPb0FhK2tKeEtsUFh1aUJDR1F1d1FWUXQvS1ZPaGIvdDhKeAo=";
/// Base64 of that key's signature of `WORKER`.
const TEST_SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIG1pbmlzaWduIHNlY3JldCBrZXkKUlVUNDB3MW9nVkpIaUhYRXl6Nkl6cmxiaUZFNDZPSC8vdnpnYXJNQmMyTTZMc1pnL0dmSzgwbFo3ZXgzclR1SDRhY2ZaWCtBUGNvZVlvbStlMFZNUkRoTmp6M2M5RlBHRFEwPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk1NzM3ODg0CWZpbGU6d29ya2VyLmJpbgloYXNoZWQKZkc3ZmxUc1l0WWo2SW10UlF1Zk9vYUFGMnRDMSttZGZCem41UGVOTWZvRzgxOHF5VmJPeUp6d0Q0ZUtnWE05aEhSc1FUYTlQL3YyL1JxOStrQzV4Q2c9PQo=";
/// SHA-256 of b"model".
const MODEL_SHA: &str = "9372c470eeadd5ecd9c3c74c2b3cb633f8e2f2fad799250a0f70d652b6b825e4";

struct Fake(HashMap<String, Vec<u8>>);

impl Fetch for Fake {
    fn get(&self, url: &str) -> std::io::Result<Box<dyn Read + Send>> {
        match self.0.get(url) {
            Some(b) => Ok(Box::new(std::io::Cursor::new(b.clone()))),
            None => Err(std::io::Error::other(format!("404 {url}"))),
        }
    }
}

fn downloads() -> Vec<Download> {
    vec![
        Download {
            file: "model.onnx".into(),
            url: "https://h/model".into(),
            check: Check::Sha256(MODEL_SHA),
            bytes: Some(5),
            executable: false,
        },
        Download {
            file: "vfs-embed".into(),
            url: "https://r/worker".into(),
            check: Check::Signed {
                sig_url: "https://r/worker.sig".into(),
                pubkey: TEST_PUBKEY.into(),
            },
            bytes: None,
            executable: true,
        },
    ]
}

fn server(model: &[u8], worker: &[u8], sig: &str) -> Fake {
    Fake(HashMap::from([
        ("https://h/model".to_string(), model.to_vec()),
        ("https://r/worker".to_string(), worker.to_vec()),
        ("https://r/worker.sig".to_string(), sig.as_bytes().to_vec()),
    ]))
}

/// VFS-29 R1a, R16: the files download, pass their checks and count as installed; progress
/// reaches the total.
#[test]
fn a_checked_download_installs_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let d = downloads();
    assert!(!install::installed(dir.path(), &d));
    let mut last = (0, 0);
    install::install(
        dir.path(),
        &d,
        &server(b"model", WORKER, TEST_SIG),
        &AtomicBool::new(false),
        &mut |done, total| last = (done, total),
    )
    .unwrap();
    assert!(install::installed(dir.path(), &d));
    assert_eq!(std::fs::read(dir.path().join("vfs-embed")).unwrap(), WORKER);
    assert_eq!(last.0, 5 + WORKER.len() as u64);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("vfs-embed"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the worker is executable");
    }
}

/// VFS-29 R16: a model with the wrong digest, or a worker whose signature does not match, is
/// refused and nothing counts as installed.
#[test]
fn a_file_that_fails_its_check_installs_nothing() {
    for (model, worker) in [
        (&b"modex"[..], WORKER),
        (&b"model"[..], &b"tampered worker"[..]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let d = downloads();
        let err = install::install(
            dir.path(),
            &d,
            &server(model, worker, TEST_SIG),
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap_err();
        assert!(matches!(err, InstallError::Check(_)), "{err}");
        assert!(!install::installed(dir.path(), &d));
        assert!(!dir.path().join("vfs-embed").exists());
    }
}

/// VFS-29 R16: a signature made with another key is refused.
#[test]
fn a_signature_from_another_key_is_refused() {
    assert!(install::verify_signature(WORKER, TEST_SIG, TEST_PUBKEY));
    assert!(!install::verify_signature(b"other", TEST_SIG, TEST_PUBKEY));
    assert!(!install::verify_signature(WORKER, TEST_SIG, "bm90IGEga2V5"));
    assert!(!install::verify_signature(WORKER, "garbage", TEST_PUBKEY));
}

/// VFS-29 R7: a stopped or failed download installs nothing; a later one completes.
#[test]
fn a_stopped_download_installs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let d = downloads();
    let err = install::install(
        dir.path(),
        &d,
        &server(b"model", WORKER, TEST_SIG),
        &AtomicBool::new(true),
        &mut |_, _| {},
    )
    .unwrap_err();
    assert!(matches!(err, InstallError::Stopped));
    let missing = Fake(HashMap::new());
    let err = install::install(
        dir.path(),
        &d,
        &missing,
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap_err();
    assert!(matches!(err, InstallError::Network(_)));
    assert!(!install::installed(dir.path(), &d));
    install::install(
        dir.path(),
        &d,
        &server(b"model", WORKER, TEST_SIG),
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    assert!(install::installed(dir.path(), &d));
}

/// VFS-29 R1a: the size shown before downloading counts the model, its tokenizer and the
/// worker; the worker comes from this platform's release asset with its signature.
#[test]
fn the_local_model_lists_its_files_and_size() {
    let d = install::local_model("https://example.test/releases/download/v1/", TEST_PUBKEY);
    assert_eq!(d.len(), 3);
    let bytes = install::download_bytes(&d);
    assert!(bytes > 135_000_000 && bytes < 200_000_000, "{bytes}");
    let worker = d.iter().find(|x| x.executable).unwrap();
    assert!(worker.url.ends_with(&install::worker_asset()));
    assert!(matches!(&worker.check, Check::Signed { sig_url, .. } if sig_url.ends_with(".sig")));
}

/// VFS-29 R7: a retry downloads only what is missing: a file already in place that passes
/// its check is kept.
#[test]
fn a_retry_keeps_checked_files() {
    let dir = tempfile::tempdir().unwrap();
    let d = downloads();
    std::fs::write(dir.path().join("model.onnx"), b"model").unwrap();
    let worker_only = Fake(HashMap::from([
        ("https://r/worker".to_string(), WORKER.to_vec()),
        (
            "https://r/worker.sig".to_string(),
            TEST_SIG.as_bytes().to_vec(),
        ),
    ]));
    install::install(
        dir.path(),
        &d,
        &worker_only,
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    assert!(install::installed(dir.path(), &d));
}
