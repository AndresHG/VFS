//! Wire shapes and parsing of both providers against the local mock (VFS-28 R4, R6.1, R7, R9).

mod common;

use common::*;
use vfs_ai::mock::{MockResponse, MockServer};
use vfs_ai::{
    AnswerEvent, CancelToken, Provider, RawCitation, RewriteRequest, Turn, Usage,
    ANSWER_MAX_TOKENS, REWRITE_MAX_TOKENS,
};

fn rewrite_request() -> RewriteRequest {
    RewriteRequest {
        system_prompt: "Reescribe la pregunta.".into(),
        question: "¿cuánto pagué de fianza por el piso de la calle Mayor?".into(),
    }
}

/// VFS-28 R4: «Probar» calls the model list with the key, which costs no tokens.
#[test]
fn validate_key_lists_models_with_the_key() {
    for provider in [Provider::Anthropic, Provider::OpenAi] {
        let mock = MockServer::start(vec![ok_json(r#"{"data":[]}"#)]);
        model_at(provider, &mock).validate_key().unwrap();
        let req = &mock.requests()[0];
        assert_eq!(
            (req.method.as_str(), req.path.as_str()),
            ("GET", "/v1/models")
        );
        match provider {
            Provider::Anthropic => {
                assert_eq!(req.header("x-api-key"), Some(TEST_KEY));
                assert_eq!(req.header("anthropic-version"), Some("2026-10-21"));
            }
            Provider::OpenAi => {
                assert_eq!(
                    req.header("authorization"),
                    Some(&*format!("Bearer {TEST_KEY}"))
                );
            }
        }
    }
}

/// VFS-28 R6.1, R7: the Anthropic rewrite uses the cheap model, strict structured output,
/// thinking off and `max_tokens` 300; its JSON parses into the rewrite.
#[test]
fn anthropic_rewrite() {
    let mock = MockServer::start(vec![ok_json(ANTHROPIC_REWRITE)]);
    let (rewrite, raw, usage) = model_at(Provider::Anthropic, &mock)
        .rewrite(&rewrite_request(), &CancelToken::new())
        .unwrap();
    assert_eq!(rewrite.keywords, ["fianza", "piso", "Mayor"]);
    assert_eq!(rewrite.folder_hint.as_deref(), Some("Contratos"));
    assert_eq!(rewrite.date_from, None);
    assert!(raw.contains("\"phrases\""));
    assert_eq!(
        usage,
        Usage {
            input_tokens: 812,
            output_tokens: 61,
            ..Usage::default()
        }
    );
    let body = mock.requests()[0].json();
    assert_eq!(mock.requests()[0].path, "/v1/messages");
    assert_eq!(body["model"], "claude-haiku-5-5");
    assert_eq!(body["max_tokens"], REWRITE_MAX_TOKENS);
    assert_eq!(body["thinking"]["type"], "disabled");
    assert_eq!(body["output_config"]["format"]["type"], "json_schema");
    assert_eq!(
        body["output_config"]["format"]["schema"]["additionalProperties"],
        false
    );
}

/// VFS-28 R6.1, R7: the OpenAI rewrite uses the cheap model, strict JSON schema and
/// `max_output_tokens` 300; cached tokens are split from the input.
#[test]
fn openai_rewrite() {
    let mock = MockServer::start(vec![ok_json(OPENAI_REWRITE)]);
    let (rewrite, _, usage) = model_at(Provider::OpenAi, &mock)
        .rewrite(&rewrite_request(), &CancelToken::new())
        .unwrap();
    assert_eq!(rewrite.synonyms, ["depósito"]);
    assert_eq!(rewrite.date_from.as_deref(), Some("2021-01-01"));
    assert_eq!(usage.input_tokens, 800);
    assert_eq!(usage.cache_read_tokens, 100);
    assert_eq!(usage.output_tokens, 70);
    let body = mock.requests()[0].json();
    assert_eq!(mock.requests()[0].path, "/v1/responses");
    assert_eq!(body["model"], "gpt-6-luna");
    assert_eq!(body["max_output_tokens"], REWRITE_MAX_TOKENS);
    assert_eq!(body["text"]["format"]["strict"], true);
    assert_eq!(body["store"], false);
}

/// VFS-28 R7, R7a: the Anthropic answer sends passages as `search_result` blocks with
/// citations on, never in the system prompt, with no structured output and `max_tokens` 1,200;
/// the stream yields the text and every citation as returned, including an index never sent
/// (28c drops it).
#[test]
fn anthropic_answer_streams_text_and_citations() {
    let mock = MockServer::start(vec![MockResponse::sse(ANTHROPIC_ANSWER)]);
    let request = answer_request();
    let mut streamed = String::new();
    let answer = model_at(Provider::Anthropic, &mock)
        .answer(&request, &CancelToken::new(), &mut |e| {
            let AnswerEvent::Text(t) = e;
            streamed.push_str(&t);
        })
        .unwrap();
    assert_eq!(streamed, answer.text());
    assert_eq!(
        answer.text(),
        "Según el contrato, la fianza fue de 1.200 €, pagada en 2021."
    );
    let cited: Vec<usize> = answer
        .blocks
        .iter()
        .flat_map(|b| b.citations.iter().map(|c| c.passage))
        .collect();
    assert_eq!(cited, [0, 2, 12]);
    assert_eq!(
        answer.blocks[1].citations[0].quote,
        "La fianza asciende a 1.200 euros."
    );
    assert_eq!(
        answer.usage,
        Usage {
            input_tokens: 9300,
            output_tokens: 587,
            ..Usage::default()
        }
    );
    assert_eq!(answer.model, "claude-sonnet-5-5");
    assert_eq!(answer.stop_reason.as_deref(), Some("end_turn"));

    let body = mock.requests()[0].json();
    assert_eq!(body["model"], "claude-sonnet-5-5");
    assert_eq!(body["max_tokens"], ANSWER_MAX_TOKENS);
    assert_eq!(body["stream"], true);
    assert_eq!(body["thinking"]["type"], "between_tools");
    assert!(
        body.get("output_config").is_none(),
        "no structured output with citations (D8)"
    );
    let system = body["system"].as_str().unwrap();
    assert!(
        !system.contains("La fianza asciende"),
        "passage text in the system prompt"
    );
    let content = body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content.len(), 11);
    assert_eq!(content[0]["type"], "search_result");
    assert_eq!(content[0]["citations"]["enabled"], true);
    assert_eq!(
        content[0]["content"][0]["text"],
        "La fianza asciende a 1.200 euros."
    );
    assert_eq!(content[10]["text"], "¿Cuánto pagué de fianza?");
}

/// VFS-28 D7: «Respuestas más baratas» answers with the cheap model, thinking disabled.
#[test]
fn cheap_answers_use_the_cheap_model() {
    let mock = MockServer::start(vec![MockResponse::sse(ANTHROPIC_ANSWER)]);
    let mut request = answer_request();
    request.cheap = true;
    model_at(Provider::Anthropic, &mock)
        .answer(&request, &CancelToken::new(), &mut |_| {})
        .unwrap();
    let body = mock.requests()[0].json();
    assert_eq!(body["model"], "claude-haiku-5-5");
    assert_eq!(body["thinking"]["type"], "disabled");
}

/// VFS-28 R9: a follow-up sends the same passages, then the earlier question and answer, then
/// the new question.
#[test]
fn follow_up_carries_the_thread() {
    for provider in [Provider::Anthropic, Provider::OpenAi] {
        let reply = match provider {
            Provider::Anthropic => MockResponse::sse(ANTHROPIC_ANSWER),
            Provider::OpenAi => MockResponse::sse(OPENAI_ANSWER),
        };
        let mock = MockServer::start(vec![reply]);
        let mut request = answer_request();
        request.earlier = vec![Turn {
            question: "¿Cuánto pagué de fianza?".into(),
            answer: "1.200 €.".into(),
        }];
        request.question = "¿Y en qué año?".into();
        model_at(provider, &mock)
            .answer(&request, &vfs_ai::CancelToken::new(), &mut |_| {})
            .unwrap();
        let body = mock.requests()[0].json();
        let turns = match provider {
            Provider::Anthropic => body["messages"].as_array().unwrap().clone(),
            Provider::OpenAi => body["input"].as_array().unwrap().clone(),
        };
        assert_eq!(turns.len(), 3, "{provider}");
        let first = turns[0]["content"].to_string();
        assert!(
            first.contains("La fianza asciende a 1.200 euros."),
            "{provider}"
        );
        assert!(first.contains("¿Cuánto pagué de fianza?"), "{provider}");
        assert_eq!(turns[1]["role"], "assistant");
        assert_eq!(turns[1]["content"], "1.200 €.");
        assert_eq!(turns[2]["content"], "¿Y en qué año?");
    }
}

/// VFS-28 R7: the OpenAI answer numbers the passages in the input, asks for strict JSON and
/// returns its citations as given (28c drops the one whose quote is not in its passage).
#[test]
fn openai_answer_parses_json_citations() {
    let mock = MockServer::start(vec![MockResponse::sse(OPENAI_ANSWER)]);
    let mut events = Vec::new();
    let answer = model_at(Provider::OpenAi, &mock)
        .answer(&answer_request(), &CancelToken::new(), &mut |e| {
            events.push(e)
        })
        .unwrap();
    assert_eq!(answer.text(), "La fianza fue de 1.200 € [P0].");
    assert_eq!(
        events,
        [AnswerEvent::Text("La fianza fue de 1.200 € [P0].".into())]
    );
    assert_eq!(
        answer.blocks[0].citations,
        [
            RawCitation {
                passage: 0,
                quote: "La fianza asciende a 1.200 euros.".into()
            },
            RawCitation {
                passage: 1,
                quote: "no está en el pasaje".into()
            },
        ]
    );
    assert_eq!(answer.usage.input_tokens, 9500);
    assert_eq!(answer.usage.output_tokens, 120);
    let body = mock.requests()[0].json();
    assert_eq!(body["model"], "gpt-6.2-sol");
    assert_eq!(body["max_output_tokens"], ANSWER_MAX_TOKENS);
    assert_eq!(body["text"]["format"]["strict"], true);
    assert!(!body["instructions"]
        .as_str()
        .unwrap()
        .contains("La fianza asciende"));
    let data = body["input"][0]["content"][0]["text"].as_str().unwrap();
    assert!(data.starts_with("[P0] Documento0.pdf · p. 1\nLa fianza asciende a 1.200 euros."));
    assert!(data.contains("[P9] "));
}

/// VFS-28 R10, R13: cancelling mid-stream stops the answer and returns the usage reported so
/// far, which prices output at `max_tokens` when none arrived.
#[test]
fn cancelled_stream_reports_partial_usage() {
    let mock = MockServer::start(vec![MockResponse::sse(ANTHROPIC_ANSWER)]);
    let cancel = CancelToken::new();
    let err = model_at(Provider::Anthropic, &mock)
        .answer(&answer_request(), &cancel, &mut |_| cancel.cancel())
        .unwrap_err();
    let vfs_ai::AiError::Cancelled { usage } = err else {
        panic!("{err:?}")
    };
    assert_eq!(usage.map(|u| u.input_tokens), Some(9300));
    let priced = Usage::for_cancelled(
        usage.map(|u| Usage {
            output_tokens: 0,
            ..u
        }),
        ANSWER_MAX_TOKENS,
    );
    assert_eq!(priced.output_tokens, u64::from(ANSWER_MAX_TOKENS));
}
