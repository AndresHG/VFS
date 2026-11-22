//! In-process text extraction with Xberg (VFS-2 D3, chosen in
//! `docs/content-search-measurements.md`). Runs only inside the worker process: a panic or
//! runaway allocation here never reaches the app.
//!
//! Xberg is built with its pure-Rust PDF backend and the office and spreadsheet extractors,
//! and without OCR, models or network features.

use std::path::Path;

use crate::protocol::SkipReason;
pub use crate::protocol::{DocMeta, SUPPORTED};

/// What the extractor made of one file's bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum Extracted {
    /// `page_starts[n]` is the byte offset in `body` where page `n + 1` starts; empty when
    /// the format has no pages.
    Text {
        title: Option<String>,
        body: String,
        page_starts: Vec<usize>,
        meta: DocMeta,
    },
    Skipped(SkipReason),
    Failed(String),
}

/// OOXML files with a password are not ZIP files but OLE compound files holding an
/// `EncryptionInfo` stream.
fn is_encrypted_ooxml(ext: &str, bytes: &[u8]) -> bool {
    const OLE_MAGIC: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    if !matches!(ext, "docx" | "xlsx" | "pptx") || !bytes.starts_with(OLE_MAGIC) {
        return false;
    }
    let needle: Vec<u8> = "EncryptionInfo"
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    bytes.windows(needle.len()).any(|w| w == needle.as_slice())
}

fn looks_like_password_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("encrypt") || m.contains("password")
}

/// Extracts the text of a document from its bytes. `ext` is the lower-cased extension.
pub fn extract_bytes(ext: &str, bytes: Vec<u8>) -> Extracted {
    if !SUPPORTED.contains(&ext) {
        return Extracted::Skipped(SkipReason::Unsupported);
    }
    if bytes.is_empty() {
        return Extracted::Skipped(SkipReason::NoText);
    }
    if is_encrypted_ooxml(ext, &bytes) {
        return Extracted::Skipped(SkipReason::Password);
    }
    let rt = match tokio::runtime::Builder::new_current_thread().build() {
        Ok(rt) => rt,
        Err(e) => return Extracted::Failed(format!("runtime: {e}")),
    };
    let input = xberg::ExtractInput {
        kind: xberg::ExtractInputKind::Bytes,
        bytes: Some(bytes),
        filename: Some(format!("document.{ext}")),
        ..Default::default()
    };
    let config = xberg::ExtractionConfig {
        use_cache: false,
        disable_ocr: true,
        // Per-page text, so chunks can cite a page (VFS-2 R11).
        pages: Some(xberg::PageConfig {
            extract_pages: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let res = match rt.block_on(xberg::extract(input, &config)) {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            return if looks_like_password_error(&msg) {
                Extracted::Skipped(SkipReason::Password)
            } else {
                Extracted::Failed(msg)
            };
        }
    };
    if let Some(err) = res.errors.first() {
        let msg = format!("{err:?}");
        return if looks_like_password_error(&msg) {
            Extracted::Skipped(SkipReason::Password)
        } else {
            Extracted::Failed(msg.chars().take(500).collect())
        };
    }
    let mut title = None;
    let mut meta = DocMeta::default();
    let mut body = String::new();
    let mut page_starts = Vec::new();
    let mut pages_seen = 0u32;
    for d in res.results {
        if title.is_none() {
            title = d.metadata.title.filter(|t| !t.trim().is_empty());
        }
        if meta.authors.is_empty() {
            meta.authors = d
                .metadata
                .authors
                .unwrap_or_default()
                .into_iter()
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect();
        }
        if meta.created.is_none() {
            meta.created = d.metadata.created_at.filter(|c| !c.trim().is_empty());
        }
        pages_seen += d
            .metadata
            .pages
            .as_ref()
            .map(|p| p.total_count)
            .unwrap_or(0)
            .max(d.pages.as_ref().map_or(0, |p| p.len() as u32));
        if !body.is_empty() {
            body.push('\n');
        }
        match d.pages.filter(|p| !p.is_empty()) {
            // Paged formats: the body is the pages in order, so offsets map to pages.
            Some(pages) => {
                for (i, p) in pages.iter().enumerate() {
                    if i > 0 {
                        body.push_str("\n\n");
                    }
                    page_starts.push(body.len());
                    body.push_str(&p.content);
                }
            }
            None => body.push_str(&d.content),
        }
    }
    if body.trim().is_empty() {
        // A PDF with pages but no text layer is a scan (VFS-2 R15, D13).
        return Extracted::Skipped(if ext == "pdf" && pages_seen > 0 {
            SkipReason::Scanned
        } else {
            SkipReason::NoText
        });
    }
    Extracted::Text {
        title,
        body,
        page_starts,
        meta,
    }
}

/// The VFS-1 extension point (`vfs_core::content::ContentExtractor`) implemented in
/// process. The content indexer never calls it directly: it goes through the worker process
/// ([`crate::supervisor::Supervisor`]), which runs [`extract_bytes`] in isolation.
pub struct XbergExtractor;

impl vfs_core::content::ContentExtractor for XbergExtractor {
    fn supports(&self, extension: &str) -> bool {
        SUPPORTED.contains(&extension.to_ascii_lowercase().as_str())
    }

    fn extract(&self, path: &Path) -> vfs_core::Result<vfs_core::content::ExtractedText> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let bytes = std::fs::read(path)?;
        match extract_bytes(&ext, bytes) {
            Extracted::Text { title, body, .. } => {
                Ok(vfs_core::content::ExtractedText { title, body })
            }
            Extracted::Skipped(r) => Err(vfs_core::Error::Io(std::io::Error::other(format!(
                "skipped: {r:?}"
            )))),
            Extracted::Failed(m) => Err(vfs_core::Error::Io(std::io::Error::other(m))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vfs_core::content::ContentExtractor;

    /// VFS-2 AC-1: only the supported document extensions are extracted.
    #[test]
    fn supports_only_document_formats() {
        let x = XbergExtractor;
        for e in ["PDF", "docx", "doc", "xls", "pptx", "odt", "rtf", "txt"] {
            assert!(x.supports(e), "{e}");
        }
        for e in ["jpg", "mp3", "mov", "epub", "pages", "exe"] {
            assert!(!x.supports(e), "{e}");
        }
        assert_eq!(
            extract_bytes("jpg", vec![1, 2, 3]),
            Extracted::Skipped(SkipReason::Unsupported)
        );
    }

    /// VFS-2 AC-2: plain text with accents and ñ comes through intact.
    #[test]
    fn extracts_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nota.txt");
        std::fs::write(&p, "La cigüeña y el niño en Peñalara.").unwrap();
        let t = XbergExtractor.extract(&p).unwrap();
        assert!(t.body.contains("cigüeña") && t.body.contains("Peñalara"));
    }

    /// VFS-2 AC-4: empty, random and encrypted inputs are skipped or failed, never text.
    #[test]
    fn hostile_bytes_do_not_yield_text() {
        assert_eq!(
            extract_bytes("pdf", Vec::new()),
            Extracted::Skipped(SkipReason::NoText)
        );
        assert!(matches!(
            extract_bytes("docx", vec![7u8; 5000]),
            Extracted::Failed(_)
        ));
        let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        ole.extend(
            "EncryptionInfo"
                .encode_utf16()
                .flat_map(|u| u.to_le_bytes()),
        );
        assert_eq!(
            extract_bytes("docx", ole),
            Extracted::Skipped(SkipReason::Password)
        );
    }
}
