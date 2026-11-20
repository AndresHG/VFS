//! VFS-2 extractor evaluation.
//!
//!   extract-eval one <backend> <file>                 extract one file, print the text
//!   extract-eval run <backend> <corpus> <out.jsonl>   every file in <corpus>/truth.jsonl, each
//!                                                     in a child process (30 s limit); scores
//!                                                     the text against the truth
//!
//! Backends (compiled in by cargo feature): xberg, pdf-extract, pdf-oxide, pdfium, calamine,
//! ooxml. A backend returns `Err("unsupported")` for formats it does not handle.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

type R<T> = Result<T, String>;

fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

#[cfg(any(feature = "xberg-pdf", feature = "xberg-office"))]
fn xberg(path: &Path, bytes: Vec<u8>) -> R<String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| e.to_string())?;
    let input = xberg::ExtractInput {
        kind: xberg::ExtractInputKind::Bytes,
        bytes: Some(bytes),
        filename: path.file_name().map(|n| n.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let config = xberg::ExtractionConfig {
        use_cache: false,
        disable_ocr: true,
        ..Default::default()
    };
    let res = rt
        .block_on(xberg::extract(input, &config))
        .map_err(|e| e.to_string())?;
    if let Some(e) = res.errors.first() {
        return Err(format!("{e:?}"));
    }
    Ok(res
        .results
        .into_iter()
        .map(|d| d.content)
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(feature = "pdf-extract")]
fn pdf_extract(path: &Path, bytes: Vec<u8>) -> R<String> {
    if ext_of(path) != "pdf" {
        return Err("unsupported".into());
    }
    pdf_extract::extract_text_from_mem(&bytes).map_err(|e| e.to_string())
}

#[cfg(feature = "pdf-oxide")]
fn pdf_oxide(path: &Path, bytes: Vec<u8>) -> R<String> {
    if ext_of(path) != "pdf" {
        return Err("unsupported".into());
    }
    let doc = pdf_oxide::PdfDocument::from_bytes(bytes).map_err(|e| e.to_string())?;
    let n = doc.page_count().map_err(|e| e.to_string())?;
    let mut out = String::new();
    for i in 0..n {
        out.push_str(&doc.extract_text(i).map_err(|e| e.to_string())?);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(feature = "pdfium")]
fn pdfium(path: &Path, bytes: Vec<u8>) -> R<String> {
    use pdfium_render::prelude::*;
    if ext_of(path) != "pdf" {
        return Err("unsupported".into());
    }
    let lib = std::env::var("PDFIUM_LIB").map_err(|_| "set PDFIUM_LIB".to_string())?;
    let pdfium = Pdfium::new(Pdfium::bind_to_library(lib).map_err(|e| e.to_string())?);
    let doc = pdfium
        .load_pdf_from_byte_vec(bytes, None)
        .map_err(|e| e.to_string())?;
    let mut out = String::new();
    for page in doc.pages().iter() {
        out.push_str(&page.text().map_err(|e| e.to_string())?.all());
        out.push('\n');
    }
    Ok(out)
}

#[cfg(feature = "calamine")]
fn calamine(path: &Path, bytes: Vec<u8>) -> R<String> {
    use calamine::Reader;
    if !matches!(ext_of(path).as_str(), "xls" | "xlsx" | "ods") {
        return Err("unsupported".into());
    }
    let mut wb = calamine::open_workbook_auto_from_rs(std::io::Cursor::new(bytes))
        .map_err(|e| e.to_string())?;
    let mut out = String::new();
    for name in wb.sheet_names() {
        let range = wb.worksheet_range(&name).map_err(|e| e.to_string())?;
        for row in range.rows() {
            for c in row {
                out.push_str(&c.to_string());
                out.push('\t');
            }
            out.push('\n');
        }
    }
    Ok(out)
}

/// The "single-purpose crates" route for zipped XML formats: zip + quick-xml, text of
/// `w:t` / `a:t` / ODF `text:*` elements, paragraph breaks on `w:p` / `a:p` / `text:p`.
#[cfg(feature = "ooxml")]
fn ooxml(path: &Path, bytes: Vec<u8>) -> R<String> {
    use quick_xml::events::Event;
    let ext = ext_of(path);
    if !matches!(ext.as_str(), "docx" | "pptx" | "odt" | "odp" | "ods") {
        return Err("unsupported".into());
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut parts: Vec<String> = match ext.as_str() {
        "docx" => vec!["word/document.xml".into()],
        "odt" | "odp" | "ods" => vec!["content.xml".into()],
        "pptx" => (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
            .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
            .collect(),
        _ => return Err("unsupported".into()),
    };
    parts.sort_by_key(|n| (n.len(), n.clone()));
    let mut out = String::new();
    for part in parts {
        let mut xml = String::new();
        let f = zip.by_name(&part).map_err(|e| e.to_string())?;
        // Zip-bomb guard: never inflate more than 64 MB of XML per part.
        f.take(64 << 20)
            .read_to_string(&mut xml)
            .map_err(|e| e.to_string())?;
        let mut r = quick_xml::Reader::from_str(&xml);
        let mut in_text = false;
        loop {
            match r.read_event() {
                Ok(Event::Start(e)) => {
                    let n = e.name();
                    let n = n.as_ref();
                    in_text = n == b"w:t" || n == b"a:t" || n.starts_with(b"text:");
                }
                Ok(Event::End(e)) => {
                    let n = e.name();
                    if matches!(n.as_ref(), b"w:p" | b"a:p" | b"text:p" | b"text:h") {
                        out.push('\n');
                    }
                    in_text = false;
                }
                Ok(Event::Text(t)) if in_text => {
                    out.push_str(&t.unescape().map_err(|e| e.to_string())?);
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(e.to_string()),
                _ => {}
            }
        }
    }
    Ok(out)
}

fn extract(backend: &str, path: &Path) -> R<String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let _ = &bytes;
    match backend {
        #[cfg(any(feature = "xberg-pdf", feature = "xberg-office"))]
        "xberg" => xberg(path, bytes),
        #[cfg(feature = "pdf-extract")]
        "pdf-extract" => pdf_extract(path, bytes),
        #[cfg(feature = "pdf-oxide")]
        "pdf-oxide" => pdf_oxide(path, bytes),
        #[cfg(feature = "pdfium")]
        "pdfium" => pdfium(path, bytes),
        #[cfg(feature = "calamine")]
        "calamine" => calamine(path, bytes),
        #[cfg(feature = "ooxml")]
        "ooxml" => ooxml(path, bytes),
        // pdftotext (Poppler, GPL) as a quality reference only; never a candidate.
        "pdftotext" => {
            if ext_of(path) != "pdf" {
                return Err("unsupported".into());
            }
            let o = Command::new("pdftotext")
                .args(["-enc", "UTF-8", "-q"])
                .arg(path)
                .arg("-")
                .output()
                .map_err(|e| e.to_string())?;
            if !o.status.success() {
                return Err(format!("pdftotext exit {:?}", o.status.code()));
            }
            Ok(String::from_utf8_lossy(&o.stdout).into_owned())
        }
        _ => Err(format!("backend {backend} not compiled in")),
    }
}

#[derive(Deserialize)]
struct Truth {
    path: String,
    format: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    marker: Option<String>,
    #[serde(default)]
    expect: Option<String>,
    #[serde(default)]
    producer: Option<String>,
    #[serde(default)]
    two_columns: bool,
    #[serde(default)]
    hyphenated: bool,
}

#[derive(Serialize)]
struct Row {
    path: String,
    format: String,
    producer: Option<String>,
    expect: Option<String>,
    bytes: u64,
    outcome: String, // ok | error | unsupported | timeout | crash
    error: Option<String>,
    ms: f64,
    peak_rss_kb: i64,
    chars: usize,
    token_recall: Option<f64>,
    accent_recall: Option<f64>,
    marker_sentence: Option<bool>,
    hyphen_words_whole: Option<f64>,
    two_columns: bool,
}

/// Lowercased word tokens; letters (accents kept) and digits.
fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

fn recall(truth: &[String], got: &HashMap<String, usize>) -> Option<f64> {
    if truth.is_empty() {
        return None;
    }
    let mut want: HashMap<&str, usize> = HashMap::new();
    for t in truth {
        *want.entry(t).or_default() += 1;
    }
    let hit: usize = want
        .iter()
        .map(|(w, n)| (*n).min(*got.get(*w).unwrap_or(&0)))
        .sum();
    Some(hit as f64 / truth.len() as f64)
}

const HYPHEN_WORDS: &[&str] = &[
    "arrendamiento",
    "comunicación",
    "administración",
    "amortización",
    "indemnización",
    "declaración",
    "calefacción",
    "documentación",
];

fn score(t: &Truth, text: &str, row: &mut Row) {
    let got_toks = tokens(text);
    let mut got: HashMap<String, usize> = HashMap::new();
    for g in &got_toks {
        *got.entry(g.clone()).or_default() += 1;
    }
    let truth = tokens(&t.text);
    row.token_recall = recall(&truth, &got);
    let accented: Vec<String> = truth.iter().filter(|w| !w.is_ascii()).cloned().collect();
    row.accent_recall = recall(&accented, &got);
    if t.marker.is_some() {
        let norm = got_toks.join(" ");
        row.marker_sentence = Some(norm.contains("el señor muñoz pagó la factura de la calefacción"));
    }
    if t.hyphenated {
        let n = HYPHEN_WORDS.iter().filter(|w| got.contains_key(**w)).count();
        row.hyphen_words_whole = Some(n as f64 / HYPHEN_WORDS.len() as f64);
    }
}

/// Runs `one` in a child; returns (outcome, error, text, ms, peak RSS in KB).
fn run_child(exe: &Path, backend: &str, file: &Path) -> (String, Option<String>, String, f64, i64) {
    let t = Instant::now();
    let mut child = Command::new(exe)
        .args(["one", backend])
        .arg(file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let pid = child.id() as libc::pid_t;
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out.read_to_end(&mut v);
        v
    });
    let ereader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.read_to_end(&mut v);
        v
    });
    let limit = Duration::from_secs(30);
    let mut status = 0;
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    let mut timed_out = false;
    loop {
        let r = unsafe { libc::wait4(pid, &mut status, libc::WNOHANG, &mut ru) };
        if r == pid {
            break;
        }
        if t.elapsed() > limit {
            unsafe { libc::kill(pid, libc::SIGKILL) };
            unsafe { libc::wait4(pid, &mut status, 0, &mut ru) };
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let stdout = reader.join().unwrap();
    let stderr = String::from_utf8_lossy(&ereader.join().unwrap()).into_owned();
    // macOS reports ru_maxrss in bytes, Linux in KB.
    let rss_kb = if cfg!(target_os = "macos") { ru.ru_maxrss / 1024 } else { ru.ru_maxrss };
    if timed_out {
        return ("timeout".into(), None, String::new(), ms, rss_kb);
    }
    let exited = libc::WIFEXITED(status);
    let code = if exited { libc::WEXITSTATUS(status) } else { -1 };
    let text = String::from_utf8_lossy(&stdout).into_owned();
    match code {
        0 => ("ok".into(), None, text, ms, rss_kb),
        3 => ("unsupported".into(), None, String::new(), ms, rss_kb),
        2 => ("error".into(), Some(stderr.trim().chars().take(200).collect()), String::new(), ms, rss_kb),
        _ => ("crash".into(), Some(format!("status {status}: {}", stderr.trim().chars().take(200).collect::<String>())), String::new(), ms, rss_kb),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("one") => {
            let r = extract(&args[2], Path::new(&args[3]));
            match r {
                Ok(t) => {
                    std::io::stdout().write_all(t.as_bytes()).unwrap();
                }
                Err(e) if e == "unsupported" => std::process::exit(3),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(2);
                }
            }
        }
        Some("run") => {
            let backend = &args[2];
            let corpus = PathBuf::from(&args[3]);
            let exe = std::env::current_exe().unwrap();
            let truth = std::fs::read_to_string(corpus.join("truth.jsonl")).unwrap();
            let mut out = std::fs::File::create(&args[4]).unwrap();
            for line in truth.lines() {
                let t: Truth = serde_json::from_str(line).unwrap();
                let file = corpus.join(&t.path);
                let bytes = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                let (outcome, error, text, ms, rss) = run_child(&exe, backend, &file);
                let mut row = Row {
                    path: t.path.clone(),
                    format: t.format.clone(),
                    producer: t.producer.clone(),
                    expect: t.expect.clone(),
                    bytes,
                    outcome,
                    error,
                    ms,
                    peak_rss_kb: rss,
                    chars: text.chars().count(),
                    token_recall: None,
                    accent_recall: None,
                    marker_sentence: None,
                    hyphen_words_whole: None,
                    two_columns: t.two_columns,
                };
                if row.outcome == "ok" {
                    score(&t, &text, &mut row);
                }
                writeln!(out, "{}", serde_json::to_string(&row).unwrap()).unwrap();
            }
        }
        _ => {
            eprintln!("usage: extract-eval one <backend> <file> | run <backend> <corpus> <out.jsonl>");
            std::process::exit(64);
        }
    }
}
