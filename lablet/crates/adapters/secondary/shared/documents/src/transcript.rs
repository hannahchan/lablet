//! The transcript document: one run's conversation, under a version and
//! with what names the run.

use lablet_model::{
    self as model, ModelRef, RunContext, RunId, Transcript, TranscriptParts, TurnParts,
};
use serde::Serialize;

use crate::content::{ContentBlock, ProviderKind, UserContent};
use crate::labels::Labels;
use crate::tool::{ToolCallOutcome, ToolSpec};
use crate::usage::Usage;

/// The version of the transcript document, raised when a reader that knows
/// the old form could misread the new one.
///
/// Dropping a field, renaming one, or changing what a field means all need a
/// raise. Adding one doesn't: a reader that doesn't know a key ignores it.
pub const TRANSCRIPT_SCHEMA_VERSION: u32 = 1;

/// One run's conversation, as the document lablet writes.
///
/// The version comes first, so a reader knows the form before the content.
/// Then what names the run, so a transcript found on its own says which run
/// it's of and joins the run's outcome and its telemetry; then the tools the
/// run offered; then the conversation. Times in the turns are offsets from
/// `started_unix_ms`.
///
/// It's written and never read, so it has no way in but
/// [`TranscriptDocument::new`] and its version is always this crate's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptDocument {
    schema_version: u32,
    #[serde(with = "crate::id::run_id")]
    run_id: RunId,
    labels: Labels,
    config_digest: String,
    lablet_version: String,
    model: Model,
    started_unix_ms: u64,
    tools: Vec<ToolSpec>,
    system: String,
    turns: Vec<Turn>,
}

impl TranscriptDocument {
    /// The document of the run that `context` names, which called `model`,
    /// was offered `tools` in that order, and held `transcript`.
    ///
    /// What names the run is read from the context, which is where the
    /// composition root states it; the outcome's copy of the id and the
    /// labels is the loop's copy of these.
    #[must_use]
    pub fn new(
        context: RunContext,
        model: ModelRef,
        tools: Vec<model::ToolSpec>,
        transcript: Transcript,
    ) -> Self {
        // What the context holds beyond what names a run describes the
        // process around it, which the wide event reports.
        let RunContext {
            run_id,
            labels,
            started_unix_ms,
            config_digest,
            agent_version,
            resource: _,
            transcript_path: _,
            skills_count: _,
            mcp: _,
            capture_content: _,
        } = context;
        let TranscriptParts { system, turns } = transcript.into_parts();
        Self {
            schema_version: TRANSCRIPT_SCHEMA_VERSION,
            run_id,
            labels: labels.into(),
            config_digest,
            lablet_version: agent_version,
            model: model.into(),
            started_unix_ms,
            tools: tools.into_iter().map(Into::into).collect(),
            system,
            turns: turns.into_iter().map(Into::into).collect(),
        }
    }
}

/// The model a run called, and how it was reached.
///
/// The provider is written beside the API that decides it, because a reader
/// outside lablet has no table of which API is whose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Model {
    provider: ProviderKind,
    api: ProviderApi,
    name: String,
    replays_reasoning: bool,
}

impl From<ModelRef> for Model {
    fn from(model: ModelRef) -> Self {
        let ModelRef {
            api,
            name,
            replays_reasoning,
        } = model;
        Self {
            provider: api.provider().into(),
            api: api.into(),
            name,
            replays_reasoning,
        }
    }
}

/// The API an adapter reached its provider through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProviderApi {
    Messages,
    Responses,
    ChatCompletions,
    Script,
}

impl From<model::ProviderApi> for ProviderApi {
    fn from(api: model::ProviderApi) -> Self {
        match api {
            model::ProviderApi::Messages => Self::Messages,
            model::ProviderApi::Responses => Self::Responses,
            model::ProviderApi::ChatCompletions => Self::ChatCompletions,
            model::ProviderApi::Script => Self::Script,
        }
    }
}

/// One model response, with what prompted it and what came of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Turn {
    input: Vec<UserContent>,
    response: Vec<ContentBlock>,
    record: TurnRecord,
    tool_calls: Vec<ToolCallOutcome>,
}

impl From<model::Turn> for Turn {
    fn from(turn: model::Turn) -> Self {
        let TurnParts {
            input,
            response,
            record,
            tool_calls,
        } = turn.into_parts();
        Self {
            input: input.into_iter().map(Into::into).collect(),
            response: response.into_iter().map(Into::into).collect(),
            record: record.into(),
            tool_calls: tool_calls.into_iter().map(Into::into).collect(),
        }
    }
}

/// What the run knew about the provider call behind a turn. The finish
/// reason is lablet's name for a reason it knows, and the provider's own
/// string for any other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TurnRecord {
    usage: Usage,
    finish: String,
    response_id: Option<String>,
    response_model: Option<String>,
    started_ms: u64,
    latency_ms: u64,
    attempts: u32,
}

impl From<model::TurnRecord> for TurnRecord {
    fn from(record: model::TurnRecord) -> Self {
        let model::TurnRecord {
            usage,
            finish,
            response_id,
            response_model,
            started_ms,
            latency_ms,
            attempts,
        } = record;
        Self {
            usage: usage.into(),
            finish: finish.as_str().to_owned(),
            response_id,
            response_model,
            started_ms,
            latency_ms,
            attempts,
        }
    }
}

#[cfg(test)]
mod tests;
