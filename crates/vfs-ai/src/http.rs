//! HTTP for both providers: the client, authentication, retries (R13) and the SSE reader.

use std::io::{BufRead, BufReader, Read};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::HeaderValue;

use crate::error::{classify, AiError, Retry};
use crate::provider::{CancelToken, Endpoint, Provider};
use crate::secret::ApiKey;

/// The Anthropic API version sent on every call.
pub const ANTHROPIC_VERSION: &str = "2026-10-21";

/// Timeouts: connecting (research note, D10); a whole non-streamed call (the key check and the
/// rewrite); a whole streamed answer, which a blocking client cannot bound per read.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const SHORT_CALL_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const STREAM_TIMEOUT: Duration = Duration::from_secs(180);

/// Retries of a rate-limited, overloaded or 5xx call (R13): at most two, honouring the
/// provider's wait, else `fallback[attempt]`. A provider wait above `max_wait` is cut to it.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub fallback: [Duration; 2],
    pub max_wait: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            fallback: [Duration::from_secs(2), Duration::from_secs(8)],
            max_wait: Duration::from_secs(60),
        }
    }
}

pub(crate) struct Http {
    client: Client,
    pub provider: Provider,
    key: ApiKey,
    base_url: String,
    retry: RetryPolicy,
}

impl Http {
    pub fn new(provider: Provider, key: ApiKey, endpoint: &Endpoint) -> Result<Http, AiError> {
        // Same crypto provider as tauri-plugin-updater (rustls with ring), installed once.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let mut builder = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(None);
        if !endpoint.system_proxy {
            builder = builder.no_proxy();
        }
        let client = builder
            .build()
            .map_err(|e| AiError::BadResponse(format!("http client: {e}")))?;
        Ok(Http {
            client,
            provider,
            key,
            base_url: endpoint.base_url.trim_end_matches('/').to_owned(),
            retry: endpoint.retry.clone(),
        })
    }

    pub fn get(&self, path: &str) -> RequestBuilder {
        self.authed(self.client.get(format!("{}{path}", self.base_url)))
    }

    pub fn post(&self, path: &str) -> RequestBuilder {
        self.authed(self.client.post(format!("{}{path}", self.base_url)))
    }

    /// Adds the key as a sensitive header, so `reqwest` never prints it.
    fn authed(&self, request: RequestBuilder) -> RequestBuilder {
        let (name, text) = match self.provider {
            Provider::Anthropic => ("x-api-key", self.key.expose().to_owned()),
            Provider::OpenAi => ("authorization", format!("Bearer {}", self.key.expose())),
        };
        let mut value = HeaderValue::from_str(&text).expect("ApiKey holds visible ASCII only");
        value.set_sensitive(true);
        let request = request.header(name, value);
        match self.provider {
            Provider::Anthropic => request.header("anthropic-version", ANTHROPIC_VERSION),
            Provider::OpenAi => request,
        }
    }

    /// Sends `build()` until it succeeds, retrying what R13 allows. Returns the successful
    /// response; its body is not read.
    pub fn send(
        &self,
        cancel: &CancelToken,
        timeout: Duration,
        build: impl Fn() -> RequestBuilder,
    ) -> Result<Response, AiError> {
        let mut attempt = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled { usage: None });
            }
            let response = build().timeout(timeout).send().map_err(transport_error)?;
            let status = response.status().as_u16();
            if response.status().is_success() {
                return Ok(response);
            }
            let headers = response.headers().clone();
            let body = response.text().unwrap_or_default();
            let classified = classify(status, &headers, &body, &self.key);
            let wait = match classified.retry {
                Retry::Yes(wait) if attempt < self.retry.fallback.len() => wait
                    .unwrap_or(self.retry.fallback[attempt])
                    .min(self.retry.max_wait),
                _ => return Err(classified.error),
            };
            attempt += 1;
            log::warn!(
                "vfs-ai: {} answered HTTP {status}; retry {attempt} in {} ms",
                self.provider,
                wait.as_millis()
            );
            sleep_unless_cancelled(wait, cancel)?;
        }
    }

    /// Classifies an error that arrived inside a 200 stream (an SSE `error` event).
    pub fn stream_error(&self, body: &str) -> AiError {
        classify(0, &Default::default(), body, &self.key).error
    }
}

/// Maps a transport failure: connect or DNS failures are «Sin conexión», timeouts a saturated
/// provider (R13).
pub(crate) fn transport_error(e: reqwest::Error) -> AiError {
    if e.is_connect() {
        AiError::Offline
    } else if e.is_timeout() {
        AiError::Saturated
    } else {
        AiError::BadResponse(format!("transport: {}", e.without_url()))
    }
}

fn sleep_unless_cancelled(wait: Duration, cancel: &CancelToken) -> Result<(), AiError> {
    let end = Instant::now() + wait;
    loop {
        if cancel.is_cancelled() {
            return Err(AiError::Cancelled { usage: None });
        }
        let now = Instant::now();
        if now >= end {
            return Ok(());
        }
        thread::sleep((end - now).min(Duration::from_millis(50)));
    }
}

/// One server-sent event.
pub(crate) struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Reads server-sent events from a response body.
pub(crate) struct SseReader<R: Read> {
    lines: BufReader<R>,
}

impl<R: Read> SseReader<R> {
    pub fn new(body: R) -> Self {
        SseReader {
            lines: BufReader::new(body),
        }
    }

    /// The next event, or `None` at the end of the stream.
    pub fn next_event(&mut self) -> Result<Option<SseEvent>, AiError> {
        let mut event = String::new();
        let mut data = String::new();
        let mut line = String::new();
        loop {
            line.clear();
            let n = self.lines.read_line(&mut line).map_err(|e| {
                if e.kind() == std::io::ErrorKind::TimedOut {
                    AiError::Saturated
                } else {
                    AiError::BadResponse(format!("stream read: {e}"))
                }
            })?;
            let text = line.trim_end_matches(['\r', '\n']);
            if n == 0 || text.is_empty() {
                if !data.is_empty() || !event.is_empty() {
                    return Ok(Some(SseEvent { event, data }));
                }
                if n == 0 {
                    return Ok(None);
                }
                continue;
            }
            if let Some(v) = text.strip_prefix("event:") {
                event = v.trim_start().to_owned();
            } else if let Some(v) = text.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(v.strip_prefix(' ').unwrap_or(v));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_reader_splits_events() {
        let body =
            "event: a\ndata: {\"x\":1}\n\n: comment\nevent: b\ndata: 2\ndata: 3\n\ndata: last";
        let mut r = SseReader::new(body.as_bytes());
        let e = r.next_event().unwrap().unwrap();
        assert_eq!((e.event.as_str(), e.data.as_str()), ("a", "{\"x\":1}"));
        let e = r.next_event().unwrap().unwrap();
        assert_eq!((e.event.as_str(), e.data.as_str()), ("b", "2\n3"));
        let e = r.next_event().unwrap().unwrap();
        assert_eq!(e.data, "last");
        assert!(r.next_event().unwrap().is_none());
    }
}
