use serde_json::{Value, json};

use super::*;
use crate::{Effort, ProviderKind, Run, RunSetup, Thinking};

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
fn an_unknown_stop_reason_does_not_deserialise() {
    assert!(serde_json::from_value::<StopReason>(json!("gave_up")).is_err());
}

#[test]
fn natural_is_the_default_completion_mode() {
    assert_eq!(CompletionMode::default(), CompletionMode::Natural);
}

// The class of each reason, in the order of `STOP_REASONS`.
const CLASSES: [StopClass; 12] = [
    StopClass::Completed,
    StopClass::Stopped,
    StopClass::Stopped,
    StopClass::Stopped,
    StopClass::Stopped,
    StopClass::Stopped,
    StopClass::Failed,
    StopClass::Failed,
    StopClass::Failed,
    StopClass::Stopped,
    StopClass::Failed,
    StopClass::Stopped,
];

#[test]
fn every_stop_reason_has_its_class() {
    for ((reason, _), class) in STOP_REASONS.into_iter().zip(CLASSES) {
        assert_eq!(reason.class(), class, "{reason}");
    }
}

fn raw(stop_reason: StopReason, structured: Option<Value>, error: Option<&str>) -> RawOutcome {
    RawOutcome {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        stop_reason,
        turns: 1,
        usage: Usage::from_inclusive(12, 3, 8, 0),
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
    assert_eq!(outcome.usage, Usage::from_inclusive(12, 3, 8, 0));
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
            r#""usage":{"input_tokens":12,"output_tokens":3,"cache_read_tokens":8,"cache_write_tokens":0},"#,
            r#""tool_calls":2,"duration_ms":250,"result":{"text":"partial","structured":null},"#,
            r#""error":"provider: 401 unauthorized"}"#
        )
    );
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
    for ((reason, _), class) in STOP_REASONS.into_iter().zip(CLASSES) {
        let outcome = RunOutcome::closing(raw(reason, None, Some("boom")));

        let expected = (class == StopClass::Failed).then_some("boom");
        assert_eq!(outcome.error(), expected, "{reason}");
    }
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

#[test]
fn a_run_context_round_trips_through_json() {
    let context = RunContext {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        config_digest: "9f2c".to_owned(),
        agent_version: "0.1.0".to_owned(),
        resource: vec![("experiment.id".to_owned(), "exp-7".to_owned())],
        transcript_path: Some(PathBuf::from("out/transcript.json")),
        skills_count: 2,
        mcp_servers: vec!["docs".to_owned()],
        capture_content: true,
    };

    let json = serde_json::to_value(&context).unwrap();
    assert_eq!(json["resource"], json!([["experiment.id", "exp-7"]]));
    assert_eq!(json["capture_content"], json!(true));
    assert_eq!(serde_json::from_value::<RunContext>(json).unwrap(), context);
}

fn summary() -> RunSummary {
    let bash = ToolName::new("bash").unwrap();
    RunSummary {
        model: ModelRef {
            provider: ProviderKind::Openai,
            name: "qwen3".to_owned(),
        },
        endpoint: Some(Endpoint {
            host: "localhost".to_owned(),
            port: 11434,
        }),
        tools: vec![bash.clone()],
        completion: CompletionMode::Explicit,
        max_turns: NonZeroU32::new(30).unwrap(),
        timeout_ms: 600_000,
        request: RequestParams {
            max_tokens: 4096,
            temperature: None,
            thinking: Thinking::default(),
            effort: Some(Effort::Low),
            seed: None,
        },
        prompt_system_bytes: 120,
        prompt_user_bytes: 40,
        provider_retries: 1,
        provider_latency_total_ms: 900,
        provider_latency_max_ms: 500,
        finish_reasons: vec![FinishReason::ToolUse, FinishReason::EndTurn],
        tool_calls_errors: 1,
        tool_calls_unknown: 0,
        tool_latency_total_ms: 35,
        tool_input_bytes: 18,
        tool_output_bytes: 2048,
        tool_calls_truncated: 1,
        per_tool: BTreeMap::from([(
            bash,
            ToolStats {
                calls: 1,
                errors: 1,
                latency_ms: 35,
            },
        )]),
        cost: Some(Cost::new(0.002).unwrap()),
        outcome: outcome(),
    }
}

/// A summary is written, never read, so there's a written form to pin and no
/// round trip to make.
#[test]
fn a_run_summary_has_one_json_form() {
    let json = serde_json::to_value(summary()).unwrap();

    assert_eq!(
        json["endpoint"],
        json!({ "host": "localhost", "port": 11434 })
    );
    assert_eq!(json["completion"], json!("explicit"));
    assert_eq!(json["timeout_ms"], json!(600_000));
    assert_eq!(json["request"]["effort"], json!("low"));
    assert_eq!(
        json["per_tool"],
        json!({ "bash": { "calls": 1, "errors": 1, "latency_ms": 35 } })
    );
    assert_eq!(json["finish_reasons"], json!(["tool_use", "end_turn"]));
    assert_eq!(json["cost"], json!(0.002));
    assert_eq!(json["max_turns"], json!(30));
}

#[test]
fn a_finished_run_carries_the_summary_and_the_conversation() {
    let summary = summary();
    let setup = RunSetup {
        run_id: summary.outcome.run_id.clone(),
        model: summary.model,
        endpoint: summary.endpoint,
        tools: summary.tools,
        completion: summary.completion,
        max_turns: summary.max_turns,
        timeout: Duration::from_secs(600),
        request: summary.request,
    };
    let finished = Run::start(setup, "Be brief.".to_owned(), "Hi.".to_owned()).finish(
        StopReason::Cancelled,
        Duration::from_millis(250),
        None,
        None,
        None,
    );

    let json = serde_json::to_value(&finished).unwrap();
    assert_eq!(
        json["summary"]["outcome"]["stop_reason"],
        json!("cancelled")
    );
    assert_eq!(
        json["transcript"],
        json!({ "system": "Be brief.", "turns": [] })
    );
}

#[test]
fn tool_stats_start_at_zero() {
    assert_eq!(
        ToolStats::default(),
        ToolStats {
            calls: 0,
            errors: 0,
            latency_ms: 0,
        }
    );
}
