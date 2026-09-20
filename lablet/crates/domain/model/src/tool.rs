//! Tools as the model sees them, and what a run records about each call.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::outcome::whole_ms;
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

/// What became of one tool call: either the run offered no tool by that name,
/// or a tool ran and ended some way.
///
/// "The model called a tool the run doesn't have" used to be three facts that
/// could disagree: a status, a missing source, and a name absent from the
/// run's tool list. It's one here. A call has a [`ToolSource`] exactly when a
/// tool ran, so the summary reads both what it counts as unknown and which
/// calls earn a per-tool entry from this one value, and the
/// `lablet.tool.source` and `gen_ai.tool.type` attributes of the call's
/// `execute_tool` span are present on exactly the same calls.
///
/// Every status but [`ToolCallEnd::Ok`] is an error result for the model, and
/// [`ToolCallStatus::as_str`] is the `error.type` of the span; a call that
/// ended `ok` has no `error.type`.
///
/// Written `"unknown"`, `"malformed_input"`, or
/// `{"ran": {"source": "builtin", "ended": "ok"}}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolCallStatus {
    /// No configured tool has the name the model called, so nothing ran.
    Unknown,
    /// The model's arguments for the call weren't valid JSON, so nothing ran.
    /// The model is sent its own text back as an error result, which is what
    /// lets it correct itself; the alternative, refusing the response, spends
    /// a retry re-rolling the same prompt and reports a tool-surface problem
    /// as provider flakiness.
    MalformedInput,
    /// A tool ran.
    Ran {
        /// Where the tool that ran comes from.
        source: ToolSource,
        /// How it ended.
        ended: ToolCallEnd,
    },
}

/// How a tool that ran ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallEnd {
    /// The tool returned a result.
    Ok,
    /// The tool reported an error in its own result, as an MCP tool does with
    /// `isError`.
    ToolError,
    /// The call ran past the tool timeout.
    Timeout,
    /// The executor failed before the tool could answer.
    Failed,
}

impl ToolCallEnd {
    /// The serde spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ToolError => "tool_error",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
        }
    }
}

impl ToolCallStatus {
    /// A call to a tool that ran and ended `ended`.
    #[must_use]
    pub const fn ran(source: ToolSource, ended: ToolCallEnd) -> Self {
        Self::Ran { source, ended }
    }

    /// The `lablet.tool.status` value, which flattens the two levels into the
    /// registry's five.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::MalformedInput => "malformed_input",
            Self::Ran { ended, .. } => ended.as_str(),
        }
    }

    /// Where the tool that ran comes from; `None` when none did.
    #[must_use]
    pub const fn source(&self) -> Option<&ToolSource> {
        match self {
            Self::Unknown | Self::MalformedInput => None,
            Self::Ran { source, .. } => Some(source),
        }
    }

    /// Whether the model is sent an error result. A name the run doesn't have
    /// is one, because the model is told so.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        !matches!(
            self,
            Self::Ran {
                ended: ToolCallEnd::Ok,
                ..
            }
        )
    }
}

display_as_str!(ToolCallEnd);

/// For people, and for the `lablet.tool.status` value.
impl core::fmt::Display for ToolCallStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What happened to one executed tool call: a member of the [`crate::Turn`]
/// whose response made the call.
///
/// The call's name and input aren't here, because the response's
/// [`crate::ToolUse`] block with the same id holds them. Nor are the sizes:
/// [`crate::ToolUse::input_bytes`] and [`ToolCallOutcome::output_bytes`]
/// measure what's stored. Whether the model was sent an error, and where the
/// tool came from, are both read from `status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallOutcome {
    /// The id of the [`crate::ToolUse`] this answers.
    pub call_id: ToolCallId,
    /// What became of the call.
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
