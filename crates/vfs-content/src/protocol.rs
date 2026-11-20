//! Messages between the content indexer and the extraction worker process (VFS-2 R2): one
//! JSON object per line on the worker's stdin (requests) and stdout (responses).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Environment variable carrying the worker's heap cap in bytes (VFS-2 D6).
pub const MEM_CAP_ENV: &str = "VFS_EXTRACT_MEM_CAP";

/// Extract one file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub path: PathBuf,
    /// Larger files are skipped without being read.
    pub max_input_bytes: u64,
    /// Extracted text beyond this many UTF-8 bytes is cut (at a character boundary).
    pub max_text_bytes: usize,
}

/// Why a file was skipped: it is not damaged, there is just no text to take (or it is not
/// ours to take). Skipped files are not retried until they change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Password-protected / encrypted.
    Password,
    /// Opened fine but holds no text (empty file, scanned PDF without a text layer).
    NoText,
    /// Larger than `max_input_bytes`; not read.
    TooLarge,
    /// The extension is not a supported document format.
    Unsupported,
}

/// Why extraction failed. Failed files are not retried until they change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    /// The file could not be opened or read (permission, lock, vanished).
    Open,
    /// The extractor rejected the file or panicked on it.
    Corrupt,
    /// The worker exceeded the wall-clock limit and was killed.
    Timeout,
    /// The worker exceeded its memory cap and aborted.
    Memory,
    /// The worker died for another reason (crash in the extractor) or broke the protocol.
    Crash,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Text {
        title: Option<String>,
        body: String,
        /// The body was cut at `max_text_bytes`.
        truncated: bool,
        /// Byte offsets in `body` where pages 1, 2, … start; empty when the format has no
        /// pages (VFS-2 R11).
        #[serde(default)]
        page_starts: Vec<usize>,
    },
    Skipped(SkipReason),
    Failed {
        reason: FailReason,
        detail: String,
    },
}

/// The worker's answer to one [`Request`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub outcome: Outcome,
    /// Bytes of the file the worker read (0 when it skipped the file unread).
    pub bytes_read: u64,
    /// Time inside the worker, read plus extraction.
    pub ms: u64,
}

impl Response {
    pub fn failed(reason: FailReason, detail: impl Into<String>) -> Response {
        Response {
            outcome: Outcome::Failed {
                reason,
                detail: detail.into(),
            },
            bytes_read: 0,
            ms: 0,
        }
    }
}
