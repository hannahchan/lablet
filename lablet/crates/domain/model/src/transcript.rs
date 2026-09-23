//! The transcript: the conversation of one run as a list of turns, which is
//! what a grader or a composer reads.
//!
//! It holds what a trajectory export needs and nothing about any one format.
//! Times are offsets from the start of the run, so an exporter that knows when
//! the run started can place every turn. Against an ATIF trajectory, `system`
//! is the `system` step that opens it, a turn's input, when it has any, is a
//! `user` step, and the rest of each [`Turn`] is one `agent` step with
//! `llm_call_count` 1:
//!
//! | ATIF | From |
//! | --- | --- |
//! | `Step.message` of the `user` step | the text of [`Turn::input`] |
//! | `Step.timestamp` | the run's start plus `record.started_ms` |
//! | `Step.model_name` | `record.response_model` |
//! | `Step.message` | [`Turn::text`] |
//! | `Step.reasoning_content` | the text of the response's thinking blocks |
//! | `Step.tool_calls[]` | the response's [`ToolUse`] blocks: `tool_call_id` is `id`, `function_name` is `name`, `arguments` is `input` |
//! | `Step.observation.results[]` | [`Turn::tool_calls`], in the same order: `source_call_id` is `call_id`, `content` is the text of `content` |
//! | `ObservationResult.extra` | the outcome's `status`, which holds the source, and its `started_ms`, `latency_ms`, `truncated_from_bytes`: ATIF has no slot for an error, a time, or a truncation |
//! | `Step.metrics` | `prompt_tokens` is `usage.input_tokens`, `completion_tokens` is `usage.output_tokens`, `cached_tokens` is `usage.cache_read_tokens`, a subset of the prompt tokens in both |
//! | `Metrics.extra` | `usage.cache_write_tokens`, and the record's `finish`, `response_id`, `latency_ms`, `attempts` |
//!
//! Redacted thinking and opaque blocks are replay material for the provider
//! that sent them and have no place in a trajectory.
//!
//! A [`Turn`] here is one model response. Anthropic's documentation uses
//! "assistant turn" for the whole tool-use loop, so a rule it states per turn,
//! such as where a thinking block must appear, spans several turns of this
//! transcript.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::message::tool_uses;
use crate::whole_ms;
use crate::{
    ContentBlock, FinishReason, Message, ProviderResponse, ToolCallOutcome, ToolResult, ToolUse,
    Usage, UserContent,
};

/// The conversation of one run: the system prompt and the turns.
///
/// A run builds one through its [`crate::Run`], which is the only way one is
/// built: lablet runs a loop and emits what it saw, and reading a transcript
/// back belongs to the side that consumes it. So every `Transcript` holds
/// these rules, because the run's states offer no way to break them:
///
/// - Something from the user comes before every response: a turn has input,
///   or the turn before it has tool call outcomes. So the first turn has
///   input, which is the task prompt, and no two responses are adjacent.
/// - The tool calls of a response have distinct ids.
/// - A turn's outcomes answer exactly the tool calls of its response, each
///   once and in call order, or the turn has no outcomes at all.
/// - Only the last turn may have tool calls and no outcomes.
///
/// A turn without outcomes either called no tools or its tools never ran: the
/// response was cut short or refused, its `task_complete` call was
/// intercepted, or the run was cancelled or reached a limit first. Which of
/// the two it was is read from whether the response holds tool calls.
///
/// A run takes one prompt, so only the first turn of a run's transcript has
/// input. The shape leaves room for a user who speaks again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Transcript {
    system: String,
    turns: Vec<Turn>,
}

/// One model response, with its input, its record, and its tool call outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Turn {
    input: Vec<UserContent>,
    response: Vec<ContentBlock>,
    record: TurnRecord,
    tool_calls: Vec<ToolCallOutcome>,
}

/// What the run knows about the provider call that produced a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRecord {
    /// The tokens the completion used. `input_tokens` includes the cached
    /// tokens; see [`Usage`].
    pub usage: Usage,
    /// Why the model stopped.
    pub finish: FinishReason,
    /// The provider's id for the response.
    pub response_id: Option<String>,
    /// The model that answered, when the provider said.
    pub response_model: Option<String>,
    /// When the attempt that returned the completion started, in whole
    /// milliseconds since the run started.
    pub started_ms: u64,
    /// How long that attempt took, in whole milliseconds.
    pub latency_ms: u64,
    /// How many attempts the provider call took, the one that succeeded
    /// included. The failed ones aren't in the transcript otherwise.
    pub attempts: u32,
}

impl Transcript {
    pub(crate) const fn new(system: String) -> Self {
        Self {
            system,
            turns: Vec::new(),
        }
    }

    /// The system prompt.
    #[must_use]
    pub fn system(&self) -> &str {
        &self.system
    }

    /// Every turn, in order.
    #[must_use]
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// The flat form of the provider call that asks for the next turn, whose
    /// input is `input`. See [`crate::Run::messages`] for the contract.
    pub(crate) fn messages<'a>(&'a self, input: &'a [UserContent]) -> Vec<Message<'a>> {
        let mut messages = Vec::new();
        let mut tool_results = Vec::new();
        for turn in &self.turns {
            push_user(&mut messages, tool_results, &turn.input);
            messages.push(Message::Assistant(&turn.response));
            tool_results = turn
                .tool_calls
                .iter()
                .map(ToolCallOutcome::result)
                .collect();
        }
        push_user(&mut messages, tool_results, input);
        messages
    }

    /// The text of the last turn, which is a run's `result.text`; empty when
    /// the model never answered.
    #[must_use]
    pub fn final_text(&self) -> String {
        self.turns.last().map(Turn::text).unwrap_or_default()
    }

    pub(crate) fn usage(&self) -> Usage {
        self.turns
            .iter()
            .fold(Usage::default(), |sum, turn| sum + turn.record.usage)
    }

    /// Adds `turn` as the last turn.
    pub(crate) fn push(&mut self, turn: Turn) {
        self.turns.push(turn);
    }

    /// How many tool calls returned an error result since the last one that
    /// didn't. A turn without tool calls leaves the count as it was.
    pub(crate) fn consecutive_tool_errors(&self) -> u32 {
        let errors = self
            .turns
            .iter()
            .rev()
            .flat_map(|turn| turn.tool_calls.iter().rev())
            .take_while(|outcome| outcome.status.is_error())
            .count();
        u32::try_from(errors).unwrap_or(u32::MAX)
    }
}

impl Turn {
    /// The turn that `response` makes, answering `input`. The attempt that
    /// returned it started `started` into the run, took `latency`, and was
    /// the last of `attempts`.
    ///
    /// Text blocks that are empty or only whitespace are dropped from both
    /// the input and the response; see [`crate::Run::responded`].
    pub(crate) fn recorded(
        mut input: Vec<UserContent>,
        response: ProviderResponse,
        started: Duration,
        latency: Duration,
        attempts: u32,
    ) -> Self {
        let record = TurnRecord {
            usage: response.usage,
            finish: response.finish,
            response_id: response.response_id,
            response_model: response.response_model,
            started_ms: whole_ms(started),
            latency_ms: whole_ms(latency),
            attempts,
        };
        input.retain(|block| !is_blank(block));
        let mut response = response.content;
        response
            .retain(|block| !matches!(block, ContentBlock::Text(text) if text.trim().is_empty()));
        Self {
            input,
            response,
            record,
            tool_calls: Vec::new(),
        }
    }

    /// Makes `outcomes` the answers to the response's calls.
    pub(crate) fn answered(&mut self, outcomes: Vec<ToolCallOutcome>) {
        self.tool_calls = outcomes;
    }

    /// What the user supplied to the turn: the task prompt for the first
    /// turn, and nothing for a turn that follows tool results.
    #[must_use]
    pub fn input(&self) -> &[UserContent] {
        &self.input
    }

    /// The blocks of the model's response, in the order the provider must see
    /// them again.
    #[must_use]
    pub fn response(&self) -> &[ContentBlock] {
        &self.response
    }

    /// What the run knows about the provider call behind the response.
    #[must_use]
    pub const fn record(&self) -> &TurnRecord {
        &self.record
    }

    /// What happened to the response's tool calls, in call order; empty when
    /// it made none or they never ran.
    #[must_use]
    pub fn tool_calls(&self) -> &[ToolCallOutcome] {
        &self.tool_calls
    }

    /// The tool calls the response makes, in order. When the turn has
    /// outcomes, the n-th outcome answers the n-th call.
    pub fn tool_uses(&self) -> impl Iterator<Item = &ToolUse> {
        tool_uses(&self.response)
    }

    /// The response's [`ContentBlock::Text`] blocks, concatenated in order.
    #[must_use]
    pub fn text(&self) -> String {
        self.response
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.as_str()),
                ContentBlock::Thinking { .. }
                | ContentBlock::RedactedThinking { .. }
                | ContentBlock::ToolUse(_)
                | ContentBlock::Opaque { .. } => None,
            })
            .collect()
    }
}

/// Adds the user message that holds `tool_results` and `input`, unless both
/// are empty: a finished run's last turn may be followed by neither.
fn push_user<'a>(
    messages: &mut Vec<Message<'a>>,
    tool_results: Vec<ToolResult<'a>>,
    input: &'a [UserContent],
) {
    if !tool_results.is_empty() || !input.is_empty() {
        messages.push(Message::User {
            tool_results,
            input,
        });
    }
}

/// Text that's empty or only whitespace carries nothing, so it's not input.
fn is_blank(block: &UserContent) -> bool {
    matches!(block, UserContent::Text(text) if text.trim().is_empty())
}

pub mod document;

#[cfg(test)]
mod tests;
