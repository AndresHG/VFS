//! Parent side of the extraction worker (VFS-2 R2): starts the worker process, sends one
//! request at a time, waits with a time limit, and kills and replaces a worker that hangs,
//! crashes or breaks the protocol. Nothing the worker does can take the caller down.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::protocol::{FailReason, Request, Response, MEM_CAP_ENV};

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
}

/// Extra bytes allowed on one response line beyond the text cap (JSON escaping, title).
const LINE_SLACK: usize = 64 * 1024;

/// Bytes of the worker's stderr kept for diagnosis.
const STDERR_TAIL: usize = 4096;

struct Running {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<std::io::Result<String>>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
    readers: Vec<JoinHandle<()>>,
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

    fn spawn(&mut self, line_cap: usize) -> std::io::Result<&mut Running> {
        let mut child = Command::new(&self.config.program)
            .args(&self.config.args)
            .env(MEM_CAP_ENV, self.config.mem_cap.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        self.spawned += 1;
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
        });
        Ok(self.running.as_mut().expect("just set"))
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

    /// Extracts one file in the worker. Never panics on worker misbehaviour: a hang, crash,
    /// oversized or malformed answer becomes a `Failed` response and the worker is replaced.
    pub fn extract(&mut self, req: &Request) -> Response {
        let line_cap = req.max_text_bytes + LINE_SLACK;
        let timeout = self.config.timeout;
        let started = Instant::now();
        if self.running.is_none() {
            if let Err(e) = self.spawn(line_cap) {
                return Response::failed(FailReason::Crash, format!("cannot start worker: {e}"));
            }
        }
        let mut payload = serde_json::to_string(req).expect("request serializes");
        payload.push('\n');
        let r = self.running.as_mut().expect("running");
        if let Err(e) = r
            .stdin
            .write_all(payload.as_bytes())
            .and_then(|_| r.stdin.flush())
        {
            let detail = self.kill();
            return Response::failed(FailReason::Crash, format!("worker gone: {e}; {detail}"));
        }
        match r.lines.recv_timeout(timeout) {
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
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.kill();
    }
}
