//! VFS-28 AC-4 (R4, R13): every recorded provider failure maps to its message kind, and only
//! rate limits and overloads are retried, at most twice.

mod common;

use std::net::TcpListener;
use std::time::Duration;

use common::*;
use vfs_ai::mock::{MockResponse, MockServer, RecordedWaits};
use vfs_ai::{
    chat_model, AiError, ApiKey, CancelToken, ChatModel, Endpoint, Provider, RewriteRequest,
};

type Call = fn(&dyn ChatModel) -> Result<(), AiError>;

fn rewrite(model: &dyn ChatModel) -> Result<(), AiError> {
    let request = RewriteRequest {
        system_prompt: "s".into(),
        question: "q".into(),
    };
    model.rewrite(&request, &CancelToken::new()).map(drop)
}

/// Runs «Probar» and a question against the same three responses and returns each error and
/// the number of requests it took.
fn both_calls(provider: Provider, response: MockResponse) -> Vec<(Result<(), AiError>, usize)> {
    let calls: [Call; 2] = [|m| m.validate_key(), |m| rewrite(m)];
    calls
        .iter()
        .map(|call| {
            let mock = MockServer::start(vec![response.clone(); 3]);
            let result = call(model_at(provider, &mock).as_ref());
            (result, mock.requests().len())
        })
        .collect()
}

fn assert_kind(provider: Provider, response: MockResponse, kind: &str, requests: usize) {
    for (result, sent) in both_calls(provider, response.clone()) {
        let err = result.expect_err("expected an error");
        assert_eq!(
            err.kind(),
            kind,
            "{provider} HTTP {} -> {err:?}",
            response.status
        );
        assert_eq!(
            sent, requests,
            "{provider} HTTP {} requests",
            response.status
        );
    }
}

/// VFS-28 AC-4: a 200 is «Clave correcta».
#[test]
fn ok_key() {
    for provider in [Provider::Anthropic, Provider::OpenAi] {
        let mock = MockServer::start(vec![ok_json(r#"{"data":[]}"#)]);
        model_at(provider, &mock).validate_key().unwrap();
        assert_eq!(mock.requests().len(), 1);
    }
}

/// VFS-28 AC-4: 401 is «La clave no es válida», not retried.
#[test]
fn invalid_key() {
    assert_kind(
        Provider::Anthropic,
        MockResponse::json(401, error_body("anthropic_401")),
        "invalid_key",
        1,
    );
    assert_kind(
        Provider::OpenAi,
        MockResponse::json(401, error_body("openai_401")),
        "invalid_key",
        1,
    );
}

/// VFS-28 AC-4: a 402, an Anthropic 400 reporting a credit balance too low, and OpenAI
/// `credit_balance_exhausted` are «no tiene saldo», never retried.
#[test]
fn no_credit_is_not_retried() {
    assert_kind(
        Provider::Anthropic,
        MockResponse::json(402, error_body("anthropic_402")),
        "no_credit",
        1,
    );
    assert_kind(
        Provider::Anthropic,
        MockResponse::json(400, error_body("anthropic_400_credit")),
        "no_credit",
        1,
    );
    assert_kind(
        Provider::OpenAi,
        MockResponse::json(429, error_body("openai_429_credit")),
        "no_credit",
        1,
    );
}

/// VFS-28 AC-4: OpenAI `insufficient_quota` and Anthropic's monthly spend cap are «límite de
/// gasto», never retried.
#[test]
fn spend_limits_are_not_retried() {
    assert_kind(
        Provider::OpenAi,
        MockResponse::json(429, error_body("openai_429_quota")),
        "spend_limit",
        1,
    );
    assert_kind(
        Provider::Anthropic,
        MockResponse::json(429, error_body("anthropic_429_spend_cap")),
        "spend_limit",
        1,
    );
}

/// VFS-28 AC-4: a 404 for the model is «Este modelo ya no está disponible», not retried.
#[test]
fn model_gone() {
    assert_kind(
        Provider::Anthropic,
        MockResponse::json(404, error_body("anthropic_404")),
        "model_gone",
        1,
    );
    assert_kind(
        Provider::OpenAi,
        MockResponse::json(404, error_body("openai_404")),
        "model_gone",
        1,
    );
}

/// VFS-28 AC-4: Anthropic 429 with and without `retry-after`, OpenAI `rate_limit_exceeded`, an
/// unmatched 429, a 529, a 503 and a 500 are retried twice, then «está saturado».
#[test]
fn rate_limits_and_overloads_retry_twice() {
    let cases = [
        (
            Provider::Anthropic,
            MockResponse::json(429, error_body("anthropic_429")).header("retry-after", "0"),
        ),
        (
            Provider::Anthropic,
            MockResponse::json(429, error_body("anthropic_429")),
        ),
        (
            Provider::Anthropic,
            MockResponse::json(529, error_body("anthropic_529")),
        ),
        (
            Provider::Anthropic,
            MockResponse::json(500, error_body("anthropic_500")),
        ),
        (
            Provider::OpenAi,
            MockResponse::json(429, error_body("openai_429_rate")),
        ),
        (
            Provider::OpenAi,
            MockResponse::json(429, error_body("openai_429_unknown")),
        ),
        (
            Provider::OpenAi,
            MockResponse::json(503, error_body("openai_503")),
        ),
    ];
    for (provider, response) in cases {
        assert_kind(provider, response, "saturated", 3);
    }
}

/// VFS-28 R13: a retry that then succeeds returns the success, after the first fallback
/// wait; both requests reached the provider.
#[test]
fn retry_then_success() {
    let mock = MockServer::start(vec![
        MockResponse::json(529, error_body("anthropic_529")),
        ok_json(r#"{"data":[]}"#),
    ]);
    let fallback = [Duration::from_secs(2), Duration::from_secs(8)];
    let (model, waits) = model_with_waits(
        Provider::Anthropic,
        &mock,
        fallback,
        RecordedWaits::default(),
    );
    model.validate_key().unwrap();
    assert_eq!(mock.requests().len(), 2);
    assert_eq!(waits.waits(), [fallback[0]]);
}

/// VFS-28 R13: two overloads wait the 2 s then the 8 s fallback, and a third is not retried.
#[test]
fn fallback_waits_two_then_eight() {
    let mock = MockServer::start(vec![
        MockResponse::json(529, error_body("anthropic_529"));
        3
    ]);
    let fallback = [Duration::from_secs(2), Duration::from_secs(8)];
    let (model, waits) = model_with_waits(
        Provider::Anthropic,
        &mock,
        fallback,
        RecordedWaits::default(),
    );
    assert_eq!(model.validate_key().unwrap_err().kind(), "saturated");
    assert_eq!(waits.waits(), fallback);
    assert_eq!(mock.requests().len(), 3);
}

/// VFS-28 R13: the provider's wait is honoured instead of the 2 s / 8 s fallback, for
/// `retry-after` and for OpenAI's `x-ratelimit-reset-*`.
#[test]
fn provider_wait_overrides_fallback() {
    let fallback = [Duration::from_secs(30); 2];
    let cases = [
        (
            Provider::Anthropic,
            MockResponse::json(429, error_body("anthropic_429")).header("retry-after", "3"),
            Duration::from_secs(3),
        ),
        (
            Provider::OpenAi,
            MockResponse::json(429, error_body("openai_429_rate"))
                .header("x-ratelimit-reset-requests", "10ms"),
            Duration::from_millis(10),
        ),
    ];
    for (provider, response, expected) in cases {
        let mock = MockServer::start(vec![response, ok_json(r#"{"data":[]}"#)]);
        let (model, waits) = model_with_waits(provider, &mock, fallback, RecordedWaits::default());
        model.validate_key().unwrap();
        assert_eq!(waits.waits(), [expected], "{provider}");
    }
}

/// VFS-28 R13: «Cancelar» pressed during a retry wait ends the call with `Cancelled`, and no
/// further request is sent.
#[test]
fn cancel_stops_a_retry_wait() {
    let mock = MockServer::start(vec![
        MockResponse::json(529, error_body("anthropic_529"));
        3
    ]);
    let waits = RecordedWaits::cancelling();
    let (model, waits) = model_with_waits(
        Provider::Anthropic,
        &mock,
        [Duration::from_secs(30); 2],
        waits,
    );
    let request = RewriteRequest {
        system_prompt: "s".into(),
        question: "q".into(),
    };
    let err = model.rewrite(&request, &CancelToken::new()).unwrap_err();
    assert!(matches!(err, AiError::Cancelled { .. }), "{err:?}");
    assert_eq!(waits.waits().len(), 1);
    assert_eq!(mock.requests().len(), 1);
}

/// VFS-28 AC-4: a refused connection is «Sin conexión», not retried.
#[test]
fn refused_connection_is_offline() {
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    for provider in [Provider::Anthropic, Provider::OpenAi] {
        let model = chat_model(
            provider,
            ApiKey::new(TEST_KEY).unwrap(),
            Endpoint::local(format!("http://127.0.0.1:{port}")),
        )
        .unwrap();
        assert_eq!(model.validate_key().unwrap_err().kind(), "offline");
        assert_eq!(rewrite(model.as_ref()).unwrap_err().kind(), "offline");
    }
}

/// VFS-28 R13: an error event inside a 200 stream is mapped too.
#[test]
fn error_inside_the_stream() {
    let sse = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\n\
               event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
    let mock = MockServer::start(vec![MockResponse::sse(sse)]);
    let err = model_at(Provider::Anthropic, &mock)
        .answer(&answer_request(), &CancelToken::new(), &mut |_| {})
        .unwrap_err();
    assert_eq!(err.kind(), "saturated");
}
