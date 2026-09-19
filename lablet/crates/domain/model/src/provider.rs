//! What a run asks of a model provider and what a provider answers with.

use std::ops::{Add, AddAssign};

use serde::{Deserialize, Serialize};

use crate::Message;

/// The provider families lablet has an adapter for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// The Anthropic Messages API.
    Anthropic,
    /// Any server that speaks OpenAI chat completions.
    Openai,
    /// The scripted provider.
    Fake,
}

impl ProviderKind {
    /// The serde spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Fake => "fake",
        }
    }
}

/// A model as a run names it: the provider and the provider's name for the model.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRef {
    /// The provider that serves the model.
    pub provider: ProviderKind,
    /// The model name sent in requests.
    pub name: String,
}

/// Where a provider's API is served, for `server.address` and `server.port`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Endpoint {
    /// The host name or address.
    pub host: String,
    /// The port.
    pub port: u16,
}

/// Token counts of one completion, or the sum over several.
///
/// `input_tokens` is the whole prompt and **includes** the cached tokens:
/// `cache_read_tokens` and `cache_write_tokens` are subsets of it, as the
/// GenAI semantic conventions count them. Adding a cache field to
/// `input_tokens` counts those tokens twice; [`Usage::uncached_input_tokens`]
/// is the subtraction. An adapter whose provider reports uncached input on its
/// own adds the cache counts in before it builds a `Usage`.
///
/// The cache fields are zero for a provider that doesn't report them. A field
/// left out when deserialising is zero; all four are always serialised.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    /// Every token of the prompt, cached or not.
    pub input_tokens: u64,
    /// Every token the model generated, reasoning included.
    pub output_tokens: u64,
    /// The part of `input_tokens` served from the provider's prompt cache.
    pub cache_read_tokens: u64,
    /// The part of `input_tokens` written to the provider's prompt cache.
    pub cache_write_tokens: u64,
}

impl Usage {
    /// `input_tokens + output_tokens`, the number a token budget counts. The
    /// cache fields aren't added, because `input_tokens` already holds them.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// The input tokens that touched no cache: `input_tokens` less both cache fields.
    #[must_use]
    pub const fn uncached_input_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_sub(self.cache_read_tokens)
            .saturating_sub(self.cache_write_tokens)
    }
}

impl Add for Usage {
    type Output = Self;

    /// Field by field, saturating, so a sum never panics.
    fn add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            cache_read_tokens: self
                .cache_read_tokens
                .saturating_add(other.cache_read_tokens),
            cache_write_tokens: self
                .cache_write_tokens
                .saturating_add(other.cache_write_tokens),
        }
    }
}

impl AddAssign for Usage {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

/// Why the model stopped generating, normalised across providers.
///
/// A known reason serialises as its `snake_case` name and [`FinishReason::Other`]
/// as the provider's own string, so a string that spells a known reason always
/// deserialises to that reason.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// The model finished its turn.
    EndTurn,
    /// The model stopped to have tools called.
    ToolUse,
    /// The output token limit cut the response short.
    MaxTokens,
    /// Any other reason, as the provider spelled it.
    #[serde(untagged)]
    Other(String),
}

impl FinishReason {
    /// The serde spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::EndTurn => "end_turn",
            Self::ToolUse => "tool_use",
            Self::MaxTokens => "max_tokens",
            Self::Other(reason) => reason,
        }
    }
}

/// One successful provider call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    /// The assistant message the model produced.
    pub message: Message,
    /// The tokens the call used.
    pub usage: Usage,
    /// Why the model stopped.
    pub finish: FinishReason,
    /// The provider's id for the response.
    pub response_id: Option<String>,
    /// The model that answered, which may be more specific than the one requested.
    pub response_model: Option<String>,
}

/// An amount of money in US dollars.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cost(f64);

impl Cost {
    /// Wraps an amount in US dollars.
    #[must_use]
    pub const fn new(usd: f64) -> Self {
        Self(usd)
    }

    /// The amount in US dollars.
    #[must_use]
    pub const fn usd(self) -> f64 {
        self.0
    }
}

/// The request parameters every provider call of a run shares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestDefaults {
    /// The cap on output tokens for each call.
    pub max_tokens: u32,
    /// The sampling temperature; `None` leaves the provider's default.
    pub temperature: Option<f32>,
    /// How the model is asked to reason.
    pub thinking: Thinking,
    /// The sampling seed, for providers that take one.
    pub seed: Option<u64>,
}

/// How the model is asked to reason before it answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Thinking {
    /// Whether reasoning is requested, refused, or left to the provider.
    pub mode: ThinkingMode,
    /// The reasoning token budget, for [`ThinkingMode::Enabled`].
    pub budget: Option<u32>,
    /// The reasoning effort, for providers that take a level.
    pub effort: Option<Effort>,
}

/// Whether reasoning is requested.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingMode {
    /// Nothing is sent, so the provider's default applies.
    #[default]
    Default,
    /// Reasoning is requested.
    Enabled,
    /// Reasoning is refused.
    Disabled,
}

/// A reasoning effort level, reported as `gen_ai.request.reasoning.level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    /// The least effort.
    Low,
    /// Moderate effort.
    Medium,
    /// High effort.
    High,
    /// Above high. One word on the wire, as the config spells it.
    #[serde(rename = "xhigh")]
    XHigh,
    /// The most effort the provider offers.
    Max,
}

impl Effort {
    /// The serde spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

display_as_str!(ProviderKind, FinishReason, Effort);

#[cfg(test)]
mod tests;
