//! `vfs content-index`, `vfs content-search`, `vfs content-off` (VFS-2 prototype; built only
//! with `--features content`). The extraction worker is this same executable started with
//! the hidden `content-worker` subcommand.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use vfs_content::worker::CappedAlloc;
use vfs_content::{ContentResponse, IndexOptions};
use vfs_core::{SearchQuery, SearchResponse, Vfs};

use crate::bench::peak_rss_bytes;
use crate::gen::{civil_from_unix, AnyResult};
use crate::human_size;

/// Heap cap for the worker mode; uncapped (usize::MAX) unless the supervisor sets
/// `VFS_EXTRACT_MEM_CAP` for a worker it starts. With the `content` feature this counting
/// allocator serves every `vfs` command (one atomic add per allocation).
#[global_allocator]
static ALLOC: CappedAlloc = CappedAlloc::new();

pub fn worker() {
    vfs_content::worker::worker_main(&ALLOC);
}

/// Peak resident memory of the largest terminated child (the worker), in bytes.
#[cfg(unix)]
fn peak_child_rss_bytes() -> Option<u64> {
    // SAFETY: getrusage only writes into the zeroed struct we own.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut ru) } != 0 {
        return None;
    }
    let v = ru.ru_maxrss as u64;
    Some(if cfg!(target_os = "macos") {
        v
    } else {
        v * 1024
    })
}

#[cfg(windows)]
fn peak_child_rss_bytes() -> Option<u64> {
    None
}

/// CPU seconds (user + system) used by this process and by its terminated children (the
/// worker): independent of how busy the machine was, unlike wall-clock time.
#[cfg(unix)]
fn cpu_seconds() -> (f64, f64) {
    let get = |who| {
        // SAFETY: getrusage only writes into the zeroed struct we own.
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(who, &mut ru) };
        let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
        t(ru.ru_utime) + t(ru.ru_stime)
    };
    (get(libc::RUSAGE_SELF), get(libc::RUSAGE_CHILDREN))
}

#[cfg(windows)]
fn cpu_seconds() -> (f64, f64) {
    (0.0, 0.0)
}

pub fn index(db: &Path, timeout_s: u64, max_files: Option<u64>, json: bool) -> AnyResult<()> {
    if !db.is_file() {
        return Err(format!("{} does not exist (run `vfs index` first)", db.display()).into());
    }
    let exe = std::env::current_exe()?;
    let mut opts = IndexOptions::new(exe, vec!["content-worker".into()]);
    opts.worker.timeout = Duration::from_secs(timeout_s);
    opts.max_files = max_files;
    let sum = vfs_content::index(db, &opts, &AtomicBool::new(false))?;
    // A run that could not go on fails the command after its summary (VFS-2 R17).
    let stopped = sum
        .stopped
        .map(|_| sum.stop_detail.clone().unwrap_or_default());
    let stats = vfs_content::indexer::stats(db)?;
    let parent = peak_rss_bytes().unwrap_or(0);
    let child = peak_child_rss_bytes().unwrap_or(0);
    let (cpu_self, cpu_worker) = cpu_seconds();
    if json {
        let v = serde_json::json!({
            "run": sum,
            "store": stats,
            "peakRssBytes": parent,
            "workerPeakRssBytes": child,
            "cpuSeconds": cpu_self,
            "workerCpuSeconds": cpu_worker,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return match stopped {
            Some(d) => Err(format!("content run stopped: {d}").into()),
            None => Ok(()),
        };
    }
    let secs = sum.elapsed_ms as f64 / 1000.0;
    println!(
        "{} queued: {} indexed, {} skipped, {} failed ({} still queued) in {secs:.1} s",
        sum.queued, sum.indexed, sum.skipped, sum.failed, sum.remaining
    );
    for (k, n) in &sum.reasons {
        println!("  {k}: {n}");
    }
    println!(
        "read {} from {} files ({:.1} MB/s, {:.0} files/s); {} worker start(s)",
        human_size(sum.bytes_read),
        sum.files_opened,
        sum.bytes_read as f64 / 1e6 / secs.max(1e-9),
        sum.files_opened as f64 / secs.max(1e-9),
        sum.worker_starts
    );
    println!(
        "content store {} ({} documents indexed in total); peak RSS {} (indexer), {} (worker)",
        human_size(stats.store_bytes),
        stats.indexed,
        human_size(parent),
        human_size(child)
    );
    match stopped {
        Some(d) => Err(format!("content run stopped: {d}").into()),
        None => Ok(()),
    }
}

/// How content hits are shown next to name hits. The app uses a separate section (VFS-2 D9);
/// the CLI also shows a mixed list for comparison.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Layout {
    /// Content hits only.
    Content,
    /// Name hits, then content hits, as two sections.
    Sections,
    /// One list: name hits first, then content hits not already listed.
    Mixed,
}

/// `«…»` around each highlighted range of the snippet.
fn marked(snippet: &str, highlights: &[[usize; 2]]) -> String {
    let mut out = String::with_capacity(snippet.len() + highlights.len() * 4);
    let mut at = 0;
    for [b, e] in highlights {
        out.push_str(&snippet[at..*b]);
        out.push('«');
        out.push_str(&snippet[*b..*e]);
        out.push('»');
        at = *e;
    }
    out.push_str(&snippet[at..]);
    out
}

fn print_content(r: &ContentResponse, skip: &[i64]) {
    let sep = std::path::MAIN_SEPARATOR;
    for h in r.hits.iter().filter(|h| !skip.contains(&h.id)) {
        let (y, m, d) = civil_from_unix(h.modified);
        println!(
            "{:>10}  {y:04}-{m:02}-{d:02}  {:6.2}  {}{sep}{}",
            human_size(h.size),
            h.score,
            h.dir,
            h.name
        );
        println!("            {}", marked(&h.snippet, &h.highlights));
    }
}

fn print_names(r: &SearchResponse) {
    let sep = std::path::MAIN_SEPARATOR;
    for h in &r.hits {
        println!(
            "{:>10}  {:?}  {}{sep}{}",
            human_size(h.size),
            h.matched,
            h.dir,
            h.name
        );
    }
}

pub fn search(db: &Path, q: &SearchQuery, layout: Layout, runs: u32, json: bool) -> AnyResult<()> {
    let t = Instant::now();
    let searcher = match vfs_content::Searcher::open(db) {
        Err(vfs_content::Error::Off) => {
            return Err("content search is off for this index (run `vfs content-index`)".into())
        }
        r => r?,
    };
    let mut content = searcher.search(q)?;
    // CPU time of the query, which a busy machine does not inflate the way it does wall time.
    let mut cpu_ms = Vec::new();
    if runs > 1 {
        // Warm repeats on one connection, as the app would run them: report the medians.
        let mut ms = Vec::new();
        for _ in 0..runs {
            let c0 = cpu_seconds().0;
            ms.push(searcher.search(q)?.elapsed_ms);
            cpu_ms.push((cpu_seconds().0 - c0) * 1000.0);
        }
        ms.sort_unstable();
        content.elapsed_ms = ms[ms.len() / 2];
        cpu_ms.sort_by(f64::total_cmp);
    }
    let median_cpu_ms = cpu_ms.get(cpu_ms.len() / 2).copied();
    let names = match layout {
        Layout::Content => None,
        Layout::Sections | Layout::Mixed => Some(Vfs::open(db)?.search(q)?),
    };
    let wall = t.elapsed().as_secs_f64() * 1000.0;
    if json {
        let v =
            serde_json::json!({ "content": content, "names": names, "medianCpuMs": median_cpu_ms });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    match (layout, &names) {
        (Layout::Sections, Some(n)) => {
            println!("Por nombre ({}):", n.total);
            print_names(n);
            println!("En el contenido de los documentos ({}):", content.total);
            print_content(&content, &[]);
        }
        (Layout::Mixed, Some(n)) => {
            print_names(n);
            let ids: Vec<i64> = n.hits.iter().filter(|h| !h.is_dir).map(|h| h.id).collect();
            print_content(&content, &ids);
        }
        _ => print_content(&content, &[]),
    }
    let more = if content.total_capped { "+" } else { "" };
    println!(
        "{} of {}{more} content hits; {} ms in the engine, {wall:.1} ms wall",
        content.hits.len(),
        content.total,
        content.elapsed_ms
    );
    Ok(())
}

pub fn off(db: &Path) -> AnyResult<()> {
    vfs_content::switch_off(db)?;
    println!("content search switched off; content store deleted");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-2 AC-11: highlighted ranges are wrapped in «».
    #[test]
    fn marks_highlights() {
        assert_eq!(
            marked("la cigüeña y la niña", &[[3, 12], [18, 23]]),
            "la «cigüeña» y la «niña»"
        );
        assert_eq!(marked("nada", &[]), "nada");
    }
}
