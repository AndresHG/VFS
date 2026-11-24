//! Ask your archive (VFS-28): the model providers, everything around a priced call, and the ask
//! flow.
//!
//! - [`ChatModel`]: `validate_key`, `rewrite` and a streamed `answer`, for Anthropic and OpenAI,
//!   over blocking `reqwest` (D10).
//! - [`ask`]: the ask flow (28c): prompts, retrieval rounds, passages, citation checks,
//!   follow-ups and the R14 diagnostics.
//! - [`ApiKey`]: the redacting secret type; [`Keys`]: the OS credential store with the
//!   protected-file and session-only fallbacks (R2).
//! - [`AiError`]: provider errors mapped to the kinds of R13, with retries.
//! - [`cost`]: the dated model and price table (R10, D7).
//! - [`AiDb`]: `ai.db`, the record of what was sent and the monthly totals (R10, R11).
//!
//! Only [`ask`] touches content search (VFS-2), through its [`ask::Corpus`] trait.

mod anthropic;
pub mod ask;
pub mod cost;
mod db;
mod error;
mod http;
mod keys;
#[cfg(feature = "test-support")]
pub mod mock;
mod openai;
mod provider;
mod secret;

pub use cost::{cost_nano, models_for, Models, Usage};
pub use db::{
    AiDb, CallKind, CallRecord, CapStatus, SentPassage, SentRecord, StoredRecord, DEFAULT_CAP_NANO,
};
pub use error::AiError;
pub use http::{RetryPolicy, RetryWait, SleepWait, ANTHROPIC_VERSION};
pub use keys::{CredentialBackend, Fallback, KeyError, KeyLocation, Keys, OsStore, StoreError};
pub use provider::{
    chat_model, Answer, AnswerBlock, AnswerEvent, AnswerRequest, CancelToken, ChatModel, Endpoint,
    Passage, Provider, RawCitation, Rewrite, RewriteRequest, Turn, ANSWER_MAX_TOKENS,
    REWRITE_MAX_TOKENS,
};
pub use secret::ApiKey;
