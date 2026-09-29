//! The outcome of a run: why it stopped and what it produced.

use crate::{RunId, RunLabels, StopClass, StopReason, Usage};

/// The outcome of a run, which the outcome document is written from.
///
/// What an outcome carries follows from the [`StopClass`] of its stop reason:
/// `error` is a message exactly when the run failed, and `result.structured`
/// is a value only when it completed. Those three fields are private for that
/// reason, and an outcome is made in two ways that both hold the rule.
/// [`RunOutcome::closing`] keeps of what it's given only what the class
/// allows, which is how [`crate::Run::finish`] closes a run. `TryFrom`
/// refuses parts that break the rule, which is how an outcome that was
/// written down is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    /// The run's id.
    pub run_id: RunId,
    /// What the run request named the run's task, experiment and trial.
    pub labels: RunLabels,
    stop_reason: StopReason,
    /// How many turns the transcript has, which is how many model responses
    /// the run received. A run whose first provider call failed took none.
    pub turns: u32,
    /// Usage summed over every successful provider call. `input_tokens`
    /// includes the cached tokens, and a count no call reported is `None`;
    /// see [`Usage`].
    pub usage: Usage,
    /// How many tool calls were executed. The intercepted `task_complete`
    /// call isn't one, and neither is a call that was never run.
    pub tool_calls: u64,
    /// Wall-clock duration of the run, in whole milliseconds.
    pub duration_ms: u64,
    result: TaskResult,
    error: Option<String>,
}

/// The parts of an outcome, held to no rule: what an outcome is made from,
/// and what [`RunOutcome::into_parts`] takes one apart into.
///
/// Every field of an outcome is here and every field is public, so whatever
/// takes the parts apart by pattern stops compiling when an outcome gains a
/// field, until it says what becomes of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeParts {
    /// The run's id.
    pub run_id: RunId,
    /// What the run request named the run's task, experiment and trial.
    pub labels: RunLabels,
    /// Why the run ended.
    pub stop_reason: StopReason,
    /// How many model responses the run received.
    pub turns: u32,
    /// Usage summed over every successful provider call.
    pub usage: Usage,
    /// How many tool calls were executed.
    pub tool_calls: u64,
    /// Wall-clock duration of the run, in whole milliseconds.
    pub duration_ms: u64,
    /// What the run produced.
    pub result: TaskResult,
    /// The message of the error that ended the run.
    pub error: Option<String>,
}

/// Why the parts of an outcome are ones no run could have produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutcomeError {
    /// An error came with a stop reason that isn't a failure.
    #[error("a run that stopped with {stop_reason} didn't fail, so it has no error")]
    ErrorWithoutFailure {
        /// The stop reason of the parts.
        stop_reason: StopReason,
    },
    /// A failure came without its error.
    #[error("a run that stopped with {stop_reason} failed, so it has an error")]
    FailureWithoutError {
        /// The stop reason of the parts.
        stop_reason: StopReason,
    },
    /// A structured result came with a run that didn't complete.
    #[error(
        "a run that stopped with {stop_reason} didn't complete, so it has no structured result"
    )]
    StructuredWithoutCompletion {
        /// The stop reason of the parts.
        stop_reason: StopReason,
    },
}

impl TryFrom<OutcomeParts> for RunOutcome {
    type Error = OutcomeError;

    /// The outcome the parts state, for parts that came from outside the
    /// loop, such as a document someone else wrote.
    ///
    /// Parts that break a rule are refused rather than mended, because
    /// whoever wrote them down meant something by them and a mended outcome
    /// would say something else.
    fn try_from(parts: OutcomeParts) -> Result<Self, OutcomeError> {
        let stop_reason = parts.stop_reason;
        let class = stop_reason.class();
        if parts.result.structured.is_some() && class != StopClass::Completed {
            return Err(OutcomeError::StructuredWithoutCompletion { stop_reason });
        }
        match (class, &parts.error) {
            (StopClass::Failed, None) => Err(OutcomeError::FailureWithoutError { stop_reason }),
            (StopClass::Completed | StopClass::Stopped, Some(_)) => {
                Err(OutcomeError::ErrorWithoutFailure { stop_reason })
            }
            (StopClass::Failed, Some(_)) | (StopClass::Completed | StopClass::Stopped, None) => {
                Ok(Self::closing(parts))
            }
        }
    }
}

impl RunOutcome {
    /// The outcome of a run, holding it to the [`StopClass`] rules: it keeps
    /// a structured result only for a run that completed and an error only
    /// for one that failed, and gives a failure that came without an error
    /// the reason's own message.
    ///
    /// So the loop passes what it has at hand, and the outcome is still one
    /// a run can have.
    #[must_use]
    pub fn closing(parts: OutcomeParts) -> Self {
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
        } = parts;
        let class = stop_reason.class();
        Self {
            run_id,
            labels,
            stop_reason,
            turns,
            usage,
            tool_calls,
            duration_ms,
            result: TaskResult {
                text: result.text,
                structured: result.structured.filter(|_| class == StopClass::Completed),
            },
            error: (class == StopClass::Failed)
                .then(|| error.unwrap_or_else(|| stop_reason.failure_message().to_owned())),
        }
    }

    /// The outcome taken apart, every field of it, for whatever writes one
    /// down. Parts that came from an outcome make the same outcome again,
    /// whichever way they're put back together.
    #[must_use]
    pub fn into_parts(self) -> OutcomeParts {
        let Self {
            run_id,
            labels,
            stop_reason,
            turns,
            usage,
            tool_calls,
            duration_ms,
            result,
            error,
        } = self;
        OutcomeParts {
            run_id,
            labels,
            stop_reason,
            turns,
            usage,
            tool_calls,
            duration_ms,
            result,
            error,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskResult {
    /// The text of the last turn; empty when there is none.
    pub text: String,
    /// The `task_complete` argument when the run completed in explicit mode,
    /// and `None` otherwise.
    pub structured: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests;
