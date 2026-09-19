//! Tools as the model sees them, and what a run records about each call.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::run::whole_ms;
use crate::{ToolCallId, ToolName, ToolResult, ToolResultContent};

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

/// For people: an MCP tool's source names its server, as `mcp:docs`.
impl core::fmt::Display for ToolSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Builtin => f.write_str(self.as_str()),
            Self::Mcp { server } => write!(f, "{}:{server}", self.as_str()),
        }
    }
}

/// How an executed tool call ended. One value says both whether the model got
/// an error result and why, so the two can't disagree.
///
/// Every status but [`ToolCallStatus::Ok`] is an error result for the model,
/// and its [`ToolCallStatus::as_str`] is the `error.type` of the call's
/// `execute_tool` span; a call that ended `ok` has no `error.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// The tool ran and returned a result.
    Ok,
    /// The tool ran and reported an error in its own result, as an MCP tool
    /// does with `isError`.
    ToolError,
    /// No configured tool has the name the model called.
    Unknown,
    /// The call ran past the tool timeout.
    Timeout,
    /// The executor failed before the tool could answer.
    Failed,
}

impl ToolCallStatus {
    /// The serde spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ToolError => "tool_error",
            Self::Unknown => "unknown",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
        }
    }

    /// Whether the model is sent an error result.
    #[must_use]
    pub const fn is_error(self) -> bool {
        !matches!(self, Self::Ok)
    }
}

display_as_str!(ToolCallStatus);

/// What happened to one executed tool call: a member of the [`crate::Turn`]
/// whose response made the call.
///
/// The call's name and input aren't here, because the response's
/// [`crate::ToolUse`] block with the same id holds them. Nor are the sizes:
/// [`crate::ToolUse::input_bytes`] and [`ToolCallOutcome::output_bytes`]
/// measure what's stored. Whether the model was sent an error is read from
/// `status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallOutcome {
    /// The id of the [`crate::ToolUse`] this answers.
    pub call_id: ToolCallId,
    /// Where the tool comes from; `None` for a name no configured tool has.
    pub source: Option<ToolSource>,
    /// How the call ended.
    pub status: ToolCallStatus,
    /// When the call started, in whole milliseconds since the run started.
    pub started_ms: u64,
    /// How long the call took, in whole milliseconds.
    pub latency_ms: u64,
    /// The size in bytes of the tool's output before the output cap cut it;
    /// `None` when nothing was cut.
    pub truncated_from_bytes: Option<u64>,
    /// What the model was sent: the tool's output, or the message of the
    /// executor's error, after the output cap.
    pub content: Vec<ToolResultContent>,
}

impl ToolCallOutcome {
    /// The outcome of the call `call_id`, which started `started` into the
    /// run and took `latency`.
    ///
    /// `content` is what the tool returned, or the message of the executor's
    /// error. `max_output_bytes` is the run's cap on it, `None` for no cap;
    /// it's applied here so that every tool's output is cut the same way: at a
    /// character boundary, with one last line that says what was cut.
    #[must_use]
    pub fn measured(
        call_id: ToolCallId,
        source: Option<ToolSource>,
        status: ToolCallStatus,
        content: Vec<ToolResultContent>,
        max_output_bytes: Option<u64>,
        started: Duration,
        latency: Duration,
    ) -> Self {
        let (content, truncated_from_bytes) = match max_output_bytes {
            Some(max_bytes) => ToolResultContent::capped(content, max_bytes),
            None => (content, None),
        };
        Self {
            call_id,
            source,
            status,
            started_ms: whole_ms(started),
            latency_ms: whole_ms(latency),
            truncated_from_bytes,
            content,
        }
    }

    /// The size in bytes of the output the model was sent, which is the size
    /// after the output cap and includes the line that says what was cut.
    #[must_use]
    pub fn output_bytes(&self) -> u64 {
        ToolResultContent::bytes(&self.content)
    }

    /// The result as a provider call sends it.
    pub(crate) fn result(&self) -> ToolResult<'_> {
        ToolResult {
            call_id: &self.call_id,
            content: &self.content,
            is_error: self.status.is_error(),
        }
    }
}

#[cfg(test)]
mod tests;
