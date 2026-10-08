//! The application ring: `RunService`, the agent loop, `Runner`, which runs
//! a run around it, and the secondary ports they consume.
//!
//! The loop owns the clock, the ports, and the order things happen in. It
//! owns no rule that the domain could hold instead: every stop decision comes
//! from `lablet-policy`, and every total from the model's `Run`. The runner
//! owns what names a run, its root span, its transcript and its wide event.

mod cancellation;
mod clock;
mod provider;
mod runner;
mod service;
mod shown;
pub mod telemetry;
mod tool;
mod toolset;
mod transcript;

pub use cancellation::{NeverCancelled, RunCancellation};
pub use clock::{Cancellation, Clock};
pub use provider::{ModelProvider, ProviderError, ProviderRequest};
pub use runner::{Ran, RunStart, Runner, Shared};
pub use service::{CallLimits, RunService};
pub use tool::{
    McpCallMeta, NetworkTransport, ToolCall, ToolError, ToolErrorKind, ToolExecutor, ToolOutput,
};
pub use toolset::{FilterList, ToolFilter, ToolSet, ToolSetError};
pub use transcript::{RunTranscript, TranscriptError, TranscriptWriter};

/// The longest message of a [`ProviderError`], and the longest error result
/// the loop makes of a [`ToolError`]'s message, in bytes.
///
/// An adapter's error text can be as long as whatever a server sent back, and
/// it reaches the outcome, every exporter and, for a tool, the model. A
/// provider error cuts its message to this when it's built, and the loop
/// cuts a tool error's message to it after the run's secrets are cut out of
/// it, since a value the bound chopped would leave its edge in the text; so
/// no adapter can leave the bound out.
pub const ERROR_MESSAGE_MAX_BYTES: usize = 2_048;

/// `message`, cut to [`ERROR_MESSAGE_MAX_BYTES`] at the last character
/// boundary the bound allows.
pub(crate) fn bounded(mut message: String) -> String {
    message.truncate(message.floor_char_boundary(ERROR_MESSAGE_MAX_BYTES));
    message
}

#[cfg(test)]
mod tests;
