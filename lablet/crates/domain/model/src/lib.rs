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
mod provider;
mod run;
mod tool;
mod transcript;

pub use id::{IdError, RunId, ToolCallId, ToolName};
pub use message::{
    ContentBlock, Message, ToolInput, ToolResult, ToolResultContent, ToolUse, UserContent,
};
pub use outcome::{
    CompletionMode, FinishedRun, OutcomeError, RunContext, RunOutcome, RunSummary, StopClass,
    StopReason, TaskResult, ToolStats,
};
pub use provider::{
    Cost, CostError, Effort, Endpoint, FinishReason, ModelRef, ProviderErrorKind, ProviderKind,
    ProviderResponse, RateError, Rates, RequestParams, ResponseError, Thinking, TokenCounts,
    UnknownReason, Usage,
};
pub use run::{Progress, Prompts, Run, RunSetup};
pub use tool::{ToolCallEnd, ToolCallOutcome, ToolCallStatus, ToolSource, ToolSpec};
pub use transcript::{Calls, Transcript, TranscriptError, Turn, TurnRecord};
