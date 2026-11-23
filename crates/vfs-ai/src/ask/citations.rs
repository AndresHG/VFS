//! The citation check (VFS-28 R7, R8): a citation is kept only if its passage was sent and its
//! quote is in that passage after the VFS-2 fold. Kept citations are numbered by first use.

use vfs_core::fold_key;

use crate::provider::AnswerBlock;

/// A citation that passed the check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    /// Index of the passage in the thread's sources.
    pub passage: usize,
    pub quote: String,
    /// The number shown for it (R8): passages are numbered from 1 in order of first citation.
    pub marker: u32,
}

/// A run of answer text with its checked citations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedBlock {
    pub text: String,
    pub citations: Vec<Citation>,
}

/// The fold citation quotes and passages are compared under: the accent and case fold of
/// VFS-2 (`fold_key`), with every run of whitespace made one space.
fn fold_text(s: &str) -> String {
    fold_key(s).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Checks every citation of `blocks` against the passages sent (`passages[i]` is the text of
/// passage `i`). Returns the checked blocks and the number of citations dropped.
pub fn check(blocks: &[AnswerBlock], passages: &[&str]) -> (Vec<CheckedBlock>, usize) {
    let folded: Vec<String> = passages.iter().map(|p| fold_text(p)).collect();
    let mut markers: Vec<usize> = Vec::new();
    let mut dropped = 0;
    let checked = blocks
        .iter()
        .map(|b| {
            let mut citations = Vec::new();
            for c in &b.citations {
                let quote = fold_text(&c.quote);
                let ok =
                    !quote.is_empty() && folded.get(c.passage).is_some_and(|p| p.contains(&quote));
                if !ok {
                    dropped += 1;
                    continue;
                }
                let marker = match markers.iter().position(|&m| m == c.passage) {
                    Some(i) => i,
                    None => {
                        markers.push(c.passage);
                        markers.len() - 1
                    }
                };
                citations.push(Citation {
                    passage: c.passage,
                    quote: c.quote.clone(),
                    marker: marker as u32 + 1,
                });
            }
            CheckedBlock {
                text: b.text.clone(),
                citations,
            }
        })
        .collect();
    (checked, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::RawCitation;

    fn block(text: &str, cites: &[(usize, &str)]) -> AnswerBlock {
        AnswerBlock {
            text: text.into(),
            citations: cites
                .iter()
                .map(|(p, q)| RawCitation {
                    passage: *p,
                    quote: (*q).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn quotes_match_after_the_fold() {
        let passages = ["La FIANZA asciende a\n1.200 euros.", "Otra cosa."];
        let (b, dropped) = check(&[block("x", &[(0, "fianza asciende a 1.200")])], &passages);
        assert_eq!(dropped, 0);
        assert_eq!(b[0].citations[0].marker, 1);
        let (_, dropped) = check(&[block("x", &[(0, "   ")])], &passages);
        assert_eq!(dropped, 1, "an empty quote proves nothing");
    }

    #[test]
    fn markers_number_passages_by_first_use() {
        let passages = ["uno", "dos", "tres"];
        let (b, _) = check(
            &[
                block("a", &[(2, "tres"), (0, "uno")]),
                block("b", &[(2, "tres")]),
            ],
            &passages,
        );
        let markers: Vec<u32> = b
            .iter()
            .flat_map(|b| b.citations.iter().map(|c| c.marker))
            .collect();
        assert_eq!(markers, [1, 2, 1]);
    }
}
