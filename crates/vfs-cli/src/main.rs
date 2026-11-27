//! `vfs` command line (VFS-1): index folders and search from a terminal, generate a
//! synthetic index (`gen-db`) and run the AC-9 benchmark on it (`bench`).

#[cfg(feature = "content")]
mod ask;
mod bench;
#[cfg(feature = "content")]
mod content;
mod gen;
mod sources;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use clap::{Parser, Subcommand, ValueEnum};
use vfs_core::{FileKind, SearchFilters, SearchQuery, Vfs};

use gen::{civil_from_unix, days_from_civil, AnyResult};

#[derive(Parser)]
#[command(
    name = "vfs",
    version,
    about = "VFS file search: index, search, bookmarks, gen-db, bench"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Add folders to the index (unless already indexed) and scan pending folders to
    /// completion in the foreground.
    Index {
        #[arg(long)]
        db: PathBuf,
        #[arg(required = true)]
        roots: Vec<PathBuf>,
    },
    /// Run the background indexer and watcher, as the app does, printing each progress
    /// snapshot as one JSON line. Folders given are added unless already indexed. Stops
    /// cleanly when stdin closes, or with `--until-idle` once every root is up to date.
    Run {
        #[arg(long)]
        db: PathBuf,
        roots: Vec<PathBuf>,
        /// Exit once the indexer is idle with every root ready and nothing pending.
        #[arg(long)]
        until_idle: bool,
        /// Start the catch-up walk at once, even within ten minutes of booting.
        #[arg(long)]
        no_boot_grace: bool,
    },
    /// Search the index by name and path, with optional filters.
    Search {
        #[arg(long)]
        db: PathBuf,
        /// Query words (may be empty when a filter is given).
        query: Vec<String>,
        /// Comma-separated kinds.
        #[arg(long, value_delimiter = ',')]
        kind: Vec<KindArg>,
        /// Modified on or after this date (YYYY-MM-DD).
        #[arg(long)]
        from: Option<String>,
        /// Modified before this date (YYYY-MM-DD).
        #[arg(long)]
        to: Option<String>,
        /// Minimum size, e.g. 500K, 10M, 4G.
        #[arg(long)]
        min_size: Option<String>,
        /// Maximum size, e.g. 500K, 10M, 4G.
        #[arg(long)]
        max_size: Option<String>,
        /// Only hits inside this folder (absolute path).
        #[arg(long)]
        folder: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        /// Print the raw response as JSON.
        #[arg(long)]
        json: bool,
        /// Which sources to search (VFS-6); bookmarks only once `vfs bookmarks on` was run.
        #[arg(long, value_enum, default_value = "all")]
        source: sources::SourceArg,
        /// Order of the results (VFS-6).
        #[arg(long, value_enum, default_value = "relevance")]
        sort: sources::SortArg,
        /// Match the words against names only: file names, folder names, bookmark titles
        /// (VFS-38 R3).
        #[arg(long)]
        names_only: bool,
    },
    /// Firefox bookmarks as a search source (VFS-6), stored next to the index.
    Bookmarks {
        #[arg(long)]
        db: PathBuf,
        #[arg(value_enum)]
        action: sources::BookmarksAction,
        /// Folders holding Firefox's profiles.ini (default: this system's).
        #[arg(long)]
        firefox_root: Vec<PathBuf>,
    },
    /// Write a deterministic synthetic index of N files (AC-9 harness).
    GenDb {
        #[arg(long)]
        files: u64,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Replace an existing database at --out.
        #[arg(long)]
        force: bool,
    },
    /// Extract and index the text of the indexed documents (VFS-2 prototype; switches
    /// content search on for this index).
    #[cfg(feature = "content")]
    ContentIndex {
        #[arg(long)]
        db: PathBuf,
        /// Wall-clock limit per file, in seconds.
        #[arg(long, default_value_t = 30)]
        timeout: u64,
        /// Stop after this many files (the next run resumes).
        #[arg(long)]
        max_files: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    /// Embed the content chunks with the local model (VFS-29), into `vectors.db` next to the
    /// index. Resumes where the last run stopped.
    #[cfg(feature = "content")]
    EmbedIndex {
        #[arg(long)]
        db: PathBuf,
        /// Folder holding the model's `model.onnx` and `tokenizer.json`.
        #[arg(long)]
        model_dir: PathBuf,
        /// The `vfs-embed` worker (default: next to this executable).
        #[arg(long)]
        worker: Option<PathBuf>,
        #[arg(long, default_value_t = 2)]
        threads: usize,
        #[arg(long)]
        json: bool,
    },
    /// Search inside documents (VFS-2 prototype), with the same filters as `search`.
    #[cfg(feature = "content")]
    ContentSearch {
        #[arg(long)]
        db: PathBuf,
        /// Words; "quoted words" form a phrase.
        query: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        kind: Vec<KindArg>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        min_size: Option<String>,
        #[arg(long)]
        max_size: Option<String>,
        #[arg(long)]
        folder: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        /// How content hits sit next to name hits.
        #[arg(long, value_enum, default_value = "content")]
        layout: content::Layout,
        /// Repeat the query this many times on one connection and report the median time.
        #[arg(long, default_value_t = 1)]
        runs: u32,
        /// Fuse with search by meaning (VFS-29): the local model's folder; needs
        /// `embed-index` run first.
        #[arg(long)]
        meaning_model_dir: Option<PathBuf>,
        /// The `vfs-embed` worker (default: next to this executable).
        #[arg(long)]
        embed_worker: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Ask the archive a question with a real provider (VFS-28; the key is read from the
    /// VFS_AI_KEY environment variable). Needs content search on for this index.
    #[cfg(feature = "content")]
    Ask {
        #[arg(long)]
        db: PathBuf,
        /// The question.
        question: Vec<String>,
        #[arg(long, value_enum, default_value = "anthropic")]
        provider: ProviderArg,
        /// Answer in English instead of Spanish.
        #[arg(long)]
        english: bool,
        /// The cheap tier: answer with the cheap model.
        #[arg(long)]
        cheap: bool,
        /// Ask even when the month's cost cap is reached.
        #[arg(long)]
        yes: bool,
        /// The record of what was sent; default `ai.db` next to the index.
        #[arg(long)]
        ai_db: Option<PathBuf>,
        #[arg(long)]
        folder: Option<String>,
    },
    /// Switch content search off: delete the content store (VFS-2 prototype).
    #[cfg(feature = "content")]
    ContentOff {
        #[arg(long)]
        db: PathBuf,
    },
    /// Extraction worker (started by `content-index`; not for direct use).
    #[cfg(feature = "content")]
    #[command(hide = true)]
    ContentWorker {
        /// Test hooks, passed through to the worker.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },
    /// Run the fixed AC-9 query set and report latency, peak RSS and PASS/FAIL.
    Bench {
        #[arg(long)]
        db: PathBuf,
        /// Measured runs per query (after one warm-up run).
        #[arg(long, default_value_t = 5)]
        runs: u32,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
        /// Order applied to every query of the set (VFS-12); `source` measures as relevance.
        #[arg(long, value_enum, default_value_t = sources::SortArg::Relevance)]
        sort: sources::SortArg,
    },
}

#[cfg(feature = "content")]
#[derive(Clone, Copy, ValueEnum)]
enum ProviderArg {
    Anthropic,
    Openai,
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    Document,
    Photo,
    Video,
    Audio,
    Other,
    Folder,
}

impl From<KindArg> for FileKind {
    fn from(k: KindArg) -> FileKind {
        match k {
            KindArg::Document => FileKind::Document,
            KindArg::Photo => FileKind::Photo,
            KindArg::Video => FileKind::Video,
            KindArg::Audio => FileKind::Audio,
            KindArg::Other => FileKind::Other,
            KindArg::Folder => FileKind::Folder,
        }
    }
}

/// `YYYY-MM-DD` to unix seconds (UTC midnight).
fn parse_date(s: &str) -> AnyResult<i64> {
    let parts: Vec<&str> = s.split('-').collect();
    let bad = || format!("invalid date {s:?}, expected YYYY-MM-DD");
    if parts.len() != 3 {
        return Err(bad().into());
    }
    let y: i32 = parts[0].parse().map_err(|_| bad())?;
    let m: u32 = parts[1].parse().map_err(|_| bad())?;
    let d: u32 = parts[2].parse().map_err(|_| bad())?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err(bad().into());
    }
    Ok(days_from_civil(y, m, d) * 86_400)
}

/// `123`, `500K`, `10M`, `4G` (binary multiples) to bytes.
fn parse_size(s: &str) -> AnyResult<u64> {
    let t = s.trim();
    let (num, mult) = match t.chars().last().map(|c| c.to_ascii_uppercase()) {
        Some('K') => (&t[..t.len() - 1], 1u64 << 10),
        Some('M') => (&t[..t.len() - 1], 1 << 20),
        Some('G') => (&t[..t.len() - 1], 1 << 30),
        Some('T') => (&t[..t.len() - 1], 1 << 40),
        _ => (t, 1),
    };
    let n: f64 = num
        .trim()
        .parse()
        .map_err(|_| format!("invalid size {s:?}, expected e.g. 500K, 10M, 4G"))?;
    Ok((n * mult as f64) as u64)
}

fn build_filters(
    kind: Vec<KindArg>,
    from: Option<String>,
    to: Option<String>,
    min_size: Option<String>,
    max_size: Option<String>,
    folder: Option<String>,
) -> AnyResult<SearchFilters> {
    Ok(SearchFilters {
        kinds: kind.into_iter().map(FileKind::from).collect(),
        modified_from: from.as_deref().map(parse_date).transpose()?,
        modified_to: to.as_deref().map(parse_date).transpose()?,
        size_min: min_size.as_deref().map(parse_size).transpose()?,
        size_max: max_size.as_deref().map(parse_size).transpose()?,
        folder,
    })
}

fn human_size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn remove_db(path: &Path) -> std::io::Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        if Path::new(&p).exists() {
            std::fs::remove_file(&p)?;
        }
    }
    Ok(())
}

fn cmd_index(db: &Path, roots: &[PathBuf]) -> AnyResult<()> {
    let vfs = Vfs::open(db)?;
    for r in roots {
        let abs = std::path::absolute(r)?;
        match vfs.add_root(&abs) {
            Ok(root) => println!("added root {} ({})", root.id, root.path),
            // Already indexed: an interrupted run resumes its pending folders.
            Err(e) if e.is_same_root() => {}
            Err(e) => return Err(e.into()),
        }
    }
    let t = Instant::now();
    let summary = vfs.scan_to_completion(&AtomicBool::new(false))?;
    let secs = t.elapsed().as_secs_f64();
    println!("{}", serde_json::to_string_pretty(&summary)?);
    println!(
        "{:.1} s, {:.0} files/s, {:.0} dirs/s",
        secs,
        summary.files_added as f64 / secs.max(1e-9),
        summary.dirs_listed as f64 / secs.max(1e-9)
    );
    Ok(())
}

/// Writes the core's warnings and errors to stderr, so `vfs run` shows what the indexer
/// logs (an unreadable folder, a failed transaction).
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("vfs: {}: {}", r.level(), r.args());
        }
    }
    fn flush(&self) {}
}

/// Prints progress snapshots as JSON lines and signals when the index is up to date.
struct LineSink {
    until_idle: bool,
    done: std::sync::mpsc::Sender<()>,
}

impl vfs_core::ProgressSink for LineSink {
    fn on_progress(&self, p: &vfs_core::IndexProgress) {
        use std::io::Write;
        if let Ok(line) = serde_json::to_string(p) {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
        if self.until_idle && is_up_to_date(p) {
            let _ = self.done.send(());
        }
    }
}

/// Idle, with at least one root, every root ready and no directory pending.
fn is_up_to_date(p: &vfs_core::IndexProgress) -> bool {
    p.state == vfs_core::IndexerState::Idle
        && !p.roots.is_empty()
        && p.roots
            .iter()
            .all(|r| r.status == vfs_core::RootStatus::Ready && r.dirs_pending == 0)
}

fn cmd_run(db: &Path, roots: &[PathBuf], until_idle: bool, no_boot_grace: bool) -> AnyResult<()> {
    if log::set_logger(&StderrLog).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    let vfs = Vfs::open(db)?;
    for r in roots {
        match vfs.add_root(&std::path::absolute(r)?) {
            Ok(_) => {}
            Err(e) if e.is_same_root() => {}
            Err(e) => return Err(e.into()),
        }
    }
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let mut options = vfs_core::IndexerOptions::default();
    if no_boot_grace {
        options.boot_grace = std::time::Duration::ZERO;
    }
    vfs.start_indexer_with(
        std::sync::Arc::new(LineSink {
            until_idle,
            done: done_tx.clone(),
        }),
        options,
    )?;
    if !until_idle {
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
            let _ = done_tx.send(());
        });
    }
    let _ = done_rx.recv();
    vfs.shutdown();
    Ok(())
}

fn cmd_search(vfs: &Vfs, query: &SearchQuery, json: bool) -> AnyResult<()> {
    let t = Instant::now();
    let r = vfs.search(query)?;
    let elapsed = t.elapsed().as_secs_f64() * 1000.0;
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    for h in &r.hits {
        let (y, m, d) = civil_from_unix(h.modified);
        let size = if h.is_dir {
            "carpeta".to_string()
        } else {
            human_size(h.size)
        };
        let sep = std::path::MAIN_SEPARATOR;
        println!(
            "{size:>10}  {y:04}-{m:02}-{d:02}  {:?}  {}{sep}{}",
            h.matched, h.dir, h.name
        );
    }
    let cut = r
        .truncated_by
        .map_or(String::new(), |t| format!(", truncated by {t:?}"));
    println!(
        "{} of {} hits{cut}; {} ms in the engine, {elapsed:.1} ms wall",
        r.hits.len(),
        r.total,
        r.elapsed_ms
    );
    Ok(())
}

fn run(cli: Cli) -> AnyResult<ExitCode> {
    match cli.cmd {
        Cmd::Index { db, roots } => cmd_index(&db, &roots)?,
        Cmd::Run {
            db,
            roots,
            until_idle,
            no_boot_grace,
        } => cmd_run(&db, &roots, until_idle, no_boot_grace)?,
        Cmd::Search {
            db,
            query,
            kind,
            from,
            to,
            min_size,
            max_size,
            folder,
            limit,
            offset,
            json,
            source,
            sort,
            names_only,
        } => {
            let filters = build_filters(kind, from, to, min_size, max_size, folder)?;
            let q = SearchQuery {
                text: query.join(" "),
                filters,
                limit,
                offset,
                sort: vfs_bookmarks::SortOrder::from(sort).files(),
                scope: if names_only {
                    vfs_core::Scope::Names
                } else {
                    vfs_core::Scope::All
                },
            };
            // Without a bookmark store (`vfs bookmarks on` never ran), plain file output.
            let files_only = source == sources::SourceArg::Files
                || !vfs_bookmarks::bookmarks_db_path(&db).exists();
            // The core applies every order but `source`, which only orders across sources.
            if files_only {
                cmd_search(&Vfs::open(&db)?, &q, json)?;
            } else {
                sources::cmd_search(&db, &q, source, sort, json)?;
            }
        }
        Cmd::Bookmarks {
            db,
            action,
            firefox_root,
        } => sources::cmd_bookmarks(&db, action, firefox_root)?,
        #[cfg(feature = "content")]
        Cmd::ContentIndex {
            db,
            timeout,
            max_files,
            json,
        } => content::index(&db, timeout, max_files, json)?,
        #[cfg(feature = "content")]
        Cmd::EmbedIndex {
            db,
            model_dir,
            worker,
            threads,
            json,
        } => content::embed_index(&db, &model_dir, worker, threads, json)?,
        #[cfg(feature = "content")]
        Cmd::ContentSearch {
            db,
            query,
            kind,
            from,
            to,
            min_size,
            max_size,
            folder,
            limit,
            offset,
            layout,
            runs,
            meaning_model_dir,
            embed_worker,
            json,
        } => {
            let filters = build_filters(kind, from, to, min_size, max_size, folder)?;
            let q = SearchQuery {
                text: query.join(" "),
                filters,
                limit,
                offset,
                sort: Default::default(),
                scope: Default::default(),
            };
            let meaning = meaning_model_dir.map(|dir| content::Meaning {
                model_dir: dir,
                worker: embed_worker,
            });
            content::search(&db, &q, layout, runs, meaning, json)?;
        }
        #[cfg(feature = "content")]
        Cmd::Ask {
            db,
            question,
            provider,
            english,
            cheap,
            yes,
            ai_db,
            folder,
        } => {
            let ai_db = ai_db.unwrap_or_else(|| db.with_file_name("ai.db"));
            ask::run(ask::AskArgs {
                db: &db,
                ai_db: &ai_db,
                provider: match provider {
                    ProviderArg::Anthropic => vfs_ai::Provider::Anthropic,
                    ProviderArg::Openai => vfs_ai::Provider::OpenAi,
                },
                lang: if english {
                    vfs_ai::ask::Lang::En
                } else {
                    vfs_ai::ask::Lang::Es
                },
                cheap,
                yes,
                filters: build_filters(Vec::new(), None, None, None, None, folder)?,
                question: question.join(" "),
            })?
        }
        #[cfg(feature = "content")]
        Cmd::ContentOff { db } => content::off(&db)?,
        #[cfg(feature = "content")]
        Cmd::ContentWorker { .. } => content::worker(),
        Cmd::GenDb {
            files,
            out,
            seed,
            force,
        } => {
            if force {
                remove_db(&out)?;
            }
            eprintln!(
                "generating {files} files (seed {seed}) into {}",
                out.display()
            );
            let r = gen::gen_db(&out, files, seed, true)?;
            println!("{}", serde_json::to_string_pretty(&r)?);
            println!(
                "{} files, {} dirs in {:.1} s; database {}",
                r.files,
                r.dirs,
                r.elapsed_ms as f64 / 1000.0,
                human_size(r.db_bytes)
            );
        }
        Cmd::Bench {
            db,
            runs,
            json,
            sort,
        } => {
            let order = vfs_bookmarks::SortOrder::from(sort).files();
            let report = bench::run(&db, runs.max(1), order)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                bench::print_text(&report);
            }
            if !report.pass {
                return Ok(ExitCode::from(1));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("vfs: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-1 AC-11: CLI filter arguments parse into the contract's units.
    #[test]
    fn parses_filter_arguments() {
        assert_eq!(parse_size("10M").unwrap(), 10 << 20);
        assert_eq!(parse_size("4g").unwrap(), 4 << 30);
        assert_eq!(parse_size("1.5K").unwrap(), 1536);
        assert_eq!(parse_size("123").unwrap(), 123);
        assert!(parse_size("diez").is_err());
        assert_eq!(parse_date("1970-01-02").unwrap(), 86_400);
        assert!(parse_date("2019-13-01").is_err());
        assert!(parse_date("2019/01/01").is_err());
        assert_eq!(human_size(1536), "1.5 KB");
    }
}
