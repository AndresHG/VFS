//! Parent side of the extraction worker (VFS-2 R2): starts the worker process, sends one
//! request at a time, waits with a time limit, and kills and replaces a worker that hangs,
//! crashes or breaks the protocol. Nothing the worker does can take the caller down.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::protocol::{FailReason, Hello, Request, Response, MEM_CAP_ENV, WORKER_VERSION};

/// How to start a worker and what it may use.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Executable and arguments that run [`crate::worker::worker_main`].
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Wall clock per file.
    pub timeout: Duration,
    /// Heap cap of the worker, in bytes (passed as `VFS_EXTRACT_MEM_CAP`).
    pub mem_cap: usize,
    /// After a stop request, how long the document in flight may still take before the
    /// worker is killed (VFS-27 R3: at most 5 s, so an update can replace the executable).
    pub stop_grace: Duration,
}

/// How often a wait for the worker's answer looks at the stop flag.
const STOP_POLL: Duration = Duration::from_millis(50);

/// A wait that returns this much later than asked means the machine was suspended.
const SUSPEND_GAP: Duration = Duration::from_secs(2);

/// The deadline after a wait that asked for `asked` and took `took`: a wait that overran by
/// more than [`SUSPEND_GAP`] spanned a sleep, and the time asleep does not count against the
/// document (VFS-27 R9).
pub(crate) fn extend_for_suspension(deadline: Instant, asked: Duration, took: Duration) -> Instant {
    if took > asked + SUSPEND_GAP {
        deadline + (took - asked)
    } else {
        deadline
    }
}

/// Why [`Supervisor::extract`] returned no answer for the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    /// No worker could be started: the run cannot continue (VFS-2 R17).
    Start(StartError),
    /// A stop was requested and the file was not finished within the grace period; the
    /// worker was killed and the file is left as it was (VFS-27 R3).
    Stopped,
}

impl From<StartError> for Halt {
    fn from(e: StartError) -> Halt {
        Halt::Start(e)
    }
}

/// Extra bytes allowed on one response line beyond the text cap (JSON escaping, title).
const LINE_SLACK: usize = 64 * 1024;

/// Bytes of the worker's stderr kept for diagnosis.
const STDERR_TAIL: usize = 4096;

/// How long a new worker may take to say hello. Generous: a background-priority process on
/// a busy disk starts slowly, and a start that never answers is a real failure (VFS-2 R17).
const HELLO_TIMEOUT: Duration = Duration::from_secs(60);

/// Committed memory a Windows worker may use beyond its allocator cap, so the allocator's
/// abort fires first (VFS-2 R18).
#[cfg(windows)]
const JOB_SLACK: usize = 128 << 20;

/// Why no worker could be had: the run cannot continue (VFS-2 R17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// The binary is missing, blocked, or did not say hello.
    CannotStart(String),
    /// The worker reported another version than [`WORKER_VERSION`].
    VersionMismatch(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::CannotStart(d) => write!(f, "worker cannot start: {d}"),
            StartError::VersionMismatch(v) => {
                write!(f, "version mismatch: worker {v}, expected {WORKER_VERSION}")
            }
        }
    }
}

struct Running {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<std::io::Result<String>>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
    readers: Vec<JoinHandle<()>>,
    /// Windows Job Object holding the worker; held only to be dropped with it, which kills
    /// the worker (VFS-2 R18).
    #[cfg(windows)]
    _job: job::Job,
}

/// One worker process, started on first use and replaced after any failure.
pub struct Supervisor {
    config: WorkerConfig,
    running: Option<Running>,
    /// Workers started so far (1 + restarts).
    pub spawned: u64,
}

impl Supervisor {
    pub fn new(config: WorkerConfig) -> Supervisor {
        Supervisor {
            config,
            running: None,
            spawned: 0,
        }
    }

    /// Starts a worker now (unless one runs) and checks its version.
    pub fn start(&mut self, line_cap: usize) -> Result<(), StartError> {
        if self.running.is_some() {
            return Ok(());
        }
        self.spawn(line_cap)
            .map_err(|e| StartError::CannotStart(e.to_string()))?;
        let r = self.running.as_mut().expect("just spawned");
        let hello = match r.lines.recv_timeout(HELLO_TIMEOUT) {
            Ok(Ok(line)) => serde_json::from_str::<Hello>(&line)
                .map_err(|e| StartError::CannotStart(format!("bad hello: {e}"))),
            Ok(Err(e)) => Err(StartError::CannotStart(e.to_string())),
            Err(RecvTimeoutError::Timeout) => {
                Err(StartError::CannotStart("no hello from the worker".into()))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(StartError::CannotStart("worker exited at start".into()))
            }
        };
        let err = match hello {
            Ok(h) if h.version == WORKER_VERSION => return Ok(()),
            Ok(h) => StartError::VersionMismatch(h.version),
            Err(e) => e,
        };
        let detail = self.kill();
        Err(match err {
            StartError::CannotStart(d) => StartError::CannotStart(format!("{d}; {detail}")),
            other => other,
        })
    }

    fn spawn(&mut self, line_cap: usize) -> std::io::Result<()> {
        let mut child = Command::new(&self.config.program)
            .args(&self.config.args)
            .env(MEM_CAP_ENV, self.config.mem_cap.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        self.spawned += 1;
        #[cfg(windows)]
        let job = match job::Job::hold(&child, self.config.mem_cap.saturating_add(JOB_SLACK)) {
            Ok(j) => j,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let (tx, rx) = mpsc::channel();
        let out_reader = std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                // A line longer than the cap is a protocol violation; read at most cap + 1.
                let res = (&mut r)
                    .take(line_cap as u64 + 1)
                    .read_line(&mut line)
                    .map(|n| (n, line));
                match res {
                    Ok((0, _)) => break,
                    Ok((_, line)) => {
                        if tx.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });
        let tail = Arc::new(Mutex::new(Vec::new()));
        let tail2 = Arc::clone(&tail);
        let err_reader = std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut t = tail2.lock().unwrap_or_else(|e| e.into_inner());
                t.extend_from_slice(&buf[..n]);
                let excess = t.len().saturating_sub(STDERR_TAIL);
                t.drain(..excess);
            }
        });
        self.running = Some(Running {
            child,
            stdin,
            lines: rx,
            stderr_tail: tail,
            readers: vec![out_reader, err_reader],
            #[cfg(windows)]
            _job: job,
        });
        Ok(())
    }

    /// Kills the current worker (if any) and reaps it.
    fn kill(&mut self) -> String {
        let Some(mut r) = self.running.take() else {
            return String::new();
        };
        let _ = r.child.kill();
        let status = r.child.wait();
        drop(r.stdin);
        for h in r.readers.drain(..) {
            let _ = h.join();
        }
        let tail = r.stderr_tail.lock().unwrap_or_else(|e| e.into_inner());
        let msg = String::from_utf8_lossy(&tail).trim().to_string();
        match status {
            Ok(s) => format!("{s}; {msg}"),
            Err(e) => format!("{e}; {msg}"),
        }
    }

    /// The longest answer line accepted for a text cap of `max_text_bytes`.
    pub fn line_cap(max_text_bytes: usize) -> usize {
        max_text_bytes + LINE_SLACK
    }

    /// Extracts one file in the worker. Never panics on worker misbehaviour: a hang, crash,
    /// oversized or malformed answer becomes a `Failed` response and the worker is replaced.
    /// `Err` when no worker can be started, which ends the run (VFS-2 R17), or when `stop` is
    /// set and the file does not finish within the grace period (VFS-27 R3).
    pub fn extract(&mut self, req: &Request, stop: &AtomicBool) -> Result<Response, Halt> {
        let line_cap = Self::line_cap(req.max_text_bytes);
        let timeout = self.config.timeout;
        self.start(line_cap)?;
        let started = Instant::now();
        self.exchange(req, line_cap, timeout, started, stop)
    }

    fn exchange(
        &mut self,
        req: &Request,
        line_cap: usize,
        timeout: Duration,
        started: Instant,
        stop: &AtomicBool,
    ) -> Result<Response, Halt> {
        let mut payload = serde_json::to_string(req).expect("request serializes");
        payload.push('\n');
        let r = self.running.as_mut().expect("running");
        if let Err(e) = r
            .stdin
            .write_all(payload.as_bytes())
            .and_then(|_| r.stdin.flush())
        {
            let detail = self.kill();
            return Ok(Response::failed(
                FailReason::Crash,
                format!("worker gone: {e}; {detail}"),
            ));
        }
        // Wait in short slices so a stop request is seen; after one, the file in flight gets
        // `stop_grace` before the worker is killed.
        let mut deadline = started + timeout;
        let mut grace_end: Option<Instant> = None;
        let received = loop {
            let now = Instant::now();
            if grace_end.is_none() && stop.load(Ordering::Relaxed) {
                grace_end = Some(now + self.config.stop_grace);
            }
            if grace_end.is_some_and(|g| now >= g) {
                self.kill();
                return Err(Halt::Stopped);
            }
            let until = grace_end
                .map_or(deadline, |g| g.min(deadline))
                .min(now + STOP_POLL);
            let asked = until.saturating_duration_since(now);
            let lines = &self.running.as_ref().expect("running").lines;
            match lines.recv_timeout(asked) {
                Err(RecvTimeoutError::Timeout) => {
                    deadline = extend_for_suspension(deadline, asked, now.elapsed());
                    if Instant::now() < deadline {
                        continue;
                    }
                    break Err(RecvTimeoutError::Timeout);
                }
                other => break other,
            }
        };
        Ok(match received {
            Ok(Ok(line)) if line.len() <= line_cap && line.ends_with('\n') => {
                match serde_json::from_str::<Response>(&line) {
                    Ok(resp) => resp,
                    Err(e) => {
                        let detail = self.kill();
                        Response::failed(FailReason::Crash, format!("bad answer: {e}; {detail}"))
                    }
                }
            }
            Ok(Ok(_)) => {
                let detail = self.kill();
                Response::failed(
                    FailReason::Crash,
                    format!("answer over {line_cap} bytes; {detail}"),
                )
            }
            Ok(Err(e)) => {
                let detail = self.kill();
                Response::failed(FailReason::Crash, format!("read error: {e}; {detail}"))
            }
            Err(RecvTimeoutError::Timeout) => {
                self.kill();
                let mut resp = Response::failed(
                    FailReason::Timeout,
                    format!("no answer within {} s", timeout.as_secs_f32()),
                );
                resp.ms = started.elapsed().as_millis() as u64;
                resp
            }
            Err(RecvTimeoutError::Disconnected) => {
                let detail = self.kill();
                // The capped allocator aborts with Rust's "memory allocation ... failed".
                let reason = if detail.contains("memory allocation") {
                    FailReason::Memory
                } else {
                    FailReason::Crash
                };
                Response::failed(reason, format!("worker exited: {detail}"))
            }
        })
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Windows Job Object for one worker (VFS-2 R18): kill-on-close, so the worker dies with the
/// parent's last handle, and a committed-memory limit just above the allocator cap.
#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };

    pub struct Job(HANDLE);

    // SAFETY: a job handle may be used and closed from any thread.
    unsafe impl Send for Job {}

    impl Job {
        /// Creates a job with the limits and puts `child` in it.
        pub fn hold(child: &Child, memory_limit: usize) -> std::io::Result<Job> {
            // SAFETY: plain Win32 calls on handles owned here or by `child`; every pointer
            // refers to a live local of the right type and size.
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let job = Job(h);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags =
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                info.ProcessMemoryLimit = memory_limit;
                if SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                if AssignProcessToJobObject(h, child.as_raw_handle() as HANDLE) == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(job)
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle was created by `hold` and is closed exactly once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-27 AC-8 (R9): a wait that overran by the length of a sleep moves the document's
    /// deadline by that much, so a document in flight across a sleep is not failed by timeout;
    /// an ordinary wait leaves it alone.
    #[test]
    fn sleep_does_not_count_against_the_document() {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(30);
        let poll = Duration::from_millis(50);
        assert_eq!(
            extend_for_suspension(deadline, poll, Duration::from_millis(60)),
            deadline
        );
        let slept = Duration::from_secs(120);
        assert_eq!(
            extend_for_suspension(deadline, poll, poll + slept),
            deadline + slept
        );
    }
}
