//! The Spanish analyzer shared by indexing and querying (VFS-2 R6): split into words,
//! lowercase, Snowball Spanish stemming, then the accent fold of `vfs_core::fold_key`.
//!
//! Stemming runs before folding because the Snowball Spanish rules expect accented input
//! (`acción` and `accion` must reach the same key either way; the fold makes them equal after
//! the stemmer strips the suffix).

use rust_stemmers::{Algorithm, Stemmer};
use unicode_normalization::UnicodeNormalization;

/// Longest word kept; longer runs (base64, hashes, garbage from a broken extractor) are
/// dropped so they cannot bloat the index.
const MAX_WORD_CHARS: usize = 40;

/// One word of a text: its byte range in the (NFC) text and its index key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub key: String,
}

/// Turns words into index keys. Owns the stemmer; cheap to create.
pub struct Analyzer {
    stemmer: Stemmer,
}

impl Default for Analyzer {
    fn default() -> Self {
        Analyzer {
            stemmer: Stemmer::create(Algorithm::Spanish),
        }
    }
}

impl Analyzer {
    /// Index key of one word: lowercase, stem, fold accents.
    pub fn key(&self, word: &str) -> String {
        let lower = word.to_lowercase();
        let stemmed = self.stemmer.stem(&lower);
        vfs_core::fold_key(&stemmed)
    }

    /// Words of `text` with byte offsets into `text`. `text` should already be NFC (see
    /// [`normalize`]); offsets refer to the string as given.
    pub fn tokens(&self, text: &str) -> Vec<Token> {
        let mut out = Vec::new();
        let mut start: Option<usize> = None;
        for (i, c) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            if c.is_alphanumeric() {
                start.get_or_insert(i);
            } else if let Some(s) = start.take() {
                let word = &text[s..i];
                if word.chars().count() <= MAX_WORD_CHARS {
                    out.push(Token {
                        start: s,
                        end: i,
                        key: self.key(word),
                    });
                }
            }
        }
        out
    }

    /// The string stored in the full-text column: keys separated by spaces, in text order,
    /// so FTS5 phrase queries see the same word positions as the document.
    pub fn index_text(&self, text: &str) -> String {
        let mut s = String::with_capacity(text.len() / 2);
        for t in self.tokens(text) {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(&t.key);
        }
        s
    }
}

/// NFC form, with line-end hyphenation rejoined (`contra-\nto` → `contrato`) and runs of
/// whitespace kept as they are. Applied once to extracted text before it is stored.
pub fn normalize(text: &str) -> String {
    let nfc: String = text.nfc().collect();
    let mut out = String::with_capacity(nfc.len());
    let mut chars = nfc.chars().peekable();
    let mut prev_alpha = false;
    while let Some(c) = chars.next() {
        if c == '-' && prev_alpha {
            // A hyphen directly followed by a line break and a lowercase letter joins the word.
            let mut look = chars.clone();
            let mut saw_newline = false;
            while let Some(&n) = look.peek() {
                if n == '\n' || n == '\r' {
                    saw_newline = true;
                    look.next();
                } else if n == ' ' || n == '\t' {
                    look.next();
                } else {
                    break;
                }
            }
            if saw_newline && look.peek().is_some_and(|n| n.is_lowercase()) {
                chars = look;
                continue;
            }
        }
        prev_alpha = c.is_alphabetic();
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-2 AC-8: plural and singular share a key; accents and case do not matter.
    #[test]
    fn stems_and_folds_spanish() {
        let a = Analyzer::default();
        assert_eq!(a.key("facturas"), a.key("factura"));
        assert_eq!(a.key("acción"), a.key("accion"));
        assert_eq!(a.key("Acciones"), a.key("acción"));
        assert_eq!(a.key("PEÑALARA"), a.key("Peñalara"));
        assert_ne!(a.key("contrato"), a.key("factura"));
    }

    /// VFS-2 AC-8: offsets point at the words, multi-byte letters included.
    #[test]
    fn tokens_carry_byte_offsets() {
        let a = Analyzer::default();
        let text = "El niño pagó: 120 €.";
        let toks = a.tokens(text);
        let words: Vec<&str> = toks.iter().map(|t| &text[t.start..t.end]).collect();
        assert_eq!(words, ["El", "niño", "pagó", "120"]);
        assert_eq!(a.index_text(text).split(' ').count(), 4);
    }

    /// VFS-2 AC-2: NFD input becomes NFC, and line-end hyphenation is rejoined.
    #[test]
    fn normalizes_nfd_and_hyphenation() {
        let nfd = "an\u{0303}o";
        assert_eq!(normalize(nfd), "año");
        assert_eq!(
            normalize("contra-\nto de alqui-\r\n  ler"),
            "contrato de alquiler"
        );
        assert_eq!(normalize("Madrid-\nBarcelona"), "Madrid-\nBarcelona");
        assert_eq!(normalize("ida - vuelta"), "ida - vuelta");
    }

    /// VFS-2 AC-8: very long runs are not indexed.
    #[test]
    fn drops_overlong_words() {
        let a = Analyzer::default();
        let junk = "x".repeat(MAX_WORD_CHARS + 1);
        assert_eq!(a.tokens(&format!("hola {junk} adiós")).len(), 2);
    }
}
