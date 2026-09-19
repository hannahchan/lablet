//! Whether a run goes on, asked at the three points of a turn where it can end.

use std::time::Duration;

use lablet_model::{CompletionMode, FinishReason, StopReason};

/// The limits of one run and how it completes.
///
/// Every value is meaningful, so the fields are public and nothing is
/// validated. A limit is met when the state reaches it, not when it passes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopPolicy {
    /// How the run decides that the model has finished.
    pub completion: CompletionMode,
    /// The turn after whose tool phase the run stops. The cap is only read
    /// after a tool phase, so `0` acts as `1`.
    pub max_turns: u32,
    /// The elapsed time at which the run stops. Zero stops it before the first
    /// provider call.
    pub timeout: Duration,
    /// The input plus output tokens at which the run stops; `None` is no budget.
    pub max_total_tokens: Option<u64>,
    /// The run of consecutive tool error results at which the run stops. `0`
    /// acts as `1`: a run is never stopped for errors it hasn't had.
    pub max_consecutive_tool_errors: u32,
}

/// What the loop knows when it asks whether to stop.
///
/// Cancellation isn't here: the loop polls its own port for it, before it asks
/// the policy, so a cancelled run reports `cancelled` whatever else holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunState {
    /// The one-based number of the current turn.
    pub turn: u32,
    /// The time since the run started.
    pub elapsed: Duration,
    /// Input plus output tokens of every completion so far, as `Usage::total` counts them.
    pub total_tokens: u64,
    /// Tool error results since the last successful tool call.
    pub consecutive_tool_errors: u32,
    /// The finish reason of the latest response; `None` before the first.
    pub last_finish: Option<FinishReason>,
    /// Whether the latest response held a tool call, `task_complete` included.
    pub last_had_tool_use: bool,
    /// Whether the latest response called `task_complete`. Only explicit mode reads it.
    pub task_complete_called: bool,
}

/// Where in a turn the loop asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPoint {
    /// Before each provider call. Reads the timeout and the token budget.
    BeforeProviderCall,
    /// After a provider response and before any tool runs. Reads only the
    /// response, so a response that finishes the task completes the run even
    /// when it also used up a limit.
    AfterProviderResponse,
    /// After the tool results of a turn are in. Reads the tool-error cap and
    /// the turn cap, then what [`StopPoint::BeforeProviderCall`] reads.
    AfterToolPhase,
}

impl StopPolicy {
    /// The reason to stop at `at`, or `None` to go on.
    ///
    /// When several conditions hold at one point, the first in this order is
    /// the reason, so one state always gives one answer:
    ///
    /// - [`StopPoint::BeforeProviderCall`]: `timeout`, then `max_total_tokens`.
    /// - [`StopPoint::AfterProviderResponse`]: when explicit mode's
    ///   `task_complete` was called, `output_truncated` if the finish reason is
    ///   `max_tokens` and `completed` otherwise, whatever else the response
    ///   holds; otherwise no stop when the response has tool calls, whatever
    ///   its finish reason;
    ///   otherwise `output_truncated` when the finish reason is `max_tokens`;
    ///   otherwise `completed` in natural mode and `ended_without_completion`
    ///   in explicit mode.
    /// - [`StopPoint::AfterToolPhase`]: `tool_errors_exhausted`, then
    ///   `max_turns`, then `timeout`, then `max_total_tokens`. The tool-error
    ///   cap leads because it alone says the run was failing, not merely long.
    ///
    /// The other stop reasons aren't decided here: `cancelled` comes from the
    /// loop's cancellation poll, and `retries_exhausted`, `context_exhausted`,
    /// and `provider_error` from a failed provider call.
    #[must_use]
    pub fn evaluate(&self, at: StopPoint, state: &RunState) -> Option<StopReason> {
        match at {
            StopPoint::BeforeProviderCall => self.limit_reached(state),
            StopPoint::AfterProviderResponse => self.response_ends_run(state),
            StopPoint::AfterToolPhase => self
                .cap_reached(state)
                .or_else(|| self.limit_reached(state)),
        }
    }

    /// The limits that grow during a provider call as well as a tool phase.
    fn limit_reached(&self, state: &RunState) -> Option<StopReason> {
        if state.elapsed >= self.timeout {
            Some(StopReason::Timeout)
        } else if self
            .max_total_tokens
            .is_some_and(|budget| state.total_tokens >= budget)
        {
            Some(StopReason::MaxTotalTokens)
        } else {
            None
        }
    }

    /// The caps that only a finished tool phase can reach.
    fn cap_reached(&self, state: &RunState) -> Option<StopReason> {
        if state.consecutive_tool_errors >= self.max_consecutive_tool_errors.max(1) {
            Some(StopReason::ToolErrorsExhausted)
        } else if state.turn >= self.max_turns {
            Some(StopReason::MaxTurns)
        } else {
            None
        }
    }

    fn response_ends_run(&self, state: &RunState) -> Option<StopReason> {
        let explicit = self.completion == CompletionMode::Explicit;
        let truncated = state.last_finish == Some(FinishReason::MaxTokens);
        if explicit && state.task_complete_called {
            // A response cut off at `max_tokens` can end inside the call's
            // arguments, and a run must not complete on a partial result.
            Some(if truncated {
                StopReason::OutputTruncated
            } else {
                StopReason::Completed
            })
        } else if state.last_had_tool_use {
            None
        } else if truncated {
            Some(StopReason::OutputTruncated)
        } else if explicit {
            Some(StopReason::EndedWithoutCompletion)
        } else {
            Some(StopReason::Completed)
        }
    }
}

#[cfg(test)]
mod tests;
