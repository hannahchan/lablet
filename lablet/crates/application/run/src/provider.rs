//! The port a model provider implements, and what one call carries.

use std::time::Duration;

use lablet_model::{
    Effort, Endpoint, Message, ModelRef, ProviderErrorKind, ProviderResponse, Thinking, ToolSpec,
    Usage,
};

use crate::bounded;

/// What one provider call asks for. Everything borrows: the messages come
/// from [`lablet_model::Run::messages`], which borrows the run, and the loop
/// renders them inside the attempt because a failed attempt changes it.
#[derive(Debug)]
pub struct ProviderRequest<'a> {
    /// The system prompt, skills already appended.
    pub system: &'a str,
    /// The conversation so far, flattened.
    pub messages: &'a [Message<'a>],
    /// The tools offered to the model.
    pub tools: &'a [ToolSpec],
    /// The cap on output tokens.
    pub max_tokens: u32,
    /// The sampling temperature; `None` leaves the provider's default.
    pub temperature: Option<f64>,
    /// How the model is asked to reason.
    pub thinking: Thinking,
    /// The reasoning effort, for providers that take a level.
    pub effort: Option<Effort>,
    /// The sampling seed, for providers that take one.
    pub seed: Option<i64>,
    /// The run id, when the run's cache scope is
    /// [`CacheScope::Run`](lablet_model::CacheScope::Run), for the adapter to
    /// send as its API allows, so that no other run reads what this one
    /// cached. It's a field of its own because it's no part of `system`: the
    /// transcript's system prompt, its size and its digest are taken from
    /// that, and are the same whatever the cache scope.
    pub cache_key: Option<&'a str>,
    /// How long the adapter may take before it gives up on this attempt:
    /// the shorter of the provider timeout and the time the run has left.
    /// Zero is a deadline the attempt has reached already, never the
    /// absence of one.
    pub deadline: Duration,
}

/// Why a provider call failed: the class and the server's hint, which the
/// retry policy reads, what the attempt used, which the run counts, and the
/// adapter's own words.
///
/// A struct rather than an enum with a payload per variant, so that the class
/// is one field the policy takes and the message is one field telemetry and
/// the outcome take. [`ToolError`](crate::ToolError) has the same shape for
/// the same reason.
///
/// The message is private because [`ProviderError::new`] cuts it to
/// [`ERROR_MESSAGE_MAX_BYTES`](crate::ERROR_MESSAGE_MAX_BYTES), and a field
/// anyone could write would let a message past the cut.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ProviderError {
    /// What kind of failure this is, which decides whether it's tried again.
    pub kind: ProviderErrorKind,
    message: String,
    /// What the provider said the attempt used, when it said. A provider can
    /// bill for a call it didn't finish, so the run counts this toward its
    /// token budget and its cost.
    pub usage: Option<Usage>,
    /// How long the server asked its caller to wait before trying again,
    /// when it asked.
    pub retry_after: Option<Duration>,
}

impl ProviderError {
    /// A failure of `kind`, described by `message`, that reported no usage
    /// and asked for no wait. The message is cut to
    /// [`ERROR_MESSAGE_MAX_BYTES`](crate::ERROR_MESSAGE_MAX_BYTES).
    pub fn new(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: bounded(message.into()),
            usage: None,
            retry_after: None,
        }
    }

    /// The same failure, of an attempt the provider said used `usage`.
    #[must_use]
    pub const fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// The same failure, from a server that asked for `wait` before the next
    /// attempt.
    #[must_use]
    pub const fn with_retry_after(mut self, wait: Duration) -> Self {
        self.retry_after = Some(wait);
        self
    }

    /// What the adapter says happened. Reaches the outcome's `error` when the
    /// run ends on this failure.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A model provider, as the loop uses one.
///
/// The model and the endpoint are asked for once, when the run starts, and
/// reported on the wide event; an adapter that isn't reached over the network
/// has no endpoint.
#[async_trait::async_trait]
pub trait ModelProvider: Send + Sync {
    /// The model this provider serves.
    fn model(&self) -> &ModelRef;

    /// Where the API is served, for `server.address` and `server.port`.
    fn endpoint(&self) -> Option<Endpoint>;

    /// One attempt of one provider call.
    ///
    /// # Errors
    ///
    /// Returns a [`ProviderError`] whose kind says whether another attempt
    /// could answer differently. An adapter enforces `request.deadline`
    /// itself, in real time, and reports reaching it as
    /// [`ProviderErrorKind::Retryable`]. The error carries the usage the
    /// provider reported for the attempt and the wait the server asked for,
    /// whenever the provider gave either.
    ///
    /// Its message holds no credentials: no user info or query string of a
    /// URL, no header value, and a response body only when the run captures
    /// content. The type holds the message's length and nothing can hold
    /// this, so it's the adapter's obligation.
    async fn complete(
        &self,
        request: ProviderRequest<'_>,
    ) -> Result<ProviderResponse, ProviderError>;
}
