//! The application ring: `RunService`, the agent loop, and the secondary
//! ports it consumes.
//!
//! The loop owns the clock, the ports, and the order things happen in. It
//! owns no rule that the domain could hold instead: every stop decision comes
//! from `lablet-policy`, and every total from the model's `Run`.

mod clock;
mod observer;
mod provider;
mod tool;

pub use clock::{Cancellation, Clock};
pub use observer::{EventKind, RunEvent, RunObserver, TraceContext};
pub use provider::{ModelProvider, ProviderError, ProviderRequest};
pub use tool::{
    McpCallMeta, NetworkTransport, ToolCall, ToolError, ToolErrorKind, ToolExecutor, ToolOutput,
};
