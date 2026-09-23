//! What the loop tells anyone watching, as it happens.

use std::time::Duration;

use lablet_model::{
    ContentBlock, Endpoint, ModelRef, RunContext, RunId, RunSummary, ToolCallId, ToolCallStatus,
    ToolName, ToolResultContent, ToolSource, ToolSpec, TurnRecord,
};

use crate::{McpCallMeta, ProviderError, TraceContext};

/// One thing that happened in a run.
///
/// The run's id is a field of the event rather than of each kind, because
/// every observer needs it on every event — it's a join key on every signal —
/// and one field is one place to read it rather than eight match arms.
#[derive(Debug, Clone)]
pub struct RunEvent {
    /// The run this happened in.
    pub run_id: RunId,
    /// What happened.
    pub kind: EventKind,
}

/// What happened, and what came with it.
///
/// Content-bearing fields are `Option` and are `None` unless the run captures
/// content. The loop decides, so an observer is never handed content it
/// shouldn't emit; the byte counts beside them are always present.
#[derive(Debug, Clone)]
pub enum EventKind {
    /// The run began. Carries what only the composition root knew.
    RunStarted {
        /// The composer's half of the wide event.
        context: Box<RunContext>,
        /// The model the run calls.
        model: ModelRef,
        /// Where the provider is served, when it's reached over the network.
        endpoint: Option<Endpoint>,
        /// The tools offered to the model, after filtering.
        tools: Vec<ToolSpec>,
        /// The system prompt, when content is captured.
        system_prompt: Option<String>,
        /// The task prompt, when content is captured.
        prompt: Option<String>,
    },
    /// A turn began, counted from 1.
    TurnStarted {
        /// Which turn.
        turn: u32,
    },
    /// One attempt of a provider call began.
    ProviderCallStarted {
        /// Which turn the attempt belongs to.
        turn: u32,
        /// Which attempt of that call, counted from 1.
        attempt: u32,
        /// How much the conversation has grown, measured by the loop so every
        /// provider reports it the same way.
        request_bytes: u64,
    },
    /// An attempt returned a response, which became a turn.
    ProviderCallFinished {
        /// Which turn.
        turn: u32,
        /// Which attempt returned it.
        attempt: u32,
        /// What the run recorded about the call.
        record: Box<TurnRecord>,
        /// The response as the transcript stored it, when content is captured.
        response: Option<Vec<ContentBlock>>,
    },
    /// An attempt failed.
    ProviderCallFailed {
        /// Which turn.
        turn: u32,
        /// Which attempt failed.
        attempt: u32,
        /// Why.
        error: ProviderError,
        /// How long the loop waits before the attempt that follows, or `None`
        /// when this attempt was the last. One field rather than a flag beside
        /// a duration, so "retrying after no wait" and "not retrying, after
        /// this wait" can't be written down. It's the decision made when the
        /// attempt failed: a run cancelled during the wait ends before the
        /// next attempt, and `RunFinished` says so.
        retry: Option<Duration>,
    },
    /// A tool call began.
    ToolCallStarted {
        /// Which turn made the call.
        turn: u32,
        /// The call's id.
        call_id: ToolCallId,
        /// The name the model called.
        name: ToolName,
        /// Where the tool comes from; `None` for a name no tool has, which is
        /// why the call's span carries no source and no tool type.
        source: Option<ToolSource>,
        /// The size of the arguments the model produced.
        input_bytes: u64,
        /// Those arguments, when content is captured.
        input: Option<serde_json::Value>,
    },
    /// A tool call ended. Every field is read from the call's outcome, so
    /// what an observer reports and what the transcript holds agree.
    ToolCallFinished {
        /// Which turn made the call.
        turn: u32,
        /// The call's id.
        call_id: ToolCallId,
        /// What became of it.
        status: ToolCallStatus,
        /// How long it took.
        latency_ms: u64,
        /// The size the model was sent, after the output cap.
        output_bytes: u64,
        /// The size before the cap cut it, when it did.
        truncated_from_bytes: Option<u64>,
        /// Present when the call went over MCP.
        mcp: Option<McpCallMeta>,
        /// What the model was sent, when content is captured.
        output: Option<Vec<ToolResultContent>>,
    },
    /// The run ended. This is the last event, and the wide event is emitted
    /// from it.
    RunFinished {
        /// The composer's half of the wide event.
        context: Box<RunContext>,
        /// The loop's half, whose `outcome` is the outcome document.
        summary: Box<RunSummary>,
    },
}

impl EventKind {
    /// The variant's name, which is how a test states the shape of a run
    /// without repeating every field.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::RunStarted { .. } => "RunStarted",
            Self::TurnStarted { .. } => "TurnStarted",
            Self::ProviderCallStarted { .. } => "ProviderCallStarted",
            Self::ProviderCallFinished { .. } => "ProviderCallFinished",
            Self::ProviderCallFailed { .. } => "ProviderCallFailed",
            Self::ToolCallStarted { .. } => "ToolCallStarted",
            Self::ToolCallFinished { .. } => "ToolCallFinished",
            Self::RunFinished { .. } => "RunFinished",
        }
    }
}

/// Something watching a run.
///
/// An observer never fails a run and never slows one: `on` returns promptly
/// and an exporter buffers. A failure to export is the observer's problem,
/// reported on lablet's diagnostic log, and the run's outcome doesn't change.
#[async_trait::async_trait]
pub trait RunObserver: Send + Sync {
    /// Takes one event.
    async fn on(&self, event: RunEvent);

    /// The span this observer opened for `call_id`, for an executor that
    /// propagates one. The loop asks after it emits `ToolCallStarted`, and
    /// puts the answer on the [`crate::ToolCall`]. An observer that keeps no
    /// spans answers `None`, which is the default.
    fn trace_context(&self, call_id: &ToolCallId) -> Option<TraceContext> {
        let _ = call_id;
        None
    }
}
