//! The two halves of the wide event: what the composition root knows about a
//! run, and what the loop knew and measured.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    CompletionMode, Cost, Endpoint, FinishReason, ModelRef, Rates, RequestParams, RunId,
    RunOutcome, ToolName, Transcript, Turn,
};

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

/// Raises a total by `amount`, saturating. One function, so no total is
/// added up differently from the rest.
fn add(total: &mut u64, amount: u64) {
    *total = total.saturating_add(amount);
}

impl RunSummary {
    /// Adds what `turn` holds to these totals.
    pub(crate) fn add_turn(&mut self, turn: &Turn) {
        let record = turn.record();
        add(
            &mut self.provider_retries,
            u64::from(record.attempts.saturating_sub(1)),
        );
        add(&mut self.provider_latency_total_ms, record.latency_ms);
        self.provider_latency_max_ms = self.provider_latency_max_ms.max(record.latency_ms);
        self.finish_reasons.push(record.finish.clone());
        for (call, outcome) in turn.tool_uses().zip(turn.tool_calls()) {
            let errors = u64::from(outcome.status.is_error());
            add(&mut self.tool_calls_errors, errors);
            add(
                &mut self.tool_calls_truncated,
                u64::from(outcome.truncated_from_bytes.is_some()),
            );
            add(&mut self.tool_latency_total_ms, outcome.latency_ms);
            add(&mut self.tool_input_bytes, call.input_bytes());
            add(&mut self.tool_output_bytes, outcome.output_bytes());
            if outcome.status.names_an_offered_tool() {
                let stats = self.per_tool.entry(call.name.clone()).or_default();
                add(&mut stats.calls, 1);
                add(&mut stats.errors, errors);
                add(&mut stats.latency_ms, outcome.latency_ms);
            } else {
                add(&mut self.tool_calls_unknown, 1);
            }
        }
    }
}

#[cfg(test)]
mod tests;
