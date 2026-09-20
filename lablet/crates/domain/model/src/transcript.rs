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
//! | `ObservationResult.extra` | the outcome's `source`, `status`, `started_ms`, `latency_ms`, `truncated_from_bytes`: ATIF has no slot for an error, a time, or a truncation |
//! | `Step.metrics` | `prompt_tokens` is `usage.input_tokens`, `completion_tokens` is `usage.output_tokens`, `cached_tokens` is `usage.cache_read_tokens`, a subset of the prompt tokens in both |
//! | `Metrics.extra` | `usage.cache_write_tokens`, and the record's `finish`, `response_id`, `latency_ms`, `attempts` |
//!
//! Redacted thinking and opaque blocks are replay material for the provider
//! that sent them and have no place in a trajectory.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::message::tool_uses;
use crate::provider::distinct_tool_use_ids;
use crate::run::whole_ms;
use crate::{
    Completion, CompletionError, ContentBlock, FinishReason, Message, ToolCallId, ToolCallOutcome,
    ToolCallStatus, ToolResult, ToolUse, Usage, UserContent,
};

/// The conversation of one run: the system prompt and the turns.
///
/// A run builds one through its [`crate::RunTally`], and reading one from its
/// serde form goes through the same steps, so every `Transcript` holds these
/// rules:
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawTranscript")]
pub struct Transcript {
    system: String,
    turns: Vec<Turn>,
}

/// What a transcript is read from, so that reading one holds it to the rules.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTranscript {
    system: String,
    turns: Vec<RawTurn>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTurn {
    input: Vec<UserContent>,
    response: Vec<ContentBlock>,
    record: TurnRecord,
    tool_calls: Vec<ToolCallOutcome>,
}

impl TryFrom<RawTranscript> for Transcript {
    type Error = TranscriptError;

    fn try_from(raw: RawTranscript) -> Result<Self, TranscriptError> {
        let mut transcript = Self::new(raw.system);
        for mut turn in raw.turns {
            distinct_tool_use_ids(&turn.response)?;
            transcript.push(&mut turn.input, turn.response, turn.record)?;
            if !turn.tool_calls.is_empty() {
                transcript.answer(turn.tool_calls)?;
            }
        }
        Ok(transcript)
    }
}

/// What the user supplied, the model's response to it, what the run knows
/// about the provider call behind the response, and what happened to the tool
/// calls it made.
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
    /// Why the model stopped, so a reader can tell a truncated or refused
    /// turn from a finished one.
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

/// Why a transcript can't take what it was given, or can't be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TranscriptError {
    /// A response in a transcript being read breaks the rule of a completion.
    #[error(transparent)]
    Response(#[from] CompletionError),
    /// A turn would put its response straight after another, or first of all.
    #[error(
        "turn {turn} has no input and no tool results come before it, so nothing from the user would precede its response"
    )]
    NothingFromTheUser {
        /// The turn that was refused, counted from 1.
        turn: usize,
    },
    /// A turn would follow one whose tool calls nothing answered.
    #[error(
        "turn {turn} made the tool calls {calls:?} and has no outcomes, which only the last turn may"
    )]
    UnansweredCalls {
        /// The turn whose calls went unanswered, counted from 1.
        turn: usize,
        /// The ids of its calls, in order.
        calls: Vec<String>,
    },
    /// A turn's tool calls are answered once.
    #[error("turn {turn}'s tool calls already have their outcomes")]
    AlreadyAnswered {
        /// The turn whose calls were answered before, counted from 1.
        turn: usize,
    },
    /// Outcomes aren't those of the last turn's tool calls.
    #[error(
        "the outcomes {outcomes:?} don't answer the tool calls {calls:?}, each once and in call order"
    )]
    OutcomesDontAnswerCalls {
        /// The ids of the calls the last response made, in order.
        calls: Vec<String>,
        /// The call ids of the outcomes, in order.
        outcomes: Vec<String>,
    },
}

impl Transcript {
    /// The transcript of a run that has made no provider call yet.
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
    /// input is `input`. See [`crate::RunTally::messages`] for the contract.
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

    /// Usage summed over every turn.
    pub(crate) fn usage(&self) -> Usage {
        self.turns
            .iter()
            .fold(Usage::default(), |sum, turn| sum + turn.record.usage)
    }

    /// How many tool calls returned an error result since the last one that
    /// didn't. A turn without tool calls leaves the count as it was.
    pub(crate) fn consecutive_tool_errors(&self) -> u32 {
        let errors = self
            .turns
            .iter()
            .rev()
            .flat_map(|turn| turn.tool_calls.iter().rev())
            .take_while(|outcome| outcome.status != ToolCallStatus::Ok)
            .count();
        u32::try_from(errors).unwrap_or(u32::MAX)
    }

    /// Makes `completion` the next turn and takes `input` as its input: the
    /// completion's content becomes the response and the rest, with the
    /// timing, the record. When the turn is refused, `input` is left as it
    /// was. See [`crate::RunTally::completion`] for the contract.
    pub(crate) fn record(
        &mut self,
        input: &mut Vec<UserContent>,
        completion: Completion,
        started: Duration,
        latency: Duration,
        attempts: u32,
    ) -> Result<&Turn, TranscriptError> {
        let record = TurnRecord {
            usage: completion.usage,
            finish: completion.finish,
            response_id: completion.response_id,
            response_model: completion.response_model,
            started_ms: whole_ms(started),
            latency_ms: whole_ms(latency),
            attempts,
        };
        self.push(input, completion.content, record)
    }

    /// Makes `outcomes` those of the last turn. See
    /// [`crate::RunTally::tool_calls`] for the contract.
    pub(crate) fn answer(&mut self, outcomes: Vec<ToolCallOutcome>) -> Result<(), TranscriptError> {
        if self
            .turns
            .last()
            .is_some_and(|turn| !turn.tool_calls.is_empty())
        {
            return Err(TranscriptError::AlreadyAnswered {
                turn: self.turns.len(),
            });
        }
        let calls = self.turns.last().map(Turn::call_ids).unwrap_or_default();
        if outcomes.is_empty() && calls.is_empty() {
            return Ok(());
        }

        let answered = ids(outcomes.iter().map(|outcome| &outcome.call_id));
        if answered != calls {
            return Err(TranscriptError::OutcomesDontAnswerCalls {
                calls,
                outcomes: answered,
            });
        }
        if let Some(turn) = self.turns.last_mut() {
            turn.tool_calls = outcomes;
        }
        Ok(())
    }

    /// `response` has distinct tool call ids.
    fn push(
        &mut self,
        input: &mut Vec<UserContent>,
        mut response: Vec<ContentBlock>,
        record: TurnRecord,
    ) -> Result<&Turn, TranscriptError> {
        let last = self.turns.last();
        if let Some(last) = last
            && last.tool_calls.is_empty()
            && last.tool_uses().next().is_some()
        {
            return Err(TranscriptError::UnansweredCalls {
                turn: self.turns.len(),
                calls: last.call_ids(),
            });
        }
        input.retain(|block| !matches!(block, UserContent::Text(text) if text.trim().is_empty()));
        if input.is_empty() && last.is_none_or(|last| last.tool_calls.is_empty()) {
            return Err(TranscriptError::NothingFromTheUser {
                turn: self.turns.len() + 1,
            });
        }
        response
            .retain(|block| !matches!(block, ContentBlock::Text(text) if text.trim().is_empty()));
        self.turns.push(Turn {
            input: std::mem::take(input),
            response,
            record,
            tool_calls: Vec::new(),
        });
        Ok(&self.turns[self.turns.len() - 1])
    }
}

impl Turn {
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

    /// The response's [`ContentBlock::Text`] blocks, concatenated in order;
    /// empty when it has none.
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

    fn call_ids(&self) -> Vec<String> {
        ids(self.tool_uses().map(|call| &call.id))
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

fn ids<'a>(ids: impl Iterator<Item = &'a ToolCallId>) -> Vec<String> {
    ids.map(|id| id.as_str().to_owned()).collect()
}

#[cfg(test)]
mod tests;
