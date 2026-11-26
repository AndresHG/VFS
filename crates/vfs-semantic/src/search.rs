//! The ranker by meaning that `retrieve` fuses with BM25 (VFS-29 R11, R13, R15): it embeds
//! the query once in a foreground worker, scans the set's 1-bit rows in memory, rescores the
//! closest with their int8 rows from `vectors.db`, and maps each text to its chunks (copies
//! included). The rows and the worker are loaded on first use and released after a while
//! unused; nothing exists before the first use.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use vfs_content::{Meaning, MeaningHit, MeaningUnavailable};

use crate::client::{WorkerConfig, WorkerEmbedder};
use crate::engine::{EmbedError, Embedder};
use crate::quant;
use crate::store::{Store, TextHash};

/// Candidates rescored with int8 per chunk asked (R11: the top 4× k).
pub const RESCORE_FACTOR: usize = 4;

/// How the ranker works.
#[derive(Debug, Clone)]
pub struct MeaningConfig {
    /// The query worker: foreground, one thread, its timeout the query limit (R13: 2 s).
    pub worker: WorkerConfig,
    /// Unused this long, the rows and the worker are released (R15: 5 minutes).
    pub idle: Duration,
}

/// The query limit of R13.
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
/// The idle release of R15.
pub const IDLE_RELEASE: Duration = Duration::from_secs(5 * 60);

struct Rows {
    set: i64,
    dim: usize,
    mean: Vec<f32>,
    hashes: Vec<TextHash>,
    bits: Vec<u8>,
    /// `(hash, file_id, ordinal)` sorted by hash.
    locations: Vec<(TextHash, i64, u32)>,
    store: Store,
}

enum Worker {
    None,
    Starting,
    Ready(Box<WorkerEmbedder>),
}

struct State {
    rows: Option<Rows>,
    worker: Worker,
    last_used: Instant,
    reaper: bool,
}

struct Inner {
    index_db: PathBuf,
    cfg: MeaningConfig,
    state: Mutex<State>,
    /// Signalled when the worker becomes ready or fails to start.
    ready: Condvar,
}

/// The app's ranker by meaning for one index. Cheap to create: it opens nothing until used.
#[derive(Clone)]
pub struct MeaningIndex {
    inner: Arc<Inner>,
}

impl MeaningIndex {
    pub fn new(index_db: PathBuf, cfg: MeaningConfig) -> MeaningIndex {
        MeaningIndex {
            inner: Arc::new(Inner {
                index_db,
                cfg,
                state: Mutex::new(State {
                    rows: None,
                    worker: Worker::None,
                    last_used: Instant::now(),
                    reaper: false,
                }),
                ready: Condvar::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Loads the rows and starts the query worker in the background, so no search waits for
    /// either (the «Documentos» tab calls this when it opens; a search that finds them missing
    /// calls it too). With no complete vector set, nothing is started.
    pub fn warm(&self) {
        let mut st = self.lock();
        st.last_used = Instant::now();
        self.ensure_reaper(&mut st);
        let worker_ready = matches!(st.worker, Worker::Ready(_));
        if matches!(st.worker, Worker::Starting) || (worker_ready && st.rows.is_some()) {
            return;
        }
        st.worker = match std::mem::replace(&mut st.worker, Worker::Starting) {
            Worker::Ready(w) => Worker::Ready(w),
            _ => Worker::Starting,
        };
        let need_rows = st.rows.is_none();
        let need_worker = !worker_ready;
        drop(st);
        let me = self.clone();
        std::thread::spawn(move || {
            let rows = if need_rows {
                me.load_rows().ok().flatten()
            } else {
                None
            };
            let have_rows = !need_rows || rows.is_some();
            let worker = if need_worker && have_rows {
                let mut w = WorkerEmbedder::new(me.inner.cfg.worker.clone());
                w.start_now(&AtomicBool::new(false))
                    .is_ok()
                    .then(|| Box::new(w))
            } else {
                None
            };
            let mut st = me.lock();
            if rows.is_some() {
                st.rows = rows;
            }
            if need_worker {
                st.worker = match worker {
                    Some(w) => Worker::Ready(w),
                    None => Worker::None,
                };
            }
            me.inner.ready.notify_all();
        });
    }

    /// Waits up to `timeout` for [`MeaningIndex::warm`] to finish; true when the rows are
    /// loaded and the worker is ready.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut st = self.lock();
        loop {
            match st.worker {
                Worker::Ready(_) => return st.rows.is_some(),
                Worker::None => return false,
                Worker::Starting => {}
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            st = self
                .inner
                .ready
                .wait_timeout(st, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Whether the rows or the worker are held in memory (R15).
    pub fn is_loaded(&self) -> bool {
        let st = self.lock();
        st.rows.is_some() || !matches!(st.worker, Worker::None)
    }

    /// One thread per loaded index releases the rows and the worker once unused for the idle
    /// time, then ends (R15); none runs before the first use.
    fn ensure_reaper(&self, st: &mut State) {
        if st.reaper {
            return;
        }
        st.reaper = true;
        let me = self.clone();
        let idle = self.inner.cfg.idle;
        std::thread::spawn(move || loop {
            std::thread::sleep(
                (idle / 10).clamp(Duration::from_millis(20), Duration::from_secs(30)),
            );
            let mut st = me.lock();
            if st.last_used.elapsed() >= idle && !matches!(st.worker, Worker::Starting) {
                st.rows = None;
                st.worker = Worker::None;
                st.reaper = false;
                return;
            }
        });
    }

    fn load_rows(&self) -> Result<Option<Rows>, String> {
        let Some(store) = Store::open_ro(&self.inner.index_db).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let Some((set, dim)) = store.search_set().map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let mean = store
            .mean(set)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| vec![0.0; dim]);
        let (hashes, bits) = store.bit_rows(set).map_err(|e| e.to_string())?;
        let locations = store.locations().map_err(|e| e.to_string())?;
        Ok(Some(Rows {
            set,
            dim,
            mean,
            hashes,
            bits,
            locations,
            store,
        }))
    }
}

impl Meaning for MeaningIndex {
    fn rank(&self, query: &str, n: usize) -> Result<Vec<MeaningHit>, MeaningUnavailable> {
        let mut st = self.lock();
        st.last_used = Instant::now();
        self.ensure_reaper(&mut st);
        // Loading the rows reads the whole set; it happens in `warm`, never inside a search.
        if st.rows.is_none() || !matches!(st.worker, Worker::Ready(_)) {
            drop(st);
            self.warm();
            return Err(MeaningUnavailable::NotReady);
        }
        let Worker::Ready(w) = &mut st.worker else {
            return Err(MeaningUnavailable::NotReady);
        };
        let q = w.embed(&[query.to_string()], true, &AtomicBool::new(false));
        let q = match q {
            Ok(mut v) if v.len() == 1 => v.remove(0),
            Ok(_) => return Err(MeaningUnavailable::Failed("no query vector".into())),
            Err(e) => {
                // A worker past the time limit was killed: start a new one for next time.
                st.worker = Worker::None;
                let msg = match e {
                    EmbedError::Failed(m) => m,
                    EmbedError::Stopped => "stopped".into(),
                };
                return Err(MeaningUnavailable::Failed(msg));
            }
        };
        let rows = st.rows.as_ref().expect("loaded");
        rank_rows(rows, &q, n).map_err(MeaningUnavailable::Failed)
    }
}

/// The scan, the rescore and the mapping to chunks.
fn rank_rows(rows: &Rows, q: &[f32], n: usize) -> Result<Vec<MeaningHit>, String> {
    if q.len() != rows.dim || rows.hashes.is_empty() {
        return Ok(Vec::new());
    }
    let width = rows.dim.div_ceil(8);
    let qbits = quant::centred_bits(q, &rows.mean);
    let mut by_distance: Vec<(u32, usize)> = rows
        .bits
        .chunks_exact(width)
        .enumerate()
        .map(|(i, b)| (quant::hamming(b, &qbits), i))
        .collect();
    let take = (n * RESCORE_FACTOR).min(by_distance.len());
    if take < by_distance.len() {
        by_distance.select_nth_unstable(take);
        by_distance.truncate(take);
    }
    let mut scored: Vec<(f32, usize)> = Vec::with_capacity(take);
    for (_, i) in by_distance {
        if let Some((q8, scale)) = rows
            .store
            .q8(rows.set, &rows.hashes[i])
            .map_err(|e| e.to_string())?
        {
            scored.push((quant::dot_int8(q, &q8, scale), i));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut out = Vec::new();
    for (score, i) in scored {
        let h = &rows.hashes[i];
        let from = rows.locations.partition_point(|l| &l.0 < h);
        for &(_, file_id, ordinal) in rows.locations[from..].iter().take_while(|l| &l.0 == h) {
            out.push(MeaningHit {
                file_id,
                ordinal,
                score,
            });
        }
        if out.len() >= n {
            break;
        }
    }
    out.truncate(n);
    Ok(out)
}
