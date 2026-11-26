//! VFS-28 AC-11 (R14): a question's log line has its rounds, passages, tokens, cost and
//! dropped citations, and none of the question text, passage text or file names. Its own test
//! binary, so the logger it installs sees only this test.

mod common;

use std::sync::{Arc, Mutex};

use chrono::{Local, TimeZone};
use common::fake_corpus::{FakeCorpus, FakeDoc};
use common::*;
use vfs_ai::ask::{self, AskOptions, Lang};
use vfs_ai::mock::{anthropic_answer, anthropic_rewrite, MockServer};
use vfs_ai::{AiDb, AnswerBlock, CancelToken, Provider, RawCitation, Rewrite};
use vfs_core::SearchFilters;

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

/// VFS-28 AC-11: the R14 line of a question and of a chat message (VFS-49 R4).
#[test]
fn question_log_has_counts_and_no_content() {
    let captured = Arc::new(Mutex::new(String::new()));
    log::set_logger(Box::leak(Box::new(Capture(captured.clone())))).unwrap();
    log::set_max_level(log::LevelFilter::Trace);

    let mut docs = vec![
        FakeDoc::new(1, "/Privado/Médico", "Informe_Garcia.pdf", 3, &[1]),
        FakeDoc::new(2, "/Privado/Médico", "Analitica_2024.pdf", 3, &[0]),
    ];
    docs[0].chunks[1] = "Glucosa en ayunas de 126 mg/dl.".into();
    let corpus = FakeCorpus::new(docs).scripted(&[&[1], &[1, 2]]);
    let rewrite = Rewrite {
        keywords: vec!["glucosa".into()],
        ..Rewrite::default()
    };
    let answer = AnswerBlock {
        text: "La glucosa era de 126 mg/dl.".into(),
        citations: vec![
            RawCitation {
                passage: 0,
                quote: "Glucosa en ayunas de 126 mg/dl.".into(),
            },
            RawCitation {
                passage: 7,
                quote: "inventado".into(),
            },
        ],
    };
    let mock = MockServer::start(vec![
        anthropic_rewrite(&rewrite),
        anthropic_answer(std::slice::from_ref(&answer)),
        anthropic_rewrite(&rewrite),
        anthropic_answer(std::slice::from_ref(&answer)),
    ]);
    let model = model_at(Provider::Anthropic, &mock);
    let dir = tempfile::tempdir().unwrap();
    let mut db = AiDb::open(&dir.path().join("ai.db")).unwrap();
    let opts = AskOptions {
        provider: Provider::Anthropic,
        lang: Lang::Es,
        cheap: false,
        answer_model: None,
        over_cap_confirmed: false,
        now: Local.with_ymd_and_hms(2026, 11, 23, 12, 0, 0).unwrap(),
    };
    let question = "¿Qué glucosa tenía en el análisis de Ramírez?";
    let cancel = CancelToken::new();
    let asked = ask::ask(
        model.as_ref(),
        &corpus,
        &mut db,
        question,
        &SearchFilters::default(),
        &opts,
        &cancel,
        &mut |_| {},
    )
    .unwrap();
    ask::chat_turn(
        model.as_ref(),
        &corpus,
        &mut db,
        "¿Y la de Ramírez del año anterior?",
        &asked.thread.turns,
        &ask::Scope::All,
        &opts,
        &cancel,
        &mut |_| {},
    )
    .unwrap();

    let log = captured.lock().unwrap().clone();
    let line = log
        .lines()
        .find(|l| l.contains("vfs-ai: ask "))
        .unwrap_or_else(|| panic!("no ask line in {log}"));
    for field in [
        "rounds=[1, 2]",
        "passages=2",
        "latency_ms=",
        "input_tokens=",
        "output_tokens=",
        "cost_nano=",
        "citations_dropped=1",
        "error=none",
    ] {
        assert!(line.contains(field), "{field} missing in {line}");
    }
    assert!(log.contains("vfs-ai: chat "), "{log}");
    for secret in [
        "Ramírez",
        "glucosa",
        "Glucosa",
        "mg/dl",
        "Informe_Garcia",
        "Analitica",
        "Privado",
        "Médico",
    ] {
        assert!(!log.contains(secret), "«{secret}» in the log:\n{log}");
    }
}
