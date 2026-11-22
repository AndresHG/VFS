//! When the background content indexer may read its next document (VFS-27 R6, R7). Pure
//! policy over a clock and probe readings the caller supplies, so it is tested with a fake
//! clock and fake probes; the app feeds it `vfs_core::probes` and calls it between documents.

use std::time::{Duration, Instant};

use vfs_core::probes::{BackOff, Step};

/// The pace numbers (VFS-27 D2, R6).
#[derive(Debug, Clone)]
pub struct PaceConfig {
    /// Input this recent means the PC is in use...
    pub in_use_within: Duration,
    /// ...and then documents are at least this far apart.
    pub in_use_gap: Duration,
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
            in_use_gap: Duration::from_secs(15),
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
    /// The disk being read is busy (R7 rule 2; Windows only).
    DiskBusy,
    /// The PC is in use, or the idle probe failed (R7 rules 3 and 4).
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
}

/// The pace state of one run.
pub struct Pace {
    cfg: PaceConfig,
    backoff: BackOff,
    last_read: Option<Instant>,
    low_space: bool,
}

impl Pace {
    pub fn new(cfg: PaceConfig) -> Pace {
        Pace {
            cfg,
            backoff: BackOff::with_defaults(),
            last_read: None,
            low_space: false,
        }
    }

    /// The rules of R6 and R7, in order: low space, name indexing, a busy disk, the PC in
    /// use.
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
        if let Step::Wait(d) = self.backoff.step(now, r.disk) {
            return Decision::Wait(d, WaitReason::DiskBusy);
        }
        let in_use = r.idle.is_none_or(|i| i <= self.cfg.in_use_within);
        if in_use {
            if let Some(last) = self.last_read {
                let since = now.saturating_duration_since(last);
                if since < self.cfg.in_use_gap {
                    return Decision::Wait(self.cfg.in_use_gap - since, WaitReason::InUse);
                }
            }
        }
        Decision::Read
    }

    /// A document was read at `now` (the in-use gap counts from here).
    pub fn read_at(&mut self, now: Instant) {
        self.last_read = Some(now);
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

    /// One decision with no disk probe (as on macOS).
    fn decide(
        p: &mut Pace,
        now: Instant,
        name_busy: bool,
        idle: Option<Duration>,
        free: Option<u64>,
    ) -> Decision {
        let mut no_probe = || None;
        p.decide(
            now,
            Readings {
                name_busy,
                idle,
                free_bytes: free,
                disk: &mut no_probe,
            },
        )
    }

    /// Reads documents back to back from `start` for `span`, waiting as told, on a fake
    /// clock; returns the times documents were read.
    fn simulate(pace: &mut Pace, idle: Option<Duration>, span: Duration) -> Vec<Duration> {
        let start = Instant::now();
        let mut now = start;
        let mut reads = Vec::new();
        while now - start < span {
            match decide(pace, now, false, idle, Some(100 * GB)) {
                Decision::Read => {
                    reads.push(now - start);
                    pace.read_at(now);
                    now += Duration::from_millis(100); // the document itself
                }
                Decision::Wait(d, _) => now += d,
            }
        }
        reads
    }

    /// VFS-27 AC-7: while a name walk runs, no document is read.
    #[test]
    fn name_indexing_holds_content_back() {
        let cfg = PaceConfig::default();
        let mut p = Pace::new(cfg.clone());
        let now = Instant::now();
        assert_eq!(
            decide(&mut p, now, true, Some(Duration::from_secs(600)), None),
            Decision::Wait(cfg.name_wait, WaitReason::NameIndexing)
        );
    }

    /// VFS-27 AC-7: with 3 of the last 5 disk samples busy no document is read; when the disk
    /// stays busy past `max_backoff`, a document is still read within `max_backoff` +
    /// `min_work`.
    #[test]
    fn busy_disk_slows_but_never_stops() {
        let mut p = Pace::new(PaceConfig::default());
        let start = Instant::now();
        let mut busy = || Some(10.0);
        let mut now = start;
        let mut first_read = None;
        let mut waited_busy = false;
        while now - start < Duration::from_secs(30) {
            let d = p.decide(
                now,
                Readings {
                    name_busy: false,
                    idle: Some(Duration::from_secs(600)),
                    free_bytes: None,
                    disk: &mut busy,
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

    /// VFS-27 AC-7: with input 10 s or 60 s ago, documents are at least 15 s apart; with
    /// input 61 s ago they come back to back; when the idle probe fails, the 15 s gap applies.
    #[test]
    fn in_use_spacing() {
        let span = Duration::from_secs(120);
        for idle in [
            Some(Duration::from_secs(10)),
            Some(Duration::from_secs(60)),
            None,
        ] {
            let reads = simulate(&mut Pace::new(PaceConfig::default()), idle, span);
            assert!(reads.len() >= 2, "{idle:?}: {reads:?}");
            for w in reads.windows(2) {
                assert!(
                    w[1] - w[0] >= Duration::from_secs(15),
                    "{idle:?}: {reads:?}"
                );
            }
        }
        let reads = simulate(
            &mut Pace::new(PaceConfig::default()),
            Some(Duration::from_secs(61)),
            Duration::from_secs(10),
        );
        assert!(reads.len() >= 50, "back to back: {} reads", reads.len());
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
