//! The dated model and price table, and the cost of a call (VFS-28 R10, D7).
//!
//! Costs are integer nano-dollars (1e-9 USD): every listed price is a whole number of
//! nano-dollars per token, so sums are exact.

use serde::{Deserialize, Serialize};

use crate::provider::Provider;

/// The date the prices and model ids below were read from the providers' pages. Updated with
/// releases (D7).
pub const PRICES_AS_OF: &str = "2026-11-22";

/// One nano-dollar is 1e-9 USD.
pub const NANO_PER_USD: i64 = 1_000_000_000;

/// Model ids, as listed by the providers on [`PRICES_AS_OF`].
pub const CLAUDE_HAIKU: &str = "claude-haiku-5-5";
pub const CLAUDE_SONNET: &str = "claude-sonnet-5-5";
pub const GPT_LUNA: &str = "gpt-6-luna";
pub const GPT_SOL: &str = "gpt-6.2-sol";

/// The models used for each call (D7): the cheap tier rewrites, the mid tier answers, and
/// «Respuestas más baratas» answers with the cheap tier too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Models {
    pub rewrite: &'static str,
    pub answer: &'static str,
    pub cheap_answer: &'static str,
}

/// The models of `provider` (D7). Anthropic ids are from Anthropic's model list; OpenAI's from
/// its pricing page, both as of [`PRICES_AS_OF`].
pub fn models_for(provider: Provider) -> Models {
    match provider {
        Provider::Anthropic => Models {
            rewrite: CLAUDE_HAIKU,
            answer: CLAUDE_SONNET,
            cheap_answer: CLAUDE_HAIKU,
        },
        Provider::OpenAi => Models {
            rewrite: GPT_LUNA,
            answer: GPT_SOL,
            cheap_answer: GPT_LUNA,
        },
    }
}

/// List prices in nano-dollars per token (= USD per million tokens × 1,000).
struct Price {
    model: &'static str,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}

/// Standard-tier list prices as of [`PRICES_AS_OF`]. Haiku 5.5 doubles its price above 100 k
/// prompt tokens; a question sends at most 10 merged passages (R6), far below that, so the
/// long-context rate is not modelled. Anthropic cache writes are 1.25× input; OpenAI lists no
/// cache-write price, so its writes are priced as input.
const PRICES: &[Price] = &[
    Price {
        model: CLAUDE_HAIKU,
        input: 100,
        output: 500,
        cache_read: 10,
        cache_write: 125,
    },
    Price {
        model: CLAUDE_SONNET,
        input: 2_000,
        output: 10_000,
        cache_read: 100,
        cache_write: 2_500,
    },
    Price {
        model: GPT_LUNA,
        input: 100,
        output: 500,
        cache_read: 10,
        cache_write: 100,
    },
    Price {
        model: GPT_SOL,
        input: 2_000,
        output: 10_000,
        cache_read: 100,
        cache_write: 2_000,
    },
];

/// Token counts of one call, from the provider's `usage` fields. `input_tokens` excludes the
/// cached input, which is counted in the cache fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl Usage {
    /// The usage a cancelled stream is priced at (R10): the tokens reported, but with output at
    /// `max_tokens` if no output count arrived.
    pub fn for_cancelled(reported: Option<Usage>, max_tokens: u32) -> Usage {
        let mut usage = reported.unwrap_or_default();
        if usage.output_tokens == 0 {
            usage.output_tokens = u64::from(max_tokens);
        }
        usage
    }
}

/// The cost of one call in nano-dollars (R10). A model missing from the table is priced at the
/// table's highest price for each kind of token.
pub fn cost_nano(model: &str, usage: &Usage) -> i64 {
    let price = PRICES.iter().find(|p| p.model == model);
    let pick = |f: fn(&Price) -> i64| match price {
        Some(p) => f(p),
        None => PRICES.iter().map(f).max().unwrap_or(0),
    };
    let tokens = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
    tokens(usage.input_tokens)
        .saturating_mul(pick(|p| p.input))
        .saturating_add(tokens(usage.output_tokens).saturating_mul(pick(|p| p.output)))
        .saturating_add(tokens(usage.cache_read_tokens).saturating_mul(pick(|p| p.cache_read)))
        .saturating_add(tokens(usage.cache_write_tokens).saturating_mul(pick(|p| p.cache_write)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every model the providers use is in the price table.
    #[test]
    fn every_used_model_has_a_price() {
        for p in [Provider::Anthropic, Provider::OpenAi] {
            let m = models_for(p);
            for id in [m.rewrite, m.answer, m.cheap_answer] {
                assert!(PRICES.iter().any(|x| x.model == id), "{id} has no price");
            }
        }
    }
}
