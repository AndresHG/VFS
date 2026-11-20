//! Addressable chunks of a document's stored text (VFS-2 R11): consecutive pieces that tile
//! the text, never crossing a page boundary, each with its ordinal, byte and character
//! offsets into the stored text, and its page number when the format has pages.
//!
//! Search uses chunks to return passages with a citation; a later, optional step can attach
//! one embedding per chunk keyed by `(file_id, ordinal)` without re-extracting anything.

/// Target chunk length in characters (about 300–400 Spanish words, the passage size the
/// design doc budgets for embeddings).
pub const CHUNK_CHARS: usize = 2000;
/// A cut is looked for in the last 40 % of a chunk, so chunks are 1,200–2,000 characters
/// except at the end of a page.
const MIN_CUT_CHARS: usize = CHUNK_CHARS * 6 / 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// 0-based position in the document.
    pub ordinal: u32,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_char: usize,
    pub end_char: usize,
    /// 1-based page number, `None` when the format has no pages.
    pub page: Option<u32>,
}

/// Byte offset of the best place to end a chunk of `seg` that starts at byte 0: the last
/// paragraph break, else sentence end, else whitespace, between MIN_CUT_CHARS and
/// CHUNK_CHARS characters; else exactly CHUNK_CHARS characters.
fn cut(seg: &str) -> usize {
    let idx: Vec<usize> = seg
        .char_indices()
        .map(|(i, _)| i)
        .take(CHUNK_CHARS + 1)
        .collect();
    if idx.len() <= CHUNK_CHARS {
        return seg.len();
    }
    let lo = idx[MIN_CUT_CHARS];
    let hi = idx[CHUNK_CHARS];
    let window = &seg[lo..hi];
    for pat in ["\n\n", ". ", ".\n", "\n", " "] {
        if let Some(p) = window.rfind(pat) {
            return lo + p + pat.len();
        }
    }
    hi
}

/// Splits `text` into chunks. `page_starts[n]` is the byte offset where page `n + 1`
/// starts (empty when the format has no pages); offsets must be ascending char boundaries.
pub fn chunks(text: &str, page_starts: &[usize]) -> Vec<Chunk> {
    let mut out = Vec::new();
    // Page segments: (page number, start, end).
    let mut segments: Vec<(Option<u32>, usize, usize)> = Vec::new();
    if page_starts.is_empty() {
        segments.push((None, 0, text.len()));
    } else {
        if page_starts[0] > 0 {
            segments.push((Some(1), 0, page_starts[0]));
        }
        for (n, &s) in page_starts.iter().enumerate() {
            let e = page_starts
                .get(n + 1)
                .copied()
                .unwrap_or(text.len())
                .min(text.len());
            if s < e {
                segments.push((Some(n as u32 + 1), s, e));
            }
        }
    }
    let mut chars_before = 0usize;
    let mut char_at = 0usize; // byte offset up to which chars_before is counted
    for (page, s, e) in segments {
        let mut start = s;
        while start < e {
            let end = start + cut(&text[start..e]);
            chars_before += text[char_at..start].chars().count();
            char_at = start;
            let len = text[start..end].chars().count();
            out.push(Chunk {
                ordinal: out.len() as u32,
                start_byte: start,
                end_byte: end,
                start_char: chars_before,
                end_char: chars_before + len,
                page,
            });
            start = end;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-2 AC-12: chunks tile the text, keep their size, and their offsets address it.
    #[test]
    fn chunks_tile_the_text() {
        let para = "La cigüeña del campanario de Peñalara. ".repeat(40); // ~1,560 chars
        let text = format!("{para}\n\n{para}\n\n{para}");
        let cs = chunks(&text, &[]);
        assert!(cs.len() >= 3);
        assert_eq!(cs[0].start_byte, 0);
        assert_eq!(cs.last().unwrap().end_byte, text.len());
        for w in cs.windows(2) {
            assert_eq!(w[0].end_byte, w[1].start_byte);
            assert_eq!(w[0].end_char, w[1].start_char);
        }
        for c in &cs {
            let piece = &text[c.start_byte..c.end_byte];
            assert_eq!(piece.chars().count(), c.end_char - c.start_char);
            let by_chars: String = text
                .chars()
                .skip(c.start_char)
                .take(c.end_char - c.start_char)
                .collect();
            assert_eq!(piece, by_chars);
            assert!(piece.chars().count() <= CHUNK_CHARS);
            assert_eq!(c.page, None);
        }
    }

    /// VFS-2 AC-12: no chunk crosses a page boundary, and each carries its page.
    #[test]
    fn chunks_respect_pages() {
        let p1 = "Primera página con la factura.\n".to_string();
        let p2 = "Segunda página, el contrato. ".repeat(100); // ~2,900 chars
        let text = format!("{p1}{p2}");
        let cs = chunks(&text, &[0, p1.len()]);
        assert_eq!(cs[0].page, Some(1));
        assert_eq!(cs[0].end_byte, p1.len());
        assert!(cs[1..].iter().all(|c| c.page == Some(2)));
        assert!(cs.len() >= 3);
        assert!(chunks("", &[]).is_empty());
    }
}
