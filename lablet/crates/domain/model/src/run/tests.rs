use serde_json::json;

use super::*;
use crate::{ProviderKind, Thinking};

// The literal spellings below are the members of `lablet.run.stop_reason` and
// `lablet.run.completion_mode` in the generated telemetry-registry crate,
// which the domain can't depend on. A spelling that changes on either side
// must change on both.
const STOP_REASONS: [(StopReason, &str); 11] = [
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

fn outcome() -> RunOutcome {
    RunOutcome {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        stop_reason: StopReason::ProviderError,
        turns: 1,
        usage: Usage::default(),
        tool_calls: 0,
        duration_ms: 250,
        result: RunResult::default(),
        error: Some("provider: 401 unauthorized".to_owned()),
    }
}

#[test]
fn a_failed_natural_run_writes_the_error_as_a_string_and_structured_as_null() {
    assert_eq!(
        serde_json::to_value(outcome()).unwrap(),
        json!({
            "run_id": "01K5F3Z8Q4X9T2M7B6W1R0VNEC",
            "stop_reason": "provider_error",
            "turns": 1,
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0,
                "cache_read_tokens": 0,
                "cache_write_tokens": 0,
            },
            "tool_calls": 0,
            "duration_ms": 250,
            "result": { "text": "", "structured": null },
            "error": "provider: 401 unauthorized",
        })
    );
}

#[test]
fn a_run_context_and_summary_round_trip_through_json() {
    let context = RunContext {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        config_digest: "9f2c".to_owned(),
        agent_version: "0.1.0".to_owned(),
        resource: vec![("experiment.id".to_owned(), "exp-7".to_owned())],
        transcript_path: Some(PathBuf::from("out/transcript.json")),
        skills_count: 2,
        mcp_servers: vec!["docs".to_owned()],
        completion: CompletionMode::Explicit,
        max_turns: 30,
        timeout_ms: 600_000,
        request: RequestDefaults {
            max_tokens: 4096,
            temperature: None,
            thinking: Thinking::default(),
            seed: None,
        },
        capture_content: true,
    };
    let bash = ToolName::new("bash").unwrap();
    let summary = RunSummary {
        model: ModelRef {
            provider: ProviderKind::Fake,
            name: "scripted".to_owned(),
        },
        tools: vec![bash.clone()],
        prompt_system_bytes: 120,
        prompt_user_bytes: 40,
        provider_calls: 2,
        provider_retries: 1,
        provider_latency_total_ms: 900,
        provider_latency_max_ms: 500,
        finish_reasons: vec![FinishReason::ToolUse, FinishReason::EndTurn],
        tool_calls_errors: 1,
        tool_latency_total_ms: 35,
        tool_input_bytes: 18,
        tool_output_bytes: 2048,
        per_tool: BTreeMap::from([(
            bash,
            ToolStats {
                calls: 1,
                errors: 1,
                latency_ms: 35,
            },
        )]),
        cost: Some(Cost::new(0.002)),
        outcome: outcome(),
    };

    let context_json = serde_json::to_value(&context).unwrap();
    assert_eq!(context_json["timeout_ms"], json!(600_000));
    assert_eq!(context_json["completion"], json!("explicit"));
    assert_eq!(
        context_json["resource"],
        json!([["experiment.id", "exp-7"]])
    );
    assert_eq!(
        serde_json::from_value::<RunContext>(context_json).unwrap(),
        context
    );

    let summary_json = serde_json::to_value(&summary).unwrap();
    assert_eq!(
        summary_json["per_tool"],
        json!({ "bash": { "calls": 1, "errors": 1, "latency_ms": 35 } })
    );
    assert_eq!(
        summary_json["finish_reasons"],
        json!(["tool_use", "end_turn"])
    );
    assert_eq!(summary_json["cost"], json!(0.002));
    assert_eq!(
        serde_json::from_value::<RunSummary>(summary_json).unwrap(),
        summary
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
