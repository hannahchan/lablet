//! The two halves of the wide event: what the composition root knows about a
//! run, and what the loop knew and measured.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::PathBuf;

use crate::{
    CompletionMode, ConfigDigest, Cost, Endpoint, FinishReason, McpServers, ModelRef,
    ProviderTotals, Rates, RequestParams, RunId, RunLabels, RunOutcome, ToolCallTotals, ToolName,
    ToolStats, Transcript, Usage,
};

/// What only the composition root knows about a run: its part of the wide
/// event. What the loop is built from, such as the limits and the request
/// parameters, it reports itself, in [`RunSummary`].
///
/// Neither digest of what the model was shown is here. The loop takes both
/// from what it sends, so no caller of the loop can pair one with a prompt or
/// a tool set it wasn't taken from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunContext {
    /// The run's id.
    pub run_id: RunId,
    /// What the run request named the run's task, experiment and trial. The
    /// loop copies them to the outcome, and an observer reads them here.
    pub labels: RunLabels,
    /// When the run started, in milliseconds since the Unix epoch. It's
    /// read where the context is filled in: the domain reads no clock, and
    /// the loop's gives instants, which have no date.
    pub started_unix_ms: u64,
    /// SHA-256 of the resolved config, which groups the runs made from it.
    pub config_digest: ConfigDigest,
    /// The lablet version.
    pub agent_version: String,
    /// Where the transcript is written, when it is.
    pub transcript_path: Option<PathBuf>,
    /// How many skill files were appended to the system prompt.
    pub skills_count: u32,
    /// The MCP servers that serve the run, with the version each gave of
    /// itself and how long they live; `None` for a run that has none.
    pub mcp: Option<McpServers>,
    /// Whether prompts, responses, and tool content may reach telemetry. The
    /// loop reads it to fill the content fields of its events, and an observer
    /// reads it before emitting the result from the summary.
    pub capture_content: bool,
}

/// The sizes of what the model was shown before its first response, in
/// bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PromptSizes {
    /// The system prompt, skills included.
    pub system_bytes: u64,
    /// The task prompt.
    pub user_bytes: u64,
    /// The tool specs, as the loop measured them: each as compact JSON, in
    /// the order they're offered.
    pub tools_bytes: u64,
}

/// What the loop knew and measured over a run: its part of the wide event,
/// built by [`crate::Run`].
///
/// The run totals that the outcome carries (its `usage`, its count of
/// `tool_calls`, `turns`, `duration_ms`, `stop_reason`, `error`) are read
/// from `outcome` and aren't repeated here, so no two fields can disagree.
/// Every total of tool calls, there and here, counts the calls something was
/// started for ([`crate::ToolCallStatus::was_started`]): a call that was
/// never run is in the transcript alone.
///
/// The totals are grouped as the wide event names them: what's under
/// `lablet.prompt` is in `prompt`, what's under `lablet.provider` in
/// `provider`, and what's under `lablet.tool_calls` in `tool_calls`. The
/// groups are for reading. Two numbers of one group can still be taken for
/// each other, and what holds each to its attribute is the observer's match
/// over the registry's keys, with the tests of what each key holds.
///
/// It has no written form of its own: the wide event is a mapping of its
/// fields to attributes, and the documents lablet writes are the outcome and
/// the transcript. A summary holds invariants that span its fields, such as
/// one finish reason per turn of the outcome and totals that agree with the
/// transcript beside them, and only [`crate::Run::finish`] establishes
/// those. The bound on the `per_tool` keys isn't among them: the tool
/// executor establishes that one, and `finish` reads its answer.
#[derive(Debug, Clone, PartialEq)]
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
    /// The cap on turns; `None` when the run had none.
    pub max_turns: Option<NonZeroU32>,
    /// The run timeout, in whole milliseconds.
    pub timeout_ms: u64,
    /// The request parameters every provider call shared.
    pub request: RequestParams,
    /// The sizes of what the model was shown before its first response.
    pub prompt: PromptSizes,
    /// SHA-256 of the tool specs, of the bytes `prompt.tools_bytes` counts,
    /// in hex.
    pub tools_digest: String,
    /// SHA-256 of the system prompt as it was sent, in hex.
    pub system_prompt_digest: String,
    /// What the provider call attempts that failed reported using, summed,
    /// and `None` when none reported anything. It's in no turn, so
    /// `outcome.usage` leaves it out, and the cost counts both.
    pub failed_usage: Option<Usage>,
    /// What every provider call attempt came to, the ones that failed
    /// included.
    pub provider: ProviderTotals,
    /// The finish reason of each completion, in call order.
    pub finish_reasons: Vec<FinishReason>,
    /// What the tool calls came to, but for how many there were, which is
    /// `outcome.tool_calls`. A call that named a tool the run didn't offer
    /// is in these and has no entry in `per_tool`.
    pub tool_calls: ToolCallTotals,
    /// Each called tool's share, by tool name. A key exists for each call
    /// that named a tool the run offered, whether or not the tool ran, so the
    /// keys are among `tools` in a run because the executor resolves only the
    /// names it offered, whatever names the model called.
    pub per_tool: BTreeMap<ToolName, ToolStats>,
    /// The rates the run was priced at; `Some` exactly when pricing was
    /// configured. They reach the wide event beside the cost so a consumer
    /// can recompute it rather than trust it.
    pub rates: Option<Rates>,
    /// The cost of the run, when pricing is configured and the amount is a
    /// number.
    pub cost: Option<Cost>,
    /// The run's outcome.
    pub outcome: RunOutcome,
}

/// What a finished run hands back: everything measured, the outcome inside
/// it, and the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedRun {
    /// The run's summary, whose `outcome` the outcome document is written
    /// from.
    pub summary: RunSummary,
    /// The whole conversation, whatever the stop reason.
    pub transcript: Transcript,
}

#[cfg(test)]
mod tests;
