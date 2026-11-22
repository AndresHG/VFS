//! VFS-28 AC-8 (R10, crate part) and AC-9's data (R11): costs from recorded usage and the
//! dated price table, the month total, the cap and the record of what was sent.

mod common;

use chrono::{Local, TimeZone};
use common::*;
use vfs_ai::cost::NANO_PER_USD;
use vfs_ai::mock::{MockResponse, MockServer};
use vfs_ai::{
    cost_nano, models_for, AiDb, CallKind, CallRecord, CancelToken, Provider, RewriteRequest,
    SentPassage, SentRecord, Usage, ANSWER_MAX_TOKENS, DEFAULT_CAP_NANO,
};

fn at(y: i32, m: u32, d: u32) -> chrono::DateTime<Local> {
    Local.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
}

/// Asks one question against recorded responses and returns its record.
fn ask(provider: Provider, when: chrono::DateTime<Local>, question: &str) -> SentRecord {
    let (rewrite_body, answer_body) = match provider {
        Provider::Anthropic => (ANTHROPIC_REWRITE, ANTHROPIC_ANSWER),
        Provider::OpenAi => (OPENAI_REWRITE, OPENAI_ANSWER),
    };
    let mock = MockServer::start(vec![ok_json(rewrite_body), MockResponse::sse(answer_body)]);
    let model = model_at(provider, &mock);
    let cancel = CancelToken::new();
    let rq = RewriteRequest {
        system_prompt: "s".into(),
        question: question.into(),
    };
    let (_, raw, rewrite_usage) = model.rewrite(&rq, &cancel).unwrap();
    let request = answer_request();
    let answer = model.answer(&request, &cancel, &mut |_| {}).unwrap();
    let models = models_for(provider);
    SentRecord {
        asked_at: when,
        provider,
        question: question.into(),
        rewrite: Some(raw),
        passages: request
            .passages
            .iter()
            .map(|p| SentPassage {
                file_name: p.title.clone(),
                page: Some(1),
                text: p.text.clone(),
            })
            .collect(),
        calls: vec![
            CallRecord {
                kind: CallKind::Rewrite,
                model: models.rewrite.into(),
                usage: rewrite_usage,
                cost_nano: cost_nano(models.rewrite, &rewrite_usage),
            },
            CallRecord {
                kind: CallKind::Answer,
                model: answer.model.clone(),
                usage: answer.usage,
                cost_nano: cost_nano(&answer.model, &answer.usage),
            },
        ],
    }
}

/// VFS-28 AC-8: each cost follows the price table, and the month total is the sum of three
/// questions.
#[test]
fn three_questions_sum_to_the_month_total() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = AiDb::open(&dir.path().join("ai.db")).unwrap();
    let day = at(2026, 11, 22);
    let records = [
        ask(Provider::Anthropic, day, "uno"),
        ask(Provider::OpenAi, day, "dos"),
        ask(Provider::Anthropic, day, "tres"),
    ];
    for r in &records {
        db.record(r).unwrap();
    }
    // Expected costs come from the recorded token counts and the price table.
    let cost = |model: &str, input, output, cached| {
        let usage = Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cached,
            ..Usage::default()
        };
        cost_nano(model, &usage)
    };
    let (a, o) = (
        models_for(Provider::Anthropic),
        models_for(Provider::OpenAi),
    );
    let anthropic = cost(a.rewrite, 812, 61, 0) + cost(a.answer, 9_300, 587, 0);
    let openai = cost(o.rewrite, 800, 70, 100) + cost(o.answer, 9_500, 120, 0);
    assert!(anthropic > 0 && openai > 0);
    let stored = db.records().unwrap();
    let costs: Vec<i64> = stored.iter().map(|r| r.cost_nano).collect();
    assert_eq!(costs, [anthropic, openai, anthropic]);
    assert_eq!(db.month_total(&day).unwrap(), 2 * anthropic + openai);
}

/// VFS-28 AC-8: a model missing from the table is priced at the table's highest price.
#[test]
fn unknown_model_costs_the_highest_price() {
    let usage = Usage {
        input_tokens: 1_000,
        output_tokens: 100,
        ..Usage::default()
    };
    let unknown = cost_nano("claude-unknown-9", &usage);
    for p in [Provider::Anthropic, Provider::OpenAi] {
        let m = models_for(p);
        for id in [m.rewrite, m.answer, m.cheap_answer] {
            assert!(
                unknown >= cost_nano(id, &usage),
                "{id} costs more than an unknown model"
            );
        }
    }
    let dearest = [Provider::Anthropic, Provider::OpenAi]
        .iter()
        .flat_map(|p| {
            let m = models_for(*p);
            [m.rewrite, m.answer, m.cheap_answer]
        })
        .map(|id| cost_nano(id, &usage))
        .max()
        .unwrap();
    assert_eq!(unknown, dearest);
}

/// VFS-28 AC-8: a cancelled stream with no usage is priced at its `max_tokens`.
#[test]
fn cancelled_stream_costs_max_tokens() {
    let usage = Usage::for_cancelled(None, ANSWER_MAX_TOKENS);
    assert_eq!(usage.output_tokens, u64::from(ANSWER_MAX_TOKENS));
    let one_token = Usage {
        output_tokens: 1,
        ..Usage::default()
    };
    let model = models_for(Provider::Anthropic).answer;
    assert_eq!(
        cost_nano(model, &usage),
        i64::from(ANSWER_MAX_TOKENS) * cost_nano(model, &one_token)
    );
}

/// VFS-28 AC-8: with a cap of 0.01 $, the next question needs confirmation once reached; the
/// default cap is 5 $; a new calendar month starts from zero.
#[test]
fn cap_and_month_rollover() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = AiDb::open(&dir.path().join("ai.db")).unwrap();
    assert_eq!(db.cap().unwrap(), DEFAULT_CAP_NANO);
    db.set_cap(NANO_PER_USD / 100).unwrap();
    let nov = at(2026, 11, 30);
    assert!(!db.cap_status(&nov).unwrap().reached());
    db.record(&ask(Provider::Anthropic, nov, "uno")).unwrap();
    let status = db.cap_status(&nov).unwrap();
    assert!(status.reached(), "{status:?}");
    let dec = at(2026, 12, 1);
    assert_eq!(db.month_total(&dec).unwrap(), 0);
    assert!(!db.cap_status(&dec).unwrap().reached());
}

/// VFS-28 AC-9 (data part, R11): both questions are listed with every passage as sent and their
/// costs; «Borrar todo» empties the record but keeps the month's spend for the cap.
#[test]
fn record_lists_and_clears() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = AiDb::open(&dir.path().join("ai.db")).unwrap();
    let day = at(2026, 11, 22);
    let first = ask(Provider::Anthropic, day, "¿fianza?");
    let mut second = ask(Provider::OpenAi, day, "¿Y el año?");
    second.rewrite = None;
    second.calls[1].kind = CallKind::FollowUp;
    db.record(&first).unwrap();
    db.record(&second).unwrap();
    let stored = db.records().unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].question, "¿fianza?");
    assert_eq!(stored[0].passages, first.passages);
    assert_eq!(stored[0].calls, first.calls);
    assert_eq!(stored[1].rewrite, None);
    assert_eq!(stored[1].calls[1].kind, CallKind::FollowUp);
    let spent = db.month_total(&day).unwrap();

    db.clear_records().unwrap();
    assert!(db.records().unwrap().is_empty());
    assert_eq!(db.month_total(&day).unwrap(), spent);
}
