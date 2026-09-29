//! The outcome document: why a run stopped and what it produced, under a
//! version.

use lablet_model::{self as model, OutcomeError, OutcomeParts, RunId, RunOutcome};
use serde::{Deserialize, Serialize};

use crate::labels::Labels;
use crate::usage::Usage;

/// The version of the outcome document, raised when a reader that knows the
/// old form could misread the new one.
///
/// Dropping a field, renaming one, or changing what a field means all need a
/// raise. Adding one doesn't: a reader that doesn't know a key ignores it.
pub const OUTCOME_SCHEMA_VERSION: u32 = 1;

/// The outcome of one run, as the document lablet writes. It's a public
/// contract, held by `lablet/tests/fixtures/outcome.json`.
///
/// One is made from a run's outcome, at this crate's version, or read.
/// Reading refuses a key the document doesn't know and a run id that isn't
/// one. What it read is reached only through `RunOutcome::try_from`, which
/// refuses a version this crate doesn't know and an outcome no run could
/// have had, so nothing takes a value from a document that wasn't checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeDocument {
    schema_version: u32,
    #[serde(with = "crate::id::run_id")]
    run_id: RunId,
    labels: Labels,
    stop_reason: StopReason,
    turns: u32,
    usage: Usage,
    tool_calls: u64,
    duration_ms: u64,
    result: TaskResult,
    error: Option<String>,
}

impl From<RunOutcome> for OutcomeDocument {
    fn from(outcome: RunOutcome) -> Self {
        let OutcomeParts {
            run_id,
            labels,
            stop_reason,
            turns,
            usage,
            tool_calls,
            duration_ms,
            result,
            error,
        } = outcome.into_parts();
        Self {
            schema_version: OUTCOME_SCHEMA_VERSION,
            run_id,
            labels: labels.into(),
            stop_reason: stop_reason.into(),
            turns,
            usage: usage.into(),
            tool_calls,
            duration_ms,
            result: result.into(),
            error,
        }
    }
}

/// Why an outcome document that parsed isn't one to take an outcome from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutcomeDocumentError {
    /// The document is in a form this crate doesn't know, so what its keys
    /// mean isn't known either.
    #[error(
        "the outcome document is in version {found} of its form, and this reads version \
         {OUTCOME_SCHEMA_VERSION}"
    )]
    UnknownVersion {
        /// The version the document states.
        found: u32,
    },
    /// The document states an outcome no run could have had.
    #[error("{0}")]
    BrokenRule(#[from] OutcomeError),
}

impl TryFrom<OutcomeDocument> for RunOutcome {
    type Error = OutcomeDocumentError;

    fn try_from(document: OutcomeDocument) -> Result<Self, OutcomeDocumentError> {
        let OutcomeDocument {
            schema_version,
            run_id,
            labels,
            stop_reason,
            turns,
            usage,
            tool_calls,
            duration_ms,
            result,
            error,
        } = document;
        if schema_version != OUTCOME_SCHEMA_VERSION {
            return Err(OutcomeDocumentError::UnknownVersion {
                found: schema_version,
            });
        }
        Ok(Self::try_from(OutcomeParts {
            run_id,
            labels: labels.into(),
            stop_reason: stop_reason.into(),
            turns,
            usage: usage.into(),
            tool_calls,
            duration_ms,
            result: result.into(),
            error,
        })?)
    }
}

/// Why a run ended, spelled as the `lablet.run.stop_reason` attribute
/// spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StopReason {
    Completed,
    EndedWithoutCompletion,
    MaxTurns,
    Timeout,
    MaxTotalTokens,
    OutputTruncated,
    ContextExhausted,
    RetriesExhausted,
    InvalidCallsExhausted,
    Cancelled,
    ProviderError,
    Refused,
}

impl From<model::StopReason> for StopReason {
    fn from(reason: model::StopReason) -> Self {
        match reason {
            model::StopReason::Completed => Self::Completed,
            model::StopReason::EndedWithoutCompletion => Self::EndedWithoutCompletion,
            model::StopReason::MaxTurns => Self::MaxTurns,
            model::StopReason::Timeout => Self::Timeout,
            model::StopReason::MaxTotalTokens => Self::MaxTotalTokens,
            model::StopReason::OutputTruncated => Self::OutputTruncated,
            model::StopReason::ContextExhausted => Self::ContextExhausted,
            model::StopReason::RetriesExhausted => Self::RetriesExhausted,
            model::StopReason::InvalidCallsExhausted => Self::InvalidCallsExhausted,
            model::StopReason::Cancelled => Self::Cancelled,
            model::StopReason::ProviderError => Self::ProviderError,
            model::StopReason::Refused => Self::Refused,
        }
    }
}

impl From<StopReason> for model::StopReason {
    fn from(reason: StopReason) -> Self {
        match reason {
            StopReason::Completed => Self::Completed,
            StopReason::EndedWithoutCompletion => Self::EndedWithoutCompletion,
            StopReason::MaxTurns => Self::MaxTurns,
            StopReason::Timeout => Self::Timeout,
            StopReason::MaxTotalTokens => Self::MaxTotalTokens,
            StopReason::OutputTruncated => Self::OutputTruncated,
            StopReason::ContextExhausted => Self::ContextExhausted,
            StopReason::RetriesExhausted => Self::RetriesExhausted,
            StopReason::InvalidCallsExhausted => Self::InvalidCallsExhausted,
            StopReason::Cancelled => Self::Cancelled,
            StopReason::ProviderError => Self::ProviderError,
            StopReason::Refused => Self::Refused,
        }
    }
}

/// What a run produced. Both keys are always written, `structured` as
/// `null` for a run that has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskResult {
    text: String,
    structured: Option<serde_json::Value>,
}

impl From<model::TaskResult> for TaskResult {
    fn from(result: model::TaskResult) -> Self {
        let model::TaskResult { text, structured } = result;
        Self { text, structured }
    }
}

impl From<TaskResult> for model::TaskResult {
    fn from(result: TaskResult) -> Self {
        let TaskResult { text, structured } = result;
        Self { text, structured }
    }
}

#[cfg(test)]
mod tests;
