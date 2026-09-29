//! What a run asks of a model provider and what a provider answers with.

use std::collections::BTreeSet;
use std::num::NonZeroU32;

use crate::message::tool_uses;
use crate::{ContentBlock, ProviderKind, Usage};

/// A model as a run names it, and how the adapter reaches it.
///
/// Two runs that reach one model through different APIs, or that differ in
/// whether its reasoning is sent back, don't hand it the same conversation
/// after the first call. A record that named the provider and the model
/// alone would call them the same.
///
/// The provider isn't a field, because the API decides it
/// ([`ProviderApi::provider`]): a field beside `api` could name a provider
/// that API doesn't belong to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelRef {
    /// The API the adapter speaks to the model's provider.
    pub api: ProviderApi,
    /// The model name sent in requests.
    pub name: String,
    /// Whether the adapter sends the reasoning of earlier responses back on
    /// later calls.
    pub replays_reasoning: bool,
}

/// The API an adapter reaches its provider through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderApi {
    /// Anthropic's Messages API.
    Messages,
    /// OpenAI's Responses API.
    Responses,
    /// OpenAI's chat completions, and every server that speaks them.
    ChatCompletions,
    /// The scripted provider's script.
    Script,
}

impl ProviderApi {
    /// How a run's record spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Messages => "messages",
            Self::Responses => "responses",
            Self::ChatCompletions => "chat_completions",
            Self::Script => "script",
        }
    }

    /// The provider family whose API this is, which is the provider of every
    /// model reached through it.
    #[must_use]
    pub const fn provider(self) -> ProviderKind {
        match self {
            Self::Messages => ProviderKind::Anthropic,
            Self::Responses | Self::ChatCompletions => ProviderKind::Openai,
            Self::Script => ProviderKind::Fake,
        }
    }
}

/// Where a provider's API is served, for `server.address` and `server.port`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
/// The spellings are the `error.type` of a failed `lablet.chat` span. That
/// attribute is an open set in the conventions, so nothing generated can pin
/// them; a unit test does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderErrorKind {
    /// Transport failure, rate limit, 5xx, overloaded, or a per-call timeout.
    Retryable,
    /// The provider rejected the request as longer than the model's context.
    ContextExhausted,
    /// The provider rejected the credentials. Another attempt changes
    /// nothing, as for [`ProviderErrorKind::Fatal`]; it's a kind of its own so
    /// that a failed call says a key was turned away.
    Auth,
    /// A bad request, an unknown model: another attempt changes nothing.
    Fatal,
    /// The adapter couldn't map the payload to the domain model. Retryable,
    /// because a garbled response needn't recur.
    Malformed,
}

impl ProviderErrorKind {
    /// The `error.type` of a failed chat span.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Retryable => "retryable",
            Self::ContextExhausted => "context_exhausted",
            Self::Auth => "auth",
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
/// the only way one does: every spelling either provider API uses for a known
/// reason gives that reason, and only a string that spells none of them is
/// kept as [`FinishReason::Other`]. A reason is written down as its
/// [`FinishReason::as_str`], which reads back as the same reason.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    /// How a reason is written down: lablet's name for a known reason, the
    /// provider's own string for another.
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

/// One successful provider call: the response and what the provider said
/// about it.
///
/// A completion's tool calls have distinct ids, because an outcome couldn't
/// otherwise say which call it answers. [`ProviderResponse::new`] refuses a
/// repeated id, and `content` isn't public, so no completion breaks the rule:
/// an adapter reports the error as a malformed response.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq)]
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
    /// Which runs may share what the provider caches of this run's requests.
    pub cache_scope: CacheScope,
}

/// Which runs share what a provider caches of a run's requests.
///
/// A run that reads what another wrote costs less and answers sooner than it
/// would have alone, so what it measures depends on which run came before
/// it. [`CacheScope::Run`] keeps runs apart that are compared on either.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CacheScope {
    /// Every run that sends the same prefix shares what's cached of it.
    #[default]
    Shared,
    /// Each request carries the run id as its cache key, for the adapter to
    /// use as its API allows.
    Run,
}

impl CacheScope {
    /// How a run's record spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Run => "run",
        }
    }
}

/// How the model is asked to reason before it answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effort {
    /// The least effort.
    Low,
    /// Moderate effort.
    Medium,
    /// High effort.
    High,
    /// Above high. One word where it's written, as the config spells it.
    XHigh,
    /// The most effort the provider offers.
    Max,
}

impl Effort {
    /// The `gen_ai.request.reasoning.level` value.
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

display_as_str!(
    ProviderApi,
    ProviderErrorKind,
    FinishReason,
    CacheScope,
    Effort
);

#[cfg(test)]
mod tests;
