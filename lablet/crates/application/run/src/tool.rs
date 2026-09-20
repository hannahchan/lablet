//! The port a tool executor implements, what one call carries, and the MCP
//! metadata a call over MCP brings back.

use std::time::Duration;

use lablet_model::{ToolCallEnd, ToolCallId, ToolName, ToolResultContent, ToolSpec};

use crate::TraceContext;

/// One tool call, as the loop hands it to an executor.
///
/// `input` is a value rather than [`lablet_model::ToolInput`], because a call
/// whose arguments didn't parse never reaches an executor: the loop settles it
/// as [`lablet_model::ToolCallStatus::MalformedInput`] and sends the model its
/// own text back.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// The id the outcome will answer to.
    pub id: ToolCallId,
    /// The tool to call.
    pub name: ToolName,
    /// The arguments the model produced.
    pub input: serde_json::Value,
    /// How long the executor may take before it gives up.
    pub deadline: Duration,
    /// The span the observer opened for this call, for an executor that
    /// propagates one. `None` when no observer keeps spans.
    pub trace_context: Option<TraceContext>,
}

/// What a tool returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// What the model is sent, before the run's output cap.
    pub content: Vec<ToolResultContent>,
    /// The tool's own report that it failed, as MCP's `isError`. A tool that
    /// ran and said so is not an executor failure.
    pub is_error: bool,
    /// Present when the call went over MCP.
    pub mcp: Option<McpCallMeta>,
}

/// How an executor failed to get an answer from a tool.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ToolError {
    /// What kind of failure this is.
    pub kind: ToolErrorKind,
    /// What the executor says happened. The model is sent this as its error
    /// result, so it's what the model has to work from.
    pub message: String,
    /// Present when the call went over MCP.
    pub mcp: Option<McpCallMeta>,
}

impl ToolError {
    /// A failure of `kind`, described by `message`, over no MCP server.
    pub fn new(kind: ToolErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            mcp: None,
        }
    }
}

/// Why an executor couldn't answer.
///
/// Every kind becomes an error result for the model rather than ending the
/// run: a tool that fails is something the model can work around, which is
/// what the consecutive-error cap is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolErrorKind {
    /// No tool this executor serves has that name.
    Unknown,
    /// The call ran past its deadline.
    Timeout,
    /// The executor failed before the tool could answer.
    Failed,
}

impl ToolErrorKind {
    /// How a tool that ran ended, for the two kinds that mean one did.
    /// [`ToolErrorKind::Unknown`] means none did, so it has no ending.
    #[must_use]
    pub const fn ended(self) -> Option<ToolCallEnd> {
        match self {
            Self::Unknown => None,
            Self::Timeout => Some(ToolCallEnd::Timeout),
            Self::Failed => Some(ToolCallEnd::Failed),
        }
    }
}

/// What a call over MCP carries back, for the `mcp.*` attributes of its span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCallMeta {
    /// The JSON-RPC method, `tools/call`.
    pub method: String,
    /// The session the transport negotiated, when it has one.
    pub session_id: Option<String>,
    /// The protocol version, when one was negotiated.
    pub protocol_version: Option<String>,
    /// The JSON-RPC request id, when the request had one.
    pub jsonrpc_request_id: Option<String>,
    /// The error code of a JSON-RPC error response.
    pub rpc_status_code: Option<String>,
    /// How the server is reached.
    pub transport: NetworkTransport,
}

/// How an MCP server is reached, which is the `network.transport` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetworkTransport {
    /// A server over stdio.
    Pipe,
    /// A server over HTTP.
    Tcp,
}

impl NetworkTransport {
    /// The `network.transport` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pipe => "pipe",
            Self::Tcp => "tcp",
        }
    }
}

/// Something that runs tools for a run.
///
/// An executor resolves only the names it offered in [`ToolExecutor::specs`],
/// and the set it offers is fixed when the run is built. That's this port's
/// obligation rather than a rule the model can hold: a
/// [`lablet_model::ToolCallStatus::Ran`] carries the source the executor
/// resolved, and the run believes it, which is what keeps the per-tool
/// telemetry keys bounded. A server that announces new tools mid-run offers
/// them to the next run, not this one.
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Every tool this executor offers, asked for once when the run starts.
    ///
    /// # Errors
    ///
    /// Returns a [`ToolError`] when the executor can't say what it offers,
    /// which ends the run before its first provider call.
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError>;

    /// Runs one call.
    ///
    /// # Errors
    ///
    /// Returns a [`ToolError`] when no answer came back. A tool that ran and
    /// reported its own failure is `Ok` with
    /// [`ToolOutput::is_error`] set, not an error here.
    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError>;
}
