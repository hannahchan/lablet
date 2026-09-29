use serde_json::{Value, json};

use super::*;
use crate::{RunId, RunLabels, StopReason, TokenCounts, Usage};

fn labels() -> RunLabels {
    RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: None,
        trial: Some("3".to_owned()),
    }
}

fn parts(stop_reason: StopReason, structured: Option<Value>, error: Option<&str>) -> OutcomeParts {
    OutcomeParts {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        labels: labels(),
        stop_reason,
        turns: 1,
        usage: Usage::from_inclusive(TokenCounts {
            input: 12,
            output: 3,
            reasoning: None,
            cache_read: Some(8),
            cache_write: Some(0),
        }),
        tool_calls: 2,
        duration_ms: 250,
        result: TaskResult {
            text: "partial".to_owned(),
            structured,
        },
        error: error.map(str::to_owned),
    }
}

fn failed() -> OutcomeParts {
    parts(
        StopReason::ProviderError,
        None,
        Some("provider: 401 unauthorized"),
    )
}

fn outcome() -> RunOutcome {
    RunOutcome::closing(failed())
}

#[test]
fn an_outcome_is_read_through_its_getters() {
    let outcome = outcome();

    assert_eq!(outcome.run_id.as_str(), "01K5F3Z8Q4X9T2M7B6W1R0VNEC");
    assert_eq!(outcome.labels, labels());
    assert_eq!(outcome.stop_reason(), StopReason::ProviderError);
    assert_eq!(outcome.turns, 1);
    assert_eq!(
        outcome.usage,
        Usage::from_inclusive(TokenCounts {
            input: 12,
            output: 3,
            reasoning: None,
            cache_read: Some(8),
            cache_write: Some(0)
        })
    );
    assert_eq!(outcome.tool_calls, 2);
    assert_eq!(outcome.duration_ms, 250);
    assert_eq!(outcome.result().text, "partial");
    assert_eq!(outcome.result().structured, None);
    assert_eq!(outcome.error(), Some("provider: 401 unauthorized"));
}

/// Every field, so a field that `into_parts` dropped or took from the wrong
/// place fails here, before anything that writes an outcome down reads it.
#[test]
fn an_outcome_taken_apart_is_every_part_it_was_made_from() {
    assert_eq!(outcome().into_parts(), failed());

    let completed = parts(StopReason::Completed, Some(json!({ "passed": true })), None);
    assert_eq!(
        RunOutcome::closing(completed.clone()).into_parts(),
        completed
    );
}

#[test]
fn a_failure_that_came_without_an_error_says_what_failed() {
    for (reason, message) in [
        (
            StopReason::ContextExhausted,
            "the response was cut short at the model's context window",
        ),
        (
            StopReason::InvalidCallsExhausted,
            "the turns in a row in which no call reached a tool reached their cap",
        ),
        (StopReason::RetriesExhausted, "a provider call failed"),
        (StopReason::ProviderError, "a provider call failed"),
    ] {
        let outcome = RunOutcome::closing(parts(reason, None, None));

        assert_eq!(outcome.error(), Some(message), "{reason}");
    }
}

#[test]
fn parts_a_run_can_have_are_the_outcome_that_closing_makes_of_them() {
    for stated in [
        failed(),
        parts(StopReason::Completed, Some(json!({ "passed": true })), None),
        parts(StopReason::Completed, None, None),
        parts(StopReason::MaxTurns, None, None),
    ] {
        let outcome = RunOutcome::try_from(stated.clone()).unwrap();

        assert_eq!(outcome, RunOutcome::closing(stated.clone()));
        assert_eq!(outcome.into_parts(), stated, "nothing was mended");
    }
}

#[test]
fn parts_no_run_can_have_are_refused_with_the_rule_they_break() {
    let argument = || Some(json!({ "passed": true }));
    for (stated, error, message) in [
        (
            parts(StopReason::Completed, None, Some("boom")),
            OutcomeError::ErrorWithoutFailure {
                stop_reason: StopReason::Completed,
            },
            "a run that stopped with completed didn't fail, so it has no error",
        ),
        (
            parts(StopReason::Timeout, None, Some("boom")),
            OutcomeError::ErrorWithoutFailure {
                stop_reason: StopReason::Timeout,
            },
            "a run that stopped with timeout didn't fail, so it has no error",
        ),
        (
            parts(StopReason::ProviderError, None, None),
            OutcomeError::FailureWithoutError {
                stop_reason: StopReason::ProviderError,
            },
            "a run that stopped with provider_error failed, so it has an error",
        ),
        (
            parts(StopReason::MaxTurns, argument(), None),
            OutcomeError::StructuredWithoutCompletion {
                stop_reason: StopReason::MaxTurns,
            },
            "a run that stopped with max_turns didn't complete, so it has no structured result",
        ),
        (
            parts(StopReason::RetriesExhausted, argument(), Some("boom")),
            OutcomeError::StructuredWithoutCompletion {
                stop_reason: StopReason::RetriesExhausted,
            },
            "a run that stopped with retries_exhausted didn't complete, so it has no structured result",
        ),
    ] {
        let refused = RunOutcome::try_from(stated).unwrap_err();

        assert_eq!(refused, error);
        assert_eq!(refused.to_string(), message);
    }
}
