//! Tools as the model sees them, and what a tool call carries besides its result.

use serde::{Deserialize, Serialize};

use crate::ToolName;

/// A tool as it's offered to the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// The name the model calls it by.
    pub name: ToolName,
    /// What the model is told the tool does.
    pub description: String,
    /// The JSON Schema of the tool's input.
    pub input_schema: serde_json::Value,
    /// Where the tool comes from.
    pub source: ToolSource,
}

/// Where a tool comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    /// One of lablet's built-in tools.
    Builtin,
    /// A tool served by a configured MCP server.
    Mcp {
        /// The server's name in the config.
        server: String,
    },
}

impl ToolSource {
    /// The serde spelling of the variant, which is the `lablet.tool.source`
    /// value; the server name isn't part of it.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Mcp { .. } => "mcp",
        }
    }
}

/// A W3C trace context as header strings, so no OpenTelemetry type enters the
/// domain.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceContext {
    /// The `traceparent` header value.
    pub traceparent: String,
    /// The `tracestate` header value, when there is one.
    pub tracestate: Option<String>,
}

/// What an MCP tool call reports about the protocol exchange, for the `mcp.*`,
/// `jsonrpc.*`, `rpc.*`, and `network.transport` attributes of its span.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct McpCallMeta {
    /// The MCP method, `tools/call` for a tool call.
    pub method: String,
    /// The session id, for a transport that has sessions.
    pub session_id: Option<String>,
    /// The protocol version negotiated with the server.
    pub protocol_version: Option<String>,
    /// The JSON-RPC request id, as a string whatever its JSON type.
    pub jsonrpc_request_id: Option<String>,
    /// The JSON-RPC error code, when the call failed with one.
    pub rpc_status_code: Option<String>,
    /// The transport under the session.
    pub transport: NetworkTransport,
}

/// The transport under an MCP session, spelled as `network.transport` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkTransport {
    /// A stdio child process.
    Pipe,
    /// Streamable HTTP.
    Tcp,
}

impl NetworkTransport {
    /// The serde spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pipe => "pipe",
            Self::Tcp => "tcp",
        }
    }
}

display_as_str!(ToolSource, NetworkTransport);

#[cfg(test)]
mod tests;
