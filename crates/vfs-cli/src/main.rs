//! `vfs` command line (VFS-1): index folders and search from a terminal, generate a
//! synthetic index (`gen-db`) and run the AC-9 benchmark on it (`bench`).

mod bench;
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
    /// Add folders to the index and scan them to completion in the foreground.
    Index {
        #[arg(long)]
        db: PathBuf,
        #[arg(required = true)]
        roots: Vec<PathBuf>,
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
    },
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
        let root = vfs.add_root(&abs)?;
        println!("added root {} ({})", root.id, root.path);
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
        } => {
            let filters = SearchFilters {
                kinds: kind.into_iter().map(FileKind::from).collect(),
                modified_from: from.as_deref().map(parse_date).transpose()?,
                modified_to: to.as_deref().map(parse_date).transpose()?,
                size_min: min_size.as_deref().map(parse_size).transpose()?,
                size_max: max_size.as_deref().map(parse_size).transpose()?,
                folder,
            };
            let q = SearchQuery {
                text: query.join(" "),
                filters,
                limit,
                offset,
            };
            if source == sources::SourceArg::Files {
                cmd_search(&Vfs::open(&db)?, &q, json)?;
            } else {
                sources::cmd_search(&db, &q, source, json)?;
            }
        }
        Cmd::Bookmarks {
            db,
            action,
            firefox_root,
        } => sources::cmd_bookmarks(&db, action, firefox_root)?,
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
        Cmd::Bench { db, runs, json } => {
            let report = bench::run(&db, runs.max(1))?;
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
