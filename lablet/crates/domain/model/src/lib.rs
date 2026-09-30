//! The domain model: the transcript and its turns, tools, usage, stop reasons,
//! run identity, the run summary, and the secrets no tool result shows, as
//! plain types and pure functions.
//!
//! These are values in memory, and none of them is a published shape: the
//! documents lablet writes and the scripts it reads have shapes of their
//! own, in the adapters, mapped from and to these. JSON is here as
//! `serde_json::Value`, for what a provider API defines as JSON, and as the
//! `Serialize` of the message form and the tool specs, which the loop
//! measures with and publishes nowhere.

/// Implements `Display` through the type's `as_str`, so what a type prints is
/// what a record spells it as.
macro_rules! display_as_str {
    ($($name:ty),+ $(,)?) => {$(
        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    )+};
}

/// Declares a list of an enum's variants that can't miss one.
///
/// `every_variant!(Name::ALL = [A, B, C])` declares `Name::ALL`, every
/// variant of a fieldless enum, each once, in the order the list names them.
/// An enum with variants that hold data lists the others under a name and a
/// doc of its own, and gives a pattern for the rest after `besides`.
///
/// Beside the list is a match with an arm for each variant it names, and
/// for the `besides` pattern, and for nothing else. So a variant the enum
/// gains doesn't compile until the list or the pattern has it, and neither
/// does a variant named twice.
#[macro_export]
macro_rules! every_variant {
    ($name:ident::ALL = [$($variant:ident),+ $(,)?]) => {
        $crate::every_variant!(@declare
            [#[doc = "Every variant, each once."]]
            $name::ALL = [$($variant),+] besides []
        );
    };
    (
        $(#[$doc:meta])+
        $name:ident::$list:ident = [$($variant:ident),+ $(,)?],
        besides $rest:pat
    ) => {
        $crate::every_variant!(@declare
            [$(#[$doc])+] $name::$list = [$($variant),+] besides [$rest]
        );
    };
    (@declare
        [$(#[$doc:meta])+] $name:ident::$list:ident = [$($variant:ident),+] besides [$($rest:pat)?]
    ) => {
        impl $name {
            $(#[$doc])+
            pub const $list: [Self; [$(stringify!($variant)),+].len()] = [$(Self::$variant),+];
        }

        #[deny(unreachable_patterns)]
        const _: () = match &$name::$list[0] {
            $($name::$variant)|+ => (),
            $($rest => (),)?
        };
    };
}

mod digest;
mod id;
mod labels;
mod mcp;
mod message;
mod outcome;
mod output;
mod price;
mod provider;
mod provider_kind;
mod run;
mod secret;
mod stop;
mod summary;
mod tool;
mod totals;
mod transcript;
mod usage;

pub use digest::{ConfigDigest, DigestError};
pub use id::{IdError, RunId, ToolCallId, ToolName};
pub use labels::RunLabels;
pub use mcp::{McpLifetime, McpServer, McpServers, NoMcpServers};
pub use message::{
    ContentBlock, Message, ToolInput, ToolResult, ToolResultContent, ToolUse, UserContent,
};
pub use outcome::{OutcomeError, OutcomeParts, RunOutcome, TaskResult};
pub use output::{KeptOutput, OutputCap, OutputCapError, OutputCut, OutputKeep};
pub use price::{Cost, CostError, RateError, Rates};
pub use provider::{
    CacheScope, Effort, Endpoint, FinishReason, ModelRef, ProviderApi, ProviderErrorKind,
    ProviderResponse, RequestParams, ResponseError, Thinking, UnknownReason,
};
pub use provider_kind::ProviderKind;
pub use run::{
    BlankTask, FailedAttempt, Final, Pending, Progress, Prompts, Responded, Run, RunSetup, Schedule,
};
pub use secret::{RedactedOutput, Secrets};
pub use stop::{Calls, CompletionMode, StopClass, StopReason};
pub use summary::{FinishedRun, PromptSizes, RunContext, RunSummary};
pub use tool::{
    Answer, ToolCallEnd, ToolCallOutcome, ToolCallStatus, ToolConcurrency, ToolSource, ToolSpec,
};
pub use totals::{Latency, ProviderTotals, ToolCallTotals, ToolStats};
pub use transcript::{Transcript, TranscriptParts, Turn, TurnParts, TurnRecord};
pub use usage::{TokenCounts, Usage};

/// Whole milliseconds, truncated. The one conversion from a `Duration` in the
/// model, so every `*_ms` value is cut the same way.
pub(crate) fn whole_ms(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
