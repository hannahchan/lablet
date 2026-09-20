//! The application ring: `RunService`, the agent loop, and the secondary
//! ports it consumes.
//!
//! The loop owns the clock, the ports, and the order things happen in. It
//! owns no rule that the domain could hold instead: every stop decision comes
//! from `lablet-policy`, and every total from the model's `Run`.

mod clock;
mod observer;
mod provider;
mod service;
mod tool;
mod toolset;
mod trace;

pub use clock::{Cancellation, Clock};
pub use observer::{EventKind, RunEvent, RunObserver};
pub use provider::{ModelProvider, ProviderError, ProviderRequest};
pub use service::{CallLimits, RunService};
pub use tool::{
    McpCallMeta, NetworkTransport, ToolCall, ToolError, ToolErrorKind, ToolExecutor, ToolOutput,
};
pub use toolset::{ToolFilter, ToolSet, ToolSetError};
pub use trace::TraceContext;

#[cfg(test)]
mod tests;
