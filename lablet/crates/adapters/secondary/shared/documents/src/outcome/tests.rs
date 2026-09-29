use lablet_model::{self as model, RunLabels, TokenCounts};
use serde_json::{Value, json};

use super::*;

fn parts(
    stop_reason: model::StopReason,
    structured: Option<Value>,
    error: Option<&str>,
) -> OutcomeParts {
    OutcomeParts {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        labels: RunLabels {
            task: Some("fix-failing-test".to_owned()),
            experiment: None,
            trial: Some("3".to_owned()),
        },
        stop_reason,
        turns: 1,
        usage: model::Usage::from_inclusive(TokenCounts {
            input: 12,
            output: 3,
            reasoning: None,
            cache_read: Some(8),
            cache_write: Some(0),
        }),
        tool_calls: 2,
        duration_ms: 250,
        result: model::TaskResult {
            text: "partial".to_owned(),
            structured,
        },
        error: error.map(str::to_owned),
    }
}

fn failed() -> RunOutcome {
    RunOutcome::closing(parts(
        model::StopReason::ProviderError,
        None,
        Some("provider: 401 unauthorized"),
    ))
}

/// The document of [`failed`] with another stop reason, result and error,
/// as someone else might have written it.
fn written(stop_reason: &str, structured: &Value, error: &Value) -> Value {
    json!({
        "schema_version": 1,
        "run_id": "01K5F3Z8Q4X9T2M7B6W1R0VNEC",
        "labels": { "task": "fix-failing-test", "experiment": null, "trial": "3" },
        "stop_reason": stop_reason,
        "turns": 1,
        "usage": {
            "input_tokens": 12,
            "output_tokens": 3,
            "reasoning_output_tokens": null,
            "cache_read_tokens": 8,
            "cache_write_tokens": 0,
        },
        "tool_calls": 2,
        "duration_ms": 250,
        "result": { "text": "partial", "structured": structured },
        "error": error,
    })
}

fn read(document: Value) -> Result<RunOutcome, OutcomeDocumentError> {
    RunOutcome::try_from(serde_json::from_value::<OutcomeDocument>(document).unwrap())
}

#[test]
fn a_failed_run_is_written_as_these_exact_bytes_with_the_version_first() {
    assert_eq!(
        serde_json::to_string(&OutcomeDocument::from(failed())).unwrap(),
        concat!(
            r#"{"schema_version":1,"run_id":"01K5F3Z8Q4X9T2M7B6W1R0VNEC","#,
            r#""labels":{"task":"fix-failing-test","experiment":null,"trial":"3"},"#,
            r#""stop_reason":"provider_error","turns":1,"#,
            r#""usage":{"input_tokens":12,"output_tokens":3,"reasoning_output_tokens":null,"cache_read_tokens":8,"cache_write_tokens":0},"#,
            r#""tool_calls":2,"duration_ms":250,"result":{"text":"partial","structured":null},"#,
            r#""error":"provider: 401 unauthorized"}"#
        )
    );
}

/// The constant is what a reader tells forms apart by, so a change to it is
/// a change to the contract and this test is where that is noticed.
#[test]
fn the_published_version_is_one() {
    assert_eq!(OUTCOME_SCHEMA_VERSION, 1);
}

/// O11 and O14, the outcome's half: a run asked for under no label, whose
/// provider reported no count but its input and output, has all six keys in
/// its document, each `null`.
#[test]
fn an_outcome_writes_each_label_and_each_count_it_lacks_as_null() {
    let outcome = RunOutcome::closing(OutcomeParts {
        labels: RunLabels::default(),
        usage: model::Usage::from_inclusive(TokenCounts {
            input: 12,
            output: 3,
            reasoning: None,
            cache_read: None,
            cache_write: None,
        }),
        ..parts(model::StopReason::MaxTurns, None, None)
    });

    let document = serde_json::to_value(OutcomeDocument::from(outcome)).unwrap();

    assert_eq!(
        document["labels"],
        json!({ "task": null, "experiment": null, "trial": null })
    );
    assert_eq!(
        document["usage"],
        json!({
            "input_tokens": 12,
            "output_tokens": 3,
            "reasoning_output_tokens": null,
            "cache_read_tokens": null,
            "cache_write_tokens": null,
        })
    );
    assert_eq!(document["error"], Value::Null);
    assert_eq!(document["result"]["structured"], Value::Null);
}

#[test]
fn an_outcome_a_run_can_have_reads_back_as_itself() {
    let argument = json!({ "passed": true });
    for outcome in [
        failed(),
        RunOutcome::closing(parts(
            model::StopReason::Completed,
            Some(argument.clone()),
            None,
        )),
        RunOutcome::closing(parts(model::StopReason::Completed, None, None)),
        RunOutcome::closing(parts(model::StopReason::MaxTurns, None, None)),
    ] {
        let written = serde_json::to_string(&OutcomeDocument::from(outcome.clone())).unwrap();

        let document = serde_json::from_str::<OutcomeDocument>(&written).unwrap();

        assert_eq!(document, OutcomeDocument::from(outcome.clone()));
        assert_eq!(RunOutcome::try_from(document), Ok(outcome));
    }
}

#[test]
fn a_document_someone_else_wrote_reads_as_the_outcome_it_states() {
    let document = written(
        "provider_error",
        &Value::Null,
        &json!("provider: 401 unauthorized"),
    );

    assert_eq!(read(document), Ok(failed()));
}

#[test]
fn an_outcome_no_run_can_have_is_refused_with_the_rule_it_breaks() {
    let (argument, boom, null) = (json!({ "passed": true }), json!("boom"), Value::Null);
    for (document, rule, message) in [
        (
            written("completed", &null, &boom),
            OutcomeError::ErrorWithoutFailure {
                stop_reason: model::StopReason::Completed,
            },
            "a run that stopped with completed didn't fail, so it has no error",
        ),
        (
            written("provider_error", &null, &null),
            OutcomeError::FailureWithoutError {
                stop_reason: model::StopReason::ProviderError,
            },
            "a run that stopped with provider_error failed, so it has an error",
        ),
        (
            written("max_turns", &argument, &null),
            OutcomeError::StructuredWithoutCompletion {
                stop_reason: model::StopReason::MaxTurns,
            },
            "a run that stopped with max_turns didn't complete, so it has no structured result",
        ),
    ] {
        let refused = read(document).unwrap_err();

        assert_eq!(refused, OutcomeDocumentError::BrokenRule(rule));
        assert_eq!(refused.to_string(), message);
    }
}

/// A later form may keep every key and change what one means, so a version
/// this crate doesn't know is refused even when everything else reads.
#[test]
fn a_document_in_a_version_this_does_not_know_is_refused_whichever_way_it_differs() {
    for found in [0, 2] {
        let mut document = written("completed", &Value::Null, &Value::Null);
        document["schema_version"] = json!(found);

        let refused = read(document).unwrap_err();

        assert_eq!(refused, OutcomeDocumentError::UnknownVersion { found });
        assert_eq!(
            refused.to_string(),
            format!(
                "the outcome document is in version {found} of its form, and this reads version 1"
            )
        );
    }
}

/// The outcome document is the contract a composer parses, so a key it
/// doesn't recognise is a misspelling to report, not one to pass over.
#[test]
fn a_key_the_document_does_not_know_is_refused_wherever_it_is() {
    for (at, unknown) in [
        ("", "stop_resaon"),
        ("/labels", "trail"),
        ("/usage", "input_token"),
        ("/result", "structrued"),
    ] {
        let mut document = written("completed", &Value::Null, &Value::Null);
        document.pointer_mut(at).unwrap()[unknown] = json!("3");

        let refused = serde_json::from_value::<OutcomeDocument>(document).unwrap_err();

        assert!(
            refused
                .to_string()
                .contains(&format!("unknown field `{unknown}`")),
            "{refused}"
        );
    }
}

/// Every outcome lablet writes holds these, so a document without one isn't
/// an outcome, and reading it as a default would be a guess.
#[test]
fn a_document_without_a_key_every_outcome_has_is_refused() {
    for missing in [
        "schema_version",
        "run_id",
        "labels",
        "stop_reason",
        "turns",
        "usage",
        "tool_calls",
        "duration_ms",
        "result",
    ] {
        let mut document = written("completed", &Value::Null, &Value::Null);
        document.as_object_mut().unwrap().remove(missing);

        let refused = serde_json::from_value::<OutcomeDocument>(document).unwrap_err();

        assert!(
            refused
                .to_string()
                .contains(&format!("missing field `{missing}`")),
            "{refused}"
        );
    }
}

#[test]
fn a_run_id_that_is_not_one_is_refused_with_the_rule_it_breaks() {
    let mut document = written("completed", &Value::Null, &Value::Null);
    document["run_id"] = json!(" 01K5F3Z8Q4X9T2M7B6W1R0VNEC");

    let refused = serde_json::from_value::<OutcomeDocument>(document).unwrap_err();

    assert!(
        refused
            .to_string()
            .contains("has leading or trailing whitespace"),
        "{refused}"
    );
}

const STOP_REASONS: [model::StopReason; 12] = [
    model::StopReason::Completed,
    model::StopReason::EndedWithoutCompletion,
    model::StopReason::MaxTurns,
    model::StopReason::Timeout,
    model::StopReason::MaxTotalTokens,
    model::StopReason::OutputTruncated,
    model::StopReason::ContextExhausted,
    model::StopReason::RetriesExhausted,
    model::StopReason::InvalidCallsExhausted,
    model::StopReason::Cancelled,
    model::StopReason::ProviderError,
    model::StopReason::Refused,
];

/// The outcome and the wide event name a run's ending with one word, so a
/// consumer that joins the two never maps one spelling to another.
#[test]
fn every_stop_reason_is_written_as_telemetry_spells_it_and_reads_back_as_itself() {
    for reason in STOP_REASONS {
        let written = serde_json::to_value(StopReason::from(reason)).unwrap();

        assert_eq!(written, json!(reason.as_str()));
        assert_eq!(
            model::StopReason::from(serde_json::from_value::<StopReason>(written).unwrap()),
            reason
        );
    }
}

#[test]
fn a_stop_reason_lablet_does_not_have_is_refused() {
    let refused =
        serde_json::from_value::<OutcomeDocument>(written("gave_up", &Value::Null, &Value::Null))
            .unwrap_err();

    assert!(
        refused.to_string().contains("unknown variant `gave_up`"),
        "{refused}"
    );
}
