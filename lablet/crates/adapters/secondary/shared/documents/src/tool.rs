//! The tools a run offered and what became of each call, as a transcript
//! records them.

use lablet_model::{self as model, ToolCallId, ToolName};
use serde::Serialize;

use crate::content::ToolResultContent;

/// A tool as it was offered to the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ToolSpec {
    #[serde(with = "crate::id::tool_name")]
    name: ToolName,
    description: String,
    input_schema: serde_json::Value,
    source: ToolSource,
    concurrency: ToolConcurrency,
}

impl From<model::ToolSpec> for ToolSpec {
    fn from(spec: model::ToolSpec) -> Self {
        let model::ToolSpec {
            name,
            description,
            input_schema,
            source,
            concurrency,
        } = spec;
        Self {
            name,
            description,
            input_schema,
            source: source.into(),
            concurrency: concurrency.into(),
        }
    }
}

/// Where a tool comes from: `"builtin"` or `{"mcp": {"server": "docs"}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolSource {
    Builtin,
    Mcp { server: String },
}

impl From<model::ToolSource> for ToolSource {
    fn from(source: model::ToolSource) -> Self {
        match source {
            model::ToolSource::Builtin => Self::Builtin,
            model::ToolSource::Mcp { server } => Self::Mcp { server },
        }
    }
}

/// Whether a call to a tool may run beside other calls of its turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolConcurrency {
    Exclusive,
    Shared,
}

impl From<model::ToolConcurrency> for ToolConcurrency {
    fn from(concurrency: model::ToolConcurrency) -> Self {
        match concurrency {
            model::ToolConcurrency::Exclusive => Self::Exclusive,
            model::ToolConcurrency::Shared => Self::Shared,
        }
    }
}

/// What happened to one tool call. The call's name and input are in the
/// response's tool-use block of the same id, and nowhere else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ToolCallOutcome {
    #[serde(with = "crate::id::tool_call_id")]
    call_id: ToolCallId,
    status: ToolCallStatus,
    started_ms: u64,
    latency_ms: u64,
    truncated_from_bytes: Option<u64>,
    content: Vec<ToolResultContent>,
}

impl From<model::ToolCallOutcome> for ToolCallOutcome {
    fn from(outcome: model::ToolCallOutcome) -> Self {
        let model::ToolCallOutcome {
            call_id,
            status,
            started_ms,
            latency_ms,
            truncated_from_bytes,
            content,
        } = outcome;
        Self {
            call_id,
            status: status.into(),
            started_ms,
            latency_ms,
            truncated_from_bytes,
            content: content.into_iter().map(Into::into).collect(),
        }
    }
}

/// What became of a call: `"unknown"`, `"malformed_input"`, `"rejected"`,
/// `"not_run"`, or `{"ran": {"source": "builtin", "ended": "ok"}}`.
///
/// It keeps the two levels the domain holds, where telemetry flattens them
/// to one value, so a reader can tell where the tool that failed came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolCallStatus {
    Unknown,
    MalformedInput,
    Rejected,
    NotRun,
    Ran {
        source: ToolSource,
        ended: ToolCallEnd,
    },
}

impl From<model::ToolCallStatus> for ToolCallStatus {
    fn from(status: model::ToolCallStatus) -> Self {
        match status {
            model::ToolCallStatus::Unknown => Self::Unknown,
            model::ToolCallStatus::MalformedInput => Self::MalformedInput,
            model::ToolCallStatus::Rejected => Self::Rejected,
            model::ToolCallStatus::NotRun => Self::NotRun,
            model::ToolCallStatus::Ran { source, ended } => Self::Ran {
                source: source.into(),
                ended: ended.into(),
            },
        }
    }
}

/// How a tool that ran ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolCallEnd {
    Ok,
    ToolError,
    Timeout,
    Failed,
    Cancelled,
}

impl From<model::ToolCallEnd> for ToolCallEnd {
    fn from(ended: model::ToolCallEnd) -> Self {
        match ended {
            model::ToolCallEnd::Ok => Self::Ok,
            model::ToolCallEnd::ToolError => Self::ToolError,
            model::ToolCallEnd::Timeout => Self::Timeout,
            model::ToolCallEnd::Failed => Self::Failed,
            model::ToolCallEnd::Cancelled => Self::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests;
