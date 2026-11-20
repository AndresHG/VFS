//! `vfs bench`: the fixed AC-9 query set through `Vfs::search`, warm and with cold SQLite
//! caches, with per-class latency percentiles, peak RSS, database size and a PASS/FAIL line.

use std::path::Path;
use std::time::Instant;

use serde::Serialize;
use vfs_core::{FileKind, SearchFilters, SearchQuery, TruncatedBy, Vfs};

use crate::gen::{days_from_civil, AnyResult, ROOT_PATH};

/// AC-9 thresholds.
pub const P95_LIMIT_MS: f64 = 150.0;
pub const RSS_LIMIT_BYTES: u64 = 300 * 1024 * 1024;

pub struct BenchQuery {
    pub class: QueryClass,
    pub label: String,
    pub query: SearchQuery,
    /// Name of a planted file (`gen::CANARIES`) this query should find.
    pub canary: Option<&'static str>,
}

/// Query classes of AC-9, in report order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryClass {
    Single,
    Multi,
    Path,
    Accent,
    Typo,
    Short,
    FilterOnly,
    TextFilter,
    Broad,
    NoHit,
}

impl QueryClass {
    pub const ALL: [QueryClass; 10] = [
        QueryClass::Single,
        QueryClass::Multi,
        QueryClass::Path,
        QueryClass::Accent,
        QueryClass::Typo,
        QueryClass::Short,
        QueryClass::FilterOnly,
        QueryClass::TextFilter,
        QueryClass::Broad,
        QueryClass::NoHit,
    ];

    /// Name in the report.
    pub fn name(self) -> &'static str {
        match self {
            QueryClass::Single => "single",
            QueryClass::Multi => "multi",
            QueryClass::Path => "path",
            QueryClass::Accent => "accent",
            QueryClass::Typo => "typo",
            QueryClass::Short => "short",
            QueryClass::FilterOnly => "filter_only",
            QueryClass::TextFilter => "text_filter",
            QueryClass::Broad => "broad",
            QueryClass::NoHit => "no_hit",
        }
    }
}

const NOTARIA: &str = "Notaría Pérez escritura 2019.pdf";
const PENALARA: &str = "Contrato de alquiler Calle Peñalara 14.pdf";
const INFORME: &str = "Informe pericial Ibáñez definitivo revisado por la gestoría.docx";
const BODA: &str = "Boda Íñigo y Begoña ceremonia completa.mp4";

fn year(y: i32) -> i64 {
    days_from_civil(y, 1, 1) * 86_400
}

/// The fixed query set: every class of AC-9's "each query" that behaves differently in the
/// engine (selective, multi-term, path-only, accent and typo variants, 1–2 characters,
/// filters alone and with text, huge hit counts, nothing found).
pub fn query_set() -> Vec<BenchQuery> {
    let mut out = Vec::new();
    let mut add = |class, text: &str, filters: SearchFilters, canary| {
        let mut label = text.to_string();
        let f = serde_json::to_value(&filters).unwrap_or_default();
        let defaults = serde_json::to_value(SearchFilters::default()).unwrap_or_default();
        if f != defaults {
            let mut parts = Vec::new();
            if let (Some(obj), Some(def)) = (f.as_object(), defaults.as_object()) {
                for (k, v) in obj {
                    if def.get(k) != Some(v) {
                        parts.push(format!("{k}={v}"));
                    }
                }
            }
            label = format!("{label} [{}]", parts.join(" "));
        }
        out.push(BenchQuery {
            class,
            label,
            query: SearchQuery {
                text: text.to_string(),
                filters,
                limit: 100,
                offset: 0,
            },
            canary,
        });
    };
    let none = SearchFilters::default;
    let kinds = |k: &[FileKind]| SearchFilters {
        kinds: k.to_vec(),
        ..SearchFilters::default()
    };
    for t in [
        "iberdrola",
        "nómina",
        "testamento",
        "hipoteca",
        "graduación",
        "whatsapp",
    ] {
        add(QueryClass::Single, t, none(), None);
    }
    add(QueryClass::Multi, "factura iberdrola 2015", none(), None);
    add(QueryClass::Multi, "boda begoña", none(), Some(BODA));
    add(
        QueryClass::Multi,
        "contrato alquiler peñalara",
        none(),
        Some(PENALARA),
    );
    add(QueryClass::Multi, "declaración renta 2012", none(), None);
    add(QueryClass::Multi, "viaje cádiz 2008", none(), None);
    add(
        QueryClass::Multi,
        "notaría pérez 2019",
        none(),
        Some(NOTARIA),
    );
    for t in [
        "gestoria/ano",
        "expedientes anexos",
        "whatsapp images",
        "fotos del movil",
        "dcim 100apple",
        "sin clasificar revisar",
    ] {
        add(QueryClass::Path, t, none(), None);
    }
    add(QueryClass::Accent, "notaria", none(), Some(NOTARIA));
    add(QueryClass::Accent, "NOTARÍA", none(), Some(NOTARIA));
    add(QueryClass::Accent, "penalara", none(), Some(PENALARA));
    add(QueryClass::Accent, "inigo begona", none(), Some(BODA));
    add(QueryClass::Accent, "ibanez pericial", none(), Some(INFORME));
    add(QueryClass::Accent, "cadiz", none(), None);
    add(QueryClass::Typo, "notaira", none(), Some(NOTARIA));
    add(QueryClass::Typo, "escirtura", none(), Some(NOTARIA));
    add(QueryClass::Typo, "peñalra", none(), Some(PENALARA));
    add(QueryClass::Typo, "pericail", none(), Some(INFORME));
    add(QueryClass::Typo, "iberdorla", none(), None);
    add(QueryClass::Typo, "testamneto", none(), None);
    for t in ["a", "ñ", "jp", "19", "wa", "de"] {
        add(QueryClass::Short, t, none(), None);
    }
    let gestoria = format!("{ROOT_PATH}/Gestoría");
    let movil = format!("{ROOT_PATH}/Fotos del móvil");
    add(QueryClass::FilterOnly, "", kinds(&[FileKind::Video]), None);
    add(QueryClass::FilterOnly, "", kinds(&[FileKind::Folder]), None);
    add(
        QueryClass::FilterOnly,
        "",
        SearchFilters {
            size_min: Some(4 << 30),
            ..none()
        },
        None,
    );
    add(
        QueryClass::FilterOnly,
        "",
        SearchFilters {
            modified_from: Some(year(2010)),
            modified_to: Some(year(2011)),
            ..none()
        },
        None,
    );
    add(
        QueryClass::FilterOnly,
        "",
        SearchFilters {
            kinds: vec![FileKind::Document],
            folder: Some(gestoria.clone()),
            ..none()
        },
        None,
    );
    add(
        QueryClass::FilterOnly,
        "",
        SearchFilters {
            size_max: Some(1024),
            ..none()
        },
        None,
    );
    add(
        QueryClass::TextFilter,
        "factura",
        kinds(&[FileKind::Document]),
        None,
    );
    add(
        QueryClass::TextFilter,
        "boda",
        kinds(&[FileKind::Video]),
        Some(BODA),
    );
    add(
        QueryClass::TextFilter,
        "img",
        SearchFilters {
            modified_from: Some(year(2019)),
            modified_to: Some(year(2020)),
            ..none()
        },
        None,
    );
    add(
        QueryClass::TextFilter,
        "notaria",
        SearchFilters {
            size_min: Some(1024),
            size_max: Some(10 << 20),
            ..none()
        },
        Some(NOTARIA),
    );
    add(
        QueryClass::TextFilter,
        "img",
        SearchFilters {
            folder: Some(movil),
            ..none()
        },
        None,
    );
    add(
        QueryClass::TextFilter,
        "escritura",
        SearchFilters {
            folder: Some(gestoria),
            ..none()
        },
        Some(NOTARIA),
    );
    add(
        QueryClass::TextFilter,
        "copia",
        kinds(&[FileKind::Folder]),
        None,
    );
    for t in ["img", "jpg", "201", "copia", "pdf", "foto"] {
        add(QueryClass::Broad, t, none(), None);
    }
    for t in ["zzqxj", "kwyjibo", "xilófono marciano", "qqqq"] {
        add(QueryClass::NoHit, t, none(), None);
    }
    out
}

/// Nearest-rank percentile of sorted samples (0 when empty).
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Latency {
    pub samples: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

impl Latency {
    fn of(samples: &[f64]) -> Latency {
        let mut s = samples.to_vec();
        s.sort_by(f64::total_cmp);
        Latency {
            samples: s.len(),
            p50_ms: percentile(&s, 0.50),
            p95_ms: percentile(&s, 0.95),
            max_ms: s.last().copied().unwrap_or(0.0),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassReport {
    pub class: String,
    pub queries: usize,
    pub warm: Latency,
    pub cold: Latency,
    /// Measured responses cut short by the search deadline.
    pub deadline_truncated: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryReport {
    pub class: String,
    pub query: String,
    pub total: u32,
    pub truncated_by: Option<TruncatedBy>,
    pub warm_p50_ms: f64,
    pub warm_max_ms: f64,
    pub cold_ms: f64,
    pub canary: Option<String>,
    /// 1-based rank of the canary in the whole ranked list; `None` when not returned.
    pub canary_rank: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchReport {
    pub db: String,
    pub db_bytes: u64,
    pub files: u64,
    pub dirs: u64,
    pub runs_per_query: u32,
    /// First `Vfs::open` of the run (includes the integrity check).
    pub open_ms: f64,
    pub classes: Vec<ClassReport>,
    pub overall: ClassReport,
    pub queries: Vec<QueryReport>,
    pub peak_rss_bytes: Option<u64>,
    /// What the cold pass could not drop.
    pub cold_note: String,
    pub p95_limit_ms: f64,
    pub rss_limit_bytes: u64,
    pub pass: bool,
    pub failures: Vec<String>,
}

/// Peak resident set size of this process.
#[cfg(unix)]
pub fn peak_rss_bytes() -> Option<u64> {
    // SAFETY: getrusage only writes into the zeroed struct we own.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } != 0 {
        return None;
    }
    let v = ru.ru_maxrss as u64;
    // ru_maxrss is bytes on macOS, kilobytes on Linux.
    Some(if cfg!(target_os = "macos") {
        v
    } else {
        v * 1024
    })
}

/// Peak working set of this process.
#[cfg(windows)]
pub fn peak_rss_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: the counters struct is ours and its size is passed alongside it.
    let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, cb) };
    (ok != 0).then_some(c.PeakWorkingSetSize as u64)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Order of the measured warm runs: `runs` rounds over the whole query set, so no query
/// runs twice in a row and none is served by the search's cache of the last ranked list.
pub fn warm_order(queries: usize, runs: u32) -> Vec<usize> {
    (0..runs).flat_map(|_| 0..queries).collect()
}

/// 1-based rank of the file named `canary` in the whole ranked list of `query` (every page,
/// up to the hit cap); `None` when the search does not return it.
/// Pages of `page_size` hits are requested.
pub fn canary_rank(
    vfs: &Vfs,
    query: &SearchQuery,
    canary: &str,
    page_size: u32,
) -> vfs_core::Result<Option<usize>> {
    let mut page = SearchQuery {
        limit: page_size,
        offset: 0,
        ..query.clone()
    };
    loop {
        let r = vfs.search(&page)?;
        if let Some(p) = r.hits.iter().position(|h| !h.is_dir && h.name == canary) {
            return Ok(Some(page.offset as usize + p + 1));
        }
        page.offset += page_size;
        if r.hits.is_empty() || page.offset >= r.total {
            return Ok(None);
        }
    }
}

/// Page size used to walk a ranked list (the search's maximum).
const PAGE: u32 = 500;

pub fn run(db: &Path, runs: u32) -> AnyResult<BenchReport> {
    if !db.is_file() {
        return Err(format!(
            "{} does not exist (create it with `vfs gen-db`)",
            db.display()
        )
        .into());
    }
    let queries = query_set();
    let t = Instant::now();
    let vfs = Vfs::open(db)?;
    let open_ms = ms(t);
    let stats = vfs.stats()?;
    vfs.search(&queries[0].query)?;
    drop(vfs);

    // Cold: a fresh `Vfs` (new connections, empty SQLite page caches) for each query.
    let mut cold = Vec::with_capacity(queries.len());
    for q in &queries {
        let vfs = Vfs::open(db)?;
        let t = Instant::now();
        vfs.search(&q.query)?;
        cold.push(ms(t));
    }

    // Warm: one `Vfs`; every query once to warm the page caches, then `runs` rounds over the
    // whole set. Consecutive searches always differ, so no measured run is served by the
    // search's cache of the last ranked list (kept for paging).
    let vfs = Vfs::open(db)?;
    let firsts = queries
        .iter()
        .map(|q| vfs.search(&q.query))
        .collect::<vfs_core::Result<Vec<_>>>()?;
    let mut warm: Vec<Vec<f64>> = vec![Vec::with_capacity(runs as usize); queries.len()];
    let mut deadline = vec![0usize; queries.len()];
    for i in warm_order(queries.len(), runs) {
        let t = Instant::now();
        let r = vfs.search(&queries[i].query)?;
        warm[i].push(ms(t));
        deadline[i] += usize::from(r.truncated_by == Some(TruncatedBy::Deadline));
    }
    let mut reports = Vec::with_capacity(queries.len());
    for (i, (q, first)) in queries.iter().zip(&firsts).enumerate() {
        let samples = &warm[i];
        let canary_rank = match q.canary {
            Some(c) => canary_rank(&vfs, &q.query, c, PAGE)?,
            None => None,
        };
        reports.push(QueryReport {
            class: q.class.name().to_string(),
            query: q.label.clone(),
            total: first.total,
            truncated_by: first.truncated_by,
            warm_p50_ms: Latency::of(samples).p50_ms,
            warm_max_ms: Latency::of(samples).max_ms,
            cold_ms: cold[i],
            canary: q.canary.map(str::to_string),
            canary_rank,
        });
    }

    let class_report = |name: &str, pick: &dyn Fn(usize) -> bool| {
        let idx: Vec<usize> = (0..queries.len()).filter(|&i| pick(i)).collect();
        let w: Vec<f64> = idx.iter().flat_map(|&i| warm[i].iter().copied()).collect();
        let c: Vec<f64> = idx.iter().map(|&i| cold[i]).collect();
        ClassReport {
            class: name.to_string(),
            queries: idx.len(),
            warm: Latency::of(&w),
            cold: Latency::of(&c),
            deadline_truncated: idx.iter().map(|&i| deadline[i]).sum(),
        }
    };
    let classes: Vec<ClassReport> = QueryClass::ALL
        .iter()
        .map(|c| class_report(c.name(), &|i| queries[i].class == *c))
        .collect();
    let overall = class_report("overall", &|_| true);
    let peak = peak_rss_bytes();

    let mut failures = Vec::new();
    for c in classes.iter().chain(std::iter::once(&overall)) {
        if c.warm.p95_ms >= P95_LIMIT_MS {
            failures.push(format!(
                "{}: warm p95 {:.1} ms >= {P95_LIMIT_MS} ms",
                c.class, c.warm.p95_ms
            ));
        }
    }
    match peak {
        Some(b) if b >= RSS_LIMIT_BYTES => failures.push(format!(
            "peak RSS {:.0} MB >= {} MB",
            mb(b),
            RSS_LIMIT_BYTES >> 20
        )),
        None => failures.push("peak RSS could not be measured".to_string()),
        _ => {}
    }
    Ok(BenchReport {
        db: db.display().to_string(),
        db_bytes: stats.db_bytes,
        files: stats.files,
        dirs: stats.dirs,
        runs_per_query: runs,
        open_ms,
        classes,
        overall,
        queries: reports,
        peak_rss_bytes: peak,
        cold_note: "SQLite page caches dropped by reopening per query; the OS file cache is \
                    not dropped (needs root), and Vfs::open's integrity check reads the file"
            .to_string(),
        p95_limit_ms: P95_LIMIT_MS,
        rss_limit_bytes: RSS_LIMIT_BYTES,
        pass: failures.is_empty(),
        failures,
    })
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// Human-readable report.
pub fn print_text(r: &BenchReport) {
    println!(
        "db {} — {} files, {} dirs, {:.1} MB on disk; first open {:.0} ms; {} runs/query",
        r.db,
        r.files,
        r.dirs,
        mb(r.db_bytes),
        r.open_ms,
        r.runs_per_query
    );
    println!(
        "{:<12} {:>3} | {:>8} {:>8} {:>8} | {:>8} {:>8} {:>8} | {:>5}",
        "class", "n", "warm p50", "p95", "max", "cold p50", "p95", "max", "dl-cut"
    );
    for c in r.classes.iter().chain(std::iter::once(&r.overall)) {
        println!(
            "{:<12} {:>3} | {:>8.2} {:>8.2} {:>8.2} | {:>8.2} {:>8.2} {:>8.2} | {:>5}",
            c.class,
            c.queries,
            c.warm.p50_ms,
            c.warm.p95_ms,
            c.warm.max_ms,
            c.cold.p50_ms,
            c.cold.p95_ms,
            c.cold.max_ms,
            c.deadline_truncated
        );
    }
    for q in r.queries.iter().filter(|q| q.canary.is_some()) {
        let rank = q
            .canary_rank
            .map_or("missing".to_string(), |n| format!("#{n}"));
        println!("canary {:<40} {rank}", q.query);
    }
    println!("cold pass: {}", r.cold_note);
    let rss = r
        .peak_rss_bytes
        .map_or("unknown".to_string(), |b| format!("{:.1} MB", mb(b)));
    println!("peak RSS: {rss}");
    if r.pass {
        println!(
            "AC-9: PASS (warm p95 < {} ms in every class, overall {:.2} ms; peak RSS {rss} < {} MB)",
            r.p95_limit_ms,
            r.overall.warm.p95_ms,
            r.rss_limit_bytes >> 20
        );
    } else {
        println!("AC-9: FAIL ({})", r.failures.join("; "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-1 AC-9: the query set covers every class the latency budget is measured on, each
    /// with several queries, and every canary query names a planted file.
    #[test]
    fn query_set_covers_classes() {
        let qs = query_set();
        for class in QueryClass::ALL {
            let n = qs.iter().filter(|q| q.class == class).count();
            assert!(n >= 4, "class {} has {n} queries", class.name());
        }
        let planted: Vec<&str> = crate::gen::CANARIES.iter().map(|c| c.1).collect();
        for q in &qs {
            if let Some(c) = q.canary {
                assert!(planted.contains(&c), "{c} is not planted");
            }
        }
        assert!(qs.iter().filter(|q| q.class == QueryClass::Short).all(|q| q
            .query
            .text
            .chars()
            .count()
            <= 2));
        assert!(qs
            .iter()
            .filter(|q| q.class == QueryClass::FilterOnly)
            .all(|q| q.query.text.is_empty()));
    }

    /// VFS-1 AC-9: warm runs go round-robin, so consecutive measured searches always differ
    /// (a repeated query would be served from the cached ranked list, not measured).
    #[test]
    fn warm_runs_never_repeat_a_query_back_to_back() {
        let n = query_set().len();
        let order = warm_order(n, 5);
        assert_eq!(order.len(), n * 5);
        assert!(order.windows(2).all(|w| w[0] != w[1]));
        for i in 0..n {
            assert_eq!(order.iter().filter(|&&j| j == i).count(), 5);
        }
    }

    /// VFS-1 AC-9, AC-10: a canary's rank is its position in the whole ranked list, found
    /// across pages; a file the search does not return has no rank.
    #[test]
    fn canary_rank_walks_every_page() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        crate::gen::gen_db(&db, 3_000, 7, false).unwrap();
        let vfs = Vfs::open(&db).unwrap();
        let (_, name) = crate::gen::CANARIES[0];
        let q = |text: &str| SearchQuery {
            text: text.into(),
            filters: SearchFilters::default(),
            limit: 1,
            offset: 0,
        };
        let page = 10;
        assert_eq!(
            canary_rank(&vfs, &q("notaría pérez 2019"), name, page).unwrap(),
            Some(1)
        );
        let all = vfs
            .search(&SearchQuery {
                limit: 500,
                ..q("pdf")
            })
            .unwrap();
        let expected = all.hits.iter().position(|h| h.name == name).map(|p| p + 1);
        assert!(expected.is_some_and(|r| r > page as usize), "{expected:?}");
        assert_eq!(canary_rank(&vfs, &q("pdf"), name, page).unwrap(), expected);
        assert_eq!(canary_rank(&vfs, &q("zzqxj"), name, page).unwrap(), None);
    }

    /// VFS-1 AC-9: p95 is the nearest-rank percentile.
    #[test]
    fn nearest_rank_percentiles() {
        let s: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&s, 0.95), 95.0);
        assert_eq!(percentile(&s, 0.50), 50.0);
        assert_eq!(percentile(&[7.0], 0.95), 7.0);
        assert_eq!(percentile(&[], 0.95), 0.0);
        let l = Latency::of(&[3.0, 1.0, 2.0]);
        assert_eq!((l.p50_ms, l.max_ms, l.samples), (2.0, 3.0, 3));
    }

    /// VFS-1 AC-9: peak RSS is measurable on this platform and plausible.
    #[test]
    fn peak_rss_is_measured() {
        let b = peak_rss_bytes().unwrap();
        assert!(b > 1 << 20 && b < 64 << 30, "{b}");
    }
}
