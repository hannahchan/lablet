//! A run as a whole: how it completes, why it stopped, the outcome document,
//! and the two halves of the wide event.
//!
//! A duration that reaches telemetry as a `*_ms` attribute is held as whole
//! milliseconds in a `u64` field named `*_ms`. [`crate::RunTally`] converts
//! as it records, so no observer rounds for itself and every observer reports
//! the same number.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    Cost, Endpoint, FinishReason, ModelRef, RequestDefaults, RunId, ToolName, Transcript, Usage,
};

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

display_as_str!(CompletionMode, StopReason);

/// The outcome document of a run. Its serde form is a public contract, held
/// by `lablet/tests/fixtures/outcome.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOutcome {
    /// The run's id.
    pub run_id: RunId,
    /// Why the run ended.
    pub stop_reason: StopReason,
    /// How many model responses the run received. A run whose first provider
    /// call failed took none.
    pub turns: u32,
    /// Usage summed over every successful provider call. `input_tokens`
    /// includes the cached tokens; see [`Usage`].
    pub usage: Usage,
    /// How many tool calls were executed. The intercepted `task_complete` call isn't one.
    pub tool_calls: u64,
    /// Wall-clock duration of the run, in whole milliseconds.
    pub duration_ms: u64,
    /// What the run produced.
    pub result: RunResult,
    /// The message of the error that ended the run, if one did.
    pub error: Option<String>,
}

/// What a run produced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunResult {
    /// The text of the last assistant message; empty when there is none.
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
/// built by [`crate::RunTally`].
///
/// The run totals that the outcome document carries (`usage`, `tool_calls`,
/// `turns`, `duration_ms`, `stop_reason`, `error`) are read from `outcome` and
/// aren't repeated here, and the number of provider calls is the length of
/// `finish_reasons`, so no two fields can disagree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    pub max_turns: u32,
    /// The run timeout, in whole milliseconds.
    pub timeout_ms: u64,
    /// The request parameters every provider call shared.
    pub request: RequestDefaults,
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
    /// The summed size of every tool call's output, in bytes.
    pub tool_output_bytes: u64,
    /// Each called tool's share, by tool name. The keys are among `tools`,
    /// whatever names the model called.
    pub per_tool: BTreeMap<ToolName, ToolStats>,
    /// The cost of the run, when pricing is configured.
    pub cost: Option<Cost>,
    /// The outcome document, which holds the run totals.
    pub outcome: RunOutcome,
}

impl RunSummary {
    /// How many provider calls returned a completion.
    #[must_use]
    pub const fn provider_calls(&self) -> u64 {
        self.finish_reasons.len() as u64
    }
}

/// What a finished run hands back: everything measured, the outcome inside
/// it, and the conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinishedRun {
    /// The run's summary, whose `outcome` is the outcome document.
    pub summary: RunSummary,
    /// The whole conversation, whatever the stop reason.
    pub transcript: Transcript,
}

#[cfg(test)]
mod tests;
