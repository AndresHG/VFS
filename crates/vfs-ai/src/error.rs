//! Provider errors mapped to the kinds of VFS-28 R13.
//!
//! The codes come from the providers' error pages, re-checked on 2026-11-22 when 28b was
//! built (R13): Anthropic `billing_error` (402), a 400 reporting a credit balance too low, a
//! 429 whose `error.details.error_code` is `enforced_spend_limit_reached` (the monthly spend
//! cap, sent without `retry-after`), `rate_limit_error` (429), `overloaded_error` (529);
//! OpenAI 429 codes `credit_balance_exhausted`, `insufficient_quota`,
//! `organization_spend_limit_exceeded`, `project_spend_limit_exceeded`,
//! `organization_usage_limit_exceeded`, `rate_limit_exceeded`, `slow_down`, and 503
//! `server_is_overloaded`.

use std::time::Duration;

use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::cost::Usage;
use crate::secret::{ApiKey, REDACTED};

/// What went wrong with a provider call. The UI turns each kind into its plain message
/// (R13); [`AiError::kind`] is the label the diagnostic log records (R14).
#[derive(Debug, thiserror::Error)]
pub enum AiError {
    /// 401 or 403: «La clave no es válida».
    #[error("the key is not valid")]
    InvalidKey,
    /// 402, Anthropic `billing_error`, a credit balance too low, or OpenAI
    /// `credit_balance_exhausted`: «Su cuenta de <proveedor> no tiene saldo». Never retried.
    #[error("the account has no credit")]
    NoCredit,
    /// A spend or usage limit of the account: «Ha llegado al límite de gasto de su cuenta».
    /// Never retried.
    #[error("the account reached its spend limit")]
    SpendLimit,
    /// Rate limited, overloaded or a 5xx after the retries, or a timeout:
    /// «<Proveedor> está saturado; pruebe en un momento».
    #[error("the provider is saturated")]
    Saturated,
    /// 404 or an unknown model: «Este modelo ya no está disponible; actualice VFS».
    #[error("the model is no longer available")]
    ModelGone,
    /// Connect or DNS failure: «Sin conexión».
    #[error("no connection")]
    Offline,
    /// The caller cancelled. `usage` holds what the provider reported before the stop, if any.
    #[error("cancelled")]
    Cancelled { usage: Option<Usage> },
    /// Any other provider error, with the provider's message (the key is never in it).
    #[error("provider error {status}: {message}")]
    Provider { status: u16, message: String },
    /// A response that does not have the expected shape.
    #[error("unexpected response: {0}")]
    BadResponse(String),
}

impl AiError {
    /// A stable label for the diagnostic log (R14). It carries no text from the provider.
    pub fn kind(&self) -> &'static str {
        match self {
            AiError::InvalidKey => "invalid_key",
            AiError::NoCredit => "no_credit",
            AiError::SpendLimit => "spend_limit",
            AiError::Saturated => "saturated",
            AiError::ModelGone => "model_gone",
            AiError::Offline => "offline",
            AiError::Cancelled { .. } => "cancelled",
            AiError::Provider { .. } => "provider",
            AiError::BadResponse(_) => "bad_response",
        }
    }
}

/// A classified HTTP failure: the error, and whether it may be retried, with the wait the
/// provider asked for if it gave one.
pub(crate) struct Classified {
    pub error: AiError,
    pub retry: Retry,
}

pub(crate) enum Retry {
    No,
    /// Retry after the provider's wait, or the policy's fallback when `None`.
    Yes(Option<Duration>),
}

/// Maps a non-success response to its R13 kind. `body` is the response text; `key` is
/// removed from any message kept.
pub(crate) fn classify(status: u16, headers: &HeaderMap, body: &str, key: &ApiKey) -> Classified {
    let json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let err = &json["error"];
    let etype = err["type"].as_str().unwrap_or("");
    let code = err["code"].as_str().unwrap_or("");
    let detail = err["details"]["error_code"].as_str().unwrap_or("");
    let message = err["message"].as_str().unwrap_or("");
    let lower = message.to_lowercase();
    let wait = provider_wait(headers);

    let no = |error| Classified {
        error,
        retry: Retry::No,
    };
    let codes = [etype, code, detail];
    let has = |names: &[&str]| codes.iter().any(|c| names.contains(c));

    if status == 401 || status == 403 || has(&["authentication_error", "permission_error"]) {
        return no(AiError::InvalidKey);
    }
    if status == 402
        || has(&["billing_error", "credit_balance_exhausted"])
        || lower.contains("credit balance")
    {
        return no(AiError::NoCredit);
    }
    if has(&[
        "insufficient_quota",
        "enforced_spend_limit_reached",
        "organization_spend_limit_exceeded",
        "project_spend_limit_exceeded",
        "organization_usage_limit_exceeded",
    ]) || lower.contains("spend limit")
        || lower.contains("usage limit")
    {
        return no(AiError::SpendLimit);
    }
    if status == 404 || has(&["model_not_found", "not_found_error"]) {
        return no(AiError::ModelGone);
    }
    if status == 429 || status >= 500 || has(&["overloaded_error", "rate_limit_error"]) {
        return Classified {
            error: AiError::Saturated,
            retry: Retry::Yes(wait),
        };
    }
    let message = if message.is_empty() {
        format!("HTTP {status}")
    } else {
        message.replace(key.expose(), REDACTED)
    };
    no(AiError::Provider { status, message })
}

/// The wait a provider asks for: `retry-after` in seconds, else OpenAI's
/// `x-ratelimit-reset-requests` / `-tokens` (`"1s"`, `"6m0s"`, `"20ms"`), whichever is longer.
fn provider_wait(headers: &HeaderMap) -> Option<Duration> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(secs) = header("retry-after").and_then(|v| v.trim().parse::<f64>().ok()) {
        if secs.is_finite() && secs >= 0.0 {
            return Some(Duration::from_secs_f64(secs));
        }
    }
    ["x-ratelimit-reset-requests", "x-ratelimit-reset-tokens"]
        .iter()
        .filter_map(|n| header(n).and_then(parse_go_duration))
        .max()
}

/// Parses a Go-style duration such as `"1m30.5s"` or `"250ms"`.
fn parse_go_duration(text: &str) -> Option<Duration> {
    let mut total = 0f64;
    let mut rest = text.trim();
    if rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        let num_end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
        let value: f64 = rest[..num_end].parse().ok()?;
        rest = &rest[num_end..];
        let unit_end = rest
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(rest.len());
        let scale = match &rest[..unit_end] {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            "ms" => 0.001,
            _ => return None,
        };
        total += value * scale;
        rest = &rest[unit_end..];
    }
    Some(Duration::from_secs_f64(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_durations() {
        assert_eq!(parse_go_duration("1s"), Some(Duration::from_secs(1)));
        assert_eq!(parse_go_duration("6m0s"), Some(Duration::from_secs(360)));
        assert_eq!(parse_go_duration("20ms"), Some(Duration::from_millis(20)));
        assert_eq!(parse_go_duration("x"), None);
    }

    /// VFS-28 AC-1 (R2): a provider message that echoes the key keeps it out.
    #[test]
    fn provider_message_never_carries_the_key() {
        let key = ApiKey::new("sk-secret-echo-9999").unwrap();
        let body = r#"{"error":{"message":"bad header sk-secret-echo-9999 here","type":"x"}}"#;
        let c = classify(400, &HeaderMap::new(), body, &key);
        let text = c.error.to_string();
        assert!(!text.contains("sk-secret-echo-9999"), "{text}");
        assert!(text.contains("«redacted»"));
    }
}
