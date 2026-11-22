//! VFS-2 full-text engine comparison on the texts of a content store.
//!
//!   index-eval build fts5|tantivy <content.db> <out>    build from docs.text, report JSON
//!   index-eval query fts5|tantivy <out> <queries.txt>   run each query (top 20), report JSON
//!
//! Both engines get the same analysis: words, lowercase, Snowball Spanish stem, accent fold.
//! FTS5 receives the keys pre-analysed (as `vfs-content` does); Tantivy analyses with its own
//! pipeline (SimpleTokenizer, LowerCaser, Stemmer(Spanish), AsciiFoldingFilter).

use std::io::Read;
use std::path::Path;
use std::time::Instant;

use rust_stemmers::{Algorithm, Stemmer};

fn peak_rss_mb() -> f64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let b = if cfg!(target_os = "macos") { ru.ru_maxrss as f64 } else { ru.ru_maxrss as f64 * 1024.0 };
    b / 1048576.0
}

fn dir_bytes(p: &Path) -> u64 {
    if p.is_file() {
        return std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    }
    std::fs::read_dir(p)
        .map(|rd| rd.flatten().map(|e| dir_bytes(&e.path())).sum())
        .unwrap_or(0)
}

/// (file_id, title, text) of every indexed document.
fn load(content_db: &Path) -> Vec<(i64, String, String)> {
    let conn = rusqlite::Connection::open(content_db).unwrap();
    let mut st = conn
        .prepare("SELECT file_id, coalesce(title, ''), text FROM docs WHERE state = 2")
        .unwrap();
    st.query_map([], |r| {
        let blob: Vec<u8> = r.get(2)?;
        let mut s = String::new();
        flate2::read::DeflateDecoder::new(&blob[..]).read_to_string(&mut s).unwrap();
        Ok((r.get(0)?, r.get(1)?, s))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

fn keys(stemmer: &Stemmer, text: &str) -> String {
    let mut out = String::with_capacity(text.len() / 2);
    for w in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && w.chars().count() <= 40) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&vfs_core::fold_key(&stemmer.stem(&w.to_lowercase())));
    }
    out
}

fn fts5_build(docs: &[(i64, String, String)], out: &Path) -> serde_json::Value {
    let stemmer = Stemmer::create(Algorithm::Spanish);
    let t = Instant::now();
    let mut conn = rusqlite::Connection::open(out).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
         CREATE VIRTUAL TABLE content_fts USING fts5(title, body, content='', contentless_delete=1,
           tokenize='unicode61 remove_diacritics 0');",
    )
    .unwrap();
    let mut analyse_ms = 0.0;
    for chunk in docs.chunks(500) {
        let tx = conn.transaction().unwrap();
        {
            let mut ins = tx.prepare("INSERT INTO content_fts(rowid, title, body) VALUES (?1, ?2, ?3)").unwrap();
            for (id, title, text) in chunk {
                let a = Instant::now();
                let (tk, bk) = (keys(&stemmer, title), keys(&stemmer, text));
                analyse_ms += a.elapsed().as_secs_f64() * 1000.0;
                ins.execute(rusqlite::params![id, tk, bk]).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    conn.execute_batch("INSERT INTO content_fts(content_fts) VALUES('optimize'); PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    drop(conn);
    serde_json::json!({
        "engine": Engine::Fts5.name(), "docs": docs.len(), "build_s": t.elapsed().as_secs_f64(),
        "analyse_s": analyse_ms / 1000.0, "index_bytes": dir_bytes(out), "peak_rss_mb": peak_rss_mb()
    })
}

fn fts5_query(out: &Path, queries: &[String]) -> Vec<f64> {
    let stemmer = Stemmer::create(Algorithm::Spanish);
    let conn = rusqlite::Connection::open(out).unwrap();
    let mut st = conn
        .prepare("SELECT rowid, bm25(content_fts, 3.0, 1.0) AS s FROM content_fts WHERE content_fts MATCH ?1 ORDER BY s LIMIT 20")
        .unwrap();
    let mut lat = Vec::new();
    for q in queries {
        // Quoted runs are phrases, other words AND-ed (as vfs-content does).
        let expr: Vec<String> = q
            .split('"')
            .enumerate()
            .flat_map(|(i, part)| {
                if i % 2 == 1 {
                    vec![format!("\"{}\"", keys(&stemmer, part))]
                } else {
                    part.split_whitespace().map(|w| format!("\"{}\"", keys(&stemmer, w))).collect()
                }
            })
            .filter(|e| e != "\"\"")
            .collect();
        let expr = expr.join(" AND ");
        let mut run = || st.query_map([&expr], |r| r.get::<_, i64>(0)).unwrap().count();
        run();
        let mut best = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            run();
            best.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        best.sort_by(|a, b| a.partial_cmp(b).unwrap());
        lat.push(best[2]);
    }
    lat
}

#[cfg(feature = "tantivy")]
mod tv {
    use super::*;
    use tantivy::collector::TopDocs;
    use tantivy::query::QueryParser;
    use tantivy::schema::{IndexRecordOption, Schema, TextFieldIndexing, TextOptions, INDEXED};
    use tantivy::tokenizer::{AsciiFoldingFilter, Language, LowerCaser, SimpleTokenizer, Stemmer as TStemmer, TextAnalyzer};
    use tantivy::{doc, Index};

    fn analyzer() -> TextAnalyzer {
        TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(LowerCaser)
            .filter(TStemmer::new(Language::Spanish))
            .filter(AsciiFoldingFilter)
            .build()
    }

    fn schema() -> Schema {
        let mut b = Schema::builder();
        let opts = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default().set_tokenizer("es").set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );
        b.add_u64_field("id", INDEXED);
        b.add_text_field("title", opts.clone());
        b.add_text_field("body", opts);
        b.build()
    }

    pub fn build(docs: &[(i64, String, String)], out: &Path) -> serde_json::Value {
        let t = Instant::now();
        std::fs::create_dir_all(out).unwrap();
        let schema = schema();
        let index = Index::create_in_dir(out, schema.clone()).unwrap();
        index.tokenizers().register("es", analyzer());
        let (id, title, body) = (
            schema.get_field("id").unwrap(),
            schema.get_field("title").unwrap(),
            schema.get_field("body").unwrap(),
        );
        // One thread and a 50 MB budget, as a low-memory PC would get.
        let mut w = index.writer_with_num_threads(1, 50_000_000).unwrap();
        for (i, tt, b) in docs {
            w.add_document(doc!(id => *i as u64, title => tt.as_str(), body => b.as_str())).unwrap();
        }
        w.commit().unwrap();
        w.wait_merging_threads().unwrap();
        serde_json::json!({
            "engine": Engine::Tantivy.name(), "docs": docs.len(), "build_s": t.elapsed().as_secs_f64(),
            "index_bytes": dir_bytes(out), "peak_rss_mb": peak_rss_mb()
        })
    }

    pub fn query(out: &Path, queries: &[String]) -> Vec<f64> {
        let index = Index::open_in_dir(out).unwrap();
        index.tokenizers().register("es", analyzer());
        let schema = index.schema();
        let (title, body) = (schema.get_field("title").unwrap(), schema.get_field("body").unwrap());
        let reader = index.reader().unwrap();
        let searcher = reader.searcher();
        let mut qp = QueryParser::for_index(&index, vec![title, body]);
        qp.set_conjunction_by_default();
        qp.set_field_boost(title, 3.0);
        let mut lat = Vec::new();
        for q in queries {
            let parsed = qp.parse_query(q).unwrap();
            let run = || searcher.search(&parsed, &TopDocs::with_limit(20).order_by_score()).unwrap().len();
            run();
            let mut best = Vec::new();
            for _ in 0..5 {
                let t = Instant::now();
                run();
                best.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            best.sort_by(|a, b| a.partial_cmp(b).unwrap());
            lat.push(best[2]);
        }
        lat
    }
}

/// The engines this harness can build and query; `Load` only loads the texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    Load,
    Fts5,
    Tantivy,
}

impl Engine {
    fn parse(s: &str) -> Engine {
        [Engine::Load, Engine::Fts5, Engine::Tantivy]
            .into_iter()
            .find(|e| e.name() == s)
            .unwrap_or_else(|| panic!("unknown engine {s}"))
    }

    fn name(self) -> &'static str {
        match self {
            Engine::Load => "load",
            Engine::Fts5 => "fts5",
            Engine::Tantivy => "tantivy",
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let engine = Engine::parse(&a[2]);
    match a[1].as_str() {
        "build" => {
            let docs = load(Path::new(&a[3]));
            let out = Path::new(&a[4]);
            let _ = std::fs::remove_file(out);
            let _ = std::fs::remove_dir_all(out);
            let r = match engine {
                // Baseline: only loading the texts, to subtract from the engines' peak RSS.
                Engine::Load => serde_json::json!({"engine": engine.name(), "docs": docs.len(),
                    "text_bytes": docs.iter().map(|d| d.2.len()).sum::<usize>(), "peak_rss_mb": peak_rss_mb()}),
                Engine::Fts5 => fts5_build(&docs, out),
                #[cfg(feature = "tantivy")]
                Engine::Tantivy => tv::build(&docs, out),
                #[cfg(not(feature = "tantivy"))]
                Engine::Tantivy => panic!("engine tantivy not compiled in"),
            };
            println!("{r}");
        }
        "query" => {
            let qs: Vec<String> = std::fs::read_to_string(&a[4]).unwrap().lines().filter(|l| !l.trim().is_empty()).map(String::from).collect();
            let out = Path::new(&a[3]);
            let lat = match engine {
                Engine::Fts5 => fts5_query(out, &qs),
                #[cfg(feature = "tantivy")]
                Engine::Tantivy => tv::query(out, &qs),
                _ => panic!("engine {} cannot be queried", engine.name()),
            };
            let mut s = lat.clone();
            s.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let pct = |p: f64| s[((s.len() as f64 * p).ceil() as usize).clamp(1, s.len()) - 1];
            let per: Vec<serde_json::Value> = qs.iter().zip(&lat).map(|(q, l)| serde_json::json!({"q": q, "ms": l})).collect();
            println!("{}", serde_json::json!({
                "engine": engine.name(), "queries": qs.len(), "p50_ms": pct(0.5), "p95_ms": pct(0.95),
                "max_ms": s[s.len() - 1], "peak_rss_mb": peak_rss_mb(), "per_query": per
            }));
        }
        _ => eprintln!("usage: see the module comment"),
    }
}
