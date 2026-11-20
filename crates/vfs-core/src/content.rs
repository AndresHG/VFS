//! Phase-2 extension point for document content search (VFS-1 AC-15). No implementation
//! and no caller in v1; the design is `docs/content-search-design.md`.

use std::path::Path;

use crate::error::Result;

/// Value of `files.content_state`. v1 writes `None` (0) only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentState {
    None = 0,
    Queued = 1,
    Indexed = 2,
    Failed = 3,
    Skipped = 4,
}

/// Text extracted from one document.
#[derive(Debug, Clone)]
pub struct ExtractedText {
    pub title: Option<String>,
    pub body: String,
}

/// Extracts the text of a document; implemented in phase 2 by an isolated worker.
pub trait ContentExtractor: Send + Sync {
    fn supports(&self, extension: &str) -> bool;
    fn extract(&self, path: &Path) -> Result<ExtractedText>;
}
