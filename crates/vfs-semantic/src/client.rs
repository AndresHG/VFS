//! Parent side of the embedding worker (VFS-29 R16): starts `vfs-embed`, sends one batch at a
//! time, waits with a time limit and a memory cap, and kills the worker when it hangs, breaks
//! the protocol, outgrows its cap or the caller stops. Nothing the worker does can take the
//! caller down.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine::{EmbedError, Embedder};
use crate::protocol::{Hello, Request, Response, WORKER_VERSION};
use crate::ModelId;

/// How to start the worker and what it may use.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// The `vfs-embed` executable, and any arguments before the worker's own.
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Folder holding the model's files.
    pub model_dir: PathBuf,
    pub model: ModelId,
    pub threads: usize,
    /// Normal priority (the query worker) instead of the background band.
    pub foreground: bool,
    /// Resident memory the worker may reach; beyond it, it is killed.
    pub mem_cap: u64,
    /// Wall clock per batch.
    pub timeout: Duration,
}

/// Resident memory measured for e5-small int8 at two threads on 512-token chunks is about
/// 0.7 GB (29c); the cap leaves room above it.
pub const DEFAULT_MEM_CAP: u64 = 1 << 30;

/// How long a new worker may take to load its model and say hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(120);
/// How often a wait looks at the stop flag and the worker's memory.
const POLL: Duration = Duration::from_millis(50);
/// Bytes of the worker's stderr kept for the log.
const STDERR_TAIL: usize = 4096;

struct Running {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<std::io::Result<String>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    /// The thread draining stderr; joined after the worker exits, so its last words are kept.
    stderr_reader: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    _job: vfs_content::process::job::Job,
}

impl Running {
    fn stderr_tail(&self) -> String {
        let buf = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&buf).trim().to_string()
    }
}

/// The local embedding worker, started on first use and kept until dropped or killed.
pub struct WorkerEmbedder {
    cfg: WorkerConfig,
    running: Option<Running>,
}

impl WorkerEmbedder {
    pub fn new(cfg: WorkerConfig) -> WorkerEmbedder {
        WorkerEmbedder { cfg, running: None }
    }

    /// Whether a worker process is alive.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Ends the worker process, if any.
    pub fn kill(&mut self) {
        if let Some(mut r) = self.running.take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }

    fn start(&mut self) -> Result<(), String> {
        let mut cmd = Command::new(&self.cfg.program);
        cmd.args(&self.cfg.args)
            .arg("--model-dir")
            .arg(&self.cfg.model_dir)
            .arg("--threads")
            .arg(self.cfg.threads.max(1).to_string());
        if self.cfg.foreground {
            cmd.arg("--foreground");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", self.cfg.program.display()))?;
        #[cfg(windows)]
        let job = match vfs_content::process::job::Job::hold(&child, self.cfg.mem_cap as usize) {
            Ok(j) => j,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cannot cap the worker's memory: {e}"));
            }
        };
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut err = child.stderr.take().expect("piped stderr");
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let tail = stderr.clone();
        let stderr_reader = std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                t.extend_from_slice(&buf[..n]);
                let over = t.len().saturating_sub(STDERR_TAIL);
                t.drain(..over);
            }
        });
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        self.running = Some(Running {
            child,
            stdin,
            lines,
            stderr,
            stderr_reader: Some(stderr_reader),
            #[cfg(windows)]
            _job: job,
        });
        let never = AtomicBool::new(false);
        let line = self.wait_line(HELLO_TIMEOUT, &never).map_err(|e| match e {
            EmbedError::Failed(m) => m,
            EmbedError::Stopped => "stopped".into(),
        })?;
        let hello: Hello = serde_json::from_str(&line).map_err(|e| {
            self.kill();
            format!("bad hello from the worker: {e}")
        })?;
        if hello.version != WORKER_VERSION || hello.dim != self.cfg.model.dim {
            self.kill();
            return Err(format!(
                "the worker is version {} with {} dimensions, not {WORKER_VERSION} with {}",
                hello.version, hello.dim, self.cfg.model.dim
            ));
        }
        Ok(())
    }

    /// The worker's next line, watching the stop flag, the deadline and the memory cap; the
    /// worker is killed on any of them.
    fn wait_line(&mut self, timeout: Duration, stop: &AtomicBool) -> Result<String, EmbedError> {
        let deadline = Instant::now() + timeout;
        loop {
            let Some(r) = self.running.as_mut() else {
                return Err(EmbedError::Failed("no worker".into()));
            };
            match r.lines.recv_timeout(POLL) {
                Ok(Ok(line)) => return Ok(line),
                Ok(Err(e)) => {
                    let msg = format!("reading from the worker: {e}");
                    self.kill();
                    return Err(EmbedError::Failed(msg));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = r.child.wait();
                    if let Some(h) = r.stderr_reader.take() {
                        let _ = h.join();
                    }
                    let msg = format!("the worker exited: {}", r.stderr_tail());
                    self.kill();
                    return Err(EmbedError::Failed(msg));
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if stop.load(Ordering::Relaxed) {
                self.kill();
                return Err(EmbedError::Stopped);
            }
            if let Some(rss) = resident_bytes(r.child.id()) {
                if rss > self.cfg.mem_cap {
                    self.kill();
                    return Err(EmbedError::Failed(format!(
                        "the worker used {} MB, over its {} MB cap",
                        rss >> 20,
                        self.cfg.mem_cap >> 20
                    )));
                }
            }
            if Instant::now() > deadline {
                self.kill();
                return Err(EmbedError::Failed(
                    "the worker did not answer in time".into(),
                ));
            }
        }
    }
}

impl Embedder for WorkerEmbedder {
    fn model(&self) -> ModelId {
        self.cfg.model
    }

    fn embed(
        &mut self,
        texts: &[String],
        query: bool,
        stop: &AtomicBool,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        if self.running.is_none() {
            self.start().map_err(EmbedError::Failed)?;
        }
        let req = serde_json::to_string(&Request {
            texts: texts.to_vec(),
            query,
        })
        .expect("request");
        let r = self.running.as_mut().expect("started");
        if writeln!(r.stdin, "{req}").is_err() || r.stdin.flush().is_err() {
            let msg = format!("the worker stopped reading: {}", r.stderr_tail());
            self.kill();
            return Err(EmbedError::Failed(msg));
        }
        let line = self.wait_line(self.cfg.timeout, stop)?;
        match serde_json::from_str::<Response>(&line) {
            Ok(Response::Vectors { vectors }) => Ok(vectors),
            Ok(Response::Failed { message }) => Err(EmbedError::Failed(message)),
            Err(e) => {
                self.kill();
                Err(EmbedError::Failed(format!(
                    "bad answer from the worker: {e}"
                )))
            }
        }
    }
}

impl Drop for WorkerEmbedder {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Resident memory of process `pid`; `None` where it cannot be read (Windows caps the worker
/// with a job object instead).
#[cfg(target_os = "macos")]
fn resident_bytes(pid: u32) -> Option<u64> {
    // SAFETY: proc_taskinfo is plain old data; all zeroes is a valid value.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    // SAFETY: proc_pidinfo writes at most `size` bytes into the zeroed struct we own.
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    (n == size).then_some(info.pti_resident_size)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn resident_bytes(pid: u32) -> Option<u64> {
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf only reads a system constant.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64;
    Some(pages * page)
}

#[cfg(windows)]
fn resident_bytes(_pid: u32) -> Option<u64> {
    None
}
