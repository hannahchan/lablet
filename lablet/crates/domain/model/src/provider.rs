//! What a run asks of a model provider and what a provider answers with.

use std::num::NonZeroU32;
use std::ops::{Add, AddAssign};

use serde::{Deserialize, Serialize};

use crate::conversation::{tool_uses, validate};
use crate::{ContentBlock, MessageError, Role, ToolUse};

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
/// is the subtraction. An adapter builds a `Usage` through the constructor
/// named for its provider's convention, [`Usage::from_inclusive`] or
/// [`Usage::from_uncached`], so the addition can't be forgotten.
///
/// The cache fields are zero for a provider that doesn't report them. A field
/// left out when deserialising is zero and a field with any other name is an
/// error, so a misspelt count isn't read as zero; all four are always
/// serialised.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
    /// From a provider whose input count already includes the cached tokens,
    /// as OpenAI-compatible servers report it.
    #[must_use]
    pub const fn from_inclusive(
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
    ) -> Self {
        Self {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
        }
    }

    /// From a provider whose input count leaves the cached tokens out, as
    /// Anthropic reports it: both cache counts are added in.
    #[must_use]
    pub const fn from_uncached(
        uncached_input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
    ) -> Self {
        Self::from_inclusive(
            uncached_input
                .saturating_add(cache_read)
                .saturating_add(cache_write),
            output,
            cache_read,
            cache_write,
        )
    }

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
/// [`FinishReason::from`] is how a provider's string becomes a reason, and
/// serde reads through it: every spelling either provider API uses for a known
/// reason gives that reason, and only a string that spells none of them is
/// kept as [`FinishReason::Other`]. A reason serialises as its
/// [`FinishReason::as_str`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum FinishReason {
    /// The model finished its turn, or reached a stop sequence.
    EndTurn,
    /// The model stopped to have tools called.
    ToolUse,
    /// The output token limit cut the response short.
    MaxTokens,
    /// The response filled the model's context window and was cut short.
    ContextWindow,
    /// The model declined to answer, or a content filter withheld the response.
    Refusal,
    /// Any other reason, as the provider spelled it.
    Other(String),
}

impl FinishReason {
    /// The serde spelling: lablet's name for a known reason, the provider's
    /// own string for another.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::EndTurn => "end_turn",
            Self::ToolUse => "tool_use",
            Self::MaxTokens => "max_tokens",
            Self::ContextWindow => "context_window",
            Self::Refusal => "refusal",
            Self::Other(reason) => reason,
        }
    }
}

impl From<String> for FinishReason {
    /// Reads lablet's spellings and those of the Anthropic and OpenAI APIs.
    fn from(reason: String) -> Self {
        match reason.as_str() {
            "end_turn" | "stop" | "stop_sequence" => Self::EndTurn,
            "tool_use" | "tool_calls" => Self::ToolUse,
            "max_tokens" | "length" => Self::MaxTokens,
            "context_window" | "model_context_window_exceeded" => Self::ContextWindow,
            "refusal" | "content_filter" => Self::Refusal,
            _ => Self::Other(reason),
        }
    }
}

impl From<FinishReason> for String {
    fn from(reason: FinishReason) -> Self {
        reason.as_str().to_owned()
    }
}

/// One successful provider call.
///
/// It holds the content of the assistant message, not a message, because the
/// role can only be the assistant's. Deserialisation validates the content and
/// refuses a field it doesn't know, so a hand-written script that misspells
/// one is an error. The fields are public, so an adapter checks the value it
/// builds with [`Completion::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawCompletion")]
pub struct Completion {
    /// The blocks of the assistant message the model produced.
    pub content: Vec<ContentBlock>,
    /// The tokens the call used.
    pub usage: Usage,
    /// Why the model stopped.
    pub finish: FinishReason,
    /// The provider's id for the response.
    pub response_id: Option<String>,
    /// The model that answered, which may be more specific than the one requested.
    pub response_model: Option<String>,
}

/// What a completion is read from, so that reading one validates it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCompletion {
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Usage,
    finish: FinishReason,
    response_id: Option<String>,
    response_model: Option<String>,
}

impl TryFrom<RawCompletion> for Completion {
    type Error = MessageError;

    fn try_from(raw: RawCompletion) -> Result<Self, MessageError> {
        let completion = Self {
            content: raw.content,
            usage: raw.usage,
            finish: raw.finish,
            response_id: raw.response_id,
            response_model: raw.response_model,
        };
        completion.validate()?;
        Ok(completion)
    }
}

impl Completion {
    /// Checks the content against the rules an assistant message is held to.
    ///
    /// # Errors
    ///
    /// Returns what [`crate::Message::validate`] returns for an assistant
    /// message of this content.
    pub fn validate(&self) -> Result<(), MessageError> {
        validate(Role::Assistant, &self.content)
    }

    /// The tool calls the response makes, in order.
    pub fn tool_uses(&self) -> impl Iterator<Item = &ToolUse> {
        tool_uses(&self.content)
    }
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
    /// The reasoning effort, for providers that take a level.
    pub effort: Option<Effort>,
    /// The sampling seed, for providers that take one.
    pub seed: Option<u64>,
}

/// How the model is asked to reason before it answers.
///
/// Written `"provider_default"`, `"adaptive"`, `{"budget": 2048}`, or
/// `"disabled"`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Thinking {
    /// Nothing is sent, so the provider's default applies.
    #[default]
    ProviderDefault,
    /// The model decides how much to reason.
    Adaptive,
    /// Reasoning is requested with this many tokens to spend on it.
    Budget(NonZeroU32),
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
