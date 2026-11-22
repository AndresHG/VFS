//! The `ChatModel` trait and the data it exchanges with the caller (VFS-28 R4, R6, R7, R9).

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

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
