use serde_json::{Value, json};

use super::*;
use crate::{RunId, StopReason, TokenCounts, Usage};

#[test]
fn an_unknown_stop_reason_does_not_deserialise() {
    assert!(serde_json::from_value::<StopReason>(json!("gave_up")).is_err());
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

fn outcome() -> RunOutcome {
    RunOutcome::closing(raw(
        StopReason::ProviderError,
        None,
        Some("provider: 401 unauthorized"),
    ))
}

#[test]
fn an_outcome_is_read_through_its_getters() {
    let outcome = outcome();

    assert_eq!(outcome.run_id.as_str(), "01K5F3Z8Q4X9T2M7B6W1R0VNEC");
    assert_eq!(outcome.stop_reason(), StopReason::ProviderError);
    assert_eq!(outcome.turns, 1);
    assert_eq!(
        outcome.usage,
        Usage::from_inclusive(TokenCounts {
            input: 12,
            output: 3,
            reasoning: 0,
            cache_read: 8,
            cache_write: 0
        })
    );
    assert_eq!(outcome.tool_calls, 2);
    assert_eq!(outcome.duration_ms, 250);
    assert_eq!(outcome.result().text, "partial");
    assert_eq!(outcome.result().structured, None);
    assert_eq!(outcome.error(), Some("provider: 401 unauthorized"));
}

#[test]
fn a_failed_natural_run_writes_these_exact_bytes() {
    assert_eq!(
        serde_json::to_string(&outcome()).unwrap(),
        concat!(
            r#"{"run_id":"01K5F3Z8Q4X9T2M7B6W1R0VNEC","stop_reason":"provider_error","turns":1,"#,
            r#""usage":{"input_tokens":12,"output_tokens":3,"reasoning_output_tokens":0,"cache_read_tokens":8,"cache_write_tokens":0},"#,
            r#""tool_calls":2,"duration_ms":250,"result":{"text":"partial","structured":null},"#,
            r#""error":"provider: 401 unauthorized"}"#
        )
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
            StopReason::ToolErrorsExhausted,
            "consecutive tool error results reached their cap",
        ),
        (StopReason::RetriesExhausted, "a provider call failed"),
        (StopReason::ProviderError, "a provider call failed"),
    ] {
        let outcome = RunOutcome::closing(raw(reason, None, None));

        assert_eq!(outcome.error(), Some(message), "{reason}");
    }
}

fn document(stop_reason: &str, structured: &Value, error: &Value) -> Value {
    json!({
        "run_id": "01K5F3Z8Q4X9T2M7B6W1R0VNEC",
        "stop_reason": stop_reason,
        "turns": 1,
        "usage": { "input_tokens": 12, "output_tokens": 3, "cache_read_tokens": 8 },
        "tool_calls": 2,
        "duration_ms": 250,
        "result": { "text": "partial", "structured": structured },
        "error": error,
    })
}

/// The outcome document is the contract a composer parses, so a field it
/// doesn't recognise is a misspelling to report, not one to pass over.
#[test]
fn an_outcome_document_with_a_field_the_model_does_not_know_is_refused() {
    let mut document = document("completed", &Value::Null, &Value::Null);
    document["stop_resaon"] = json!("completed");

    let refused = serde_json::from_value::<RunOutcome>(document).unwrap_err();
    assert!(
        refused.to_string().contains("unknown field `stop_resaon`"),
        "{refused}"
    );
}

#[test]
fn an_outcome_a_run_can_have_reads_back_as_itself() {
    let failed = document(
        "provider_error",
        &Value::Null,
        &json!("provider: 401 unauthorized"),
    );
    let completed = document("completed", &json!({ "passed": true }), &Value::Null);
    let stopped = document("max_turns", &Value::Null, &Value::Null);

    assert_eq!(
        serde_json::from_value::<RunOutcome>(failed).unwrap(),
        outcome()
    );
    for document in [completed, stopped] {
        let outcome = serde_json::from_value::<RunOutcome>(document.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(&outcome).unwrap()["result"],
            document["result"]
        );
        assert_eq!(outcome.error(), None);
    }
}

#[test]
fn an_outcome_no_run_can_have_does_not_deserialise() {
    let (argument, boom, null) = (json!({ "passed": true }), json!("boom"), Value::Null);
    for (document, message) in [
        (
            document("completed", &null, &boom),
            "a run that stopped with completed didn't fail, so it has no error",
        ),
        (
            document("timeout", &null, &boom),
            "a run that stopped with timeout didn't fail, so it has no error",
        ),
        (
            document("provider_error", &null, &null),
            "a run that stopped with provider_error failed, so it has an error",
        ),
        (
            document("max_turns", &argument, &null),
            "a run that stopped with max_turns didn't complete, so it has no structured result",
        ),
        (
            document("retries_exhausted", &argument, &boom),
            "a run that stopped with retries_exhausted didn't complete, so it has no structured result",
        ),
    ] {
        let error = serde_json::from_value::<RunOutcome>(document).unwrap_err();

        assert_eq!(error.to_string(), message);
    }
}
