//! Which provider family serves a run.
//!
//! Apart from the request and the response because the conversation names it
//! too: a [`crate::ContentBlock::Opaque`] carries the provider that
//! understands its payload, and content can't depend on the provider protocol
//! when the protocol is built from content.

use serde::{Deserialize, Serialize};

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

display_as_str!(ProviderKind);

#[cfg(test)]
mod tests;
