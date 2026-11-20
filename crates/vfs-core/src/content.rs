//! Extension point for document content search (VFS-1 AC-15; milestone Fase 3). The app has no
//! caller; the VFS-2 prototype in `crates/vfs-content` implements it behind a CLI feature.

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

/// Extracts the text of a document; to be implemented by an isolated worker (Fase 3).
pub trait ContentExtractor: Send + Sync {
    fn supports(&self, extension: &str) -> bool;
    fn extract(&self, path: &Path) -> Result<ExtractedText>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, ingest};

    struct PlainText;

    impl ContentExtractor for PlainText {
        fn supports(&self, extension: &str) -> bool {
            extension.eq_ignore_ascii_case("txt")
        }

        fn extract(&self, path: &Path) -> Result<ExtractedText> {
            Ok(ExtractedText {
                title: None,
                body: std::fs::read_to_string(path)?,
            })
        }
    }

    /// VFS-1 AC-15: the extension point exists but is unused in v1 — every indexed file has
    /// `content_state = None`, and an extractor can be written against the trait.
    #[test]
    fn extension_point_is_present_and_unused() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = db::open_rw(&dir.path().join("index.db")).unwrap();
        let tx = conn.transaction().unwrap();
        let root = ingest::insert_root(&tx, "/", "/datos").unwrap();
        let top: i64 = tx
            .query_row("SELECT id FROM dirs WHERE root_id = ?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        let file = ingest::NewFile {
            name: "contrato.txt",
            size: 5,
            mtime: 0,
            lossy: false,
        };
        let id = ingest::insert_file(&tx, top, &file).unwrap();
        let state: i64 = tx
            .query_row("SELECT content_state FROM files WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(state, ContentState::None as i64);

        let doc = dir.path().join("contrato.txt");
        std::fs::write(&doc, "hola").unwrap();
        let extractor: Box<dyn ContentExtractor> = Box::new(PlainText);
        assert!(extractor.supports("TXT") && !extractor.supports("pdf"));
        assert_eq!(extractor.extract(&doc).unwrap().body, "hola");
    }
}
