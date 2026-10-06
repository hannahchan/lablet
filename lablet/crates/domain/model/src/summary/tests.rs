use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;
use crate::{
    CacheScope, CompletionMode, Cost, Effort, Endpoint, FinishReason, Latency, ModelRef,
    OutcomeParts, Prompts, ProviderApi, ProviderTotals, Rates, RequestParams, Run, RunId,
    RunLabels, RunOutcome, RunSetup, StopReason, TaskResult, Thinking, TokenCounts, ToolCallTotals,
    ToolName, ToolStats, Usage,
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
        prompt: PromptSizes {
            system_bytes: 120,
            user_bytes: 40,
            tools_bytes: 312,
        },
        tools_digest: "5f70".to_owned(),
        system_prompt_digest: "c1a5".to_owned(),
        failed_usage: Some(Usage::from_inclusive(TokenCounts {
            input: 9,
            output: 0,
            reasoning: None,
            cache_read: Some(8),
            cache_write: None,
        })),
        provider: ProviderTotals {
            retries: 1,
            latency: Latency::of(400) + Latency::of(500),
        },
        finish_reasons: vec![FinishReason::ToolUse, FinishReason::EndTurn],
        tool_calls: ToolCallTotals {
            errors: 1,
            unknown: 0,
            truncated: 1,
            latency_ms: 35,
            input_bytes: 18,
            output_bytes: 2048,
        },
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
        tools_bytes: summary.prompt.tools_bytes,
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
        Duration::from_micros(250_400),
        None,
        None,
        None,
        None,
    );

    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Cancelled
    );
    assert_eq!(finished.duration, Duration::from_micros(250_400));
    assert_eq!(finished.summary.outcome.duration_ms, 250);
    assert_eq!(finished.transcript.system(), "Be brief.");
    assert!(finished.transcript.turns().is_empty());
}

fn started_at(started: SystemTime) -> RunContext {
    RunContext {
        run_id: RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap(),
        labels: labels(),
        started,
        config_digest: crate::ConfigDigest::new("0".repeat(64)).unwrap(),
        agent_version: "0.1.0".to_owned(),
        transcript_path: None,
        skills_count: 0,
        mcp: None,
        capture_content: false,
    }
}

#[test]
fn the_documents_start_is_the_run_s_start_cut_to_whole_milliseconds() {
    let started = UNIX_EPOCH + Duration::from_nanos(1_790_000_000_123_999_999);

    assert_eq!(started_at(started).started_unix_ms(), 1_790_000_000_123);
}

#[test]
fn a_run_that_started_before_the_epoch_started_at_it_in_the_documents() {
    let started = UNIX_EPOCH - Duration::from_nanos(1);

    assert_eq!(started_at(started).started_unix_ms(), 0);
}

#[test]
fn the_sizes_of_a_prompt_start_at_nothing() {
    assert_eq!(
        PromptSizes::default(),
        PromptSizes {
            system_bytes: 0,
            user_bytes: 0,
            tools_bytes: 0,
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
