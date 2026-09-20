//! How a run decides the model has finished, and the vocabulary of why a run
//! ended. The stop policy in `lablet-policy` reads all of it; the transcript
//! and the summary each hold a part.

use serde::{Deserialize, Serialize};

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
/// [`crate::RunOutcome`] may carry.
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
    pub(crate) const fn failure_message(self) -> &'static str {
        match self {
            Self::ContextExhausted => "the response was cut short at the model's context window",
            Self::ToolErrorsExhausted => "consecutive tool error results reached their cap",
            _ => "a provider call failed",
        }
    }
}

display_as_str!(CompletionMode, StopReason);

/// The tool calls of a response, as far as completion reads them.
///
/// [`crate::Turn::calls`] is the only place a response is read this way, so the loop
/// can't classify a response differently from how the stop policy expects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Calls {
    /// The response called no tool.
    None,
    /// The response called tools, and `task_complete` wasn't one of them.
    Tools,
    /// The response called `task_complete`, alone or among other tools. Only
    /// [`CompletionMode::Explicit`] has such a tool, so a natural-mode
    /// response is never read as this.
    TaskComplete,
}

#[cfg(test)]
mod tests;
