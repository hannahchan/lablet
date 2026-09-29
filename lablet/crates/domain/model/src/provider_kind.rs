//! Which provider family serves a run.
//!
//! Apart from the request and the response because the conversation names it
//! too: a [`crate::ContentBlock::Opaque`] carries the provider that
//! understands its payload, and content can't depend on the provider protocol
//! when the protocol is built from content.

use serde::Serialize;

/// The provider families lablet has an adapter for.
///
/// It serialises because a [`crate::ContentBlock::Opaque`] holds one and the
/// loop measures a request by serialising its messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// The Anthropic Messages API.
    Anthropic,
    /// OpenAI, and any server that speaks one of its APIs.
    Openai,
    /// The scripted provider.
    Fake,
}

impl ProviderKind {
    /// How a run's record spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Fake => "fake",
        }
    }
}

display_as_str!(ProviderKind);

#[cfg(test)]
mod tests;
