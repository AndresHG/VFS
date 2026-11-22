//! VFS-28 AC-1 (crate part, R2): after a 401, a 500 and a forced panic with the key in use,
//! the diagnostic log, the panic report and `ai.db` do not contain the key. Every log record
//! of every crate (reqwest, keyring, this one) at every level is captured. The VFS-20 report
//! is built from this log; the settings file is the app's (28d).

mod common;

use std::sync::{Arc, Mutex};

use chrono::Local;
use common::*;
use vfs_ai::mock::{MockResponse, MockServer};
use vfs_ai::{
    cost_nano, models_for, AiDb, ApiKey, CallKind, CallRecord, CancelToken, Fallback, Keys,
    Provider, RewriteRequest, SentPassage, SentRecord, Usage,
};

/// Collects every log line into a buffer the test reads.
struct Capture(Arc<Mutex<String>>);

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        let line = format!("{} {} {}\n", record.level(), record.target(), record.args());
        self.0.lock().unwrap().push_str(&line);
    }
    fn flush(&self) {}
}

fn assert_no_key(what: &str, haystack: &[u8]) {
    let needle = TEST_KEY.as_bytes();
    assert!(
        !haystack.windows(needle.len()).any(|w| w == needle),
        "{what} contains the key"
    );
    let tail = &TEST_KEY[8..];
    assert!(
        !haystack.windows(tail.len()).any(|w| w == tail.as_bytes()),
        "{what} contains most of the key"
    );
}

/// VFS-28 AC-1: the key appears in no log line, panic report or `ai.db` byte.
#[test]
fn key_never_leaks() {
    let captured = Arc::new(Mutex::new(String::new()));
    log::set_logger(Box::leak(Box::new(Capture(captured.clone())))).unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    let panics = Arc::new(Mutex::new(String::new()));
    let sink = panics.clone();
    std::panic::set_hook(Box::new(move |info| {
        sink.lock().unwrap().push_str(&info.to_string())
    }));

    // A 401 and a 500 (retried twice) on both providers, with the key echoed in the body.
    for provider in [Provider::Anthropic, Provider::OpenAi] {
        let echo = format!(r#"{{"error":{{"type":"api_error","message":"echo {TEST_KEY}"}}}}"#);
        let mock = MockServer::start(vec![
            MockResponse::json(401, error_body("anthropic_401")),
            MockResponse::json(500, echo.clone()),
            MockResponse::json(500, echo.clone()),
            MockResponse::json(500, echo),
        ]);
        let model = model_at(provider, &mock);
        let e401 = model.validate_key().unwrap_err();
        let request = RewriteRequest {
            system_prompt: "s".into(),
            question: "q".into(),
        };
        let e500 = model.rewrite(&request, &CancelToken::new()).unwrap_err();
        log::error!("errors: {e401} / {e401:?} / {e500} / {e500:?}");
    }

    // A forced panic while the key is in use.
    let key = ApiKey::new(TEST_KEY).unwrap();
    let result = std::panic::catch_unwind(|| {
        let held = key.clone();
        panic!("forced panic holding {held:?} and {held}");
    });
    assert!(result.is_err());
    let _ = std::panic::take_hook();

    // ai.db after a recorded question, and a protected key file kept elsewhere.
    let dir = tempfile::tempdir().unwrap();
    let mut db = AiDb::open(&dir.path().join("ai.db")).unwrap();
    let usage = Usage {
        input_tokens: 10,
        output_tokens: 5,
        ..Usage::default()
    };
    db.record(&SentRecord {
        asked_at: Local::now(),
        provider: Provider::Anthropic,
        question: "q".into(),
        rewrite: None,
        passages: vec![SentPassage {
            file_name: "a.pdf".into(),
            page: Some(1),
            text: "t".into(),
        }],
        calls: vec![CallRecord {
            kind: CallKind::Answer,
            model: models_for(Provider::Anthropic).answer.into(),
            usage,
            cost_nano: cost_nano(models_for(Provider::Anthropic).answer, &usage),
        }],
    })
    .unwrap();
    let keys = Keys::new(dir.path().join("ai-keys"));
    keys.save_fallback(Provider::Anthropic, &key, Fallback::SessionOnly)
        .unwrap();
    drop(db);

    assert_no_key("the log", captured.lock().unwrap().as_bytes());
    let report = panics.lock().unwrap().clone();
    assert!(report.contains("«redacted»"), "{report}");
    assert_no_key("the panic report", report.as_bytes());
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            assert_no_key(&path.display().to_string(), &std::fs::read(&path).unwrap());
        }
    }
    assert!(
        captured.lock().unwrap().contains("retry 2"),
        "the retries were logged"
    );
}
