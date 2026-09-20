//! The content a user supplies, a model produces, and a tool returns, and the
//! flat message form a provider call sends.

use serde::{Deserialize, Serialize};

use crate::{ProviderKind, ToolCallId, ToolName};

/// One piece of what the user supplies to a turn. It's text; the type is an
/// enum so that another kind of content can arrive without changing the serde
/// form, and so that a turn's input can hold nothing only a model produces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserContent {
    /// Text input, such as the task prompt.
    Text(String),
}

impl UserContent {
    pub(crate) fn bytes(content: &[Self]) -> u64 {
        content
            .iter()
            .map(|Self::Text(text)| text.len() as u64)
            .fold(0, u64::saturating_add)
    }
}

/// One block of a model response.
///
/// A tool result isn't one: results are [`crate::ToolCallOutcome`]s of a turn
/// and reach a provider in a [`Message::User`], so no response can hold one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text.
    Text(String),
    /// Model reasoning, replayed unchanged because the provider verifies it.
    Thinking {
        /// The reasoning text; empty when the provider withholds it.
        text: String,
        /// The provider's signature over the block; `None` for a provider that signs nothing.
        signature: Option<String>,
    },
    /// Reasoning the provider returned encrypted, replayed unchanged.
    RedactedThinking {
        /// The encrypted payload.
        data: String,
    },
    /// The model's request to call a tool.
    ToolUse(ToolUse),
    /// Any other provider-specific block, replayed unchanged to that provider.
    Opaque {
        /// The provider that understands the payload.
        provider: ProviderKind,
        /// The block as the provider sent it.
        payload: serde_json::Value,
    },
}

/// The model's request to call a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolUse {
    /// The call's id, which its outcome answers to.
    pub id: ToolCallId,
    /// The tool to call.
    pub name: ToolName,
    /// The arguments the model produced.
    pub input: ToolInput,
}

/// The arguments of a tool call, which the model doesn't always get right.
///
/// Text that isn't JSON is kept as the model wrote it rather than refused,
/// because the answer is to show the model its own output and let it try
/// again: the call is in the transcript, it counts in the tool statistics, and
/// its outcome is [`crate::ToolCallStatus::MalformedInput`]. Refusing the
/// whole response instead spends a retry re-rolling the same prompt, and
/// reports a tool-surface problem as provider flakiness.
///
/// Written `{"json": {...}}` or `{"unparsed": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolInput {
    /// Arguments that parsed.
    Json(serde_json::Value),
    /// Arguments that didn't, as the model wrote them.
    Unparsed(String),
}

impl ToolUse {
    /// The size of the call's input: the byte length of the arguments as the
    /// provider would carry them, which is compact JSON when they parsed and
    /// the model's own text when they didn't.
    #[must_use]
    pub fn input_bytes(&self) -> u64 {
        match &self.input {
            ToolInput::Json(value) => value.to_string().len() as u64,
            ToolInput::Unparsed(text) => text.len() as u64,
        }
    }
}

/// One piece of what a tool returned. Tool results are text; the type is an
/// enum so that another kind of content can arrive without changing the serde
/// form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultContent {
    /// Text output.
    Text(String),
}

impl ToolResultContent {
    /// The text that stands in for content a tool returned and lablet doesn't
    /// carry, such as an image: `[image omitted: image/png, 48213 bytes]`.
    /// Every executor words it here, so the model and a reader of transcripts
    /// see one form.
    #[must_use]
    pub fn omitted(kind: &str, mime_type: &str, bytes: u64) -> Self {
        Self::Text(format!("[{kind} omitted: {mime_type}, {bytes} bytes]"))
    }
}

/// One message of the flat form a provider call sends, borrowed from the
/// [`crate::Transcript`] that renders it.
///
/// The variant is the role, so an adapter maps each to its own wire form and
/// no message can hold a block its role may not send. Serialising one gives
/// the size the loop measures as request bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Message<'a> {
    /// From the user, before a turn's response: the results of the tool calls
    /// of the turn before, in call order, then the turn's input. At least one
    /// of the two isn't empty. Anthropic takes both as the blocks of one user
    /// message, results first; an OpenAI-compatible server takes one `tool`
    /// message for each result and then a user message for the input.
    User {
        /// The results of the previous turn's tool calls.
        tool_results: Vec<ToolResult<'a>>,
        /// What the user supplied to the turn.
        input: &'a [UserContent],
    },
    /// A model response.
    Assistant(&'a [ContentBlock]),
}

/// What a tool call returned, as the model is sent it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ToolResult<'a> {
    /// The id of the [`ToolUse`] this answers.
    pub call_id: &'a ToolCallId,
    /// What the model is shown.
    pub content: &'a [ToolResultContent],
    /// Whether the call failed, which is whether its status is anything but `ok`.
    pub is_error: bool,
}

/// The tool calls among `content`, in order.
pub(crate) fn tool_uses(content: &[ContentBlock]) -> impl Iterator<Item = &ToolUse> {
    content.iter().filter_map(|block| match block {
        ContentBlock::ToolUse(tool_use) => Some(tool_use),
        _ => None,
    })
}

#[cfg(test)]
mod tests;
