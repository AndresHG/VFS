//! When the background content indexer may read its next document (VFS-27 R6, R7). Pure
//! policy over a clock and probe readings the caller supplies, so it is tested with a fake
//! clock and fake probes; the app feeds it `vfs_core::probes` and calls it between documents.
//!
//! The pace follows measured headroom (owner decision T2): documents are read back to back at
//! background priority while the disk and the processor have room, and the run backs off while
//! either is busy. Recent input only adds a short gap.

use std::time::{Duration, Instant};

use vfs_core::probes::{BackOff, CpuSample, Step};

/// The pace numbers (VFS-27 D2, R6, R7).
#[derive(Debug, Clone)]
pub struct PaceConfig {
    /// Input this recent means the PC is in use...
    pub in_use_within: Duration,
    /// ...and then each document is followed by a gap as long as it took, at most this.
    pub in_use_gap_max: Duration,
    /// Share of all processors used by other programs at or above which a sample is busy.
    pub cpu_busy_at: f64,
    /// How long to wait before asking again while the name indexer is busy.
    pub name_wait: Duration,
    /// Below this much free space on the app-data drive, content indexing pauses...
    pub low_space_pause: u64,
    /// ...until free space is above this, or the user resumes.
    pub low_space_resume: u64,
    /// How long to wait before asking again while paused for low space.
    pub low_space_wait: Duration,
}

impl Default for PaceConfig {
    fn default() -> Self {
        PaceConfig {
            in_use_within: Duration::from_secs(60),
            in_use_gap_max: Duration::from_secs(1),
            cpu_busy_at: 0.7,
            name_wait: Duration::from_secs(1),
            low_space_pause: 1 << 30,
            low_space_resume: 2 << 30,
            low_space_wait: Duration::from_secs(30),
        }
    }
}

/// Why the next document waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitReason {
    /// A name walk, or a name pass over many folders, runs (R7 rule 1).
    NameIndexing,
    /// The disk being read is busy (R7 rule 2; where there is a disk probe).
    DiskBusy,
    /// Other programs keep the processor busy (R7 rule 3).
    CpuBusy,
    /// The short gap after a document while the PC is in use (R7 rule 4); shown as progress.
    InUse,
    /// Free space on the app-data drive is low (R6): a pause, shown with its reason.
    LowSpace,
}

/// The decision before a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Read,
    /// Ask again after this long.
    Wait(Duration, WaitReason),
}

/// What the probes say right now.
pub struct Readings<'a> {
    /// A name walk, or a name pass over more than `incremental_max_dirs` folders, is running.
    pub name_busy: bool,
    /// Time since the last keyboard or mouse input; `None` when the probe failed.
    pub idle: Option<Duration>,
    /// Free bytes on the app-data drive; `None` when unknown (no pause on unknown).
    pub free_bytes: Option<u64>,
    /// One disk-load sample (a queue length), or `None`; always `None` where there is no
    /// probe, so the busy-disk rule never fires there.
    pub disk: &'a mut dyn FnMut() -> Option<f64>,
    /// The machine's processor use since the previous sample, or `None` when unknown (then
    /// the processor rule never fires).
    pub cpu: &'a mut dyn FnMut() -> Option<CpuSample>,
}

/// The pace state of one run.
pub struct Pace {
    cfg: PaceConfig,
    disk: BackOff,
    cpu: BackOff,
    /// When the last document ended, and how long it took.
    last: Option<(Instant, Duration)>,
    /// Worker time since the last processor sample, taken out of the machine's share.
    worked: Duration,
    low_space: bool,
}

impl Pace {
    pub fn new(cfg: PaceConfig) -> Pace {
        let cpu = BackOff::with_busy_at(cfg.cpu_busy_at);
        Pace {
            cfg,
            disk: BackOff::with_defaults(),
            cpu,
            last: None,
            worked: Duration::ZERO,
            low_space: false,
        }
    }

    /// The rules of R6 and R7, in order: low space, name indexing, a busy disk, a busy
    /// processor, the gap while the PC is in use.
    pub fn decide(&mut self, now: Instant, r: Readings<'_>) -> Decision {
        if let Some(free) = r.free_bytes {
            if free < self.cfg.low_space_pause {
                self.low_space = true;
            } else if free > self.cfg.low_space_resume {
                self.low_space = false;
            }
        }
        if self.low_space {
            return Decision::Wait(self.cfg.low_space_wait, WaitReason::LowSpace);
        }
        if r.name_busy {
            return Decision::Wait(self.cfg.name_wait, WaitReason::NameIndexing);
        }
        if let Step::Wait(d) = self.disk.step(now, r.disk) {
            return Decision::Wait(d, WaitReason::DiskBusy);
        }
        // Other programs' share: the machine's, less the most the worker itself can explain
        // (one processor for the time it worked).
        let worked = &mut self.worked;
        let cpu = r.cpu;
        let mut others = || {
            let s = cpu()?;
            let mine = std::mem::take(worked).min(s.over).as_secs_f64()
                / (s.over.as_secs_f64() * f64::from(s.cores.max(1)));
            Some((s.busy - mine).max(0.0))
        };
        if let Step::Wait(d) = self.cpu.step(now, &mut others) {
            return Decision::Wait(d, WaitReason::CpuBusy);
        }
        let in_use = r.idle.is_none_or(|i| i <= self.cfg.in_use_within);
        if let (true, Some((ended, took))) = (in_use, self.last) {
            let gap = took.min(self.cfg.in_use_gap_max);
            let since = now.saturating_duration_since(ended);
            if since < gap {
                return Decision::Wait(gap - since, WaitReason::InUse);
            }
        }
        Decision::Read
    }

    /// A document ended at `now` after `took` (the in-use gap follows from it).
    pub fn finished(&mut self, now: Instant, took: Duration) {
        self.last = Some((now, took));
        self.worked += took;
    }

    /// The user resumed: a low-space pause ends until free space drops below the pause
    /// threshold again (R6).
    pub fn user_resumed(&mut self) {
        self.low_space = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vfs_core::probes::BackOff;

    const GB: u64 = 1 << 30;
    const LONG_AGO: Option<Duration> = Some(Duration::from_secs(600));

    /// One decision with no disk or processor probe.
    fn decide(
        p: &mut Pace,
        now: Instant,
        name_busy: bool,
        idle: Option<Duration>,
        free: Option<u64>,
    ) -> Decision {
        let mut no_disk = || None;
        let mut no_cpu = || None;
        p.decide(
            now,
            Readings {
                name_busy,
                idle,
                free_bytes: free,
                disk: &mut no_disk,
                cpu: &mut no_cpu,
            },
        )
    }

    /// A fake machine on a fake clock: documents take `doc` each, other programs use
    /// `others` of its `cores` processors, and its disk reports `queue` (`None`: no probe).
    struct Machine {
        doc: Duration,
        others: f64,
        cores: u32,
        queue: Option<f64>,
        idle: Option<Duration>,
    }

    /// The pace's run on `m` for `span`: the documents read, and the waits by reason.
    fn run(m: &Machine, span: Duration) -> (usize, Vec<WaitReason>) {
        let mut pace = Pace::new(PaceConfig::default());
        let start = Instant::now();
        let mut now = start;
        let mut reads = 0;
        let mut waits = Vec::new();
        // The processor counters: last sample time, and worker time since then.
        let mut sampled = start;
        let mut worker = Duration::ZERO;
        while now - start < span {
            let mut disk = || m.queue;
            let mut cpu = || {
                let over = now - sampled;
                if over.is_zero() {
                    return None;
                }
                let mine = worker.as_secs_f64() / over.as_secs_f64();
                let s = CpuSample {
                    busy: ((m.others * f64::from(m.cores) + mine) / f64::from(m.cores)).min(1.0),
                    cores: m.cores,
                    over,
                };
                sampled = now;
                worker = Duration::ZERO;
                Some(s)
            };
            let d = pace.decide(
                now,
                Readings {
                    name_busy: false,
                    idle: m.idle,
                    free_bytes: Some(100 * GB),
                    disk: &mut disk,
                    cpu: &mut cpu,
                },
            );
            match d {
                Decision::Read => {
                    reads += 1;
                    now += m.doc;
                    worker += m.doc;
                    pace.finished(now, m.doc);
                }
                Decision::Wait(w, reason) => {
                    waits.push(reason);
                    now += w;
                }
            }
        }
        (reads, waits)
    }

    const MINUTE: Duration = Duration::from_secs(60);

    /// A fast machine, lightly used: an SSD (no busy disk), 8 processors with 5 % used by
    /// other programs, 100 ms documents.
    fn fast(idle: Option<Duration>) -> Machine {
        Machine {
            doc: Duration::from_millis(100),
            others: 0.05,
            cores: 8,
            queue: None,
            idle,
        }
    }

    /// VFS-27 AC-7: while a name walk runs, no document is read.
    #[test]
    fn name_indexing_holds_content_back() {
        let cfg = PaceConfig::default();
        let mut p = Pace::new(cfg.clone());
        let now = Instant::now();
        assert_eq!(
            decide(&mut p, now, true, LONG_AGO, None),
            Decision::Wait(cfg.name_wait, WaitReason::NameIndexing)
        );
    }

    /// VFS-27 AC-7: on a fast machine in light use (input 10 s ago, or an idle probe that
    /// fails) documents come at least every other document's time: 250 or more a minute with
    /// 100 ms documents, never a few a minute. With no input for over 60 s they come back to
    /// back. Nothing waits for the disk or the processor.
    #[test]
    fn fast_machine_reads_at_full_pace() {
        for idle in [Some(Duration::from_secs(10)), None] {
            let (reads, waits) = run(&fast(idle), MINUTE);
            assert!(reads >= 250, "{idle:?}: {reads} a minute");
            assert!(
                waits.iter().all(|w| *w == WaitReason::InUse),
                "{idle:?}: {waits:?}"
            );
        }
        let (reads, waits) = run(&fast(Some(Duration::from_secs(61))), MINUTE);
        assert!(reads >= 590, "back to back: {reads} a minute");
        assert!(waits.is_empty(), "{waits:?}");
    }

    /// VFS-27 AC-7: the worker's own work never counts as a busy processor, even on two
    /// processors where one document keeps half of the machine busy.
    #[test]
    fn own_work_is_not_a_busy_processor() {
        let m = Machine {
            doc: Duration::from_millis(400),
            others: 0.1,
            cores: 2,
            queue: None,
            idle: LONG_AGO,
        };
        let (reads, waits) = run(&m, MINUTE);
        assert!(waits.is_empty(), "{waits:?}");
        assert_eq!(reads, 150);
    }

    /// VFS-27 AC-7: on the target PC with its disk kept busy (an HDD at queue depth while a
    /// sync client runs) and slow documents, content yields most of the time but still reads
    /// a document within `max_backoff` + `min_work` of each back-off.
    #[test]
    fn busy_hdd_yields_but_never_stops() {
        let slow = |queue| Machine {
            doc: Duration::from_secs(1),
            others: 0.1,
            cores: 2,
            queue,
            idle: Some(Duration::from_secs(5)),
        };
        let (calm, _) = run(&slow(Some(0.5)), 5 * MINUTE);
        let (busy, waits) = run(&slow(Some(3.0)), 5 * MINUTE);
        assert!(waits.contains(&WaitReason::DiskBusy), "{waits:?}");
        assert!(busy * 2 < calm, "busy {busy} against calm {calm} in 5 min");
        let rule = BackOff::with_defaults();
        let cycle = rule.max_backoff() + rule.min_work();
        let at_least = (5 * MINUTE).as_secs() / (cycle.as_secs() + 2);
        assert!(busy as u64 >= at_least, "{busy} < {at_least}");
    }

    /// VFS-27 AC-7: while other programs use 90 % of the processors, content yields with the
    /// same guarantee: it still reads within `max_backoff` + `min_work` of each back-off.
    #[test]
    fn busy_processor_yields_but_never_stops() {
        let m = |others| Machine {
            doc: Duration::from_millis(200),
            others,
            cores: 4,
            queue: None,
            idle: LONG_AGO,
        };
        let (calm, _) = run(&m(0.2), MINUTE);
        let (busy, waits) = run(&m(0.9), MINUTE);
        assert!(waits.contains(&WaitReason::CpuBusy), "{waits:?}");
        assert!(!waits.contains(&WaitReason::DiskBusy), "{waits:?}");
        assert!(busy * 2 < calm, "busy {busy} against calm {calm}");
        assert!(busy >= 5, "never stops: {busy} a minute");
    }

    /// VFS-27 AC-7: with 3 of the last 5 disk samples busy no document is read; when the disk
    /// stays busy past `max_backoff`, a document is still read within `max_backoff` +
    /// `min_work`.
    #[test]
    fn busy_disk_slows_but_never_stops() {
        let mut p = Pace::new(PaceConfig::default());
        let start = Instant::now();
        let mut busy = || Some(10.0);
        let mut no_cpu = || None;
        let mut now = start;
        let mut first_read = None;
        let mut waited_busy = false;
        while now - start < Duration::from_secs(30) {
            let d = p.decide(
                now,
                Readings {
                    name_busy: false,
                    idle: LONG_AGO,
                    free_bytes: None,
                    disk: &mut busy,
                    cpu: &mut no_cpu,
                },
            );
            match d {
                Decision::Read if now - start >= Duration::from_secs(3) => {
                    first_read = Some(now - start);
                    break;
                }
                Decision::Read => now += Duration::from_secs(1),
                Decision::Wait(w, reason) => {
                    assert_eq!(reason, WaitReason::DiskBusy);
                    waited_busy = true;
                    now += w;
                }
            }
        }
        assert!(waited_busy, "3 busy samples of 5 must hold documents back");
        let first = first_read.expect("a document is read despite the busy disk");
        let rule = BackOff::with_defaults();
        // Three samples, one per second, make the window busy; then the back-off bound.
        let bound = Duration::from_secs(3) + rule.max_backoff() + rule.min_work();
        assert!(first <= bound, "{first:?} > {bound:?}");
    }

    /// VFS-27 AC-7: while the PC is in use the gap after a document is as long as the
    /// document took, at most `in_use_gap_max`: a 5 s document is followed by 1 s.
    #[test]
    fn in_use_gap_is_short() {
        let cfg = PaceConfig::default();
        let mut p = Pace::new(cfg.clone());
        let now = Instant::now();
        p.finished(now, Duration::from_secs(5));
        let idle = Some(Duration::from_secs(10));
        assert_eq!(
            decide(&mut p, now, false, idle, None),
            Decision::Wait(cfg.in_use_gap_max, WaitReason::InUse)
        );
        assert_eq!(
            decide(&mut p, now + cfg.in_use_gap_max, false, idle, None),
            Decision::Read
        );
        assert_eq!(decide(&mut p, now, false, LONG_AGO, None), Decision::Read);
    }

    /// VFS-27 AC-6: at 0.9 GB free, content indexing pauses with the low-space reason; at
    /// 1.5 GB it stays paused; at 2.5 GB it resumes. A user resume ends the pause until space
    /// drops below 1 GB again.
    #[test]
    fn low_space_pause_and_resume() {
        let mut p = Pace::new(PaceConfig::default());
        let idle = Some(Duration::from_secs(600));
        let mut now = Instant::now();
        let mut at = |p: &mut Pace, free: u64| {
            now += Duration::from_secs(60);
            decide(p, now, false, idle, Some(free))
        };
        assert_eq!(at(&mut p, 5 * GB), Decision::Read);
        assert!(matches!(
            at(&mut p, 9 * GB / 10),
            Decision::Wait(_, WaitReason::LowSpace)
        ));
        assert!(matches!(
            at(&mut p, 3 * GB / 2),
            Decision::Wait(_, WaitReason::LowSpace)
        ));
        assert_eq!(at(&mut p, 5 * GB / 2), Decision::Read);

        assert!(matches!(
            at(&mut p, 9 * GB / 10),
            Decision::Wait(_, WaitReason::LowSpace)
        ));
        p.user_resumed();
        assert_eq!(at(&mut p, 3 * GB / 2), Decision::Read);
        assert!(matches!(
            at(&mut p, 9 * GB / 10),
            Decision::Wait(_, WaitReason::LowSpace)
        ));
    }
}
