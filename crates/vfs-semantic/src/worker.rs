//! Worker side of local embedding (VFS-29 R16): runs in its own process, loads the model once
//! (ONNX Runtime on the CPU, the model's own tokenizer; feature `embedder`), then answers one
//! [`Request`] per line on stdin with unit-length vectors on stdout. Nothing it does reaches
//! the network.
//!
//! Tests start the same binary with `--test-hook=` arguments instead of a model (as the
//! extraction worker's tests do): a fake model whose vectors come from a hash of the text, a
//! worker that never answers, one that allocates without bound. A build without `embedder`
//! runs only those.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use crate::protocol::{Hello, Request, Response, WORKER_VERSION};

/// Test-only behaviours (`--test-hook=<kind>[:<arg>]`); VFS never passes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestHook {
    /// Answer with vectors of this many dimensions derived from each text's hash.
    Fake(usize),
    /// Read requests but never answer.
    Hang,
    /// Allocate and touch memory without bound on the first request.
    Alloc,
}

/// Command-line options of the worker binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub model_dir: PathBuf,
    pub threads: usize,
    /// Run at normal priority: the worker that embeds search queries, which a user waits on.
    pub foreground: bool,
    pub hooks: Vec<TestHook>,
}

impl Args {
    /// `--model-dir DIR [--threads N] [--foreground] [--test-hook=…]`.
    pub fn parse(args: impl Iterator<Item = String>) -> Result<Args, String> {
        let mut a = Args {
            model_dir: PathBuf::new(),
            threads: 1,
            foreground: false,
            hooks: Vec::new(),
        };
        let mut it = args;
        while let Some(arg) = it.next() {
            if let Some(h) = arg.strip_prefix("--test-hook=") {
                a.hooks.push(match h.split_once(':') {
                    Some(("fake", dim)) => TestHook::Fake(dim.parse().map_err(|_| "bad fake dim")?),
                    None if h == "hang" => TestHook::Hang,
                    None if h == "alloc" => TestHook::Alloc,
                    _ => return Err(format!("unknown test hook {h}")),
                });
                continue;
            }
            match arg.as_str() {
                "--model-dir" => a.model_dir = it.next().ok_or("--model-dir needs a value")?.into(),
                "--threads" => {
                    a.threads = it
                        .next()
                        .and_then(|v| v.parse().ok())
                        .filter(|&n| n > 0)
                        .ok_or("--threads needs a positive number")?
                }
                "--foreground" => a.foreground = true,
                other => return Err(format!("unknown argument {other}")),
            }
        }
        if a.model_dir.as_os_str().is_empty() {
            return Err("--model-dir is required".into());
        }
        Ok(a)
    }
}

/// A deterministic unit vector of `dim` values from the hash of `text` (the fake model).
pub fn fake_vector(text: &str, dim: usize) -> Vec<f32> {
    let mut bytes = vec![0u8; dim * 4];
    blake3::Hasher::new()
        .update(text.as_bytes())
        .finalize_xof()
        .fill(&mut bytes);
    let mut v: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| u32::from_le_bytes(*b) as f32 / u32::MAX as f32 - 0.5)
        .collect();
    crate::quant::normalise(&mut v);
    v
}

/// Mean of `len` token vectors of `dim` values each, scaled to unit length.
pub fn mean_pool(hidden: &[f32], len: usize, dim: usize) -> Vec<f32> {
    let mut v = vec![0f32; dim];
    for row in hidden.chunks_exact(dim).take(len) {
        for (acc, x) in v.iter_mut().zip(row) {
            *acc += x;
        }
    }
    crate::quant::normalise(&mut v);
    v
}

#[cfg(feature = "embedder")]
mod onnx {
    use std::path::Path;

    use ort::session::builder::{AutoDevicePolicy, GraphOptimizationLevel};
    use ort::session::Session;
    use ort::value::Tensor;
    use tokenizers::Tokenizer;

    use crate::{ModelId, LOCAL_MODEL};

    /// The loaded model.
    pub struct Model {
        pub id: ModelId,
        tokenizer: Tokenizer,
        session: Session,
    }

    impl Model {
        /// Loads the model and tokenizer files from `dir`, on the CPU only: ONNX Runtime's
        /// automatic device choice hands the graph to Core ML on macOS, which was twice as
        /// slow and took twice the memory here (measured in 29c).
        pub fn load(dir: &Path, threads: usize) -> Result<Model, String> {
            let id = LOCAL_MODEL;
            let mut tokenizer =
                Tokenizer::from_file(dir.join(id.tokenizer_file)).map_err(|e| e.to_string())?;
            tokenizer
                .with_truncation(Some(tokenizers::TruncationParams {
                    max_length: id.max_tokens,
                    ..Default::default()
                }))
                .map_err(|e| e.to_string())?;
            tokenizer.with_padding(None);
            let env = ort::init().build().map_err(|e| e.to_string())?;
            let e = |e: ort::Error<_>| e.to_string();
            let session = Session::builder(&env)
                .map_err(|e| e.to_string())?
                .with_auto_device(AutoDevicePolicy::PreferCPU)
                .map_err(e)?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(e)?
                .with_intra_threads(threads)
                .map_err(e)?
                .with_inter_threads(1)
                .map_err(e)?
                .commit_from_file(dir.join(id.model_file))
                .map_err(|e| e.to_string())?;
            Ok(Model {
                id,
                tokenizer,
                session,
            })
        }

        /// One unit-length vector per text, mean-pooled over its tokens.
        pub fn embed(&mut self, texts: &[String], query: bool) -> Result<Vec<Vec<f32>>, String> {
            let prefix = if query {
                self.id.query_prefix
            } else {
                self.id.doc_prefix
            };
            let mut out = Vec::with_capacity(texts.len());
            for t in texts {
                let enc = self
                    .tokenizer
                    .encode(format!("{prefix}{t}"), true)
                    .map_err(|e| e.to_string())?;
                let mut ids: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
                if ids.is_empty() {
                    ids.push(0);
                }
                let n = ids.len();
                let tensor =
                    |v: Vec<i64>| Tensor::from_array(([1usize, n], v)).map_err(|e| e.to_string());
                let outputs = self
                    .session
                    .run(ort::inputs![
                        "input_ids" => tensor(ids)?,
                        "attention_mask" => tensor(vec![1; n])?,
                        "token_type_ids" => tensor(vec![0; n])?,
                    ])
                    .map_err(|e| e.to_string())?;
                let (_, hidden) = outputs[0]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| e.to_string())?;
                out.push(super::mean_pool(hidden, n, self.id.dim));
            }
            Ok(out)
        }
    }
}

enum Backend {
    Fake(usize),
    #[cfg(feature = "embedder")]
    Onnx(Box<onnx::Model>),
}

impl Backend {
    fn dim(&self) -> usize {
        match self {
            Backend::Fake(d) => *d,
            #[cfg(feature = "embedder")]
            Backend::Onnx(m) => m.id.dim,
        }
    }

    fn embed(&mut self, texts: &[String], _query: bool) -> Result<Vec<Vec<f32>>, String> {
        match self {
            Backend::Fake(d) => Ok(texts.iter().map(|t| fake_vector(t, *d)).collect()),
            #[cfg(feature = "embedder")]
            Backend::Onnx(m) => m.embed(texts, _query),
        }
    }
}

fn load(args: &Args) -> Result<Backend, String> {
    if let Some(TestHook::Fake(d)) = args.hooks.iter().find(|h| matches!(h, TestHook::Fake(_))) {
        return Ok(Backend::Fake(*d));
    }
    #[cfg(feature = "embedder")]
    {
        onnx::Model::load(&args.model_dir, args.threads).map(|m| Backend::Onnx(Box::new(m)))
    }
    #[cfg(not(feature = "embedder"))]
    {
        Err("this build has no model runtime (feature `embedder`)".into())
    }
}

/// Entry point of the `vfs-embed` binary.
pub fn worker_main() {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("vfs-embed: {e}");
            std::process::exit(2);
        }
    };
    if !args.foreground {
        vfs_content::process::enter_background_priority();
    }
    let mut backend = match load(&args) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("vfs-embed: cannot load the model: {e}");
            std::process::exit(3);
        }
    };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let hello = Hello {
        version: WORKER_VERSION.to_string(),
        dim: backend.dim(),
        background: vfs_content::process::background_priority().0,
    };
    if writeln!(out, "{}", serde_json::to_string(&hello).expect("hello")).is_err()
        || out.flush().is_err()
    {
        return;
    }
    let mut hoard: Vec<Vec<u8>> = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { return };
        if args.hooks.contains(&TestHook::Hang) {
            continue;
        }
        if args.hooks.contains(&TestHook::Alloc) {
            loop {
                hoard.push(vec![1u8; 64 << 20]);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let resp = match serde_json::from_str::<Request>(&line) {
            Ok(req) => match backend.embed(&req.texts, req.query) {
                Ok(vectors) => Response::Vectors { vectors },
                Err(message) => Response::Failed { message },
            },
            Err(e) => Response::Failed {
                message: format!("bad request: {e}"),
            },
        };
        let line = serde_json::to_string(&resp).expect("response");
        if writeln!(out, "{line}").is_err() || out.flush().is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Result<Args, String> {
        Args::parse(a.iter().map(|s| s.to_string()))
    }

    /// VFS-29 R16: the worker's arguments; a model folder is required.
    #[test]
    fn parses_arguments() {
        let a = args(&["--model-dir", "m", "--threads", "2", "--test-hook=fake:8"]).unwrap();
        assert_eq!(a.model_dir, PathBuf::from("m"));
        assert_eq!(a.threads, 2);
        assert!(!a.foreground);
        assert_eq!(a.hooks, vec![TestHook::Fake(8)]);
        assert!(args(&[]).is_err());
        assert!(args(&["--model-dir", "m", "--threads", "0"]).is_err());
        assert!(args(&["--model-dir", "m", "--test-hook=nope"]).is_err());
    }

    /// VFS-29 R16: pooling averages the token vectors and returns a unit vector; the fake
    /// model is deterministic and unit length.
    #[test]
    fn pooling_and_fake_vectors() {
        let v = mean_pool(&[1.0, 0.0, 3.0, 0.0, 9.0, 9.0], 2, 2);
        assert!((v[0] - 1.0).abs() < 1e-6 && v[1].abs() < 1e-6);
        let f = fake_vector("factura del dentista", 16);
        assert_eq!(f, fake_vector("factura del dentista", 16));
        assert!((f.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-5);
        assert_ne!(f, fake_vector("gastos médicos", 16));
    }
}
