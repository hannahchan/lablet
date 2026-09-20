//! Whether a run goes on, asked at the three points of a turn where it can end.

use std::num::NonZeroU32;
use std::time::Duration;

use lablet_model::{Calls, CompletionMode, FinishReason, Progress, StopReason};

/// The limits of one run and how it completes.
///
/// Every value is meaningful, so the fields are public and nothing is
/// validated. A limit is met when the run reaches it, not when it passes it.
///
/// When several conditions hold at one point, the first in the order its
/// method documents is the reason, so one input always gives one answer. Four
/// stop reasons aren't decided here: `cancelled` comes from the loop's
/// cancellation poll, which comes before it asks the policy, and
/// `retries_exhausted`, `context_exhausted` for a rejected request, and
/// `provider_error` from a failed provider call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopPolicy {
    /// How the run decides that the model has finished.
    pub completion: CompletionMode,
    /// The number of turns after whose tool phase the run stops.
    pub max_turns: NonZeroU32,
    /// The elapsed time at which the run stops. Zero stops it before the first
    /// provider call.
    pub timeout: Duration,
    /// The input plus output tokens, summed over every provider call, at which
    /// the run stops; `None` is no budget. Context that's sent again is counted
    /// again, as it's billed.
    pub max_total_tokens: Option<u64>,
    /// The run of consecutive tool error results at which the run stops.
    pub max_consecutive_tool_errors: NonZeroU32,
}

impl StopPolicy {
    /// The reason to stop before a provider call, or `None` to make it:
    /// `timeout`, then `max_total_tokens`.
    #[must_use]
    pub fn before_call(&self, progress: &Progress) -> Option<StopReason> {
        self.limit_reached(progress)
    }

    /// The reason to stop after a provider response and before any tool runs,
    /// or `None` to run the response's tool calls.
    ///
    /// It reads only the response, so a response that finishes the task
    /// completes the run even when it also used up a limit. The finish reason
    /// is read before the calls:
    ///
    /// - [`FinishReason::Refusal`] is `refused`.
    /// - [`FinishReason::MaxTokens`] is `output_truncated` and
    ///   [`FinishReason::ContextWindow`] is `context_exhausted`, whatever the
    ///   response called. A response that was cut short can end inside a
    ///   call's arguments, and a cut-off input can still parse as a valid,
    ///   smaller one, so none of its calls may run and a `task_complete` among
    ///   them doesn't complete the run.
    /// - Otherwise the calls decide. `task_complete` in explicit mode is
    ///   `completed`; any other call goes on; no call is `completed` in
    ///   natural mode and `ended_without_completion` in explicit mode.
    ///
    /// [`FinishReason::Other`] with no call therefore completes a natural-mode
    /// run. A reason lablet doesn't know, from an OpenAI-compatible server, is
    /// usually that server's word for a normal end, and the wide event carries
    /// every finish reason for whoever needs to tell.
    #[must_use]
    pub fn after_response(&self, finish: &FinishReason, calls: Calls) -> Option<StopReason> {
        match finish {
            FinishReason::Refusal => return Some(StopReason::Refused),
            FinishReason::MaxTokens => return Some(StopReason::OutputTruncated),
            FinishReason::ContextWindow => return Some(StopReason::ContextExhausted),
            FinishReason::EndTurn | FinishReason::ToolUse | FinishReason::Other(_) => {}
        }
        match (self.completion, calls) {
            (CompletionMode::Explicit, Calls::TaskComplete)
            | (CompletionMode::Natural, Calls::None) => Some(StopReason::Completed),
            (CompletionMode::Explicit, Calls::None) => Some(StopReason::EndedWithoutCompletion),
            (CompletionMode::Natural, Calls::TaskComplete) | (_, Calls::Tools) => None,
        }
    }

    /// The reason to stop after the tool results of a turn are in, or `None`
    /// to go on to the next turn: `tool_errors_exhausted`, then `max_turns`,
    /// then `timeout`, then `max_total_tokens`. The tool-error cap leads
    /// because it alone says the run was failing, not merely long.
    #[must_use]
    pub fn after_tools(&self, progress: &Progress) -> Option<StopReason> {
        self.cap_reached(progress)
            .or_else(|| self.limit_reached(progress))
    }

    /// Whether a backoff of `wait`, begun `elapsed` into the run, ends before
    /// the run timeout. When it doesn't, the run stops with `timeout` now
    /// instead of sleeping up to a limit it's known to reach.
    #[must_use]
    pub fn allows_wait(&self, elapsed: Duration, wait: Duration) -> bool {
        elapsed.saturating_add(wait) < self.timeout
    }

    /// The limits that grow during a provider call as well as a tool phase.
    fn limit_reached(&self, progress: &Progress) -> Option<StopReason> {
        if progress.elapsed >= self.timeout {
            Some(StopReason::Timeout)
        } else if self
            .max_total_tokens
            .is_some_and(|budget| progress.usage.total() >= budget)
        {
            Some(StopReason::MaxTotalTokens)
        } else {
            None
        }
    }

    /// The caps that only a finished tool phase can reach.
    fn cap_reached(&self, progress: &Progress) -> Option<StopReason> {
        if progress.consecutive_tool_errors >= self.max_consecutive_tool_errors.get() {
            Some(StopReason::ToolErrorsExhausted)
        } else if progress.turns >= self.max_turns.get() {
            Some(StopReason::MaxTurns)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests;
