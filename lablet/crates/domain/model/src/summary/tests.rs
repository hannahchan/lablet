use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::outcome::RawOutcome;
use crate::{
    CompletionMode, Cost, Effort, Endpoint, FinishReason, ModelRef, Prompts, ProviderKind, Rates,
    RequestParams, Run, RunOutcome, RunSetup, StopReason, Thinking, TokenCounts, ToolName,
    ToolStats, Usage,
};
use crate::{RunId, TaskResult};

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
        rates: Some(Rates::new(3.0, 15.0, 0.3, 3.75).unwrap()),
        cost: Some(Cost::new(0.002).unwrap()),
        outcome: outcome(),
    }
}

/// A summary is written, never read, so there's a written form to pin and no
/// round trip to make.
#[test]
fn a_run_summary_has_one_json_form() {
    let json = serde_json::to_value(summary()).unwrap();

    // Every field, because each one is a wide-event attribute: a rename or a
    // field that stops being written loses an attribute silently otherwise.
    assert_eq!(
        json,
        json!({
            "model": { "provider": "openai", "name": "qwen3" },
            "endpoint": { "host": "localhost", "port": 11434 },
            "tools": ["bash"],
            "completion": "explicit",
            "max_turns": 30,
            "timeout_ms": 600_000,
            "request": {
                "max_tokens": 4096,
                "temperature": null,
                "thinking": "provider_default",
                "effort": "low",
                "seed": null,
            },
            "prompt_system_bytes": 120,
            "prompt_user_bytes": 40,
            "provider_retries": 1,
            "provider_latency_total_ms": 900,
            "provider_latency_max_ms": 500,
            "finish_reasons": ["tool_use", "end_turn"],
            "tool_calls_errors": 1,
            "tool_calls_unknown": 0,
            "tool_latency_total_ms": 35,
            "tool_input_bytes": 18,
            "tool_output_bytes": 2048,
            "tool_calls_truncated": 1,
            "per_tool": { "bash": { "calls": 1, "errors": 1, "latency_ms": 35 } },
            "rates": { "input": 3.0, "output": 15.0, "cache_read": 0.3, "cache_write": 3.75 },
            "cost": 0.002,
            "outcome": serde_json::to_value(outcome()).unwrap(),
        })
    );
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
    let finished = Run::start(
        setup,
        Prompts {
            system: "Be brief.".to_owned(),
            task: "Hi.".to_owned(),
        },
    )
    .finish(
        StopReason::Cancelled,
        Duration::from_millis(250),
        None,
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
