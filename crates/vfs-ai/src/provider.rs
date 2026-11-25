//! The `ChatModel` trait and the data it exchanges with the caller (VFS-28 R4, R6, R7, R9).

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cost::Usage;
use crate::error::AiError;
use crate::http::{Http, RetryPolicy};
use crate::secret::ApiKey;

/// `max_tokens` of the rewrite call (R7).
pub const REWRITE_MAX_TOKENS: u32 = 300;
/// `max_tokens` of the answer call (R7).
pub const ANSWER_MAX_TOKENS: u32 = 1_200;

/// The model provider the user chose (D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
}

impl Provider {
    /// The account name of its key in the credential store (R2), and its label in `ai.db`.
    pub fn id(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
        }
    }

    pub fn from_id(id: &str) -> Option<Provider> {
        match id {
            "anthropic" => Some(Provider::Anthropic),
            "openai" => Some(Provider::OpenAi),
            _ => None,
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Provider::Anthropic => "Anthropic",
            Provider::OpenAi => "OpenAI",
        })
    }
}

/// Where a provider is reached.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// Scheme and host, no trailing slash: `https://api.anthropic.com`.
    pub base_url: String,
    /// Use the system proxy (the default). Off for a local mock server.
    pub system_proxy: bool,
    pub retry: RetryPolicy,
}

impl Endpoint {
    /// The provider's public API, through the system proxy.
    pub fn public(provider: Provider) -> Endpoint {
        let base_url = match provider {
            Provider::Anthropic => "https://api.anthropic.com",
            Provider::OpenAi => "https://api.openai.com",
        };
        Endpoint {
            base_url: base_url.into(),
            system_proxy: true,
            retry: RetryPolicy::default(),
        }
    }

    /// A server at `base_url` reached without a proxy, such as the test mock.
    pub fn local(base_url: impl Into<String>) -> Endpoint {
        Endpoint {
            base_url: base_url.into(),
            system_proxy: false,
            retry: RetryPolicy::default(),
        }
    }
}

/// Stops a running call: checked between stream events and during retry waits (R13).
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The rewrite call's input (R6.1). The caller writes the prompt.
#[derive(Debug, Clone)]
pub struct RewriteRequest {
    pub system_prompt: String,
    pub question: String,
}

/// The rewrite model's structured output (R6.1). Absent optional fields are `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rewrite {
    pub keywords: Vec<String>,
    pub phrases: Vec<String>,
    pub synonyms: Vec<String>,
    pub kind: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub folder_hint: Option<String>,
}

/// A passage sent to the answer model (R6.4). Its index in [`AnswerRequest::passages`] is the
/// index citations refer to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    /// A stable id for the passage, such as `vfs://passage/3`; never a folder path.
    pub source: String,
    /// What the model sees as its title: the file name and page (D11), such as
    /// «Contrato.pdf · p. 4».
    pub title: String,
    pub text: String,
}

/// An earlier question and answer of the same thread (R9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub question: String,
    pub answer: String,
}

/// The answer call's input (R7, R7a, R9). Passages go to the model only as passage data, never
/// in the system prompt.
#[derive(Debug, Clone)]
pub struct AnswerRequest {
    pub system_prompt: String,
    pub passages: Vec<Passage>,
    /// Earlier questions and answers of this thread, oldest first; empty for a new question.
    pub earlier: Vec<Turn>,
    pub question: String,
    /// «Respuestas más baratas»: answer with the cheap model (D7).
    pub cheap: bool,
    /// A model to answer with instead of the tier's; `None` in the app. The live reference run
    /// sets it (VFS-46).
    pub model: Option<&'static str>,
}

impl AnswerRequest {
    /// The model this request answers with on `provider`: [`AnswerRequest::model`], else the
    /// tier's (D7).
    pub fn answer_model(&self, provider: Provider) -> &'static str {
        let models = crate::cost::models_for(provider);
        match self.model {
            Some(m) => m,
            None if self.cheap => models.cheap_answer,
            None => models.answer,
        }
    }
}

/// A citation as the provider returned it. 28c checks that `passage` was sent and that `quote`
/// is in it (R7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCitation {
    /// Anthropic `search_result_index`, or OpenAI `passage_id`.
    pub passage: usize,
    /// Anthropic `cited_text`, or OpenAI `quote`.
    pub quote: String,
}

/// A run of answer text with the citations attached to it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnswerBlock {
    pub text: String,
    pub citations: Vec<RawCitation>,
}

/// A finished answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The model requested, which is what the price table prices (R10).
    pub model: String,
    pub blocks: Vec<AnswerBlock>,
    pub usage: Usage,
    pub stop_reason: Option<String>,
}

impl Answer {
    /// The answer's whole text.
    pub fn text(&self) -> String {
        self.blocks.iter().map(|b| b.text.as_str()).collect()
    }
}

/// Progress of a streamed answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerEvent {
    /// More answer text. OpenAI answers are strict JSON, so their text arrives in one event
    /// once the answer is complete.
    Text(String),
}

/// A model provider (R4, R6, R7). Calls block; run them off the UI thread.
pub trait ChatModel: Send + Sync {
    fn provider(&self) -> Provider;

    /// Checks the key with the provider's model-list call, which costs no tokens (R4).
    fn validate_key(&self) -> Result<(), AiError>;

    /// Rewrites a question with the cheap model and strict structured output (R6.1).
    fn rewrite(
        &self,
        request: &RewriteRequest,
        cancel: &CancelToken,
    ) -> Result<(Rewrite, String, Usage), AiError>;

    /// Streams an answer over the given passages (R7, R9). `on_event` sees the text as it
    /// arrives. A cancelled stream returns [`AiError::Cancelled`] with the usage reported so far.
    fn answer(
        &self,
        request: &AnswerRequest,
        cancel: &CancelToken,
        on_event: &mut dyn FnMut(AnswerEvent),
    ) -> Result<Answer, AiError>;
}

/// A [`ChatModel`] for `provider` with `key`, reached at `endpoint`.
pub fn chat_model(
    provider: Provider,
    key: ApiKey,
    endpoint: Endpoint,
) -> Result<Box<dyn ChatModel>, AiError> {
    let http = Http::new(provider, key, &endpoint)?;
    Ok(match provider {
        Provider::Anthropic => Box::new(crate::anthropic::Anthropic::new(http)),
        Provider::OpenAi => Box::new(crate::openai::OpenAi::new(http)),
    })
}

/// The answer's schema, shared by both providers (R7, R8, VFS-46 R2). `found` is the model's own verdict: a cheap model that says
/// the passages lack the answer still cites a nearby fact (VFS-46 R2), so `found: false`
/// discards its citations and the answer shows as not found.
pub(crate) fn answer_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "answer": {"type": "string"},
            "found": {
                "type": "boolean",
                "description": "true only if the passages state the answer to the question; false when it would have to be guessed or calculated from other facts",
            },
            "citations": {
                "type": "array",
                "description": "empty when found is false",
                "items": {
                    "type": "object",
                    "properties": {
                        "passage_id": {"type": "integer"},
                        "quote": {
                            "type": "string",
                            "description": "a short span copied character for character from that passage's text, without its [P] header",
                        },
                    },
                    "required": ["passage_id", "quote"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["answer", "found", "citations"],
        "additionalProperties": false,
    })
}

/// The passages as numbered data, one block each: `[P3] Contrato.pdf · p. 4` then the text.
pub(crate) fn numbered_passages(request: &AnswerRequest) -> String {
    let mut out = String::new();
    for (i, p) in request.passages.iter().enumerate() {
        out.push_str(&format!("[P{i}] {}\n{}\n\n", p.title, p.text));
    }
    out
}

#[derive(Deserialize)]
struct JsonAnswer {
    answer: String,
    found: bool,
    citations: Vec<JsonCitation>,
}

#[derive(Deserialize)]
struct JsonCitation {
    passage_id: i64,
    quote: String,
}

/// Streams the `answer` field of a JSON answer while it arrives, so the text shows as it is
/// written and «Cancelar» can stop it midway (R13). The schema puts `answer` first; when the
/// text does not start with it, everything is sent at the end.
#[derive(Default)]
pub(crate) struct AnswerStreamer {
    sent: String,
}

impl AnswerStreamer {
    /// Sends the part of the answer in `so_far`, the JSON received until now, not sent yet.
    pub(crate) fn feed(&mut self, so_far: &str, on_event: &mut dyn FnMut(AnswerEvent)) {
        let Some(prefix) = answer_prefix(so_far) else {
            return;
        };
        if let Some(new) = prefix.strip_prefix(self.sent.as_str()) {
            if !new.is_empty() {
                on_event(AnswerEvent::Text(new.to_owned()));
                self.sent = prefix;
            }
        }
    }

    /// Sends what the stream did not of the final answer `text`.
    pub(crate) fn finish(&mut self, text: &str, on_event: &mut dyn FnMut(AnswerEvent)) {
        match text.strip_prefix(self.sent.as_str()) {
            Some("") => {}
            Some(rest) => on_event(AnswerEvent::Text(rest.to_owned())),
            None => on_event(AnswerEvent::Text(text.to_owned())),
        }
        self.sent = text.to_owned();
    }
}

/// The decoded value of a leading `{"answer":"…` in a partial JSON text, up to its closing
/// quote or to the last whole character received.
fn answer_prefix(json: &str) -> Option<String> {
    let rest = json.trim_start().strip_prefix('{')?.trim_start();
    let rest = rest.strip_prefix("\"answer\"")?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let mut chars = rest.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => {
                let decoded = match chars.next() {
                    Some('n') => '\n',
                    Some('t') => '\t',
                    Some('r') => '\r',
                    Some('b') => '\u{8}',
                    Some('f') => '\u{c}',
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        let unit = u16::from_str_radix(&hex, 16)
                            .ok()
                            .filter(|_| hex.len() == 4);
                        let Some(unit) = unit else { break };
                        if (0xD800..0xDC00).contains(&unit) {
                            let low: String = chars.by_ref().skip(2).take(4).collect();
                            let Ok(low) = u16::from_str_radix(&low, 16) else {
                                break;
                            };
                            match char::decode_utf16([unit, low]).next() {
                                Some(Ok(c)) => c,
                                _ => break,
                            }
                        } else {
                            match char::from_u32(u32::from(unit)) {
                                Some(c) => c,
                                None => break,
                            }
                        }
                    }
                    Some(other) => other,
                    None => break,
                };
                out.push(decoded);
            }
            c => out.push(c),
        }
    }
    Some(out)
}

/// Parses an answer in [`answer_schema`]. `found: false` discards the citations (VFS-46 R2).
pub(crate) fn parse_json_answer(text: &str) -> Result<AnswerBlock, AiError> {
    let parsed: JsonAnswer = serde_json::from_str(text)
        .map_err(|_| AiError::BadResponse("answer is not the schema".into()))?;
    let cited = if parsed.found {
        parsed.citations
    } else {
        Vec::new()
    };
    Ok(AnswerBlock {
        text: parsed.answer,
        citations: cited
            .into_iter()
            .map(|c| RawCitation {
                passage: usize::try_from(c.passage_id).unwrap_or(usize::MAX),
                quote: c.quote,
            })
            .collect(),
    })
}

/// The JSON Schema of [`Rewrite`] for strict structured output: every field required,
/// optional ones nullable, no extra properties (both providers' strict mode).
pub(crate) fn rewrite_schema() -> serde_json::Value {
    let list = serde_json::json!({"type": "array", "items": {"type": "string"}});
    let opt = serde_json::json!({"type": ["string", "null"]});
    serde_json::json!({
        "type": "object",
        "properties": {
            "keywords": list, "phrases": list, "synonyms": list,
            "kind": opt, "date_from": opt, "date_to": opt, "folder_hint": opt,
        },
        "required": ["keywords", "phrases", "synonyms", "kind", "date_from", "date_to", "folder_hint"],
        "additionalProperties": false,
    })
}

#[cfg(test)]
mod streamer_tests {
    use super::*;

    /// VFS-46 AC-6: the answer field streams as its JSON arrives, escapes decoded, and the end
    /// sends only what is left.
    #[test]
    fn the_answer_field_streams_as_it_arrives() {
        let full =
            r#"{"answer":"La fianza \"fue\" de 1.200 \u20ac.\nFin","found":true,"citations":[]}"#;
        let mut got = Vec::new();
        let mut s = AnswerStreamer::default();
        for end in 1..=full.len() {
            if full.is_char_boundary(end) {
                s.feed(&full[..end], &mut |AnswerEvent::Text(t)| got.push(t));
            }
        }
        let text = "La fianza \"fue\" de 1.200 €.\nFin";
        s.finish(text, &mut |AnswerEvent::Text(t)| got.push(t));
        assert!(got.len() > 3, "streamed in pieces: {got:?}");
        assert_eq!(got.concat(), text);
    }

    /// Text that does not start with the answer field is sent whole at the end.
    #[test]
    fn other_shapes_are_sent_at_the_end() {
        let mut got = Vec::new();
        let mut s = AnswerStreamer::default();
        s.feed(r#"{"found":true,"answer":"x"#, &mut |AnswerEvent::Text(
            t,
        )| got.push(t));
        s.finish("xy", &mut |AnswerEvent::Text(t)| got.push(t));
        assert_eq!(got, ["xy"]);
    }
}
