//! «Bibliografía» (VFS-28 R12): the documents an answer cited, or every one found, with the
//! author, year and title of their metadata (VFS-2 R16), and their export as RIS.
//!
//! The fallbacks of R12 that are words («s. a.», «(fecha del archivo)») are drawn by the UI
//! from its catalogs; an entry only says which fields are missing.

use std::path::Path;

use chrono::{Datelike, Local, TimeZone};
use serde::Serialize;
use vfs_ai::ask::Document;

/// One document of the bibliography.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibEntry {
    pub file_id: i64,
    /// From the metadata; empty when it has none (shown as «s. a.»).
    pub authors: Vec<String>,
    pub year: i32,
    /// The year is the file's modification year, not the document's (R12).
    pub year_from_file: bool,
    /// From the metadata; `None` falls back to the file name.
    pub title: Option<String>,
    pub file_name: String,
    /// Absolute path, native separators.
    pub path: String,
    /// Pages of the passages cited from it, ascending.
    pub pages: Vec<u32>,
}

/// The first four-digit year in a metadata date such as `2021-03-04T10:00:00Z` or
/// `D:20210304`.
fn year_of(created: &str) -> Option<i32> {
    let chars: Vec<char> = created.chars().collect();
    chars.windows(4).find_map(|w| {
        if !w.iter().all(char::is_ascii_digit) {
            return None;
        }
        let y: i32 = w.iter().collect::<String>().parse().ok()?;
        (1000..=2999).contains(&y).then_some(y)
    })
}

/// The entry of `doc`, with the pages cited from it.
pub fn entry(doc: &Document, pages: Vec<u32>) -> BibEntry {
    let meta_year = doc.created.as_deref().and_then(year_of);
    let file_year = Local
        .timestamp_opt(doc.modified, 0)
        .earliest()
        .map_or(1970, |t| t.year());
    BibEntry {
        file_id: doc.file_id,
        authors: doc
            .authors
            .iter()
            .map(|a| a.trim().to_owned())
            .filter(|a| !a.is_empty())
            .collect(),
        year: meta_year.unwrap_or(file_year),
        year_from_file: meta_year.is_none(),
        title: doc
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned),
        file_name: doc.name.clone(),
        path: Path::new(&doc.dir)
            .join(&doc.name)
            .to_string_lossy()
            .into_owned(),
        pages,
    }
}

/// `path` as a `file://` URL: `\` becomes `/`, and every byte outside the unreserved set, `/`
/// and a drive's colon is percent-encoded (R12 `UR`).
pub fn file_url(path: &str) -> String {
    let slashed = path.replace('\\', "/");
    let drive = slashed.as_bytes().get(1) == Some(&b':');
    let mut out = String::from(if slashed.starts_with('/') {
        "file://"
    } else {
        "file:///"
    });
    for (i, b) in slashed.bytes().enumerate() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            b':' if drive && i == 1 => out.push(':'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// One RIS line; a line break inside a value would end the field, so whitespace runs become
/// one space.
fn field(tag: &str, value: &str) -> String {
    let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{tag}  - {flat}\r\n")
}

/// The entries as RIS (R12): one `TY  - GEN` record each, with `AU` per author, `PY`, `TI`
/// (the title, or the file name), `UR` as a `file://` URL and `N1` with the pages cited.
pub fn ris(entries: &[BibEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        out += &field("TY", "GEN");
        for a in &e.authors {
            out += &field("AU", a);
        }
        out += &field("PY", &e.year.to_string());
        out += &field("TI", e.title.as_deref().unwrap_or(&e.file_name));
        out += &field("UR", &file_url(&e.path));
        if !e.pages.is_empty() {
            let pages: Vec<String> = e.pages.iter().map(u32::to_string).collect();
            out += &field("N1", &format!("pp. {}", pages.join(", ")));
        }
        out += "ER  - \r\n\r\n";
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(name: &str, title: Option<&str>, authors: &[&str], created: Option<&str>) -> Document {
        Document {
            file_id: 7,
            name: name.into(),
            dir: "/Archivo/Casa".into(),
            modified: Local
                .with_ymd_and_hms(2019, 6, 1, 12, 0, 0)
                .unwrap()
                .timestamp(),
            title: title.map(str::to_owned),
            authors: authors.iter().map(|s| s.to_string()).collect(),
            created: created.map(str::to_owned),
        }
    }

    /// VFS-28 AC-10: a PDF with title, author and date metadata gives them; a DOCX with none
    /// gives no author, no title (the file name stands in) and the file's year, marked.
    #[test]
    fn entries_fall_back_as_r12_says() {
        let pdf = entry(
            &doc(
                "informe.pdf",
                Some("Informe anual"),
                &["Ana Ruiz"],
                Some("D:20210304"),
            ),
            vec![2, 3],
        );
        assert_eq!(pdf.authors, ["Ana Ruiz"]);
        assert_eq!((pdf.year, pdf.year_from_file), (2021, false));
        assert_eq!(pdf.title.as_deref(), Some("Informe anual"));
        assert_eq!(
            pdf.path,
            Path::new("/Archivo/Casa")
                .join("informe.pdf")
                .to_string_lossy()
        );

        let docx = entry(&doc("nota.docx", Some("  "), &[], None), vec![]);
        assert!(docx.authors.is_empty());
        assert_eq!(docx.title, None);
        assert_eq!((docx.year, docx.year_from_file), (2019, true));
    }

    /// VFS-28 AC-10: the RIS export parses tag by tag into one `TY  - GEN` record per document,
    /// each closed by `ER`, with the fields of R12.
    #[test]
    fn ris_has_one_record_per_document() {
        let entries = [
            entry(
                &doc(
                    "informe.pdf",
                    Some("Informe\nanual"),
                    &["Ana Ruiz", "Luis Gil"],
                    Some("2021"),
                ),
                vec![2, 3],
            ),
            entry(&doc("nota año.docx", None, &[], None), vec![]),
        ];
        let text = ris(&entries);
        let mut records: Vec<Vec<(String, String)>> = vec![Vec::new()];
        for line in text.lines().filter(|l| !l.is_empty()) {
            let (tag, value) = line.split_once("  - ").expect("every line is TAG  - value");
            assert_eq!(tag.len(), 2, "{line}");
            records
                .last_mut()
                .unwrap()
                .push((tag.into(), value.trim_end().into()));
            if tag == "ER" {
                records.push(Vec::new());
            }
        }
        records.pop();
        assert_eq!(records.len(), 2);
        for r in &records {
            assert_eq!(r[0], ("TY".into(), "GEN".into()));
        }
        let tags: Vec<&str> = records[0].iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(tags, ["TY", "AU", "AU", "PY", "TI", "UR", "N1", "ER"]);
        assert!(records[0].contains(&("TI".into(), "Informe anual".into())));
        assert!(records[0].contains(&("N1".into(), "pp. 2, 3".into())));
        assert!(records[1].contains(&("TI".into(), "nota año.docx".into())));
        assert!(records[1].contains(&("PY".into(), "2019".into())));
        assert!(records[1]
            .iter()
            .any(|(t, v)| t == "UR" && v.ends_with("/nota%20a%C3%B1o.docx")));
    }

    #[test]
    fn file_urls_are_encoded() {
        assert_eq!(file_url("/a b/c.pdf"), "file:///a%20b/c.pdf");
        assert_eq!(
            file_url("C:\\Docs\\Año.pdf"),
            "file:///C:/Docs/A%C3%B1o.pdf"
        );
        assert_eq!(year_of("2021-03-04T10:00:00Z"), Some(2021));
        assert_eq!(year_of("sin fecha"), None);
    }
}
