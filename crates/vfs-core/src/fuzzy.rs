//! One-typo tolerance (VFS-1 AC-10). Owned by task T2b.

/// Minimum folded term length for typo tolerance; shorter terms match exactly.
pub(crate) const FUZZY_MIN_LEN: usize = 6;
/// Longest term that gets typo tolerance: candidate generation is quadratic in memory and
/// cubic in time in the term's length, and no real word is this long.
pub(crate) const FUZZY_MAX_LEN: usize = 64;

/// Shortest part kept in a candidate query: FTS5 trigram cannot match fewer characters.
const MIN_PART: usize = 3;

/// FTS5 `MATCH` expressions whose union contains every string within one insertion,
/// deletion, substitution or transposition of the folded `term`: [`candidate_parts`], each
/// alternative as `"a" AND "b"`. Empty outside [`FUZZY_MIN_LEN`]..=[`FUZZY_MAX_LEN`].
pub(crate) fn candidate_queries(term: &str) -> Vec<String> {
    candidate_parts(term)
        .iter()
        .map(|parts| {
            parts
                .iter()
                .map(|p| phrase(&p.chars().collect::<Vec<_>>()))
                .collect::<Vec<_>>()
                .join(" AND ")
        })
        .collect()
}

/// Alternatives of substrings, at least one of which a string within one edit of the
/// folded `term` contains in full: for each edit position i, `t[..i]` and `t[i+1..]` (plus
/// `t[i+2..]` for transpositions), parts shorter than 3 characters dropped. Empty outside
/// [`FUZZY_MIN_LEN`]..=[`FUZZY_MAX_LEN`] characters.
pub(crate) fn candidate_parts(term: &str) -> Vec<Vec<String>> {
    let t: Vec<char> = term.chars().collect();
    let n = t.len();
    if !(FUZZY_MIN_LEN..=FUZZY_MAX_LEN).contains(&n) {
        return Vec::new();
    }
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut push = |parts: Vec<String>| {
        if !out.contains(&parts) {
            out.push(parts);
        }
    };
    // Substitution or deletion of t[i], or insertion before t[i] (the haystack then holds
    // t[i..], which contains t[i+1..]). One part is always >= 3 chars since n - 1 >= 5.
    for i in 0..n {
        push(long_parts(&[&t[..i], &t[i + 1..]]));
    }
    // Transposition of t[i] and t[i+1].
    for i in 0..n - 1 {
        let mut parts = long_parts(&[&t[..i], &t[i + 2..]]);
        if parts.is_empty() {
            // Both sides under 3 chars (a 6-char term swapped in the middle): the haystack
            // contains the transposed term itself, so match that exactly.
            let mut swapped = t.clone();
            swapped.swap(i, i + 1);
            parts.push(swapped.into_iter().collect());
        }
        push(parts);
    }
    out
}

/// The distinct strings one deletion or one adjacent transposition away from the folded
/// `term`, at least [`MIN_PART`] chars long: the spellings a typed typo most often hides
/// (an extra or a swapped letter). Empty outside [`FUZZY_MIN_LEN`]..=[`FUZZY_MAX_LEN`].
pub(crate) fn near_variants(term: &str) -> Vec<String> {
    let t: Vec<char> = term.chars().collect();
    let n = t.len();
    if !(FUZZY_MIN_LEN..=FUZZY_MAX_LEN).contains(&n) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut push = |v: Vec<char>| {
        let s: String = v.into_iter().collect();
        if s != term && s.chars().count() >= MIN_PART && !out.contains(&s) {
            out.push(s);
        }
    };
    for i in 0..n {
        let mut v = t.clone();
        v.remove(i);
        push(v);
    }
    for i in 0..n - 1 {
        let mut v = t.clone();
        v.swap(i, i + 1);
        push(v);
    }
    out
}

/// True when `name` starts with a string within one edit of `term` (the typo analogue of
/// "the name starts with the first term").
pub(crate) fn starts_within_one(name: &str, term: &str) -> bool {
    let window: String = name.chars().take(term.chars().count() + 1).collect();
    contains_within_one(&window, term)
}

/// The parts of at least [`MIN_PART`] chars.
fn long_parts(parts: &[&[char]]) -> Vec<String> {
    parts
        .iter()
        .filter(|p| p.len() >= MIN_PART)
        .map(|p| p.iter().collect())
        .collect()
}

/// An FTS5 phrase string: the text in double quotes with `"` doubled.
fn phrase(chars: &[char]) -> String {
    let mut s = String::with_capacity(chars.len() + 2);
    s.push('"');
    for &c in chars {
        if c == '"' {
            s.push('"');
        }
        s.push(c);
    }
    s.push('"');
    s
}

/// True when some substring of `haystack` is within optimal-string-alignment distance 1 of
/// `needle` (Sellers' approximate substring search). Both arguments are folded keys.
pub(crate) fn contains_within_one(haystack: &str, needle: &str) -> bool {
    if haystack.contains(needle) {
        return true;
    }
    let h: Vec<char> = haystack.chars().collect();
    let p: Vec<char> = needle.chars().collect();
    // Column j of row i: distance from p[..i] to the best substring of h ending at j.
    // Row 0 is all zeros (a match may start anywhere); values are capped at 2.
    let width = h.len() + 1;
    let mut prev2 = vec![0u8; width];
    let mut prev = vec![0u8; width];
    let mut cur = vec![0u8; width];
    for i in 1..=p.len() {
        cur[0] = i.min(2) as u8;
        for j in 1..width {
            let cost = u8::from(p[i - 1] != h[j - 1]);
            let mut d = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && p[i - 1] == h[j - 2] && p[i - 2] == h[j - 1] {
                d = d.min(prev2[j - 2] + 1);
            }
            cur[j] = d.min(2);
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.iter().any(|&d| d <= 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulates FTS5 trigram evaluation of a candidate query: every phrase is a substring.
    fn query_matches(query: &str, haystack: &str) -> bool {
        query.split(" AND ").all(|part| {
            let inner = &part[1..part.len() - 1];
            haystack.contains(&inner.replace("\"\"", "\""))
        })
    }

    fn any_query_matches(term: &str, haystack: &str) -> bool {
        candidate_queries(term)
            .iter()
            .any(|q| query_matches(q, haystack))
    }

    /// Every string within one edit of `w`, over an alphabet of `w`'s chars plus `extra`.
    fn single_edits(w: &str, extra: &[char]) -> Vec<String> {
        let t: Vec<char> = w.chars().collect();
        let mut alphabet: Vec<char> = t.clone();
        alphabet.extend_from_slice(extra);
        let s = |v: Vec<char>| v.into_iter().collect::<String>();
        let mut out = Vec::new();
        for i in 0..t.len() {
            let mut del = t.clone();
            del.remove(i);
            out.push(s(del));
            for &c in &alphabet {
                let mut sub = t.clone();
                sub[i] = c;
                out.push(s(sub));
            }
            if i + 1 < t.len() {
                let mut tr = t.clone();
                tr.swap(i, i + 1);
                out.push(s(tr));
            }
        }
        for i in 0..=t.len() {
            for &c in &alphabet {
                let mut ins = t.clone();
                ins.insert(i, c);
                out.push(s(ins));
            }
        }
        out
    }

    /// VFS-1 AC-10: terms shorter than 6 characters get no fuzzy queries.
    #[test]
    fn short_terms_have_no_queries() {
        for t in ["", "a", "nota", "notar", "año", "ñañañ"] {
            assert!(candidate_queries(t).is_empty(), "{t}");
        }
        assert!(!candidate_queries("notari").is_empty());
    }

    /// VFS-23 AC-3, VFS-1 AC-9, AC-10, S9: terms over `FUZZY_MAX_LEN` get no typo work, which
    /// would grow with the cube of their length.
    #[test]
    fn overlong_terms_have_no_queries() {
        let max = "notaria".repeat(10)[..FUZZY_MAX_LEN].to_string();
        assert!(!candidate_parts(&max).is_empty());
        let over = format!("{max}x");
        assert!(candidate_parts(&over).is_empty());
        assert!(candidate_queries(&over).is_empty());
        assert!(near_variants(&over).is_empty());
    }

    /// VFS-1 AC-10: the queries follow the per-position split, dropping parts under 3 chars.
    #[test]
    fn queries_split_per_edit_position() {
        let q = candidate_queries("notaira");
        assert!(q.contains(&r#""otaira""#.to_string()));
        assert!(q.contains(&r#""not" AND "ira""#.to_string()));
        assert!(q.contains(&r#""notair""#.to_string()));
        // Transposition at 3 (a<->i): "not" + "ra" -> "ra" dropped.
        assert!(q.contains(&r#""not""#.to_string()));
        for query in &q {
            for part in query.split(" AND ") {
                assert!(part.chars().count() - 2 >= MIN_PART, "{query}");
            }
        }
        // Repeated characters do not produce duplicate queries.
        let q = candidate_queries("aaaaaaa");
        let mut dedup = q.clone();
        dedup.dedup();
        assert_eq!(q, dedup);
        assert_eq!(phrase(&['a', '"', 'b']), r#""a""b""#);
    }

    /// VFS-1 AC-10: a 6-char term swapped in the middle (both sides under 3 chars) still
    /// gets a selective query, never an empty one that would match everything.
    #[test]
    fn six_char_middle_transposition() {
        let q = candidate_queries("abcdef");
        assert!(q.contains(&r#""abdcef""#.to_string()));
        assert!(q.iter().all(|s| !s.is_empty()));
        assert!(any_query_matches("abcdef", "x abdcef.pdf"));
        assert!(contains_within_one("x abdcef.pdf", "abcdef"));
    }

    /// VFS-1 AC-10: each edit type at the start, middle and end of a term is found by the
    /// candidate queries and accepted by verification.
    #[test]
    fn edit_types_at_start_middle_end() {
        let term = "notaria";
        let cases = [
            // substitution
            "xotaria", "notxria", "notarix", // insertion
            "xnotaria", "notxaria", "notariax", // deletion
            "otaria", "notria", "notari", // transposition
            "ontaria", "notraia", "notaira", "notarai",
        ];
        for v in cases {
            let hay = format!("escritura {v} perez.pdf");
            assert!(any_query_matches(term, &hay), "query {v}");
            assert!(contains_within_one(&hay, term), "verify {v}");
        }
    }

    /// VFS-1 AC-10: a transposition across the old middle split point (`notaira`, plan 2.5).
    #[test]
    fn transposition_across_split_point() {
        // "notaria": old split m = 3 between "not" and "aria"; swap t[2] and t[3].
        assert!(any_query_matches("notaria", "noatria perez.pdf"));
        assert!(contains_within_one("noatria perez.pdf", "notaria"));
        assert!(any_query_matches("notaira", "notaria perez.pdf"));
        assert!(contains_within_one("notaria perez.pdf", "notaira"));
        assert!(contains_within_one("gestoria", "gestorai"));
        assert!(contains_within_one("escritura", "escirtura"));
    }

    /// VFS-1 AC-10: 6-character terms tolerate every single edit.
    #[test]
    fn six_char_terms() {
        for w in ["gestor", "factur"] {
            for v in single_edits(w, &['z', 'ñ']) {
                assert!(any_query_matches(w, &v), "{w} -> {v}");
                assert!(contains_within_one(&v, w), "{w} -> {v}");
            }
        }
    }

    /// VFS-1 AC-10: multi-byte characters are handled by char, not byte.
    #[test]
    fn multibyte_characters() {
        let w = "ñandúes"; // 7 chars, 9 bytes
        assert!(!candidate_queries(w).is_empty());
        assert!(candidate_queries("ñandú").is_empty()); // 5 chars, 7 bytes
        for v in single_edits(w, &['ç', '写']) {
            let hay = format!("写真 {v} 2019");
            assert!(any_query_matches(w, &hay), "{v}");
            assert!(contains_within_one(&hay, w), "{v}");
        }
        assert!(contains_within_one("αθηνα", "αθνηα"));
        assert!(!contains_within_one("写真写真", "写真写真写真"));
    }

    /// VFS-1 AC-10: strings at distance 2 are rejected.
    #[test]
    fn no_false_positive_at_distance_two() {
        for hay in [
            "nxtxria", "ntaia", "xxnotar", "onatria", "notxrxa", "otari", "ntoarai", "",
        ] {
            assert!(!contains_within_one(hay, "notaria"), "{hay}");
        }
        assert!(!contains_within_one("factura 2019.pdf", "notaria"));
        assert!(contains_within_one("anything", ""));
    }

    /// VFS-1 AC-10: property-style check that every single-edit variant of a sample of
    /// words, embedded in surrounding text, is found by a query and accepted by verification.
    #[test]
    fn every_single_edit_is_accepted() {
        let words = [
            "notaria",
            "escritura",
            "gestoria",
            "hipoteca",
            "vacaciones",
            "fotografia",
            "presupuesto",
            "aaaaaa",
            "abcabc",
        ];
        for w in words {
            for v in single_edits(w, &['x', '_', ' ', '1']) {
                for hay in [v.clone(), format!("2019 {v}.pdf"), format!("a/{v}/b")] {
                    assert!(any_query_matches(w, &hay), "query {w} -> {hay}");
                    assert!(contains_within_one(&hay, w), "verify {w} -> {hay}");
                }
            }
        }
    }

    /// VFS-1 AC-10: near variants are the single deletions and adjacent transpositions of
    /// a long term, so the intended spelling of "notaira" (a swap) and "notariaa" (an extra
    /// letter) is among them; short terms get none.
    #[test]
    fn near_variants_cover_swaps_and_extra_letters() {
        let v = near_variants("notaira");
        assert!(v.contains(&"notaria".to_string()));
        assert!(v.contains(&"otaira".to_string()));
        assert!(!v.contains(&"notaira".to_string()));
        assert!(near_variants("notariaa").contains(&"notaria".to_string()));
        assert!(near_variants("nota").is_empty());
        let mut sorted = v.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), v.len(), "no duplicates");
    }

    /// VFS-1 AC-10: a name starts within one edit of a term only when the edit is at its
    /// start, not when the near match is further in.
    #[test]
    fn starts_within_one_is_anchored() {
        assert!(starts_within_one("notaria perez.pdf", "notaira"));
        assert!(starts_within_one("notria.pdf", "notaria"));
        assert!(!starts_within_one("escritura notaria.zip", "notaira"));
    }
}
