//! Tools as the model sees them, and what a run records about each call.

use std::time::Duration;

use serde::Serialize;

use crate::whole_ms;
use crate::{KeptOutput, OutputCap, ToolCallId, ToolName, ToolResult, ToolResultContent};

/// A tool as it's offered to the model.
///
/// A spec serialises, with the source and the concurrency it holds, because
/// the loop measures what a run offers by it: the size of the specs and
/// their digest are taken from each spec as compact JSON. Those bytes are
/// counted, hashed and discarded. A change to this form changes the digest
/// of every run, so runs from before and after it would no longer group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolSpec {
    /// The name the model calls it by.
    pub name: ToolName,
    /// What the model is told the tool does.
    pub description: String,
    /// The JSON Schema of the tool's input.
    pub input_schema: serde_json::Value,
    /// Where the tool comes from.
    pub source: ToolSource,
    /// Whether a call to it may run beside other calls in the same turn.
    pub concurrency: ToolConcurrency,
}

/// Whether a call to a tool may run at the same time as other calls of its
/// turn.
///
/// Consecutive calls to `Shared` tools form one group, which runs
/// concurrently; a call to an `Exclusive` tool runs alone. Groups run in call
/// order, so a read the model placed after a write still runs after it. A
/// tool nobody classified is `Exclusive`, which is how every call ran before
/// tools could say otherwise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolConcurrency {
    /// A call runs alone: the tool can change what another call sees.
    #[default]
    Exclusive,
    /// A call may run beside other shared calls: the tool only reads.
    Shared,
}

/// Where a tool comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
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
    /// The `lablet.tool.source` value, which is the variant alone; the
    /// server name isn't part of it.
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

/// What became of one tool call.
///
/// "The model called a tool the run doesn't have" used to be three facts that
/// could disagree: a status, a missing source, and a name absent from the
/// run's tool list. It's one here. A status holds a [`ToolSource`] exactly
/// when a tool ran, and the summary reads both what it counts as unknown and
/// which calls earn a per-tool entry from this one value.
///
/// The `lablet.tool.source` and `gen_ai.tool.type` attributes of the call's
/// `execute_tool` span aren't read from it. They're on every call whose name
/// resolved to a tool, a tool that ran or not: a call whose arguments didn't
/// parse and one the loop rejected have both, from the source the loop
/// announced the call with, and a status that holds none.
///
/// Every status but [`ToolCallEnd::Ok`] is an error result for the model, and
/// [`ToolCallStatus::as_str`] is the `error.type` of the span; a call that
/// ended `ok` has no `error.type`, and a call that was never run has no span.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolCallStatus {
    /// No configured tool has the name the model called, so nothing ran.
    Unknown,
    /// The model's arguments for the call weren't valid JSON, so nothing ran.
    /// The model is sent its own text back as an error result, which is what
    /// lets it correct itself; the alternative, refusing the response, spends
    /// a retry re-rolling the same prompt and reports a tool-surface problem
    /// as provider flakiness.
    MalformedInput,
    /// The loop declined a call it won't act on, so nothing ran: a
    /// `task_complete` call that wasn't the response's only call. Acting on
    /// it would report work as done that the same response only asked for,
    /// so the model is asked to make the call on its own.
    Rejected,
    /// The call's turn came when the run had no time left, or once the run
    /// had been cancelled, so nothing was started for it. The model did
    /// nothing wrong, so it isn't an invalid call. It's in the transcript,
    /// because a turn's outcomes answer every call of its response, and
    /// nowhere else: no total counts it, and no event reports it, so it has
    /// no span.
    NotRun,
    /// A tool ran.
    Ran {
        /// Where the tool that ran comes from.
        source: ToolSource,
        /// How it ended.
        ended: ToolCallEnd,
    },
}

/// How a tool that ran ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCallEnd {
    /// The tool returned a result.
    Ok,
    /// The tool reported an error in its own result, as an MCP tool does with
    /// `isError`.
    ToolError,
    /// The call reached its deadline, which is the shorter of its
    /// executor's own limit and the time the run had left.
    Timeout,
    /// The executor failed before the tool could answer.
    Failed,
    /// The run was cancelled while the call ran, so the call was stopped
    /// where it was and nothing it would have returned was kept. It ran, so
    /// it isn't [`ToolCallStatus::NotRun`]: it has a span and counts in the
    /// totals, as an error, for the time it took. No executor reports it;
    /// the loop does, when it drops the call.
    Cancelled,
}

impl ToolCallEnd {
    /// The `lablet.tool.status` value of a call to a tool that ran.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ToolError => "tool_error",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
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
    /// registry's one.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::MalformedInput => "malformed_input",
            Self::Rejected => "rejected",
            Self::NotRun => "not_run",
            Self::Ran { ended, .. } => ended.as_str(),
        }
    }

    /// Where the tool that ran comes from; `None` when none did.
    #[must_use]
    pub const fn source(&self) -> Option<&ToolSource> {
        match self {
            Self::Unknown | Self::MalformedInput | Self::Rejected | Self::NotRun => None,
            Self::Ran { source, .. } => Some(source),
        }
    }

    /// Whether the call named a tool the run offered.
    ///
    /// A call whose arguments didn't parse did, and so did one the loop
    /// rejected: nothing ran, but the name was real, so it earns its per-tool
    /// entry and isn't one of the calls to a name the run doesn't have. The
    /// loop resolves the name before it reads the arguments, so a bad name
    /// with bad arguments is [`ToolCallStatus::Unknown`].
    ///
    /// Nothing resolved the name of a call that was never run, so it isn't
    /// known to have named one: a per-tool entry is earned by a name that
    /// was looked up, or the model could add keys by inventing names.
    #[must_use]
    pub const fn names_an_offered_tool(&self) -> bool {
        match self {
            Self::Unknown | Self::NotRun => false,
            Self::MalformedInput | Self::Rejected | Self::Ran { .. } => true,
        }
    }

    /// Whether the call is one the model got wrong, so the loop answered it
    /// and it reached no tool. A call that reached a tool isn't one whatever
    /// the tool returned, and neither is a call the executor failed: those
    /// say how the tools did, where this says the model can't call them. A
    /// call that was never run isn't one either: the run's time was gone,
    /// or the run was cancelled, before anything could be said of the call.
    #[must_use]
    pub const fn is_invalid(&self) -> bool {
        match self {
            Self::Unknown | Self::MalformedInput | Self::Rejected => true,
            Self::NotRun | Self::Ran { .. } => false,
        }
    }

    /// Whether anything was started for the call: the loop took it up,
    /// opened its span, and answered it, with a tool or without
    /// one. Only a call that was never run wasn't, and the run's totals
    /// count the calls that were.
    #[must_use]
    pub const fn was_started(&self) -> bool {
        match self {
            Self::NotRun => false,
            Self::Unknown | Self::MalformedInput | Self::Rejected | Self::Ran { .. } => true,
        }
    }

    /// Whether the model is sent an error result. A name the run doesn't have
    /// is one, because the model is told so, and so is a call that was never
    /// run, though the run that left it unrun stops before the model is sent
    /// anything.
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

every_variant!(ToolCallEnd::ALL = [Ok, ToolError, Timeout, Failed, Cancelled]);

every_variant!(
    /// Every status of a call no tool ran for, each once. The rest are a
    /// [`ToolCallStatus::Ran`], one for each of [`ToolCallEnd::ALL`] and a
    /// tool from any source.
    ToolCallStatus::NOTHING_RAN = [Unknown, MalformedInput, Rejected, NotRun],
    besides ToolCallStatus::Ran { .. }
);

every_variant!(ToolConcurrency::ALL = [Exclusive, Shared]);

/// For people, and for the `lablet.tool.status` value.
impl core::fmt::Display for ToolCallStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What happened to one tool call: a member of the [`crate::Turn`] whose
/// response made the call.
///
/// The call's name and input aren't here, because the response's
/// [`crate::ToolUse`] block with the same id holds them. Nor are the sizes:
/// [`crate::ToolUse::input_bytes`] and [`ToolCallOutcome::output_bytes`]
/// measure what's stored. Error and source are read from `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// What the loop has to say about one tool call: its outcome, without the id
/// of the call it answers.
///
/// Only [`crate::Pending::answer`] adds the id, from the call it asked about,
/// so an answer can't be recorded against the wrong call. The loop reads the
/// capped sizes from it before handing it over, so what it reports about the
/// call is what the transcript will hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    status: ToolCallStatus,
    started_ms: u64,
    latency_ms: u64,
    truncated_from_bytes: Option<u64>,
    content: Vec<ToolResultContent>,
}

impl Answer {
    /// The answer to a call that started `started` into the run and took
    /// `latency`.
    ///
    /// `cap` is the run's cap on `output`, `None` for no cap. It's applied
    /// here, and nowhere else, so that every tool's output is cut the same
    /// way, and once: what comes out is content, and this takes only what an
    /// executor kept.
    #[must_use]
    pub fn measured(
        status: ToolCallStatus,
        output: KeptOutput,
        cap: Option<OutputCap>,
        started: Duration,
        latency: Duration,
    ) -> Self {
        let (content, truncated_from_bytes) = output.cut(cap);
        Self {
            status,
            started_ms: whole_ms(started),
            latency_ms: whole_ms(latency),
            truncated_from_bytes,
            content,
        }
    }

    /// What became of the call.
    #[must_use]
    pub const fn status(&self) -> &ToolCallStatus {
        &self.status
    }

    /// When the call started, in whole milliseconds since the run started.
    #[must_use]
    pub const fn started_ms(&self) -> u64 {
        self.started_ms
    }

    /// How long the call took, in whole milliseconds.
    #[must_use]
    pub const fn latency_ms(&self) -> u64 {
        self.latency_ms
    }

    /// The size in bytes of the output before the cap cut it; `None` when
    /// nothing was cut.
    #[must_use]
    pub const fn truncated_from_bytes(&self) -> Option<u64> {
        self.truncated_from_bytes
    }

    /// What the model is sent, after the cap.
    #[must_use]
    pub fn content(&self) -> &[ToolResultContent] {
        &self.content
    }

    /// The size in bytes of what the model is sent.
    #[must_use]
    pub fn output_bytes(&self) -> u64 {
        ToolResultContent::bytes(&self.content)
    }

    /// The outcome of the call `call_id`.
    pub(crate) fn answering(self, call_id: ToolCallId) -> ToolCallOutcome {
        let Self {
            status,
            started_ms,
            latency_ms,
            truncated_from_bytes,
            content,
        } = self;
        ToolCallOutcome {
            call_id,
            status,
            started_ms,
            latency_ms,
            truncated_from_bytes,
            content,
        }
    }
}

impl ToolCallOutcome {
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
