//! The running count of a run: what the loop adds to as things happen, what
//! the stop policy reads from it, and the summary it becomes.
//!
//! This is where a `Duration` becomes whole milliseconds. Each latency is
//! truncated as it's recorded, so a total and the per-tool values beside it
//! are sums of the same numbers.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::{
    Completion, CompletionMode, Cost, Endpoint, FinishReason, ModelRef, RequestDefaults, RunId,
    RunOutcome, RunResult, RunSummary, StopReason, ToolName, ToolStats, Usage,
};

/// What the loop knows about a run before its first provider call.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSetup {
    /// The model the run calls.
    pub model: ModelRef,
    /// Where the provider's API is served; `None` for a provider that isn't
    /// reached over the network.
    pub endpoint: Option<Endpoint>,
    /// The tools offered to the model, after the allow and deny lists. Only
    /// these names get per-tool statistics.
    pub tools: Vec<ToolName>,
    /// How the run decides that the model has finished.
    pub completion: CompletionMode,
    /// The cap on turns.
    pub max_turns: u32,
    /// The run timeout.
    pub timeout: Duration,
    /// The request parameters every provider call shares.
    pub request: RequestDefaults,
    /// Size of the system prompt in bytes, skills included.
    pub prompt_system_bytes: u64,
    /// Size of the task prompt in bytes.
    pub prompt_user_bytes: u64,
}

/// How far a run has come, which is what its limits are held against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    /// Model responses received so far.
    pub turns: u32,
    /// The time since the run started.
    pub elapsed: Duration,
    /// Usage summed over every completion so far.
    pub usage: Usage,
    /// Tool error results since the last successful tool call.
    pub consecutive_tool_errors: u32,
}

/// The running count of one run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunTally {
    setup: RunSetup,
    usage: Usage,
    finish_reasons: Vec<FinishReason>,
    provider_retries: u64,
    provider_latency_total_ms: u64,
    provider_latency_max_ms: u64,
    tool_calls: u64,
    tool_calls_errors: u64,
    tool_calls_unknown: u64,
    tool_latency_total_ms: u64,
    tool_input_bytes: u64,
    tool_output_bytes: u64,
    per_tool: BTreeMap<ToolName, ToolStats>,
    consecutive_tool_errors: u32,
}

impl RunTally {
    /// The tally of a run that has done nothing yet.
    #[must_use]
    pub const fn start(setup: RunSetup) -> Self {
        Self {
            setup,
            usage: Usage::from_inclusive(0, 0, 0, 0),
            finish_reasons: Vec::new(),
            provider_retries: 0,
            provider_latency_total_ms: 0,
            provider_latency_max_ms: 0,
            tool_calls: 0,
            tool_calls_errors: 0,
            tool_calls_unknown: 0,
            tool_latency_total_ms: 0,
            tool_input_bytes: 0,
            tool_output_bytes: 0,
            per_tool: BTreeMap::new(),
            consecutive_tool_errors: 0,
        }
    }

    /// Records a provider call attempt that returned `completion` after `latency`.
    pub fn completion(&mut self, completion: &Completion, latency: Duration) {
        self.usage += completion.usage;
        self.finish_reasons.push(completion.finish.clone());
        self.provider_latency(latency);
    }

    /// Records a provider call attempt that failed after `latency`.
    pub fn failed_attempt(&mut self, latency: Duration) {
        self.provider_latency(latency);
    }

    /// Records that a provider call is being attempted again. A failed attempt
    /// that nothing follows isn't a retry.
    pub const fn retry(&mut self) {
        self.provider_retries += 1;
    }

    /// Records an executed tool call. The intercepted `task_complete` call
    /// isn't one.
    ///
    /// A name the run didn't offer counts in the totals and as an unknown
    /// call, and gets no per-tool entry: the model can call any name, and the
    /// per-tool keys of the wide event must stay bounded by the config.
    pub fn tool_call(
        &mut self,
        name: &ToolName,
        is_error: bool,
        latency: Duration,
        input_bytes: u64,
        output_bytes: u64,
    ) {
        let latency_ms = whole_ms(latency);
        let errors = u64::from(is_error);
        self.tool_calls += 1;
        self.tool_calls_errors += errors;
        self.tool_latency_total_ms = self.tool_latency_total_ms.saturating_add(latency_ms);
        self.tool_input_bytes = self.tool_input_bytes.saturating_add(input_bytes);
        self.tool_output_bytes = self.tool_output_bytes.saturating_add(output_bytes);
        self.consecutive_tool_errors = if is_error {
            self.consecutive_tool_errors.saturating_add(1)
        } else {
            0
        };
        if self.setup.tools.contains(name) {
            let stats = self.per_tool.entry(name.clone()).or_default();
            stats.calls += 1;
            stats.errors += errors;
            stats.latency_ms = stats.latency_ms.saturating_add(latency_ms);
        } else {
            self.tool_calls_unknown += 1;
        }
    }

    /// How far the run has come, `elapsed` after it started.
    #[must_use]
    pub fn progress(&self, elapsed: Duration) -> Progress {
        Progress {
            turns: self.turns(),
            elapsed,
            usage: self.usage,
            consecutive_tool_errors: self.consecutive_tool_errors,
        }
    }

    /// Usage summed over every completion so far, which is what a run's cost
    /// is priced from.
    #[must_use]
    pub const fn usage(&self) -> Usage {
        self.usage
    }

    /// Closes the tally of a run that stopped with `stop`, `duration` after it
    /// started.
    #[must_use]
    pub fn finish(
        self,
        run_id: RunId,
        stop: StopReason,
        duration: Duration,
        result: RunResult,
        error: Option<String>,
        cost: Option<Cost>,
    ) -> RunSummary {
        let outcome = RunOutcome {
            run_id,
            stop_reason: stop,
            turns: self.turns(),
            usage: self.usage,
            tool_calls: self.tool_calls,
            duration_ms: whole_ms(duration),
            result,
            error,
        };
        RunSummary {
            model: self.setup.model,
            endpoint: self.setup.endpoint,
            tools: self.setup.tools,
            completion: self.setup.completion,
            max_turns: self.setup.max_turns,
            timeout_ms: whole_ms(self.setup.timeout),
            request: self.setup.request,
            prompt_system_bytes: self.setup.prompt_system_bytes,
            prompt_user_bytes: self.setup.prompt_user_bytes,
            provider_retries: self.provider_retries,
            provider_latency_total_ms: self.provider_latency_total_ms,
            provider_latency_max_ms: self.provider_latency_max_ms,
            finish_reasons: self.finish_reasons,
            tool_calls_errors: self.tool_calls_errors,
            tool_calls_unknown: self.tool_calls_unknown,
            tool_latency_total_ms: self.tool_latency_total_ms,
            tool_input_bytes: self.tool_input_bytes,
            tool_output_bytes: self.tool_output_bytes,
            per_tool: self.per_tool,
            cost,
            outcome,
        }
    }

    /// A run's turns are the model responses it received, so a run whose first
    /// provider call failed took none.
    fn turns(&self) -> u32 {
        u32::try_from(self.finish_reasons.len()).unwrap_or(u32::MAX)
    }

    fn provider_latency(&mut self, latency: Duration) {
        let latency_ms = whole_ms(latency);
        self.provider_latency_total_ms = self.provider_latency_total_ms.saturating_add(latency_ms);
        self.provider_latency_max_ms = self.provider_latency_max_ms.max(latency_ms);
    }
}

/// Whole milliseconds, truncated.
fn whole_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
