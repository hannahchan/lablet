use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::outcome::RawOutcome;
use crate::{
    CacheScope, CompletionMode, Cost, Effort, Endpoint, FinishReason, ModelRef, Prompts,
    ProviderApi, ProviderKind, Rates, RequestParams, Run, RunOutcome, RunSetup, StopReason,
    Thinking, TokenCounts, ToolName, ToolStats, Usage,
};
use crate::{RunId, RunLabels, TaskResult};

/// Every field, because each one is a wide-event attribute or a join key of
/// every record.
#[test]
fn a_run_context_has_one_json_form() {
    let context = RunContext {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        labels: labels(),
        started_unix_ms: 1_790_000_000_123,
        config_digest: "9f2c".to_owned(),
        agent_version: "0.1.0".to_owned(),
        resource: vec![("experiment.id".to_owned(), "exp-7".to_owned())],
        transcript_path: Some(PathBuf::from("out/transcript.json")),
        skills_count: 2,
        mcp_servers: vec!["docs".to_owned(), "tickets".to_owned()],
        mcp_server_versions: vec!["1.4.0".to_owned(), "0.9.2".to_owned()],
        mcp_lifetime: Some(McpLifetime::Lablet),
        capture_content: true,
    };
    let expected = json!({
        "run_id": "01K5F3Z8Q4X9T2M7B6W1R0VNEC",
        "labels": { "task": "fix-failing-test", "experiment": null, "trial": "3" },
        "started_unix_ms": 1_790_000_000_123_u64,
        "config_digest": "9f2c",
        "agent_version": "0.1.0",
        "resource": [["experiment.id", "exp-7"]],
        "transcript_path": "out/transcript.json",
        "skills_count": 2,
        "mcp_servers": ["docs", "tickets"],
        "mcp_server_versions": ["1.4.0", "0.9.2"],
        "mcp_lifetime": "lablet",
        "capture_content": true,
    });

    assert_eq!(serde_json::to_value(&context).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<RunContext>(expected).unwrap(),
        context
    );
}

#[test]
fn a_run_without_mcp_servers_has_no_lifetime_for_them() {
    let context = RunContext {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        labels: RunLabels::default(),
        started_unix_ms: 0,
        config_digest: "9f2c".to_owned(),
        agent_version: "0.1.0".to_owned(),
        resource: Vec::new(),
        transcript_path: None,
        skills_count: 0,
        mcp_servers: Vec::new(),
        mcp_server_versions: Vec::new(),
        mcp_lifetime: None,
        capture_content: false,
    };

    let json = serde_json::to_value(&context).unwrap();
    assert_eq!(json["mcp_lifetime"], Value::Null);
    assert_eq!(
        json["labels"],
        json!({ "task": null, "experiment": null, "trial": null })
    );
    assert_eq!(serde_json::from_value::<RunContext>(json).unwrap(), context);
}

#[test]
fn an_mcp_lifetime_prints_what_it_serialises_as() {
    for (lifetime, spelling) in [(McpLifetime::Run, "run"), (McpLifetime::Lablet, "lablet")] {
        assert_eq!(lifetime.to_string(), spelling);
        assert_eq!(serde_json::to_value(lifetime).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<McpLifetime>(json!(spelling)).unwrap(),
            lifetime
        );
    }
}

fn summary() -> RunSummary {
    let bash = ToolName::new("bash").unwrap();
    RunSummary {
        model: ModelRef {
            provider: ProviderKind::Openai,
            api: ProviderApi::ChatCompletions,
            name: "qwen3".to_owned(),
            replays_reasoning: true,
        },
        endpoint: Some(Endpoint {
            host: "localhost".to_owned(),
            port: 11434,
        }),
        tools: vec![bash.clone()],
        completion: CompletionMode::Explicit,
        max_turns: NonZeroU32::new(30),
        timeout_ms: 600_000,
        request: RequestParams {
            max_tokens: 4096,
            temperature: None,
            thinking: Thinking::default(),
            effort: Some(Effort::Low),
            seed: None,
            cache_scope: CacheScope::Run,
        },
        prompt_system_bytes: 120,
        prompt_user_bytes: 40,
        prompt_tools_bytes: 312,
        tools_digest: "5f70".to_owned(),
        system_prompt_digest: "c1a5".to_owned(),
        failed_usage: Some(Usage::from_inclusive(TokenCounts {
            input: 9,
            output: 0,
            reasoning: None,
            cache_read: Some(8),
            cache_write: None,
        })),
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
            "model": {
                "provider": "openai",
                "api": "chat_completions",
                "name": "qwen3",
                "replays_reasoning": true,
            },
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
                "cache_scope": "run",
            },
            "prompt_system_bytes": 120,
            "prompt_user_bytes": 40,
            "prompt_tools_bytes": 312,
            "tools_digest": "5f70",
            "system_prompt_digest": "c1a5",
            "failed_usage": {
                "input_tokens": 9,
                "output_tokens": 0,
                "reasoning_output_tokens": null,
                "cache_read_tokens": 8,
                "cache_write_tokens": null,
            },
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
            "outcome": {
                "run_id": "01K5F3Z8Q4X9T2M7B6W1R0VNEC",
                "labels": { "task": "fix-failing-test", "experiment": null, "trial": "3" },
                "stop_reason": "provider_error",
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
                "result": { "text": "partial", "structured": null },
                "error": "provider: 401 unauthorized",
            },
        })
    );
}

/// `null` rather than a key left out, so every run's summary has the same
/// keys.
#[test]
fn a_run_summary_without_a_turn_cap_writes_it_as_null() {
    let uncapped = RunSummary {
        max_turns: None,
        ..summary()
    };

    let json = serde_json::to_value(uncapped).unwrap();
    assert_eq!(json["max_turns"], Value::Null);
    assert!(json.as_object().unwrap().contains_key("max_turns"));
}

#[test]
fn a_finished_run_carries_the_summary_and_the_conversation() {
    let summary = summary();
    let setup = RunSetup {
        run_id: summary.outcome.run_id.clone(),
        labels: summary.outcome.labels.clone(),
        model: summary.model,
        endpoint: summary.endpoint,
        tools: summary.tools,
        tools_bytes: summary.prompt_tools_bytes,
        tools_digest: summary.tools_digest,
        system_prompt_digest: summary.system_prompt_digest,
        completion: summary.completion,
        max_turns: summary.max_turns,
        timeout: Duration::from_secs(600),
        request: summary.request,
    };
    let finished = Run::start(
        setup,
        Prompts::new("Be brief.", "Hi.").expect("the task isn't blank"),
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

fn labels() -> RunLabels {
    RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: None,
        trial: Some("3".to_owned()),
    }
}

fn raw(stop_reason: StopReason, structured: Option<Value>, error: Option<&str>) -> RawOutcome {
    RawOutcome {
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

fn outcome() -> RunOutcome {
    RunOutcome::closing(raw(
        StopReason::ProviderError,
        None,
        Some("provider: 401 unauthorized"),
    ))
}
