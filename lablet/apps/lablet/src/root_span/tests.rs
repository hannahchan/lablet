//! The root span's struct against the run it's filled from, and its status
//! by stop reason.

use std::num::NonZeroU32;
use std::time::Duration;

use lablet_model::{
    CacheScope, CompletionMode, ConfigDigest, Cost, FinishReason, ModelRef, Prompts, ProviderApi,
    ProviderResponse, RequestParams, Responded, Run, RunId, RunLabels, RunSetup, Thinking,
    ToolName, Usage,
};
use lablet_run::telemetry::Value;
use lablet_run::telemetry::generated::key;
use lablet_test_support::{PROMPT, SYSTEM};

use super::*;
use crate::telemetry::generated::{LabletInvokeAgentErrorType, LabletRunStopReason};

const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
const CONFIG_DIGEST: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

fn context() -> RunContext {
    RunContext {
        run_id: RunId::new(RUN).unwrap(),
        labels: RunLabels {
            task: Some("fix-failing-test".to_owned()),
            experiment: None,
            trial: None,
        },
        started_unix_ms: 1_790_000_000_000,
        config_digest: ConfigDigest::new(CONFIG_DIGEST).unwrap(),
        agent_version: "0.4.2".to_owned(),
        transcript_path: None,
        skills_count: 0,
        mcp: None,
        capture_content: false,
    }
}

fn setup(context: &RunContext) -> RunSetup {
    RunSetup {
        run_id: context.run_id.clone(),
        labels: context.labels.clone(),
        model: ModelRef {
            api: ProviderApi::Script,
            name: "scripted-1".to_owned(),
            replays_reasoning: false,
        },
        endpoint: None,
        tools: vec![ToolName::new("bash").unwrap()],
        tools_bytes: 0,
        tools_digest: String::new(),
        system_prompt_digest: String::new(),
        completion: CompletionMode::Natural,
        max_turns: NonZeroU32::new(30),
        timeout: Duration::from_secs(600),
        request: RequestParams {
            max_tokens: 4_096,
            temperature: None,
            thinking: Thinking::ProviderDefault,
            effort: None,
            seed: None,
            cache_scope: CacheScope::Shared,
        },
    }
}

/// A run that completed on its first response, which used `usage` and cost
/// `cost`.
fn completed(usage: Usage, cost: Option<Cost>) -> RunSummary {
    let response = ProviderResponse::new(
        vec![lablet_model::ContentBlock::Text("Done.".to_owned())],
        usage,
        FinishReason::EndTurn,
        None,
        None,
    )
    .unwrap();
    let run = Run::start(setup(&context()), Prompts::new(SYSTEM, PROMPT).unwrap());
    let Responded::Final(done) = run.responded(
        response,
        Duration::from_millis(5),
        Duration::from_millis(250),
    ) else {
        panic!("a response that calls no tool is the run's last");
    };
    done.finish(
        StopReason::Completed,
        Duration::from_millis(12_345),
        None,
        None,
        None,
        cost,
    )
    .summary
}

/// A run that stopped for `reason` with no response and the provider's
/// words for its error.
fn stopped(reason: StopReason) -> RunSummary {
    Run::start(setup(&context()), Prompts::new(SYSTEM, PROMPT).unwrap())
        .finish(
            reason,
            Duration::from_millis(12_345),
            None,
            Some("529 overloaded".to_owned()),
            None,
            None,
        )
        .summary
}

#[test]
fn the_span_is_named_for_the_operation_and_the_agent() {
    assert_eq!(name(), "invoke_agent lablet");
    let filled = invoke_agent(&context(), &completed(Usage::default(), None));
    assert_eq!(filled.name(), name());
}

#[test]
fn a_completed_run_fills_what_names_it_and_its_totals_and_no_error() {
    let usage = Usage {
        input_tokens: 1_200,
        output_tokens: 80,
        reasoning_output_tokens: Some(30),
        cache_read_tokens: Some(1_000),
        cache_write_tokens: None,
    };
    let summary = completed(usage, Some(Cost::new(0.0421).unwrap()));

    let filled = invoke_agent(&context(), &summary);

    assert_eq!(
        filled,
        LabletInvokeAgent {
            join: Join::from(&context()),
            gen_ai_agent_version: "0.4.2".to_owned(),
            gen_ai_request_model: "scripted-1".to_owned(),
            gen_ai_usage_input_tokens: 1_200,
            gen_ai_usage_output_tokens: 80,
            lablet_run_stop_reason: LabletRunStopReason::Completed,
            lablet_run_turns: 1,
            lablet_tool_calls_total: 0,
            error_type: None,
            gen_ai_usage_cache_read_input_tokens: Some(1_000),
            gen_ai_usage_cache_write_input_tokens: None,
            gen_ai_usage_reasoning_output_tokens: Some(30),
            lablet_run_cost_usd: Some(0.0421),
        }
    );
    let attributes = filled.attributes();
    let task = attributes
        .iter()
        .find(|attribute| attribute.key() == key::LABLET_TASK_ID)
        .unwrap();
    assert_eq!(
        task.value(),
        Some(&Value::Text("fix-failing-test".to_owned()))
    );
    assert_eq!(status(&summary.outcome), Status::Unset);
}

#[test]
fn a_run_that_did_not_complete_names_its_stop_reason_as_its_error_type_and_status() {
    let failed = stopped(StopReason::RetriesExhausted);
    let cancelled = stopped(StopReason::Cancelled);

    assert_eq!(
        invoke_agent(&context(), &failed).error_type,
        Some(LabletInvokeAgentErrorType::RetriesExhausted)
    );
    assert_eq!(
        status(&failed.outcome),
        Status::error("529 overloaded".to_owned())
    );
    assert_eq!(
        invoke_agent(&context(), &cancelled).error_type,
        Some(LabletInvokeAgentErrorType::Cancelled)
    );
    assert_eq!(
        status(&cancelled.outcome),
        Status::error(String::new()),
        "no error ended a run that was cancelled, so the status says only that it didn't complete"
    );
}
