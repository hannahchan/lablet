use std::time::Duration;

use super::*;
use crate::{
    CacheScope, CompletionMode, Cost, Effort, Endpoint, FinishReason, ModelRef, OutcomeParts,
    Prompts, ProviderApi, Rates, RequestParams, Run, RunId, RunLabels, RunOutcome, RunSetup,
    StopReason, TaskResult, Thinking, TokenCounts, ToolName, ToolStats, Usage,
};

fn summary() -> RunSummary {
    let bash = ToolName::new("bash").unwrap();
    RunSummary {
        model: ModelRef {
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

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Cancelled
    );
    assert_eq!(finished.summary.outcome.duration_ms, 250);
    assert_eq!(finished.transcript.system(), "Be brief.");
    assert!(finished.transcript.turns().is_empty());
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

fn outcome() -> RunOutcome {
    RunOutcome::closing(OutcomeParts {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        labels: labels(),
        stop_reason: StopReason::ProviderError,
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
            structured: None,
        },
        error: Some("provider: 401 unauthorized".to_owned()),
    })
}
