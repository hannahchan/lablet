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
mod provider;
mod run;
mod tally;
mod tool;
mod transcript;

pub use id::{IdError, RunId, ToolCallId, ToolName};
pub use message::{ContentBlock, Message, ToolResult, ToolResultContent, ToolUse, UserContent};
pub use provider::{
    Completion, CompletionError, Cost, Effort, Endpoint, FinishReason, ModelRef, ProviderKind,
    RequestDefaults, Thinking, Usage,
};
pub use run::{
    CompletionMode, FinishedRun, OutcomeError, RunContext, RunOutcome, RunResult, RunSummary,
    StopClass, StopReason, ToolStats,
};
pub use tally::{Progress, RunSetup, RunTally};
pub use tool::{ToolCallOutcome, ToolCallStatus, ToolSource, ToolSpec};
pub use transcript::{Transcript, TranscriptError, Turn, TurnRecord};
