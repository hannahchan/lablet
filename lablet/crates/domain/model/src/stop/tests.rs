use serde_json::{Value, json};

use crate::{
    CompletionMode, OutcomeParts, RunId, RunLabels, RunOutcome, StopClass, StopReason, TaskResult,
    TokenCounts, ToolName, Usage,
};

// The literal spellings below are the members of `lablet.run.stop_reason` and
// `lablet.run.completion_mode` in lablet-run's generated telemetry module,
// which the domain can't depend on. A spelling that changes on either side
// must change on both.
const fn stop_reason_spelling(reason: StopReason) -> &'static str {
    match reason {
        StopReason::Completed => "completed",
        StopReason::EndedWithoutCompletion => "ended_without_completion",
        StopReason::MaxTurns => "max_turns",
        StopReason::Timeout => "timeout",
        StopReason::MaxTotalTokens => "max_total_tokens",
        StopReason::OutputTruncated => "output_truncated",
        StopReason::ContextExhausted => "context_exhausted",
        StopReason::RetriesExhausted => "retries_exhausted",
        StopReason::InvalidCallsExhausted => "invalid_calls_exhausted",
        StopReason::Cancelled => "cancelled",
        StopReason::ProviderError => "provider_error",
        StopReason::Refused => "refused",
    }
}

const fn completion_mode_spelling(mode: CompletionMode) -> &'static str {
    match mode {
        CompletionMode::Natural => "natural",
        CompletionMode::Explicit => "explicit",
    }
}

#[test]
fn every_stop_reason_is_spelled_and_prints_as_its_telemetry_spelling() {
    for reason in StopReason::ALL {
        let spelling = stop_reason_spelling(reason);
        assert_eq!(reason.as_str(), spelling);
        assert_eq!(reason.to_string(), spelling);
    }
}

#[test]
fn every_completion_mode_is_spelled_and_prints_as_its_telemetry_spelling() {
    for mode in CompletionMode::ALL {
        let spelling = completion_mode_spelling(mode);
        assert_eq!(mode.as_str(), spelling);
        assert_eq!(mode.to_string(), spelling);
    }
}

#[test]
fn natural_is_the_default_completion_mode() {
    assert_eq!(CompletionMode::default(), CompletionMode::Natural);
}

#[test]
fn only_explicit_mode_intercepts_a_call_and_only_one_to_task_complete() {
    let task_complete = ToolName::task_complete();
    let bash = ToolName::new("bash").unwrap();

    assert!(CompletionMode::Explicit.intercepts(&task_complete));
    assert!(!CompletionMode::Explicit.intercepts(&bash));
    assert!(
        !CompletionMode::Natural.intercepts(&task_complete),
        "a tool of that name in natural mode is a tool like any other"
    );
    assert!(!CompletionMode::Natural.intercepts(&bash));
}

// Each reason with its class, written apart from `StopReason::class` so a
// reason moved to another class there differs from this.
const fn class_of(reason: StopReason) -> StopClass {
    match reason {
        StopReason::Completed => StopClass::Completed,
        StopReason::EndedWithoutCompletion
        | StopReason::MaxTurns
        | StopReason::Timeout
        | StopReason::MaxTotalTokens
        | StopReason::OutputTruncated
        | StopReason::Cancelled
        | StopReason::Refused => StopClass::Stopped,
        StopReason::ContextExhausted
        | StopReason::RetriesExhausted
        | StopReason::InvalidCallsExhausted
        | StopReason::ProviderError => StopClass::Failed,
    }
}

#[test]
fn every_stop_reason_has_its_class() {
    for reason in StopReason::ALL {
        assert_eq!(reason.class(), class_of(reason), "{reason}");
    }
}

#[test]
fn closing_keeps_a_structured_result_only_for_a_completed_run() {
    let argument = json!({ "passed": true });
    for reason in StopReason::ALL {
        let outcome = RunOutcome::closing(parts(reason, Some(argument.clone()), None));

        let expected = (reason == StopReason::Completed).then(|| argument.clone());
        assert_eq!(outcome.result().structured, expected, "{reason}");
        assert_eq!(outcome.result().text, "partial");
    }
}

#[test]
fn closing_keeps_an_error_only_for_a_failed_run() {
    for reason in StopReason::ALL {
        let outcome = RunOutcome::closing(parts(reason, None, Some("boom")));

        let expected = (class_of(reason) == StopClass::Failed).then_some("boom");
        assert_eq!(outcome.error(), expected, "{reason}");
    }
}

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
