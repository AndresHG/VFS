//! Indexer scheduling policy (VFS-5): which priority a pass runs at, when a walk backs off
//! because the disk is busy, how long the start-up catch-up waits after boot, and the
//! diagnostic records that make all of it visible on the target PC.
//!
//! Everything here is plain logic over a clock value, load samples and a priority backend,
//! so it is tested without Windows; the real probes live in `platform`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::platform::{self, DiskLoad, ThreadPriority};
use crate::types::ScanSummary;

/// Log target of the diagnostic records. The desktop logger lets info records through only
/// for this target, and these records never carry a file or folder name (VFS-5 R8).
pub const DIAG_TARGET: &str = "vfs::diag";

/// Thresholds of the policy. Every default is an estimate until the measurement named in
/// `specs/vfs-5-windows-tuning/spec.md` (Configuration) replaces it.
#[derive(Clone, Debug)]
pub(crate) struct SchedConfig {
    /// Up to this many pending directories with no root walked in full count as small
    /// incremental work, listed in background mode (VFS-5 R2).
    pub incremental_max_dirs: u64,
    /// Interval between disk-load samples.
    pub sample_every: Duration,
    /// Average disk queue length at or above which a sample is busy. The indexer issues one
    /// synchronous request at a time, so it adds at most 1 itself.
    pub busy_queue: f64,
    /// Samples remembered...
    pub window: usize,
    /// ...and how many of them must be busy for the disk to count as busy.
    pub busy_in_window: usize,
    /// Longest continuous back-off...
    pub max_backoff: Duration,
    /// ...after which the walk works at least this long (20 % duty with the defaults).
    pub min_work: Duration,
    /// The start-up catch-up waits until the system has been up this long (VFS-5 R5).
    pub boot_grace: Duration,
    /// Interval of the per-pass diagnostic record.
    pub diag_every: Duration,
    /// Interval of the memory diagnostic record.
    pub diag_memory_every: Duration,
}

impl Default for SchedConfig {
    fn default() -> Self {
        SchedConfig {
            incremental_max_dirs: 64,
            sample_every: Duration::from_secs(1),
            busy_queue: 2.0,
            window: 5,
            busy_in_window: 3,
            max_backoff: Duration::from_secs(8),
            min_work: Duration::from_secs(2),
            boot_grace: Duration::from_secs(10 * 60),
            diag_every: Duration::from_secs(60),
            diag_memory_every: Duration::from_secs(10 * 60),
        }
    }
}

/// Priority of a pass (VFS-5 R1, R2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// First index, full re-walk or a large batch: walk profile, with back-off.
    Walk,
    /// Small incremental work: each listing in background mode, each directory committed
    /// alone, outside background mode.
    Incremental,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Walk => "walk",
            Mode::Incremental => "incremental",
        }
    }
}

/// The mode for a pass: `Walk` while any root is walked in full or more than
/// `incremental_max_dirs` directories are pending.
pub(crate) fn choose_mode(walking: bool, pending: u64, cfg: &SchedConfig) -> Mode {
    if walking || pending > cfg.incremental_max_dirs {
        Mode::Walk
    } else {
        Mode::Incremental
    }
}

/// How long the start-up catch-up waits so that it starts once the system has been up for
/// `boot_grace`; zero when the uptime is unknown.
pub(crate) fn catchup_delay(uptime: Option<Duration>, boot_grace: Duration) -> Duration {
    uptime.map_or(Duration::ZERO, |u| boot_grace.saturating_sub(u))
}

/// What the walk does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Work,
    /// Pause this long, then ask again.
    Wait(Duration),
}

/// Busy-disk back-off (VFS-5 R4): a window of busy/calm samples, a pause while the window is
/// busy, and a forced work slice after `max_backoff` so progress never stops.
pub(crate) struct BackOff {
    cfg: SchedConfig,
    samples: VecDeque<bool>,
    last_sample: Option<Instant>,
    paused_since: Option<Instant>,
    forced_until: Option<Instant>,
}

impl BackOff {
    pub(crate) fn new(cfg: SchedConfig) -> BackOff {
        BackOff {
            cfg,
            samples: VecDeque::new(),
            last_sample: None,
            paused_since: None,
            forced_until: None,
        }
    }

    /// Decides at `now`, taking a new sample from `load` (a queue length, or `None` when the
    /// probe has nothing) once `sample_every` has passed since the last one.
    pub(crate) fn step(&mut self, now: Instant, load: &mut dyn FnMut() -> Option<f64>) -> Step {
        if self
            .last_sample
            .is_none_or(|t| now.saturating_duration_since(t) >= self.cfg.sample_every)
        {
            self.last_sample = Some(now);
            let busy = load().is_some_and(|q| q >= self.cfg.busy_queue);
            self.samples.push_back(busy);
            while self.samples.len() > self.cfg.window {
                self.samples.pop_front();
            }
        }
        if self.forced_until.is_some_and(|t| now < t) {
            return Step::Work;
        }
        self.forced_until = None;
        if self.samples.iter().filter(|&&b| b).count() < self.cfg.busy_in_window {
            self.paused_since = None;
            return Step::Work;
        }
        let since = *self.paused_since.get_or_insert(now);
        let paused = now.saturating_duration_since(since);
        if paused >= self.cfg.max_backoff {
            self.paused_since = None;
            self.forced_until = Some(now + self.cfg.min_work);
            return Step::Work;
        }
        Step::Wait(self.cfg.sample_every.min(self.cfg.max_backoff - paused))
    }
}

/// Checkpoints run by the indexer connection's WAL hook, timed for the log (VFS-5 R7).
#[derive(Default)]
pub(crate) struct CheckpointStats {
    count: AtomicU64,
    total_us: AtomicU64,
    max_us: AtomicU64,
}

impl CheckpointStats {
    pub(crate) fn record(&self, d: Duration) {
        let us = d.as_micros().min(u128::from(u64::MAX)) as u64;
        self.count.fetch_add(1, Ordering::Relaxed);
        self.total_us.fetch_add(us, Ordering::Relaxed);
        self.max_us.fetch_max(us, Ordering::Relaxed);
    }

    /// (count, total, max) since the last call.
    fn take(&self) -> (u64, Duration, Duration) {
        (
            self.count.swap(0, Ordering::Relaxed),
            Duration::from_micros(self.total_us.swap(0, Ordering::Relaxed)),
            Duration::from_micros(self.max_us.swap(0, Ordering::Relaxed)),
        )
    }
}

/// Replaces `wal_autocheckpoint` on the indexer's connection with the same passive
/// checkpoint at `pages` WAL pages, timed into `stats` for the diagnostic log (VFS-5 R7).
/// Installing a WAL hook turns SQLite's automatic checkpoint off, so this is the only one.
/// A failed checkpoint is logged, never turned into a failed commit.
pub(crate) fn install_checkpoint_hook(
    conn: &mut rusqlite::Connection,
    stats: Arc<CheckpointStats>,
    pages: i64,
) -> rusqlite::Result<()> {
    conn.wal_hook(Some(move |wal: &mut rusqlite::hooks::Wal, n: i32| {
        if i64::from(n) >= pages {
            let start = Instant::now();
            if let Err(e) = wal.checkpoint() {
                log::warn!("checkpoint: {e}");
            }
            stats.record(start.elapsed());
        }
        Ok(())
    }))
}

/// Counters of one diagnostic period.
#[derive(Default, Clone)]
struct Period {
    backoff: Duration,
    commits: u64,
    commit_total: Duration,
    commit_max: Duration,
    checkpoints: u64,
    checkpoint_total: Duration,
    checkpoint_max: Duration,
}

impl Period {
    fn add(&mut self, other: &Period) {
        self.backoff += other.backoff;
        self.commits += other.commits;
        self.commit_total += other.commit_total;
        self.commit_max = self.commit_max.max(other.commit_max);
        self.checkpoints += other.checkpoints;
        self.checkpoint_total += other.checkpoint_total;
        self.checkpoint_max = self.checkpoint_max.max(other.checkpoint_max);
    }
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

fn avg_ms(total: Duration, n: u64) -> u128 {
    if n == 0 {
        0
    } else {
        total.as_millis() / u128::from(n)
    }
}

fn per_min(n: u64, period: Duration) -> u64 {
    let secs = period.as_secs_f64();
    if secs <= 0.0 {
        0
    } else {
        (n as f64 * 60.0 / secs).round() as u64
    }
}

/// Where diagnostic records go: the log under [`DIAG_TARGET`] in the app, a vector in tests.
pub(crate) type DiagSink = Box<dyn FnMut(&str) + Send>;

/// The default sink: an info record under [`DIAG_TARGET`].
pub(crate) fn log_sink() -> DiagSink {
    Box::new(|line| log::info!(target: DIAG_TARGET, "{line}"))
}

/// Builds the diagnostic records (VFS-5 R7). Every value is a count, a duration, a size or a
/// root id: no record names a file or a folder (VFS-5 R8).
pub(crate) struct Diag {
    sink: DiagSink,
    every: Duration,
    memory_every: Duration,
    /// Start of the current period and the pass totals at that moment.
    period_start: Instant,
    period_base: ScanSummary,
    period: Period,
    pass_start: Option<Instant>,
    pass: Period,
    last_memory: Option<Instant>,
    overflows: u64,
}

impl Diag {
    pub(crate) fn new(cfg: &SchedConfig, sink: DiagSink, now: Instant) -> Diag {
        Diag {
            sink,
            every: cfg.diag_every,
            memory_every: cfg.diag_memory_every,
            period_start: now,
            period_base: ScanSummary::default(),
            period: Period::default(),
            pass_start: None,
            pass: Period::default(),
            last_memory: None,
            overflows: 0,
        }
    }

    pub(crate) fn emit(&mut self, line: &str) {
        (self.sink)(line);
    }

    pub(crate) fn backoff(&mut self, d: Duration) {
        self.period.backoff += d;
    }

    pub(crate) fn commit(&mut self, d: Duration) {
        self.period.commits += 1;
        self.period.commit_total += d;
        self.period.commit_max = self.period.commit_max.max(d);
    }

    fn take_checkpoints(&mut self, stats: &CheckpointStats) {
        let (n, total, max) = stats.take();
        self.period.checkpoints += n;
        self.period.checkpoint_total += total;
        self.period.checkpoint_max = self.period.checkpoint_max.max(max);
    }

    /// A watcher buffer overflowed for `root_id`.
    pub(crate) fn overflow(&mut self, root_id: i64) {
        self.overflows += 1;
        let line = format!("watch_overflow root={root_id} total={}", self.overflows);
        self.emit(&line);
    }

    fn fields(mode: Mode, d: &ScanSummary, p: &Period, span: Duration, retries: usize) -> String {
        format!(
            "mode={} period_s={} dirs={} files_added={} files_removed={} dirs_per_min={} \
             files_per_min={} backoff_s={} commits={} commit_ms_avg={} commit_ms_max={} \
             checkpoints={} checkpoint_ms_avg={} checkpoint_ms_max={} retries_pending={} errors={}",
            mode.name(),
            span.as_secs(),
            d.dirs_listed,
            d.files_added,
            d.files_removed,
            per_min(d.dirs_listed, span),
            per_min(d.files_added + d.files_removed, span),
            p.backoff.as_secs(),
            p.commits,
            avg_ms(p.commit_total, p.commits),
            ms(p.commit_max),
            p.checkpoints,
            avg_ms(p.checkpoint_total, p.checkpoints),
            ms(p.checkpoint_max),
            retries,
            d.errors,
        )
    }

    fn delta(now: &ScanSummary, base: &ScanSummary) -> ScanSummary {
        ScanSummary {
            dirs_listed: now.dirs_listed.saturating_sub(base.dirs_listed),
            files_added: now.files_added.saturating_sub(base.files_added),
            files_removed: now.files_removed.saturating_sub(base.files_removed),
            errors: now.errors.saturating_sub(base.errors),
            ..ScanSummary::default()
        }
    }

    /// Called between batches of a pass: writes a `pass` record once `diag_every` has passed.
    pub(crate) fn tick(
        &mut self,
        now: Instant,
        mode: Mode,
        summary: &ScanSummary,
        retries: usize,
        ckpt: &CheckpointStats,
    ) {
        if self.pass_start.is_none() {
            self.pass_start = Some(self.period_start);
        }
        let span = now.saturating_duration_since(self.period_start);
        if span < self.every {
            return;
        }
        self.take_checkpoints(ckpt);
        let d = Self::delta(summary, &self.period_base);
        let line = format!(
            "pass {}",
            Self::fields(mode, &d, &self.period, span, retries)
        );
        self.emit(&line);
        self.pass.add(&self.period);
        self.period = Period::default();
        self.period_base = summary.clone();
        self.period_start = now;
    }

    /// Called when a pass ends: writes a `pass_end` record with the totals of the whole pass
    /// (nothing if no directory was listed), then starts a new period.
    pub(crate) fn pass_end(
        &mut self,
        now: Instant,
        mode: Mode,
        summary: &ScanSummary,
        retries: usize,
        ckpt: &CheckpointStats,
    ) {
        self.take_checkpoints(ckpt);
        self.pass.add(&self.period);
        let start = self.pass_start.take().unwrap_or(self.period_start);
        if summary.dirs_listed > 0 || summary.errors > 0 {
            let span = now.saturating_duration_since(start);
            let line = format!(
                "pass_end {}",
                Self::fields(mode, summary, &self.pass, span, retries)
            );
            self.emit(&line);
        }
        self.pass = Period::default();
        self.period = Period::default();
        self.period_base = ScanSummary::default();
        self.period_start = now;
    }

    /// Whether a `memory` record is due: the first call, then every `diag_memory_every`.
    pub(crate) fn memory_due(&self, now: Instant) -> bool {
        self.last_memory
            .is_none_or(|t| now.saturating_duration_since(t) >= self.memory_every)
    }

    /// Writes a `memory` record: working set, WAL and index size.
    pub(crate) fn memory(&mut self, now: Instant, wal_bytes: Option<u64>, db_bytes: Option<u64>) {
        self.last_memory = Some(now);
        let mb = |b: Option<u64>| b.map_or_else(|| "?".to_owned(), |b| (b >> 20).to_string());
        let line = format!(
            "memory working_set_mb={} wal_mb={} index_mb={}",
            mb(platform::working_set_bytes()),
            mb(wal_bytes),
            mb(db_bytes)
        );
        self.emit(&line);
    }
}

/// Everything the scan loop needs to schedule itself: mode, priority backend, back-off and
/// the diagnostic counters it feeds.
pub(crate) struct Sched {
    pub cfg: SchedConfig,
    pub mode: Mode,
    prio: Box<dyn ThreadPriority>,
    load: Option<Box<dyn DiskLoad>>,
    backoff: BackOff,
    /// Drives holding the roots and the index, for choosing the disk to watch.
    pub drives: Vec<String>,
    pub diag: Diag,
    pub checkpoints: Arc<CheckpointStats>,
}

/// A priority backend that does nothing (synchronous scans of the CLI and tests).
struct NoPriority;

impl ThreadPriority for NoPriority {
    fn walk_profile(&mut self) {}
    fn begin_background(&mut self) {}
    fn end_background(&mut self) {}
}

impl Sched {
    pub(crate) fn new(
        cfg: SchedConfig,
        prio: Box<dyn ThreadPriority>,
        load: Option<Box<dyn DiskLoad>>,
        sink: DiagSink,
    ) -> Sched {
        let now = Instant::now();
        Sched {
            backoff: BackOff::new(cfg.clone()),
            diag: Diag::new(&cfg, sink, now),
            cfg,
            mode: Mode::Walk,
            prio,
            load,
            drives: Vec::new(),
            checkpoints: Arc::default(),
        }
    }

    /// No priority changes, no probe, no records: the synchronous scan of the CLI and tests.
    pub(crate) fn inert() -> Sched {
        Sched::new(
            SchedConfig::default(),
            Box::new(NoPriority),
            None,
            Box::new(|_| {}),
        )
    }

    /// Applies the walk profile, the thread's base state.
    pub(crate) fn walk_profile(&mut self) {
        self.prio.walk_profile();
    }

    pub(crate) fn has_probe(&self) -> bool {
        self.load.is_some()
    }

    /// Runs a directory listing: in background mode for incremental work, else as is.
    pub(crate) fn listing<T>(&mut self, f: impl FnOnce() -> T) -> T {
        if self.mode != Mode::Incremental {
            return f();
        }
        self.prio.begin_background();
        let r = f();
        self.prio.end_background();
        r
    }

    /// The back-off decision for the next listing; always `Work` outside walk mode.
    pub(crate) fn step(&mut self) -> Step {
        if self.mode != Mode::Walk {
            return Step::Work;
        }
        let (load, drives) = (&mut self.load, &self.drives);
        self.backoff.step(Instant::now(), &mut || {
            load.as_mut()
                .and_then(|l| l.sample())
                .and_then(|s| platform::queue_for(&s, drives))
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// VFS-5 AC-8, R7: the hook that replaces `wal_autocheckpoint` checkpoints once the WAL
    /// reaches its page threshold and counts each checkpoint, so the WAL stays bounded.
    #[test]
    fn checkpoint_hook_checkpoints_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let mut conn = crate::db::open_rw(&db).unwrap();
        let stats = Arc::new(CheckpointStats::default());
        let pages = 8;
        install_checkpoint_hook(&mut conn, Arc::clone(&stats), pages).unwrap();
        conn.execute_batch("CREATE TABLE t(b BLOB)").unwrap();
        let page: i64 = conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .unwrap();
        for _ in 0..50 {
            conn.execute("INSERT INTO t VALUES (zeroblob(?1))", [page * 2])
                .unwrap();
        }
        let (count, _, _) = stats.take();
        assert!(count >= 1, "no checkpoint recorded");
        // Without checkpoints the WAL would hold every one of the ~100+ pages written.
        let wal = std::fs::metadata(dir.path().join("index.db-wal"))
            .unwrap()
            .len();
        assert!(
            wal < (page * pages * 4) as u64,
            "WAL grew to {wal} bytes, checkpoints did not reset it"
        );
    }

    fn cfg() -> SchedConfig {
        SchedConfig::default()
    }

    /// VFS-5 AC-1, AC-2: a walk or a large batch uses the walk profile; a few pending
    /// directories with no root walked are incremental work.
    #[test]
    fn mode_follows_walks_and_batch_size() {
        let c = cfg();
        assert_eq!(choose_mode(true, 0, &c), Mode::Walk);
        assert_eq!(choose_mode(true, 1, &c), Mode::Walk);
        assert_eq!(choose_mode(false, 1, &c), Mode::Incremental);
        assert_eq!(
            choose_mode(false, c.incremental_max_dirs, &c),
            Mode::Incremental
        );
        assert_eq!(
            choose_mode(false, c.incremental_max_dirs + 1, &c),
            Mode::Walk
        );
    }

    /// Steps the back-off over `secs` of fake time in 100 ms slices, a listing taking one
    /// slice, with `load` giving the queue length at each sample; returns the time spent
    /// working and waiting.
    fn simulate(
        b: &mut BackOff,
        t0: Instant,
        secs: u64,
        load: impl Fn(Duration) -> Option<f64>,
    ) -> (Duration, Duration) {
        let slice = Duration::from_millis(100);
        let (mut work, mut wait) = (Duration::ZERO, Duration::ZERO);
        let mut t = Duration::ZERO;
        while t < Duration::from_secs(secs) {
            let now = t0 + t;
            match b.step(now, &mut || load(t)) {
                Step::Work => {
                    work += slice;
                    t += slice;
                }
                Step::Wait(d) => {
                    assert!(d > Duration::ZERO);
                    wait += d;
                    t += d;
                }
            }
        }
        (work, wait)
    }

    /// VFS-5 AC-5: a calm disk never waits; a busy window (3 of 5 samples at or above the
    /// queue threshold) pauses the walk; it resumes once the window is calm again; an
    /// unavailable probe never waits.
    #[test]
    fn backoff_pauses_on_busy_window_and_resumes() {
        let c = cfg();
        let t0 = Instant::now();
        let mut b = BackOff::new(c.clone());
        let (_, wait) = simulate(&mut b, t0, 30, |_| Some(c.busy_queue - 0.5));
        assert_eq!(wait, Duration::ZERO);

        let mut b = BackOff::new(c.clone());
        let (_, wait) = simulate(&mut b, t0, 30, |_| None);
        assert_eq!(wait, Duration::ZERO, "no probe: never busy");

        // Busy for the first 5 s, calm afterwards.
        let mut b = BackOff::new(c.clone());
        let busy_until = Duration::from_secs(5);
        let load = |t: Duration| Some(if t < busy_until { c.busy_queue } else { 0.0 });
        let mut first_wait = None;
        let mut last_wait = None;
        let mut t = Duration::ZERO;
        while t < Duration::from_secs(20) {
            match b.step(t0 + t, &mut || load(t)) {
                Step::Work => t += Duration::from_millis(100),
                Step::Wait(d) => {
                    first_wait.get_or_insert(t);
                    last_wait = Some(t);
                    t += d;
                }
            }
        }
        let first = first_wait.expect("a busy window pauses the walk");
        assert!(
            first >= Duration::from_secs(2),
            "one busy sample is not enough"
        );
        assert!(first <= busy_until);
        let last = last_wait.unwrap();
        assert!(
            last < busy_until + c.sample_every * c.window as u32,
            "resumes once the window is calm (last wait at {last:?})"
        );
    }

    /// VFS-5 AC-6: with every sample busy, the walk still works `min_work` after each
    /// `max_backoff` pause: at least 20 % of a minute.
    #[test]
    fn backoff_never_stalls_on_a_permanently_busy_disk() {
        let c = cfg();
        let mut b = BackOff::new(c.clone());
        let (work, wait) = simulate(&mut b, Instant::now(), 60, |_| Some(100.0));
        assert!(wait > Duration::ZERO);
        let duty = work.as_secs_f64() / (work + wait).as_secs_f64();
        let floor = c.min_work.as_secs_f64() / (c.min_work + c.max_backoff).as_secs_f64();
        assert!(duty >= floor * 0.9, "duty {duty:.2} below floor {floor:.2}");
        assert!(duty >= 0.18, "duty {duty:.2}");
    }

    /// VFS-5 AC-7: the catch-up waits until uptime reaches the boot grace; a system up for
    /// longer, or with an unknown uptime, does not wait.
    #[test]
    fn catchup_waits_for_boot_grace() {
        let g = Duration::from_secs(600);
        assert_eq!(
            catchup_delay(Some(Duration::from_secs(120)), g),
            Duration::from_secs(480)
        );
        assert_eq!(
            catchup_delay(Some(Duration::from_secs(3600)), g),
            Duration::ZERO
        );
        assert_eq!(catchup_delay(None, g), Duration::ZERO);
    }

    /// VFS-5 AC-8: a period record and a pass-end record carry every field of R7, and a
    /// memory record carries working set, WAL and index size.
    #[test]
    fn diag_records_carry_every_field() {
        let lines = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let l2 = Arc::clone(&lines);
        let c = cfg();
        let t0 = Instant::now();
        let mut d = Diag::new(&c, Box::new(move |s| l2.lock().unwrap().push(s.into())), t0);
        let ckpt = CheckpointStats::default();
        let mut s = ScanSummary {
            dirs_listed: 120,
            files_added: 3000,
            ..ScanSummary::default()
        };
        d.commit(Duration::from_millis(40));
        d.backoff(Duration::from_secs(5));
        ckpt.record(Duration::from_millis(250));
        d.tick(t0 + Duration::from_secs(10), Mode::Walk, &s, 0, &ckpt);
        assert!(
            lines.lock().unwrap().is_empty(),
            "no record before diag_every"
        );
        d.tick(t0 + c.diag_every, Mode::Walk, &s, 2, &ckpt);
        s.dirs_listed += 30;
        s.errors = 1;
        d.pass_end(t0 + c.diag_every * 2, Mode::Walk, &s, 2, &ckpt);
        d.memory(t0, Some(5 << 20), Some(900 << 20));
        d.overflow(7);
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 4, "{lines:?}");
        for (line, kind) in lines[..2].iter().zip(["pass ", "pass_end "]) {
            assert!(line.starts_with(kind), "{line}");
            for field in [
                "mode=walk",
                "dirs=",
                "files_added=",
                "files_removed=",
                "dirs_per_min=",
                "files_per_min=",
                "backoff_s=5",
                "commits=1",
                "commit_ms_avg=40",
                "commit_ms_max=40",
                "checkpoints=1",
                "checkpoint_ms_max=250",
                "retries_pending=2",
                "errors=",
            ] {
                assert!(line.contains(field), "{field} missing in {line}");
            }
        }
        assert!(lines[0].contains("dirs=120 ") && lines[0].contains("dirs_per_min=120"));
        assert!(lines[1].contains("dirs=150 ") && lines[1].contains("errors=1"));
        assert!(lines[2].starts_with("memory working_set_mb="));
        assert!(lines[2].contains("wal_mb=5") && lines[2].contains("index_mb=900"));
        assert_eq!(lines[3], "watch_overflow root=7 total=1");
    }

    /// Records the priority calls of a fake backend into a shared log.
    pub(crate) struct Recorder(pub Arc<std::sync::Mutex<Vec<String>>>);

    impl ThreadPriority for Recorder {
        fn walk_profile(&mut self) {
            self.0.lock().unwrap().push("walk_profile".into());
        }
        fn begin_background(&mut self) {
            self.0.lock().unwrap().push("bg_begin".into());
        }
        fn end_background(&mut self) {
            self.0.lock().unwrap().push("bg_end".into());
        }
    }

    /// VFS-5 AC-1, AC-2: listings run in background mode only in incremental mode, and walk
    /// mode never enters it.
    #[test]
    fn listing_wraps_background_only_for_incremental_work() {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut s = Sched::new(
            cfg(),
            Box::new(Recorder(Arc::clone(&log))),
            None,
            Box::new(|_| {}),
        );
        s.mode = Mode::Walk;
        let l = Arc::clone(&log);
        s.listing(|| l.lock().unwrap().push("list".into()));
        s.mode = Mode::Incremental;
        let l = Arc::clone(&log);
        s.listing(|| l.lock().unwrap().push("list".into()));
        assert_eq!(*log.lock().unwrap(), ["list", "bg_begin", "list", "bg_end"]);
    }
}
