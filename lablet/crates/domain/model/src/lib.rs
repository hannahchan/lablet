//! The domain model: the transcript and its turns, tools, usage, stop reasons,
//! run identity, and the run summary, as plain types and pure functions with
//! serde derives.
//!
//! The derived serde form of these types is the one JSON form of a
//! conversation: the transcript, the outcome document, and the fake provider's
//! scripts all reuse it.

/// Implements `Display` through the type's `as_str`, so what a type prints is
/// what it serialises as.
macro_rules! display_as_str {
    ($($name:ty),+ $(,)?) => {$(
        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    )+};
}

mod id;
mod message;
mod outcome;
mod price;
mod provider;
mod provider_kind;
mod run;
mod stop;
mod summary;
mod tool;
mod transcript;
mod usage;

pub use id::{IdError, RunId, ToolCallId, ToolName};
pub use message::{
    ContentBlock, Message, ToolInput, ToolResult, ToolResultContent, ToolUse, UserContent,
};
pub use outcome::{OutcomeError, RunOutcome, TaskResult};
pub use price::{Cost, CostError, RateError, Rates};
pub use provider::{
    Effort, Endpoint, FinishReason, ModelRef, ProviderErrorKind, ProviderResponse, RequestParams,
    ResponseError, Thinking, UnknownReason,
};
pub use provider_kind::ProviderKind;
pub use run::{Progress, Prompts, Run, RunSetup};
pub use stop::{Calls, CompletionMode, StopClass, StopReason};
pub use summary::{FinishedRun, RunContext, RunSummary, ToolStats};
pub use tool::{ToolCallEnd, ToolCallOutcome, ToolCallStatus, ToolSource, ToolSpec};
pub use transcript::{Transcript, TranscriptError, Turn, TurnRecord};
pub use usage::{TokenCounts, Usage};

/// Whole milliseconds, truncated. The one conversion from a `Duration` in the
/// model, so every `*_ms` value is cut the same way.
pub(crate) fn whole_ms(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
