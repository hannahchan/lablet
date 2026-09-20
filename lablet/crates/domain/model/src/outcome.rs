//! The outcome document of a run: why it stopped and what it produced.

use serde::{Deserialize, Serialize};

use crate::{RunId, StopClass, StopReason, Usage};

/// The outcome document of a run. Its serde form is a public contract, held
/// by `lablet/tests/fixtures/outcome.json`.
///
/// What an outcome carries follows from the [`StopClass`] of its stop reason:
/// `error` is a message exactly when the run failed, and `result.structured`
/// is a value only when it completed. Those three fields are private for that
/// reason: [`crate::Run::finish`] builds outcomes that way, and reading one
/// from its serde form refuses any other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawOutcome")]
pub struct RunOutcome {
    /// The run's id.
    pub run_id: RunId,
    stop_reason: StopReason,
    /// How many turns the transcript has, which is how many model responses
    /// the run received. A run whose first provider call failed took none.
    pub turns: u32,
    /// Usage summed over every successful provider call. `input_tokens`
    /// includes the cached tokens; see [`Usage`].
    pub usage: Usage,
    /// How many tool calls were executed. The intercepted `task_complete` call isn't one.
    pub tool_calls: u64,
    /// Wall-clock duration of the run, in whole milliseconds.
    pub duration_ms: u64,
    result: TaskResult,
    error: Option<String>,
}

/// What an outcome is read from, so that reading one holds it to the rules,
/// and what [`crate::Run::finish`] fills in to close a run.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawOutcome {
    pub(crate) run_id: RunId,
    pub(crate) stop_reason: StopReason,
    pub(crate) turns: u32,
    pub(crate) usage: Usage,
    pub(crate) tool_calls: u64,
    pub(crate) duration_ms: u64,
    pub(crate) result: TaskResult,
    pub(crate) error: Option<String>,
}

/// Why an outcome document is one no run could have produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutcomeError {
    /// An error came with a stop reason that isn't a failure.
    #[error("a run that stopped with {stop_reason} didn't fail, so it has no error")]
    ErrorWithoutFailure {
        /// The stop reason of the document.
        stop_reason: StopReason,
    },
    /// A failure came without its error.
    #[error("a run that stopped with {stop_reason} failed, so it has an error")]
    FailureWithoutError {
        /// The stop reason of the document.
        stop_reason: StopReason,
    },
    /// A structured result came with a run that didn't complete.
    #[error(
        "a run that stopped with {stop_reason} didn't complete, so it has no structured result"
    )]
    StructuredWithoutCompletion {
        /// The stop reason of the document.
        stop_reason: StopReason,
    },
}

impl TryFrom<RawOutcome> for RunOutcome {
    type Error = OutcomeError;

    fn try_from(raw: RawOutcome) -> Result<Self, OutcomeError> {
        let stop_reason = raw.stop_reason;
        let class = stop_reason.class();
        if raw.result.structured.is_some() && class != StopClass::Completed {
            return Err(OutcomeError::StructuredWithoutCompletion { stop_reason });
        }
        match (class, &raw.error) {
            (StopClass::Failed, None) => Err(OutcomeError::FailureWithoutError { stop_reason }),
            (StopClass::Completed | StopClass::Stopped, Some(_)) => {
                Err(OutcomeError::ErrorWithoutFailure { stop_reason })
            }
            (StopClass::Failed, Some(_)) | (StopClass::Completed | StopClass::Stopped, None) => {
                Ok(Self::closing(raw))
            }
        }
    }
}

impl RunOutcome {
    /// The outcome of a run, holding it to the [`StopClass`] rules and giving a
    /// failure that came without an error the reason's own message.
    pub(crate) fn closing(raw: RawOutcome) -> Self {
        let class = raw.stop_reason.class();
        Self {
            run_id: raw.run_id,
            stop_reason: raw.stop_reason,
            turns: raw.turns,
            usage: raw.usage,
            tool_calls: raw.tool_calls,
            duration_ms: raw.duration_ms,
            result: TaskResult {
                text: raw.result.text,
                structured: raw
                    .result
                    .structured
                    .filter(|_| class == StopClass::Completed),
            },
            error: (class == StopClass::Failed).then(|| {
                raw.error
                    .unwrap_or_else(|| raw.stop_reason.failure_message().to_owned())
            }),
        }
    }

    /// Why the run ended.
    #[must_use]
    pub const fn stop_reason(&self) -> StopReason {
        self.stop_reason
    }

    /// What the run produced. `structured` is a value only when the run completed.
    #[must_use]
    pub const fn result(&self) -> &TaskResult {
        &self.result
    }

    /// The message of the error that ended the run: a message when the run
    /// failed, `None` otherwise.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskResult {
    /// The text of the last turn; empty when there is none.
    pub text: String,
    /// The `task_complete` argument when the run completed in explicit mode.
    /// Written as `null` otherwise, never left out.
    pub structured: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests;
