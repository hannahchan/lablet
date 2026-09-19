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
/// The fields are public, so a value built field by field or deserialised
/// hasn't been checked: [`Message::new`] and [`Message::validate`] are the
/// checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Who the message is from.
    pub role: Role,
    /// The blocks of the message, in the order the provider must see them again.
    pub content: Vec<ContentBlock>,
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
        /// The provider's signature over the block; empty for a provider that signs nothing.
        signature: String,
    },
    /// Reasoning the provider returned encrypted, replayed unchanged.
    RedactedThinking {
        /// The encrypted payload.
        data: String,
    },
    /// The model's request to call a tool.
    ToolUse {
        /// The call's id, which its result answers to.
        id: ToolCallId,
        /// The tool to call.
        name: ToolName,
        /// The arguments, as the JSON the model produced.
        input: serde_json::Value,
    },
    /// What a tool call returned.
    ToolResult {
        /// The id of the [`ContentBlock::ToolUse`] this answers.
        call_id: ToolCallId,
        /// What the model is shown.
        content: Vec<ToolResultContent>,
        /// Whether the call failed.
        is_error: bool,
    },
    /// Any other provider-specific block, replayed unchanged to that provider.
    Opaque {
        /// The provider that understands the payload.
        provider: ProviderKind,
        /// The block as the provider sent it.
        payload: serde_json::Value,
    },
}

/// One piece of a tool result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultContent {
    /// Text output.
    Text(String),
    /// Structured output.
    Json(serde_json::Value),
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
        let mut uses = BTreeSet::new();
        let mut results = BTreeSet::new();
        for block in &self.content {
            match block {
                ContentBlock::ToolUse { id, .. } => {
                    let id = id.as_str();
                    if self.role != Role::Assistant {
                        return Err(MessageError::ToolUseFromUser { id: id.to_owned() });
                    }
                    if !uses.insert(id) {
                        return Err(MessageError::DuplicateToolUse { id: id.to_owned() });
                    }
                }
                ContentBlock::ToolResult { call_id, .. } => {
                    let id = call_id.as_str();
                    if self.role != Role::User {
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
                | ContentBlock::ToolUse { .. }
                | ContentBlock::ToolResult { .. }
                | ContentBlock::Opaque { .. } => None,
            })
            .collect()
    }

    /// The [`ContentBlock::ToolResult`] block that answers `call_id`, if the
    /// message holds one.
    #[must_use]
    pub fn tool_result(&self, call_id: &ToolCallId) -> Option<&ContentBlock> {
        self.content.iter().find(
            |block| matches!(block, ContentBlock::ToolResult { call_id: id, .. } if id == call_id),
        )
    }
}

#[cfg(test)]
mod tests;
