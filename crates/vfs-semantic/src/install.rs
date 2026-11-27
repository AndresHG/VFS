//! Downloading the local model on request (VFS-29 R1a, R16, D18): the model and its
//! tokenizer from Hugging Face at a pinned revision, checked against pinned SHA-256 digests,
//! and the `vfs-embed` worker from this version's release, checked against the release
//! signature made with the updater's key. Nothing is installed until every file is complete
//! and checked; a stopped or failed download leaves no file behind but its `.part`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::LOCAL_MODEL;

/// How a downloaded file is checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// Its SHA-256, hex.
    Sha256(&'static str),
    /// A minisign signature at `sig_url` (base64 of the `.sig` text, as Tauri's updater
    /// writes it), verified with `pubkey` (base64 of the public key text, as in
    /// `tauri.conf.json`).
    Signed { sig_url: String, pubkey: String },
}

/// One file to download into the model folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    /// Its name in the model folder.
    pub file: String,
    pub url: String,
    pub check: Check,
    /// Its size, when known in advance (for the size shown before downloading, R1a).
    pub bytes: Option<u64>,
    pub executable: bool,
}

/// Something that fetches a URL: the app's HTTP client, or a fake in tests.
pub trait Fetch {
    fn get(&self, url: &str) -> std::io::Result<Box<dyn Read + Send>>;
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("stopped")]
    Stopped,
    #[error("download failed: {0}")]
    Network(String),
    #[error("{0} did not pass its check")]
    Check(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Hugging Face revision of multilingual-e5-small the digests below belong to.
const E5_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";
const E5_MODEL_SHA256: &str = "dd476dd0c2514e9b9be83aeb3853fac0763e0bdf4a71645407587d77c48a2d88";
const E5_MODEL_BYTES: u64 = 118_346_824;
const E5_TOKENIZER_SHA256: &str =
    "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39";
const E5_TOKENIZER_BYTES: u64 = 17_082_730;

/// The worker's size in a release build (22.4 MB on macOS arm64, 29c), until the release
/// gives the real one.
pub const WORKER_BYTES_ESTIMATE: u64 = 23_000_000;

/// The worker's file name in the model folder.
pub fn worker_file() -> String {
    format!("vfs-embed{}", std::env::consts::EXE_SUFFIX)
}

/// The release asset of the worker for this platform, e.g. `vfs-embed-windows-x86_64.exe`.
pub fn worker_asset() -> String {
    format!(
        "vfs-embed-{}-{}{}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::EXE_SUFFIX
    )
}

/// The files of the default local model (D16): `release_base` is this version's release
/// download URL, ending in `/`; `pubkey` is the updater's public key.
pub fn local_model(release_base: &str, pubkey: &str) -> Vec<Download> {
    let hf = format!("https://huggingface.co/intfloat/multilingual-e5-small/resolve/{E5_REVISION}");
    let asset = worker_asset();
    vec![
        Download {
            file: LOCAL_MODEL.model_file.into(),
            url: format!("{hf}/onnx/model_qint8_avx512_vnni.onnx"),
            check: Check::Sha256(E5_MODEL_SHA256),
            bytes: Some(E5_MODEL_BYTES),
            executable: false,
        },
        Download {
            file: LOCAL_MODEL.tokenizer_file.into(),
            url: format!("{hf}/tokenizer.json"),
            check: Check::Sha256(E5_TOKENIZER_SHA256),
            bytes: Some(E5_TOKENIZER_BYTES),
            executable: false,
        },
        Download {
            file: worker_file(),
            url: format!("{release_base}{asset}"),
            check: Check::Signed {
                sig_url: format!("{release_base}{asset}.sig"),
                pubkey: pubkey.into(),
            },
            bytes: None,
            executable: true,
        },
    ]
}

/// Bytes to download, for the size shown before asking (R1a).
pub fn download_bytes(downloads: &[Download]) -> u64 {
    downloads
        .iter()
        .map(|d| d.bytes.unwrap_or(WORKER_BYTES_ESTIMATE))
        .sum()
}

/// The marker written once every file is in place and checked.
const INSTALLED: &str = "installed.json";

/// Whether the model folder holds every file, checked (the marker lists them).
pub fn installed(dir: &Path, downloads: &[Download]) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join(INSTALLED)) else {
        return false;
    };
    let Ok(files) = serde_json::from_str::<Vec<String>>(&text) else {
        return false;
    };
    downloads
        .iter()
        .all(|d| files.contains(&d.url) && dir.join(&d.file).is_file())
}

/// Downloads and checks every file into `dir`; `progress(done, total)` follows the bytes.
/// `stop` abandons the download between reads.
pub fn install(
    dir: &Path,
    downloads: &[Download],
    fetch: &dyn Fetch,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), InstallError> {
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::remove_file(dir.join(INSTALLED));
    let total = download_bytes(downloads);
    let mut done = 0u64;
    for d in downloads {
        let part = dir.join(format!("{}.part", d.file));
        let result = fetch_one(d, &part, fetch, stop, &mut |n| {
            done += n;
            progress(done, total.max(done));
        });
        if let Err(e) = result {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        std::fs::rename(&part, dir.join(&d.file))?;
    }
    let urls: Vec<&str> = downloads.iter().map(|d| d.url.as_str()).collect();
    std::fs::write(
        dir.join(INSTALLED),
        serde_json::to_string(&urls).expect("json"),
    )?;
    Ok(())
}

fn fetch_one(
    d: &Download,
    part: &PathBuf,
    fetch: &dyn Fetch,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<(), InstallError> {
    let mut body = fetch
        .get(&d.url)
        .map_err(|e| InstallError::Network(e.to_string()))?;
    let mut out = std::fs::File::create(part)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut kept = Vec::new();
    let signed = matches!(d.check, Check::Signed { .. });
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(InstallError::Stopped);
        }
        let n = body
            .read(&mut buf)
            .map_err(|e| InstallError::Network(e.to_string()))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        if signed {
            kept.extend_from_slice(&buf[..n]);
        }
        progress(n as u64);
    }
    out.sync_all()?;
    drop(out);
    let ok = match &d.check {
        Check::Sha256(hex) => {
            let got: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            got == *hex
        }
        Check::Signed { sig_url, pubkey } => {
            let mut sig = String::new();
            fetch
                .get(sig_url)
                .and_then(|mut r| r.read_to_string(&mut sig))
                .map_err(|e| InstallError::Network(e.to_string()))?;
            verify_signature(&kept, &sig, pubkey)
        }
    };
    if !ok {
        return Err(InstallError::Check(d.file.clone()));
    }
    #[cfg(unix)]
    if d.executable {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(part, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Whether `sig_b64` (base64 of a minisign signature) signs `data` for `pubkey_b64` (base64
/// of a minisign public key), the encoding Tauri's updater uses for both.
pub fn verify_signature(data: &[u8], sig_b64: &str, pubkey_b64: &str) -> bool {
    let b64 = base64::engine::general_purpose::STANDARD;
    let text = |s: &str| {
        b64.decode(s.trim())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    };
    let (Some(sig), Some(pk)) = (text(sig_b64), text(pubkey_b64)) else {
        return false;
    };
    match (
        minisign_verify::PublicKey::decode(&pk),
        minisign_verify::Signature::decode(&sig),
    ) {
        (Ok(pk), Ok(sig)) => pk.verify(data, &sig, false).is_ok(),
        _ => false,
    }
}
