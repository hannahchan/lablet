use serde_json::{Value, json};

use crate::TaskResult;
use crate::outcome::RawOutcome;
use crate::{CompletionMode, RunId, RunOutcome, StopClass, StopReason, TokenCounts, Usage};

// The literal spellings below are the members of `lablet.run.stop_reason` and
// `lablet.run.completion_mode` in the generated telemetry-registry crate,
// which the domain can't depend on. A spelling that changes on either side
// must change on both.
const STOP_REASONS: [(StopReason, &str); 12] = [
    (StopReason::Completed, "completed"),
    (
        StopReason::EndedWithoutCompletion,
        "ended_without_completion",
    ),
    (StopReason::MaxTurns, "max_turns"),
    (StopReason::Timeout, "timeout"),
    (StopReason::MaxTotalTokens, "max_total_tokens"),
    (StopReason::OutputTruncated, "output_truncated"),
    (StopReason::ContextExhausted, "context_exhausted"),
    (StopReason::RetriesExhausted, "retries_exhausted"),
    (StopReason::ToolErrorsExhausted, "tool_errors_exhausted"),
    (StopReason::Cancelled, "cancelled"),
    (StopReason::ProviderError, "provider_error"),
    (StopReason::Refused, "refused"),
];

const COMPLETION_MODES: [(CompletionMode, &str); 2] = [
    (CompletionMode::Natural, "natural"),
    (CompletionMode::Explicit, "explicit"),
];

#[test]
fn every_stop_reason_prints_and_serialises_as_its_telemetry_spelling() {
    for (reason, spelling) in STOP_REASONS {
        assert_eq!(reason.to_string(), spelling);
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<StopReason>(json!(spelling)).unwrap(),
            reason
        );
    }
}

#[test]
fn every_completion_mode_prints_and_serialises_as_its_telemetry_spelling() {
    for (mode, spelling) in COMPLETION_MODES {
        assert_eq!(mode.to_string(), spelling);
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<CompletionMode>(json!(spelling)).unwrap(),
            mode
        );
    }
}

#[test]
fn natural_is_the_default_completion_mode() {
    assert_eq!(CompletionMode::default(), CompletionMode::Natural);
}

// Each reason with its class. Paired rather than a list in the order of
// `STOP_REASONS`, so reordering either can't silently relabel them all.
const CLASSES: [(StopReason, StopClass); 12] = [
    (StopReason::Completed, StopClass::Completed),
    (StopReason::EndedWithoutCompletion, StopClass::Stopped),
    (StopReason::MaxTurns, StopClass::Stopped),
    (StopReason::Timeout, StopClass::Stopped),
    (StopReason::MaxTotalTokens, StopClass::Stopped),
    (StopReason::OutputTruncated, StopClass::Stopped),
    (StopReason::ContextExhausted, StopClass::Failed),
    (StopReason::RetriesExhausted, StopClass::Failed),
    (StopReason::ToolErrorsExhausted, StopClass::Failed),
    (StopReason::Cancelled, StopClass::Stopped),
    (StopReason::ProviderError, StopClass::Failed),
    (StopReason::Refused, StopClass::Stopped),
];

#[test]
fn every_stop_reason_has_its_class() {
    for (reason, class) in CLASSES {
        assert_eq!(reason.class(), class, "{reason}");
    }
}

#[test]
fn closing_keeps_a_structured_result_only_for_a_completed_run() {
    let argument = json!({ "passed": true });
    for (reason, _) in STOP_REASONS {
        let outcome = RunOutcome::closing(raw(reason, Some(argument.clone()), None));

        let expected = (reason == StopReason::Completed).then(|| argument.clone());
        assert_eq!(outcome.result().structured, expected, "{reason}");
        assert_eq!(outcome.result().text, "partial");
    }
}

#[test]
fn closing_keeps_an_error_only_for_a_failed_run() {
    for (reason, class) in CLASSES {
        let outcome = RunOutcome::closing(raw(reason, None, Some("boom")));

        let expected = (class == StopClass::Failed).then_some("boom");
        assert_eq!(outcome.error(), expected, "{reason}");
    }
}

fn raw(stop_reason: StopReason, structured: Option<Value>, error: Option<&str>) -> RawOutcome {
    RawOutcome {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        stop_reason,
        turns: 1,
        usage: Usage::from_inclusive(TokenCounts {
            input: 12,
            output: 3,
            reasoning: 0,
            cache_read: 8,
            cache_write: 0,
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
