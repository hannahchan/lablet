//! The port a tool executor implements, what one call carries, and the MCP
//! metadata a call over MCP brings back.

use std::time::Duration;

use lablet_model::{KeptOutput, OutputKeep, ToolCallEnd, ToolCallId, ToolName, ToolSpec};

use crate::{TraceContext, bounded};

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
    /// How much of the tool's text the run's output cap can use, which is
    /// what the executor's [`KeptOutput`] is made from. `None` when the run
    /// has no cap, which keeps everything.
    pub keep: Option<OutputKeep>,
    /// The span the observer opened for this call, for an executor that
    /// propagates one. `None` when no observer keeps spans.
    pub trace_context: Option<TraceContext>,
}

/// What a tool returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// What the executor kept of the tool's text, and the size of all of it.
    /// The loop cuts it to the run's output cap, so no executor decides what
    /// the model is sent of a long output.
    pub output: KeptOutput,
    /// The tool's own report that it failed, as MCP's `isError`. A tool that
    /// ran and said so is not an executor failure.
    pub is_error: bool,
    /// Present when the call went over MCP.
    pub mcp: Option<McpCallMeta>,
}

/// How an executor failed to get an answer from a tool.
///
/// The message is private because [`ToolError::new`] cuts it to
/// [`ERROR_MESSAGE_MAX_BYTES`](crate::ERROR_MESSAGE_MAX_BYTES), and a field
/// anyone could write would let a message past the cut.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ToolError {
    /// What kind of failure this is.
    pub kind: ToolErrorKind,
    message: String,
    /// Present when the call went over MCP.
    pub mcp: Option<McpCallMeta>,
}

impl ToolError {
    /// A failure of `kind`, described by `message`, over no MCP server. The
    /// message is cut to
    /// [`ERROR_MESSAGE_MAX_BYTES`](crate::ERROR_MESSAGE_MAX_BYTES).
    pub fn new(kind: ToolErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: bounded(message.into()),
            mcp: None,
        }
    }

    /// What the executor says happened. The model is sent this as its error
    /// result, so it's what the model has to work from.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The same failure, with what the MCP call carried back. A call that
    /// failed still has a span, and these are its `mcp.*` attributes.
    #[must_use]
    pub fn over_mcp(mut self, mcp: McpCallMeta) -> Self {
        self.mcp = Some(mcp);
        self
    }
}

/// Why an executor couldn't answer.
///
/// Every kind becomes an error result for the model and none ends the run:
/// a tool that fails is something the model can work around, and how it does
/// is part of what a run measures.
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
    /// The executor feeds the tool's text to a [`KeptOutput`] made from
    /// [`ToolCall::keep`], as the text arrives, so it never holds more than
    /// the run's cap can use however much the tool writes. An output kept
    /// to any other limit is cut from what was kept, and says so.
    ///
    /// # Errors
    ///
    /// Returns a [`ToolError`] when no answer came back. A tool that ran and
    /// reported its own failure is `Ok` with
    /// [`ToolOutput::is_error`] set, not an error here.
    ///
    /// The error's message holds no credentials: no user info or query
    /// string of a URL, no header value, and a response body only when the
    /// run captures content. The type holds the message's length and nothing
    /// can hold this, so it's the executor's obligation.
    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError>;
}
