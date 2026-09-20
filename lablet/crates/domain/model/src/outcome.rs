//! A run as a whole: how it completes, why it stopped, the outcome document,
//! and the two halves of the wide event.
//!
//! A duration that reaches telemetry as a `*_ms` attribute is held as whole
//! milliseconds in a `u64` field named `*_ms`. The model converts as it
//! records, so no observer rounds for itself and every observer reports the
//! same number.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    Cost, Endpoint, FinishReason, ModelRef, Rates, RequestParams, RunId, ToolName, Transcript,
    Usage,
};

/// Whole milliseconds, truncated. The one conversion from a `Duration` in the
/// model, so every `*_ms` value is cut the same way.
pub(crate) fn whole_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// How a run decides that the model has finished.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionMode {
    /// The run completes when the model returns a turn with no tool calls.
    #[default]
    Natural,
    /// The run completes when the model calls `task_complete`.
    Explicit,
}

impl CompletionMode {
    /// The name of the tool that ends a run in [`CompletionMode::Explicit`]:
    /// the loop offers it, intercepts it rather than executing it, and the
    /// stop policy reads it. One definition, because those three have to agree.
    pub const TASK_COMPLETE: &'static str = "task_complete";

    /// The serde spelling, which is the `lablet.run.completion_mode` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Natural => "natural",
            Self::Explicit => "explicit",
        }
    }
}

/// Why a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished, by a turn with no tool calls or by calling `task_complete`.
    Completed,
    /// In explicit mode, the model returned a turn with no tool calls and never
    /// called `task_complete`.
    EndedWithoutCompletion,
    /// The run reached its turn cap.
    MaxTurns,
    /// The run reached its timeout.
    Timeout,
    /// Input plus output tokens reached the run's token budget.
    MaxTotalTokens,
    /// The last response hit the output token limit. Its tool calls, if any,
    /// weren't executed.
    OutputTruncated,
    /// The conversation outgrew the model's context: the provider rejected the
    /// request as too long, or cut the response short at the window.
    ContextExhausted,
    /// One provider call failed on every attempt the retry policy allows.
    RetriesExhausted,
    /// Consecutive tool error results reached their cap.
    ToolErrorsExhausted,
    /// The run was cancelled.
    Cancelled,
    /// The provider returned an error that isn't retryable.
    ProviderError,
    /// The model declined to answer, or a content filter withheld the response.
    Refused,
}

impl StopReason {
    /// The serde spelling, which is the `lablet.run.stop_reason` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::EndedWithoutCompletion => "ended_without_completion",
            Self::MaxTurns => "max_turns",
            Self::Timeout => "timeout",
            Self::MaxTotalTokens => "max_total_tokens",
            Self::OutputTruncated => "output_truncated",
            Self::ContextExhausted => "context_exhausted",
            Self::RetriesExhausted => "retries_exhausted",
            Self::ToolErrorsExhausted => "tool_errors_exhausted",
            Self::Cancelled => "cancelled",
            Self::ProviderError => "provider_error",
            Self::Refused => "refused",
        }
    }
}

/// What kind of ending a [`StopReason`] is, which decides what a
/// [`RunOutcome`] may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StopClass {
    /// The model finished the task. Only this outcome may have a structured result.
    Completed,
    /// The run ended in an orderly way short of completing. No error comes with it.
    Stopped,
    /// An error ended the run, and the outcome always says which.
    Failed,
}

impl StopReason {
    /// The kind of ending this reason is.
    #[must_use]
    pub const fn class(self) -> StopClass {
        match self {
            Self::Completed => StopClass::Completed,
            Self::EndedWithoutCompletion
            | Self::MaxTurns
            | Self::Timeout
            | Self::MaxTotalTokens
            | Self::OutputTruncated
            | Self::Cancelled
            | Self::Refused => StopClass::Stopped,
            Self::ContextExhausted
            | Self::RetriesExhausted
            | Self::ToolErrorsExhausted
            | Self::ProviderError => StopClass::Failed,
        }
    }

    /// What a failed run reports when no error text came with the failure.
    const fn failure_message(self) -> &'static str {
        match self {
            Self::ContextExhausted => "the response was cut short at the model's context window",
            Self::ToolErrorsExhausted => "consecutive tool error results reached their cap",
            _ => "a provider call failed",
        }
    }
}

display_as_str!(CompletionMode, StopReason);

/// The outcome document of a run. Its serde form is a public contract, held
/// by `lablet/tests/fixtures/outcome.json`.
///
/// What an outcome carries follows from the [`StopClass`] of its stop reason:
/// `error` is a message exactly when the run failed, and `result.structured`
/// is a value only when it completed. Those three fields are private for that
/// reason: [`crate::Run::finish`] builds outcomes that way, and reading one
/// from its serde form refuses any other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawOutcome")]
pub struct RunOutcome {
    /// The run's id.
    pub run_id: RunId,
    stop_reason: StopReason,
    /// How many turns the transcript has, which is how many model responses
    /// the run received. A run whose first provider call failed took none.
    pub turns: u32,
    /// Usage summed over every successful provider call. `input_tokens`
    /// includes the cached tokens; see [`Usage`].
    pub usage: Usage,
    /// How many tool calls were executed. The intercepted `task_complete` call isn't one.
    pub tool_calls: u64,
    /// Wall-clock duration of the run, in whole milliseconds.
    pub duration_ms: u64,
    result: TaskResult,
    error: Option<String>,
}

/// What an outcome is read from, so that reading one holds it to the rules,
/// and what [`crate::Run::finish`] fills in to close a run.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawOutcome {
    pub(crate) run_id: RunId,
    pub(crate) stop_reason: StopReason,
    pub(crate) turns: u32,
    pub(crate) usage: Usage,
    pub(crate) tool_calls: u64,
    pub(crate) duration_ms: u64,
    pub(crate) result: TaskResult,
    pub(crate) error: Option<String>,
}

/// Why an outcome document is one no run could have produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutcomeError {
    /// An error came with a stop reason that isn't a failure.
    #[error("a run that stopped with {stop_reason} didn't fail, so it has no error")]
    ErrorWithoutFailure {
        /// The stop reason of the document.
        stop_reason: StopReason,
    },
    /// A failure came without its error.
    #[error("a run that stopped with {stop_reason} failed, so it has an error")]
    FailureWithoutError {
        /// The stop reason of the document.
        stop_reason: StopReason,
    },
    /// A structured result came with a run that didn't complete.
    #[error(
        "a run that stopped with {stop_reason} didn't complete, so it has no structured result"
    )]
    StructuredWithoutCompletion {
        /// The stop reason of the document.
        stop_reason: StopReason,
    },
}

impl TryFrom<RawOutcome> for RunOutcome {
    type Error = OutcomeError;

    fn try_from(raw: RawOutcome) -> Result<Self, OutcomeError> {
        let stop_reason = raw.stop_reason;
        let class = stop_reason.class();
        if raw.result.structured.is_some() && class != StopClass::Completed {
            return Err(OutcomeError::StructuredWithoutCompletion { stop_reason });
        }
        match (class, &raw.error) {
            (StopClass::Failed, None) => Err(OutcomeError::FailureWithoutError { stop_reason }),
            (StopClass::Completed | StopClass::Stopped, Some(_)) => {
                Err(OutcomeError::ErrorWithoutFailure { stop_reason })
            }
            (StopClass::Failed, Some(_)) | (StopClass::Completed | StopClass::Stopped, None) => {
                Ok(Self::closing(raw))
            }
        }
    }
}

impl RunOutcome {
    /// The outcome of a run, holding it to the [`StopClass`] rules and giving a
    /// failure that came without an error the reason's own message.
    pub(crate) fn closing(raw: RawOutcome) -> Self {
        let class = raw.stop_reason.class();
        Self {
            run_id: raw.run_id,
            stop_reason: raw.stop_reason,
            turns: raw.turns,
            usage: raw.usage,
            tool_calls: raw.tool_calls,
            duration_ms: raw.duration_ms,
            result: TaskResult {
                text: raw.result.text,
                structured: raw
                    .result
                    .structured
                    .filter(|_| class == StopClass::Completed),
            },
            error: (class == StopClass::Failed).then(|| {
                raw.error
                    .unwrap_or_else(|| raw.stop_reason.failure_message().to_owned())
            }),
        }
    }

    /// Why the run ended.
    #[must_use]
    pub const fn stop_reason(&self) -> StopReason {
        self.stop_reason
    }

    /// What the run produced. `structured` is a value only when the run completed.
    #[must_use]
    pub const fn result(&self) -> &TaskResult {
        &self.result
    }

    /// The message of the error that ended the run: a message when the run
    /// failed, `None` otherwise.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskResult {
    /// The text of the last turn; empty when there is none.
    pub text: String,
    /// The `task_complete` argument when the run completed in explicit mode.
    /// Written as `null` otherwise, never left out.
    pub structured: Option<serde_json::Value>,
}

/// What only the composition root knows about a run: its part of the wide
/// event. What the loop is built from, such as the limits and the request
/// parameters, it reports itself, in [`RunSummary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunContext {
    /// The run's id.
    pub run_id: RunId,
    /// SHA-256 of the resolved config, in hex.
    pub config_digest: String,
    /// The lablet version.
    pub agent_version: String,
    /// The composer's extra resource attributes, in config order.
    pub resource: Vec<(String, String)>,
    /// Where the transcript is written, when it is.
    pub transcript_path: Option<PathBuf>,
    /// How many skill files were appended to the system prompt.
    pub skills_count: u32,
    /// The names of the configured MCP servers.
    pub mcp_servers: Vec<String>,
    /// Whether prompts, responses, and tool content may reach telemetry. The
    /// loop reads it to fill the content fields of its events, and an observer
    /// reads it before emitting the result from the summary.
    pub capture_content: bool,
}

/// One tool's share of a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolStats {
    /// How many times the tool was called.
    pub calls: u64,
    /// How many of those calls returned an error result.
    pub errors: u64,
    /// The summed latency of those calls, in whole milliseconds.
    pub latency_ms: u64,
}

/// What the loop knew and measured over a run: its part of the wide event,
/// built by [`crate::Run`].
///
/// The run totals that the outcome document carries (`usage`, `tool_calls`,
/// `turns`, `duration_ms`, `stop_reason`, `error`) are read from `outcome` and
/// aren't repeated here, so no two fields can disagree.
///
/// It's written, never read: the wide event is emitted from it, and the
/// documents lablet writes are the outcome and the transcript, each of which
/// checks itself on the way in. A summary holds invariants that span its
/// fields, such as one finish reason per turn of the outcome and totals that
/// agree with the transcript beside them, and only [`crate::Run::finish`]
/// establishes those. So there's no `Deserialize`, rather than one that would
/// take a summary no run could have produced. The bound on the `per_tool`
/// keys isn't among them: the tool executor establishes that one, and
/// `finish` reads its answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunSummary {
    /// The model the run called.
    pub model: ModelRef,
    /// Where the provider's API is served; `None` for a provider that isn't
    /// reached over the network.
    pub endpoint: Option<Endpoint>,
    /// The tools offered to the model, after the allow and deny lists.
    pub tools: Vec<ToolName>,
    /// How the run decided that the model had finished.
    pub completion: CompletionMode,
    /// The cap on turns.
    pub max_turns: NonZeroU32,
    /// The run timeout, in whole milliseconds.
    pub timeout_ms: u64,
    /// The request parameters every provider call shared.
    pub request: RequestParams,
    /// Size of the system prompt in bytes, skills included.
    pub prompt_system_bytes: u64,
    /// Size of the task prompt in bytes.
    pub prompt_user_bytes: u64,
    /// How many provider call attempts were made beyond the first of their
    /// call. A call that fails on its only attempt adds none.
    pub provider_retries: u64,
    /// The summed latency of every provider call attempt, in whole milliseconds.
    pub provider_latency_total_ms: u64,
    /// The latency of the slowest provider call attempt, in whole milliseconds.
    pub provider_latency_max_ms: u64,
    /// The finish reason of each completion, in call order.
    pub finish_reasons: Vec<FinishReason>,
    /// How many tool calls returned an error result.
    pub tool_calls_errors: u64,
    /// How many tool calls named a tool the run didn't offer. They're in the
    /// totals and have no entry in `per_tool`.
    pub tool_calls_unknown: u64,
    /// The summed latency of every tool call, in whole milliseconds.
    pub tool_latency_total_ms: u64,
    /// The summed size of every tool call's input, in bytes.
    pub tool_input_bytes: u64,
    /// The summed size of every tool call's output as the model was sent it, in bytes.
    pub tool_output_bytes: u64,
    /// How many tool calls had their output cut by the output cap.
    pub tool_calls_truncated: u64,
    /// Each called tool's share, by tool name. A key exists for each call a
    /// tool ran for, which is the same set as `tools` in a run because the
    /// executor resolves only the names it offered, whatever names the model
    /// called.
    pub per_tool: BTreeMap<ToolName, ToolStats>,
    /// The rates the run was priced at; `Some` exactly when pricing was
    /// configured. They reach the wide event beside the cost so a consumer
    /// can recompute it rather than trust it.
    pub rates: Option<Rates>,
    /// The cost of the run, when pricing is configured and the amount is a
    /// number.
    pub cost: Option<Cost>,
    /// The outcome document.
    pub outcome: RunOutcome,
}

/// What a finished run hands back: everything measured, the outcome inside
/// it, and the conversation. Written, never read, for the reason
/// [`RunSummary`] gives.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FinishedRun {
    /// The run's summary, whose `outcome` is the outcome document.
    pub summary: RunSummary,
    /// The whole conversation, whatever the stop reason.
    pub transcript: Transcript,
}

#[cfg(test)]
mod tests;
