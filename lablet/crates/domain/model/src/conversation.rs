//! The conversation: messages and the content blocks they're made of.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{ProviderKind, ToolCallId, ToolName};

/// Who a message is from. The system prompt isn't a message, and tool results
/// travel in user messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The task prompt, or the results of a turn's tool calls.
    User,
    /// A model response.
    Assistant,
}

/// One message of the conversation.
///
/// [`Message::new`] and deserialisation validate. The fields are public, so a
/// value built or changed field by field hasn't been checked until
/// [`Message::validate`] says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawMessage")]
pub struct Message {
    /// Who the message is from.
    pub role: Role,
    /// The blocks of the message, in the order the provider must see them again.
    pub content: Vec<ContentBlock>,
}

/// What a message is read from, so that reading one validates it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMessage {
    role: Role,
    content: Vec<ContentBlock>,
}

impl TryFrom<RawMessage> for Message {
    type Error = MessageError;

    fn try_from(raw: RawMessage) -> Result<Self, MessageError> {
        Self::new(raw.role, raw.content)
    }
}

/// One block of a message's content.
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
    /// What a tool call returned.
    ToolResult(ToolResult),
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
    /// The call's id, which its result answers to.
    pub id: ToolCallId,
    /// The tool to call.
    pub name: ToolName,
    /// The arguments, as the JSON the model produced.
    pub input: serde_json::Value,
}

impl ToolUse {
    /// The size of the call's input: the byte length of `input` as compact JSON.
    #[must_use]
    pub fn input_bytes(&self) -> u64 {
        self.input.to_string().len() as u64
    }
}

/// What a tool call returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResult {
    /// The id of the [`ToolUse`] this answers.
    pub call_id: ToolCallId,
    /// What the model is shown.
    pub content: Vec<ToolResultContent>,
    /// Whether the call failed. Left out of the input, it reads as `false`;
    /// misspelt, it's an error, not a success.
    #[serde(default)]
    pub is_error: bool,
}

impl ToolResult {
    /// The size of the call's output: the summed byte length of its text.
    #[must_use]
    pub fn content_bytes(&self) -> u64 {
        self.content
            .iter()
            .map(|ToolResultContent::Text(text)| text.len() as u64)
            .fold(0, u64::saturating_add)
    }
}

/// One piece of a tool result. Tool results are text; the type is an enum so
/// that another kind of content can arrive without changing the serde form.
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

/// Why a message is one no provider would accept.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MessageError {
    /// Two tool-use blocks share an id, so a result couldn't say which it answers.
    #[error("tool call id {id:?} is on more than one tool-use block of the message")]
    DuplicateToolUse {
        /// The repeated id.
        id: String,
    },
    /// Two tool-result blocks answer the same call.
    #[error("tool call id {id:?} has more than one tool-result block in the message")]
    DuplicateToolResult {
        /// The repeated id.
        id: String,
    },
    /// A tool-use block sits in a user message.
    #[error("tool-use block {id:?} is in a user message; only the assistant calls tools")]
    ToolUseFromUser {
        /// The id of the misplaced block.
        id: String,
    },
    /// A tool-result block sits in an assistant message.
    #[error("tool-result block {id:?} is in an assistant message; results travel in user messages")]
    ToolResultFromAssistant {
        /// The call id of the misplaced block.
        id: String,
    },
}

impl Message {
    /// Builds a message and validates it.
    ///
    /// # Errors
    ///
    /// Returns what [`Message::validate`] returns.
    pub fn new(role: Role, content: Vec<ContentBlock>) -> Result<Self, MessageError> {
        let message = Self { role, content };
        message.validate()?;
        Ok(message)
    }

    /// Checks the rules both provider APIs hold a message to: tool-use blocks
    /// only from the assistant and with ids unique within the message,
    /// tool-result blocks only from the user and one for each call.
    ///
    /// # Errors
    ///
    /// Returns the [`MessageError`] for the first block, in order, that breaks a rule.
    pub fn validate(&self) -> Result<(), MessageError> {
        validate(self.role, &self.content)
    }

    /// The message's [`ContentBlock::Text`] blocks, concatenated in order;
    /// empty when it has none.
    #[must_use]
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.as_str()),
                ContentBlock::Thinking { .. }
                | ContentBlock::RedactedThinking { .. }
                | ContentBlock::ToolUse(_)
                | ContentBlock::ToolResult(_)
                | ContentBlock::Opaque { .. } => None,
            })
            .collect()
    }

    /// The tool calls the message makes, in order.
    pub fn tool_uses(&self) -> impl Iterator<Item = &ToolUse> {
        tool_uses(&self.content)
    }

    /// The tool results the message carries, in order.
    pub fn tool_results(&self) -> impl Iterator<Item = &ToolResult> {
        self.content.iter().filter_map(|block| match block {
            ContentBlock::ToolResult(result) => Some(result),
            _ => None,
        })
    }

    /// The result that answers `call_id`, if the message holds one.
    #[must_use]
    pub fn tool_result(&self, call_id: &ToolCallId) -> Option<&ToolResult> {
        self.tool_results()
            .find(|result| &result.call_id == call_id)
    }
}

/// The rules of [`Message::validate`], for content that isn't in a message yet.
pub(crate) fn validate(role: Role, content: &[ContentBlock]) -> Result<(), MessageError> {
    let mut uses = BTreeSet::new();
    let mut results = BTreeSet::new();
    for block in content {
        match block {
            ContentBlock::ToolUse(ToolUse { id, .. }) => {
                let id = id.as_str();
                if role != Role::Assistant {
                    return Err(MessageError::ToolUseFromUser { id: id.to_owned() });
                }
                if !uses.insert(id) {
                    return Err(MessageError::DuplicateToolUse { id: id.to_owned() });
                }
            }
            ContentBlock::ToolResult(ToolResult { call_id, .. }) => {
                let id = call_id.as_str();
                if role != Role::User {
                    return Err(MessageError::ToolResultFromAssistant { id: id.to_owned() });
                }
                if !results.insert(id) {
                    return Err(MessageError::DuplicateToolResult { id: id.to_owned() });
                }
            }
            ContentBlock::Text(_)
            | ContentBlock::Thinking { .. }
            | ContentBlock::RedactedThinking { .. }
            | ContentBlock::Opaque { .. } => {}
        }
    }
    Ok(())
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
