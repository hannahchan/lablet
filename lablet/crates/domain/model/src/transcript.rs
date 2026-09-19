//! The transcript: the whole conversation of a run, with what each assistant
//! turn cost.
//!
//! It holds what a trajectory export needs and nothing about any one format.
//! One assistant message is one turn, which a trajectory format calls a step;
//! a turn's tool results are in the user message that follows it and join to
//! its calls by call id; a turn's metrics are its [`TurnRecord`], where prompt
//! tokens are `input_tokens`, completion tokens are `output_tokens`, and
//! cached tokens are `cache_read_tokens`.

use serde::{Deserialize, Serialize};

use crate::{Completion, ContentBlock, FinishReason, Message, Role, ToolCallId, ToolResult, Usage};

/// The conversation of one run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    /// The system prompt.
    pub system: String,
    /// Every message, in order. A slice of it is what a provider call sends.
    pub messages: Vec<Message>,
    /// One record for each assistant message, in order: `turns[n]` describes
    /// the completion that produced the n-th assistant message.
    pub turns: Vec<TurnRecord>,
}

/// What the run knows about the completion behind one assistant message:
/// everything of the [`Completion`] but its content, which is the message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRecord {
    /// The tokens the completion used. `input_tokens` includes the cached
    /// tokens; see [`Usage`].
    pub usage: Usage,
    /// Why the model stopped, so a reader can tell a truncated or refused
    /// turn from a finished one.
    pub finish: FinishReason,
    /// The provider's id for the response.
    pub response_id: Option<String>,
    /// The model that answered, when the provider said.
    pub response_model: Option<String>,
}

/// One assistant turn of a [`Transcript`], joined to its record and its tool results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Turn<'a> {
    /// The assistant message.
    pub message: &'a Message,
    /// The record of the completion that produced it; `None` when the
    /// transcript holds fewer records than assistant messages.
    pub record: Option<&'a TurnRecord>,
    /// The user message that carries the results of this turn's tool calls;
    /// `None` when the turn called no tools or the run ended before they ran.
    pub observation: Option<&'a Message>,
}

impl Transcript {
    /// A transcript that holds only the system prompt.
    #[must_use]
    pub const fn new(system: String) -> Self {
        Self {
            system,
            messages: Vec::new(),
            turns: Vec::new(),
        }
    }

    /// Appends a user message made of `content`.
    pub fn push_user(&mut self, content: Vec<ContentBlock>) {
        self.messages.push(Message {
            role: Role::User,
            content,
        });
    }

    /// Appends the assistant message `completion` holds, and the rest of the
    /// completion as its record. Taking the completion whole keeps `turns` in
    /// step with the assistant messages.
    pub fn push_assistant(&mut self, completion: Completion) {
        self.messages.push(Message {
            role: Role::Assistant,
            content: completion.content,
        });
        self.turns.push(TurnRecord {
            usage: completion.usage,
            finish: completion.finish,
            response_id: completion.response_id,
            response_model: completion.response_model,
        });
    }

    /// Every assistant turn, in order, joined to its record and to the message
    /// that holds its tool results.
    #[must_use]
    pub fn turns(&self) -> Vec<Turn<'_>> {
        let mut records = self.turns.iter();
        let mut turns = Vec::new();
        for (index, message) in self.messages.iter().enumerate() {
            if message.role != Role::Assistant {
                continue;
            }
            let observation = self
                .messages
                .get(index + 1)
                .filter(|next| next.tool_results().next().is_some());
            turns.push(Turn {
                message,
                record: records.next(),
                observation,
            });
        }
        turns
    }

    /// The text of the last assistant message, which is a run's `result.text`;
    /// empty when the model never answered.
    #[must_use]
    pub fn final_text(&self) -> String {
        self.messages
            .iter()
            .rfind(|message| message.role == Role::Assistant)
            .map(Message::text)
            .unwrap_or_default()
    }
}

impl<'a> Turn<'a> {
    /// The result that answers the call `call_id` of this turn, if the tools ran.
    #[must_use]
    pub fn result_of(&self, call_id: &ToolCallId) -> Option<&'a ToolResult> {
        self.observation?.tool_result(call_id)
    }
}

#[cfg(test)]
mod tests;
