//! Accent, case and normalization folding applied identically to file names, directory
//! paths, root paths and query terms (VFS-1 AC-8, AC-10).

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Unicode lowercase, then NFKD, then drop combining marks, then lowercase again (a
/// compatibility decomposition such as `℡` -> `TEL` can yield uppercase; the second pass
/// keeps the key idempotent). ASCII input takes a byte-lowercasing fast path.
pub fn fold_key(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    s.chars()
        .flat_map(char::to_lowercase)
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::fold_key;

    /// VFS-1 AC-10: case is ignored.
    #[test]
    fn folds_case() {
        assert_eq!(fold_key("NOTARIA"), "notaria");
        assert_eq!(fold_key("IMG_0001.JPG"), "img_0001.jpg");
        assert_eq!(fold_key("ÁRBOL"), "arbol");
    }

    /// VFS-1 AC-10: accents are ignored ("notaria" finds "Notaría").
    #[test]
    fn folds_accents() {
        assert_eq!(fold_key("Notaría"), "notaria");
        assert_eq!(fold_key("año"), "ano");
        assert_eq!(fold_key("Gestoría/Año 2019"), "gestoria/ano 2019");
        assert_eq!(fold_key("Pérez çà ü"), "perez ca u");
    }

    /// VFS-1 AC-8: names in NFC and NFD fold to the same key.
    #[test]
    fn nfc_equals_nfd() {
        let nfc = "Notar\u{00ED}a Pe\u{0301}rez";
        let nfd = "Notari\u{0301}a Pe\u{0301}rez";
        assert_ne!(nfc, nfd);
        assert_eq!(fold_key(nfc), fold_key(nfd));
        assert_eq!(fold_key(nfd), "notaria perez");
    }

    /// VFS-1 AC-10: compatibility forms fold to plain lowercase and folding is idempotent.
    #[test]
    fn compatibility_forms_and_idempotence() {
        assert_eq!(fold_key("ＡＢＣ"), "abc");
        assert_eq!(fold_key("ﬁchero"), "fichero");
        assert_eq!(fold_key("\u{2121}"), "tel");
        assert_eq!(fold_key("İstanbul"), "istanbul");
        for s in ["Notaría", "\u{2121}", "İ", "ÀÉÎÕÜ ñ", "plain"] {
            assert_eq!(fold_key(&fold_key(s)), fold_key(s), "{s}");
        }
    }

    /// VFS-1 AC-10: empty input and non-Latin scripts pass through (lowercased).
    #[test]
    fn empty_and_other_scripts() {
        assert_eq!(fold_key(""), "");
        assert_eq!(fold_key("ΑΘΗΝΑ"), "αθηνα");
        assert_eq!(fold_key("写真"), "写真");
    }
}
