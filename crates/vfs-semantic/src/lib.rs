//! Search by meaning for VFS (VFS-29): `vectors.db` beside `content.db`, the embedding run
//! over its chunks, and the client of the local embedding worker.
//!
//! Nothing here runs, and no file is created, unless the caller turns meaning search on
//! (R15). The model itself runs in the `vfs-embed` worker process (feature `embedder`), so the
//! app links neither ONNX Runtime nor the tokenizer (R16).

pub mod client;
pub mod engine;
pub mod install;
pub mod protocol;
pub mod quant;
pub mod related;
pub mod search;
pub mod service;
pub mod store;
pub mod worker;

pub use engine::{run, EmbedError, Embedder, Progress, RunOptions, RunSummary};

/// An embedding model and how VFS uses it; a vector set is kept per (provider, name, dim)
/// (R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelId {
    pub provider: &'static str,
    pub name: &'static str,
    pub dim: usize,
    /// Prefixes the model expects before a search query and before a document passage.
    pub query_prefix: &'static str,
    pub doc_prefix: &'static str,
    /// Tokens kept per text; a VFS chunk is about 420–440 (29b).
    pub max_tokens: usize,
    /// Files in the model folder.
    pub model_file: &'static str,
    pub tokenizer_file: &'static str,
}

/// The local model VFS ships by default (D16): multilingual-e5-small, int8 ONNX (MIT).
pub const LOCAL_MODEL: ModelId = ModelId {
    provider: "local",
    name: "multilingual-e5-small-int8",
    dim: 384,
    query_prefix: "query: ",
    doc_prefix: "passage: ",
    max_tokens: 512,
    model_file: "model.onnx",
    tokenizer_file: "tokenizer.json",
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("vector store has unknown version {0}")]
    StoreVersion(i64),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Content(#[from] vfs_content::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
