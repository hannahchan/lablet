//! What a model produced, a user supplied and a tool returned, as a
//! transcript records it and a script states it.

use lablet_model::{self as model, ToolCallId, ToolName};
use serde::{Deserialize, Serialize};

/// One block of a model response: `{"text": "..."}`, `{"thinking": {...}}`,
/// `{"redacted_thinking": {...}}`, `{"tool_use": {...}}` or
/// `{"opaque": {...}}`.
///
/// A key it doesn't know is refused when it's read. A signature may be left
/// out, which is how a block of thinking that carries none is written by
/// hand; it's written as `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentBlock {
    /// Plain text.
    Text(String),
    /// Model reasoning.
    Thinking {
        /// The reasoning text; empty when the provider withheld it.
        text: String,
        /// The provider's signature over the block, when it signed one.
        signature: Option<String>,
    },
    /// Reasoning the provider returned encrypted.
    RedactedThinking {
        /// The encrypted payload.
        data: String,
    },
    /// The model's request to call a tool.
    ToolUse(ToolUse),
    /// Any other block, as its provider sent it.
    Opaque {
        /// The provider that understands the payload.
        provider: ProviderKind,
        /// The block as the provider sent it.
        payload: serde_json::Value,
    },
}

impl From<model::ContentBlock> for ContentBlock {
    fn from(block: model::ContentBlock) -> Self {
        match block {
            model::ContentBlock::Text(text) => Self::Text(text),
            model::ContentBlock::Thinking { text, signature } => Self::Thinking { text, signature },
            model::ContentBlock::RedactedThinking { data } => Self::RedactedThinking { data },
            model::ContentBlock::ToolUse(call) => Self::ToolUse(call.into()),
            model::ContentBlock::Opaque { provider, payload } => Self::Opaque {
                provider: provider.into(),
                payload,
            },
        }
    }
}

impl From<ContentBlock> for model::ContentBlock {
    fn from(block: ContentBlock) -> Self {
        match block {
            ContentBlock::Text(text) => Self::Text(text),
            ContentBlock::Thinking { text, signature } => Self::Thinking { text, signature },
            ContentBlock::RedactedThinking { data } => Self::RedactedThinking { data },
            ContentBlock::ToolUse(call) => Self::ToolUse(call.into()),
            ContentBlock::Opaque { provider, payload } => Self::Opaque {
                provider: provider.into(),
                payload,
            },
        }
    }
}

/// The model's request to call a tool.
///
/// The id and the name are read through the domain's rules for them, so a
/// script that names a tool no provider would accept is refused where it
/// says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolUse {
    /// The call's id, which its outcome answers to.
    #[serde(with = "crate::id::tool_call_id")]
    pub id: ToolCallId,
    /// The tool to call.
    #[serde(with = "crate::id::tool_name")]
    pub name: ToolName,
    /// The arguments the model produced.
    pub input: ToolInput,
}

impl From<model::ToolUse> for ToolUse {
    fn from(call: model::ToolUse) -> Self {
        let model::ToolUse { id, name, input } = call;
        Self {
            id,
            name,
            input: input.into(),
        }
    }
}

impl From<ToolUse> for model::ToolUse {
    fn from(call: ToolUse) -> Self {
        let ToolUse { id, name, input } = call;
        Self {
            id,
            name,
            input: input.into(),
        }
    }
}

/// The arguments of a tool call: `{"json": {...}}` when they parsed, and
/// `{"unparsed": "..."}`, the model's own text, when they didn't.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolInput {
    /// Arguments that parsed.
    Json(serde_json::Value),
    /// Arguments that didn't, as the model wrote them.
    Unparsed(String),
}

impl From<model::ToolInput> for ToolInput {
    fn from(input: model::ToolInput) -> Self {
        match input {
            model::ToolInput::Json(value) => Self::Json(value),
            model::ToolInput::Unparsed(text) => Self::Unparsed(text),
        }
    }
}

impl From<ToolInput> for model::ToolInput {
    fn from(input: ToolInput) -> Self {
        match input {
            ToolInput::Json(value) => Self::Json(value),
            ToolInput::Unparsed(text) => Self::Unparsed(text),
        }
    }
}

/// A provider family, as an opaque block names the one that understands it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// The Anthropic Messages API.
    Anthropic,
    /// OpenAI, and any server that speaks one of its APIs.
    Openai,
    /// The scripted provider.
    Fake,
}

impl From<model::ProviderKind> for ProviderKind {
    fn from(kind: model::ProviderKind) -> Self {
        match kind {
            model::ProviderKind::Anthropic => Self::Anthropic,
            model::ProviderKind::Openai => Self::Openai,
            model::ProviderKind::Fake => Self::Fake,
        }
    }
}

impl From<ProviderKind> for model::ProviderKind {
    fn from(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::Anthropic => Self::Anthropic,
            ProviderKind::Openai => Self::Openai,
            ProviderKind::Fake => Self::Fake,
        }
    }
}

/// One piece of what the user supplied to a turn: `{"text": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UserContent {
    Text(String),
}

impl From<model::UserContent> for UserContent {
    fn from(content: model::UserContent) -> Self {
        match content {
            model::UserContent::Text(text) => Self::Text(text),
        }
    }
}

/// One piece of what the model was sent of a tool's output:
/// `{"text": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolResultContent {
    Text(String),
}

impl From<model::ToolResultContent> for ToolResultContent {
    fn from(content: model::ToolResultContent) -> Self {
        match content {
            model::ToolResultContent::Text(text) => Self::Text(text),
        }
    }
}

#[cfg(test)]
mod tests;
