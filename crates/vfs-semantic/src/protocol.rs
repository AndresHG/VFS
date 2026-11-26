//! Messages between VFS and the embedding worker process (VFS-29 R16): one JSON object per
//! line on the worker's stdin (requests) and stdout (responses), like the extraction worker's.

use serde::{Deserialize, Serialize};

/// The version a worker must report in its [`Hello`]: the crate version plus the protocol
/// revision. A worker from another build is refused.
pub const WORKER_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+e1");

/// First line a worker writes, once its model is loaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub version: String,
    /// Dimension of the vectors the model returns.
    pub dim: usize,
    /// Whether the worker runs in the OS background band (as the extraction worker reports it).
    pub background: bool,
}

/// Embed some texts: document chunks, or one search query (`query`), which models such as
/// multilingual-e5 prefix differently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub texts: Vec<String>,
    #[serde(default)]
    pub query: bool,
}

/// The answer to one [`Request`]: one unit-length vector per text, in order, or why not.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Response {
    Vectors { vectors: Vec<Vec<f32>> },
    Failed { message: String },
}
