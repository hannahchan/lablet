//! What a run asks of a model provider and what a provider answers with.

use std::collections::BTreeSet;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::message::tool_uses;
use crate::{ContentBlock, Usage};

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

/// Why a provider call failed, as far as a policy reads it. The adapter
/// classifies its own error and carries the message; this is the part the
/// domain decides on.
///
/// The four spellings are the `error.type` of a failed `lablet.chat` span.
/// That attribute is an open set in the conventions, so nothing generated can
/// pin them; a unit test does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    /// Transport failure, rate limit, 5xx, overloaded, or a per-call timeout.
    Retryable,
    /// The provider rejected the request as longer than the model's context.
    ContextExhausted,
    /// Auth, a bad request, an unknown model: another attempt changes nothing.
    Fatal,
    /// The adapter couldn't map the payload to the domain model. Retryable,
    /// because a garbled response needn't recur.
    Malformed,
}

impl ProviderErrorKind {
    /// The serde spelling, which is the span's `error.type`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Retryable => "retryable",
            Self::ContextExhausted => "context_exhausted",
            Self::Fatal => "fatal",
            Self::Malformed => "malformed",
        }
    }

    /// Whether another attempt could answer differently.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Retryable | Self::Malformed)
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
    Other(UnknownReason),
}

/// A finish reason lablet has no name for, kept as the provider spelled it.
///
/// Its string is private so that [`FinishReason::from`] is the only way to
/// build one. Otherwise `Other("refusal".to_owned())` would typecheck, and a
/// refusal that skipped normalisation reads at the stop policy as an ordinary
/// end: the run would complete, and exit 0, on a response the model declined
/// to give.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnknownReason(String);

impl UnknownReason {
    /// The reason as the provider spelled it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
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
            Self::Other(reason) => reason.as_str(),
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
            _ => Self::Other(UnknownReason(reason)),
        }
    }
}

impl From<FinishReason> for String {
    fn from(reason: FinishReason) -> Self {
        reason.as_str().to_owned()
    }
}

/// One successful provider call: the response and what the provider said
/// about it.
///
/// A completion's tool calls have distinct ids, because an outcome couldn't
/// otherwise say which call it answers. [`ProviderResponse::new`] and
/// deserialisation both refuse a repeated id, and `content` isn't public, so
/// no completion breaks the rule: an adapter reports the error as a malformed
/// response. Deserialisation also refuses a field it doesn't know, so a
/// hand-written script that misspells one is an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawProviderResponse")]
pub struct ProviderResponse {
    pub(crate) content: Vec<ContentBlock>,
    /// The tokens the call used.
    pub usage: Usage,
    /// Why the model stopped.
    pub finish: FinishReason,
    /// The provider's id for the response.
    pub response_id: Option<String>,
    /// The model that answered, which may be more specific than the one requested.
    pub response_model: Option<String>,
}

/// What a completion is read from, so that reading one checks it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProviderResponse {
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Usage,
    finish: FinishReason,
    response_id: Option<String>,
    response_model: Option<String>,
}

impl TryFrom<RawProviderResponse> for ProviderResponse {
    type Error = ResponseError;

    fn try_from(raw: RawProviderResponse) -> Result<Self, ResponseError> {
        Self::new(
            raw.content,
            raw.usage,
            raw.finish,
            raw.response_id,
            raw.response_model,
        )
    }
}

/// Why content can't be the response of a completion.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResponseError {
    /// Two tool-use blocks share an id, so an outcome couldn't say which it answers.
    #[error("tool call id {id:?} is on more than one tool-use block of the response")]
    DuplicateToolUse {
        /// The repeated id.
        id: String,
    },
}

impl ProviderResponse {
    /// A completion whose response is `content`.
    ///
    /// # Errors
    ///
    /// Returns [`ResponseError::DuplicateToolUse`] for the first tool-use
    /// block, in order, whose id an earlier one has.
    pub fn new(
        content: Vec<ContentBlock>,
        usage: Usage,
        finish: FinishReason,
        response_id: Option<String>,
        response_model: Option<String>,
    ) -> Result<Self, ResponseError> {
        distinct_tool_use_ids(&content)?;
        Ok(Self {
            content,
            usage,
            finish,
            response_id,
            response_model,
        })
    }

    /// The blocks of the response, in the order the provider sent them.
    #[must_use]
    pub fn content(&self) -> &[ContentBlock] {
        &self.content
    }
}

/// Refuses content in which two tool-use blocks share an id.
pub(crate) fn distinct_tool_use_ids(content: &[ContentBlock]) -> Result<(), ResponseError> {
    let mut seen = BTreeSet::new();
    for call in tool_uses(content) {
        if !seen.insert(&call.id) {
            return Err(ResponseError::DuplicateToolUse {
                id: call.id.as_str().to_owned(),
            });
        }
    }
    Ok(())
}

/// The request parameters every provider call of a run shares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestParams {
    /// The cap on output tokens for each call.
    pub max_tokens: u32,
    /// The sampling temperature; `None` leaves the provider's default. An
    /// `f64` because that's what telemetry carries: an `f32` of 0.7 widens to
    /// 0.699999988.
    pub temperature: Option<f64>,
    /// How the model is asked to reason.
    pub thinking: Thinking,
    /// The reasoning effort, for providers that take a level.
    pub effort: Option<Effort>,
    /// The sampling seed, for providers that take one. An `i64` because
    /// that's what `gen_ai.request.seed` carries; a `u64` above `i64::MAX`
    /// would reach telemetry as some other number.
    pub seed: Option<i64>,
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

display_as_str!(ProviderKind, ProviderErrorKind, FinishReason, Effort);

#[cfg(test)]
mod tests;
