//! The domain model: conversation, tools, usage, stop reasons, run identity, and
//! the run summary, as plain types and pure functions with serde derives.
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

mod conversation;
mod id;
mod provider;
mod run;
mod tally;
mod tool;
mod transcript;

pub use conversation::{
    ContentBlock, Message, MessageError, Role, ToolResult, ToolResultContent, ToolUse,
};
pub use id::{IdError, RunId, ToolCallId, ToolName};
pub use provider::{
    Completion, Cost, Effort, Endpoint, FinishReason, ModelRef, ProviderKind, RequestDefaults,
    Thinking, Usage,
};
pub use run::{
    CompletionMode, FinishedRun, RunContext, RunOutcome, RunResult, RunSummary, StopReason,
    ToolStats,
};
pub use tally::{Progress, RunSetup, RunTally};
pub use tool::{McpCallMeta, NetworkTransport, ToolSource, ToolSpec, TraceContext};
pub use transcript::{Transcript, Turn, TurnRecord};
