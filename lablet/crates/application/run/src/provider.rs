//! The port a model provider implements, and what one call carries.

use std::time::Duration;

use lablet_model::{
    Effort, Endpoint, Message, ModelRef, ProviderErrorKind, ProviderResponse, Thinking, ToolSpec,
};

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
    /// How long the adapter may take before it gives up on this attempt.
    pub deadline: Duration,
}

/// Why a provider call failed: the class the retry policy reads, and the
/// adapter's own words.
///
/// A struct rather than an enum with a payload per variant, so that the class
/// is one field the policy takes and the message is one field telemetry and
/// the outcome take. [`ToolError`](crate::ToolError) has the same shape for
/// the same reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ProviderError {
    /// What kind of failure this is, which decides whether it's tried again.
    pub kind: ProviderErrorKind,
    /// What the adapter says happened. Reaches the outcome's `error` when the
    /// run ends on this failure.
    pub message: String,
}

impl ProviderError {
    /// A failure of `kind`, described by `message`.
    pub fn new(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
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
    /// [`ProviderErrorKind::Retryable`].
    async fn complete(&self, request: ProviderRequest<'_>) -> Result<ProviderResponse, ProviderError>;
}
