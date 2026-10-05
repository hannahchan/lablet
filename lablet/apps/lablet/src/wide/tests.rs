//! Every key of the wide event against what the run's context and its
//! summary hold for it, on a run that has everything a run may have and on
//! runs without, each struct's attributes read as the record would carry
//! them.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use lablet_model::{
    CacheScope, CompletionMode, ConfigDigest, Cost, Endpoint, FinishReason, Latency, McpLifetime,
    McpServer, McpServers, ModelRef, OutcomeParts, PromptSizes, Prompts, ProviderApi,
    ProviderResponse, ProviderTotals, Rates, RequestParams, Responded, Run, RunContext, RunId,
    RunLabels, RunOutcome, RunSetup, StopReason, TaskResult, ToolCallTotals, Usage,
};
use lablet_run::telemetry::{Attribute, Value};
use lablet_test_support::{PROMPT, SYSTEM};
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use serde_json::json;

use super::*;

/// Every key the wide event carries: the root's own, and the join keys,
/// which `lablet-run`'s module declares.
mod key {
    pub use lablet_run::telemetry::generated::key::{
        GEN_AI_CONVERSATION_ID, LABLET_CONFIG_DIGEST, LABLET_EXPERIMENT_ID, LABLET_TASK_ID,
        LABLET_TRIAL, SESSION_ID,
    };

    pub use crate::telemetry::generated::key::*;
}

const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
const STARTED_UNIX_MS: u64 = 1_790_000_000_000;
const CONFIG_DIGEST: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
const TOOLS_DIGEST: &str = "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a";
const SYSTEM_DIGEST: &str = "dbc1b4c900ffe48d575b5da5c638040125f65db0fe3e24494b76ea986457d986";
const MODEL: &str = "scripted-1";

/// How long the runs these summaries are of took.
const DURATION_MS: u64 = 12_345;

fn run_id() -> RunId {
    RunId::new(RUN).unwrap()
}

fn tool_name(name: &str) -> ToolName {
    ToolName::new(name).unwrap()
}

fn labelled() -> RunLabels {
    RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: Some("terse-tool-descriptions".to_owned()),
        trial: Some("3".to_owned()),
    }
}

fn context() -> RunContext {
    RunContext {
        run_id: run_id(),
        labels: RunLabels::default(),
        started_unix_ms: STARTED_UNIX_MS,
        config_digest: ConfigDigest::new(CONFIG_DIGEST).unwrap(),
        agent_version: "0.1.0".to_owned(),
        transcript_path: None,
        skills_count: 0,
        mcp: None,
        capture_content: false,
    }
}

fn model() -> ModelRef {
    ModelRef {
        api: ProviderApi::Script,
        name: MODEL.to_owned(),
        replays_reasoning: false,
    }
}

/// The parameters of a run that set nothing a provider has a default for.
fn request() -> RequestParams {
    RequestParams {
        max_tokens: 4_096,
        temperature: None,
        thinking: Thinking::ProviderDefault,
        effort: None,
        seed: None,
        cache_scope: CacheScope::Shared,
    }
}

/// The parameters of a run that set every one.
fn every_parameter() -> RequestParams {
    RequestParams {
        temperature: Some(0.7),
        effort: Some(lablet_model::Effort::High),
        seed: Some(-42),
        ..request()
    }
}

/// What a provider that reports neither reasoning nor its cache says a call
/// used.
fn two_counts() -> Usage {
    Usage {
        input_tokens: 1_200,
        output_tokens: 80,
        ..Usage::default()
    }
}

fn setup(context: &RunContext) -> RunSetup {
    RunSetup {
        run_id: context.run_id.clone(),
        labels: context.labels.clone(),
        model: model(),
        endpoint: None,
        tools: vec![tool_name("bash"), tool_name("mcp__docs__search")],
        tools_bytes: 0,
        tools_digest: String::new(),
        system_prompt_digest: String::new(),
        completion: CompletionMode::Natural,
        max_turns: NonZeroU32::new(30),
        timeout: Duration::from_secs(600),
        request: request(),
    }
}

/// The summary of a run of `context` that stopped for `reason` without a
/// response, so its totals are those of no turns. A reason that's a failure
/// has the provider's words for its error.
fn stopped(context: &RunContext, reason: StopReason) -> RunSummary {
    Run::start(setup(context), Prompts::new(SYSTEM, PROMPT).unwrap())
        .finish(
            reason,
            Duration::from_millis(DURATION_MS),
            None,
            Some("529 overloaded".to_owned()),
            None,
            None,
        )
        .summary
}

/// The summary of a run of `context` that completed on its first response,
/// which used `usage`.
fn completed(context: &RunContext, usage: Usage) -> RunSummary {
    let response = ProviderResponse::new(
        vec![lablet_model::ContentBlock::Text("Done.".to_owned())],
        usage,
        FinishReason::EndTurn,
        None,
        None,
    )
    .unwrap();
    let run = Run::start(setup(context), Prompts::new(SYSTEM, PROMPT).unwrap());
    let Responded::Final(done) = run.responded(
        response,
        Duration::from_millis(5),
        Duration::from_millis(250),
    ) else {
        panic!("a response that calls no tool is the run's last");
    };
    done.finish(
        StopReason::Completed,
        Duration::from_millis(DURATION_MS),
        None,
        None,
        None,
        None,
    )
    .summary
}

/// A context that holds everything a run's context may be without.
fn every_part_of_a_context() -> RunContext {
    let server = |name: &str, version: &str| McpServer {
        name: name.to_owned(),
        version: version.to_owned(),
    };
    RunContext {
        labels: labelled(),
        agent_version: "0.4.2".to_owned(),
        transcript_path: Some(PathBuf::from("runs/01K5F3Z8/transcript.json")),
        skills_count: 801,
        mcp: Some(
            McpServers::new(
                McpLifetime::Lablet,
                vec![server("docs", "1.4.0"), server("search", "0.9.2")],
            )
            .unwrap(),
        ),
        capture_content: true,
        ..context()
    }
}

/// A summary that holds everything a run's summary may be without, and of
/// which every number is one that no other of its numbers is, so that a
/// number in another's place is seen.
fn every_part_of_a_summary() -> RunSummary {
    RunSummary {
        model: ModelRef {
            api: ProviderApi::Messages,
            name: "claude-sonnet-5".to_owned(),
            replays_reasoning: true,
        },
        endpoint: Some(Endpoint {
            host: "api.anthropic.com".to_owned(),
            port: 443,
        }),
        tools: vec![
            tool_name("bash"),
            tool_name("read_file"),
            tool_name("task_complete"),
        ],
        completion: CompletionMode::Explicit,
        max_turns: NonZeroU32::new(30),
        timeout_ms: 600_000,
        request: RequestParams {
            thinking: Thinking::Adaptive,
            cache_scope: CacheScope::Run,
            ..every_parameter()
        },
        prompt: PromptSizes {
            system_bytes: 101,
            user_bytes: 102,
            tools_bytes: 103,
        },
        tools_digest: TOOLS_DIGEST.to_owned(),
        system_prompt_digest: SYSTEM_DIGEST.to_owned(),
        failed_usage: Some(Usage {
            input_tokens: 201,
            output_tokens: 202,
            reasoning_output_tokens: Some(203),
            cache_read_tokens: Some(204),
            cache_write_tokens: Some(205),
        }),
        provider: ProviderTotals {
            retries: 301,
            latency: Latency::of(302) + Latency::of(303),
        },
        finish_reasons: vec![FinishReason::ToolUse, FinishReason::EndTurn],
        tool_calls: ToolCallTotals {
            errors: 401,
            unknown: 402,
            truncated: 403,
            latency_ms: 404,
            input_bytes: 405,
            output_bytes: 406,
        },
        per_tool: BTreeMap::from([
            (
                tool_name("read_file"),
                ToolStats {
                    calls: 511,
                    errors: 512,
                    latency_ms: 513,
                },
            ),
            (
                tool_name("bash"),
                ToolStats {
                    calls: 501,
                    errors: 502,
                    latency_ms: 503,
                },
            ),
        ]),
        rates: Some(Rates::new(3.0, 15.0, 0.3, 3.75).unwrap()),
        cost: Some(Cost::new(0.0421).unwrap()),
        outcome: RunOutcome::closing(OutcomeParts {
            run_id: run_id(),
            labels: labelled(),
            stop_reason: StopReason::Completed,
            turns: 601,
            usage: Usage {
                input_tokens: 701,
                output_tokens: 702,
                reasoning_output_tokens: Some(703),
                cache_read_tokens: Some(704),
                cache_write_tokens: Some(705),
            },
            tool_calls: 602,
            duration_ms: 603,
            result: TaskResult {
                text: "Forty-two.".to_owned(),
                structured: Some(json!({ "answer": 42 })),
            },
            error: None,
        }),
    }
}

/// The attributes of `event` that hold a value, by key, as the record
/// carries them.
fn held(event: &LabletRun) -> BTreeMap<String, Value> {
    let attributes = event.attributes();
    let keys: Vec<&str> = attributes.iter().map(Attribute::key).collect();
    let distinct: BTreeSet<&str> = keys.iter().copied().collect();
    assert_eq!(keys.len(), distinct.len(), "a key is there twice: {keys:?}");
    attributes
        .iter()
        .filter_map(|attribute| {
            attribute
                .value()
                .map(|value| (attribute.key().to_owned(), value.clone()))
        })
        .collect()
}

/// The attributes of the wide event of a run, of which nothing was lost.
fn wide(context: &RunContext, summary: &RunSummary) -> BTreeMap<String, Value> {
    held(&wide_event(context, summary))
}

fn text(text: &str) -> Value {
    Value::Text(text.to_owned())
}

fn texts_of(texts: &[&str]) -> Value {
    Value::Texts(texts.iter().map(|text| (*text).to_owned()).collect())
}

/// What the wide event of [`every_part_of_a_context`] and
/// [`every_part_of_a_summary`] holds to say which run it's of and what the
/// run was set up with.
fn held_of_the_setup() -> Vec<(&'static str, Value)> {
    vec![
        (key::GEN_AI_CONVERSATION_ID, text(RUN)),
        (key::SESSION_ID, text(RUN)),
        (key::GEN_AI_AGENT_NAME, text("lablet")),
        (key::GEN_AI_AGENT_VERSION, text("0.4.2")),
        (key::LABLET_CONFIG_DIGEST, text(CONFIG_DIGEST)),
        (key::LABLET_TOOLS_DIGEST, text(TOOLS_DIGEST)),
        (key::LABLET_PROMPT_SYSTEM_DIGEST, text(SYSTEM_DIGEST)),
        (key::LABLET_TASK_ID, text("fix-failing-test")),
        (key::LABLET_EXPERIMENT_ID, text("terse-tool-descriptions")),
        (key::LABLET_TRIAL, text("3")),
        (key::GEN_AI_PROVIDER_NAME, text("anthropic")),
        (key::GEN_AI_REQUEST_MODEL, text("claude-sonnet-5")),
        (key::SERVER_ADDRESS, text("api.anthropic.com")),
        (key::SERVER_PORT, Value::Int(443)),
        (key::GEN_AI_REQUEST_MAX_TOKENS, Value::Int(4_096)),
        (key::GEN_AI_REQUEST_SEED, Value::Int(-42)),
        (key::GEN_AI_REQUEST_REASONING_LEVEL, text("high")),
        (key::GEN_AI_REQUEST_TEMPERATURE, Value::Float(0.7)),
        (key::LABLET_REQUEST_THINKING, text("adaptive")),
        (key::LABLET_REQUEST_API, text("messages")),
        (key::LABLET_REQUEST_REASONING_REPLAYED, Value::Flag(true)),
        (key::LABLET_REQUEST_CACHE_SCOPE, text("run")),
        (key::LABLET_RUN_COMPLETION_MODE, text("explicit")),
        (key::LABLET_RUN_MAX_TURNS, Value::Int(30)),
        (key::LABLET_RUN_TIMEOUT_MS, Value::Int(600_000)),
        (
            key::LABLET_TOOLS_NAMES,
            texts_of(&["bash", "read_file", "task_complete"]),
        ),
        (key::LABLET_TOOLS_COUNT, Value::Int(3)),
        (key::LABLET_MCP_SERVERS, texts_of(&["docs", "search"])),
        (
            key::LABLET_MCP_SERVER_VERSIONS,
            texts_of(&["1.4.0", "0.9.2"]),
        ),
        (key::LABLET_MCP_LIFETIME, text("lablet")),
        (key::LABLET_PROMPT_SYSTEM_BYTES, Value::Int(101)),
        (key::LABLET_PROMPT_USER_BYTES, Value::Int(102)),
        (key::LABLET_PROMPT_TOOLS_BYTES, Value::Int(103)),
        (key::LABLET_SKILLS_COUNT, Value::Int(801)),
    ]
}

/// What the same wide event holds to say what came of the run, when 901
/// records were lost.
fn held_of_the_outcome() -> Vec<(&'static str, Value)> {
    vec![
        (key::LABLET_RUN_STOP_REASON, text("completed")),
        (key::LABLET_RUN_DURATION_MS, Value::Int(603)),
        (key::LABLET_RUN_TURNS, Value::Int(601)),
        (key::LABLET_RESULT_TEXT_BYTES, Value::Int(10)),
        (key::LABLET_RESULT_HAS_STRUCTURED, Value::Flag(true)),
        (
            key::LABLET_RUN_TRANSCRIPT_PATH,
            text("runs/01K5F3Z8/transcript.json"),
        ),
        (key::LABLET_TELEMETRY_DROPPED_RECORDS, Value::Int(901)),
        (key::LABLET_RESULT_TEXT, text("Forty-two.")),
        (key::LABLET_RESULT_STRUCTURED, text(r#"{"answer":42}"#)),
        (key::LABLET_PROVIDER_RETRIES, Value::Int(301)),
        (key::LABLET_PROVIDER_LATENCY_MS_TOTAL, Value::Int(605)),
        (key::LABLET_PROVIDER_LATENCY_MS_MAX, Value::Int(303)),
        (key::GEN_AI_USAGE_INPUT_TOKENS, Value::Int(701)),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, Value::Int(702)),
        (key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS, Value::Int(703)),
        (key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS, Value::Int(704)),
        (key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS, Value::Int(705)),
        (key::LABLET_PROVIDER_FAILED_INPUT_TOKENS, Value::Int(201)),
        (key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS, Value::Int(202)),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
            Value::Int(204),
        ),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
            Value::Int(205),
        ),
        (
            key::GEN_AI_RESPONSE_FINISH_REASONS,
            texts_of(&["tool_use", "end_turn"]),
        ),
        (key::LABLET_RUN_COST_USD, Value::Float(0.0421)),
        (key::LABLET_PRICING_INPUT_USD_PER_MTOK, Value::Float(3.0)),
        (key::LABLET_PRICING_OUTPUT_USD_PER_MTOK, Value::Float(15.0)),
        (
            key::LABLET_PRICING_CACHE_READ_USD_PER_MTOK,
            Value::Float(0.3),
        ),
        (
            key::LABLET_PRICING_CACHE_WRITE_USD_PER_MTOK,
            Value::Float(3.75),
        ),
        (key::LABLET_TOOL_CALLS_TOTAL, Value::Int(602)),
        (key::LABLET_TOOL_CALLS_ERRORS, Value::Int(401)),
        (key::LABLET_TOOL_CALLS_UNKNOWN, Value::Int(402)),
        (key::LABLET_TOOL_CALLS_TRUNCATED, Value::Int(403)),
        (key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL, Value::Int(404)),
        (key::LABLET_TOOL_CALLS_INPUT_BYTES_TOTAL, Value::Int(405)),
        (key::LABLET_TOOL_CALLS_OUTPUT_BYTES_TOTAL, Value::Int(406)),
        ("lablet.tool.calls.bash", Value::Int(501)),
        ("lablet.tool.errors.bash", Value::Int(502)),
        ("lablet.tool.latency_ms.bash", Value::Int(503)),
        ("lablet.tool.calls.read_file", Value::Int(511)),
        ("lablet.tool.errors.read_file", Value::Int(512)),
        ("lablet.tool.latency_ms.read_file", Value::Int(513)),
    ]
}

#[test]
fn every_key_holds_what_the_runs_context_and_its_summary_hold_for_it() {
    let filled = wide_event(&every_part_of_a_context(), &every_part_of_a_summary());
    // The count of lost records is each destination's to fill.
    let event = LabletRun {
        lablet_telemetry_dropped_records: 901,
        ..filled
    };

    let expected: BTreeMap<String, Value> = [held_of_the_setup(), held_of_the_outcome()]
        .concat()
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
    let attributes = held(&event);
    assert_eq!(attributes, expected);
    assert_eq!(
        attributes.len(),
        event.attributes().len() - 2,
        "every key but the two of a run that didn't complete holds a value"
    );
    assert_eq!(attributes.get(key::ERROR_TYPE), None);
    assert_eq!(attributes.get(key::LABLET_RUN_ERROR), None);
}

#[test]
fn the_wide_event_is_an_info_record_in_the_context_of_the_root_span_at_the_runs_end() {
    let at = UNIX_EPOCH + Duration::from_millis(1_790_000_012_345);
    let root = SpanContext::new(
        TraceId::from(7_u128),
        SpanId::from(9_u64),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );

    let record = wide_event(&context(), &completed(&context(), two_counts())).record(at, &root);

    assert_eq!(record.name, "lablet.run");
    assert_eq!(record.severity, opentelemetry::logs::Severity::Info);
    assert_eq!(record.span, root);
    assert_eq!(record.at, at);
}

#[test]
fn a_key_a_run_has_nothing_for_is_left_out_and_a_count_of_nothing_is_written() {
    let mut summary = completed(&context(), two_counts());
    summary.max_turns = None;
    summary.tools = Vec::new();

    let attributes = wide(&context(), &summary);

    for key in [
        key::LABLET_RUN_MAX_TURNS,
        key::LABLET_RUN_COST_USD,
        key::LABLET_PRICING_INPUT_USD_PER_MTOK,
        key::LABLET_PRICING_OUTPUT_USD_PER_MTOK,
        key::LABLET_PRICING_CACHE_READ_USD_PER_MTOK,
        key::LABLET_PRICING_CACHE_WRITE_USD_PER_MTOK,
        key::LABLET_MCP_SERVERS,
        key::LABLET_MCP_SERVER_VERSIONS,
        key::LABLET_MCP_LIFETIME,
        key::LABLET_TASK_ID,
        key::LABLET_EXPERIMENT_ID,
        key::LABLET_TRIAL,
        key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
        key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
        key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
        key::LABLET_PROVIDER_FAILED_INPUT_TOKENS,
        key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS,
        key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
        key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
        key::GEN_AI_REQUEST_SEED,
        key::GEN_AI_REQUEST_TEMPERATURE,
        key::GEN_AI_REQUEST_REASONING_LEVEL,
        key::SERVER_ADDRESS,
        key::SERVER_PORT,
        key::LABLET_RUN_TRANSCRIPT_PATH,
        key::ERROR_TYPE,
        key::LABLET_RUN_ERROR,
        key::LABLET_RESULT_TEXT,
        key::LABLET_RESULT_STRUCTURED,
    ] {
        assert_eq!(attributes.get(key), None, "{key}");
    }
    for key in [
        key::LABLET_TOOL_CALLS_TOTAL,
        key::LABLET_TOOL_CALLS_ERRORS,
        key::LABLET_TOOL_CALLS_UNKNOWN,
        key::LABLET_TOOL_CALLS_TRUNCATED,
        key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL,
        key::LABLET_TOOL_CALLS_INPUT_BYTES_TOTAL,
        key::LABLET_TOOL_CALLS_OUTPUT_BYTES_TOTAL,
        key::LABLET_PROVIDER_RETRIES,
        key::LABLET_TOOLS_COUNT,
        key::LABLET_SKILLS_COUNT,
        key::LABLET_TELEMETRY_DROPPED_RECORDS,
    ] {
        assert_eq!(attributes.get(key), Some(&Value::Int(0)), "{key}");
    }
    assert_eq!(
        attributes.get(key::LABLET_TOOLS_NAMES),
        Some(&texts_of(&[]))
    );
}

#[test]
fn a_count_a_failed_attempt_did_not_report_is_left_out_beside_the_two_it_always_has() {
    let mut summary = completed(&context(), two_counts());
    summary.failed_usage = Some(Usage {
        input_tokens: 800,
        cache_write_tokens: Some(0),
        ..Usage::default()
    });

    let attributes = wide(&context(), &summary);

    assert_eq!(
        attributes.get(key::LABLET_PROVIDER_FAILED_INPUT_TOKENS),
        Some(&Value::Int(800))
    );
    assert_eq!(
        attributes.get(key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS),
        Some(&Value::Int(0))
    );
    assert_eq!(
        attributes.get(key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS),
        None
    );
    assert_eq!(
        attributes.get(key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS),
        Some(&Value::Int(0))
    );
}

#[test]
fn a_run_that_did_not_complete_names_its_stop_reason_as_its_error_type() {
    let failed = wide(
        &context(),
        &stopped(&context(), StopReason::RetriesExhausted),
    );
    let stopped = wide(&context(), &stopped(&context(), StopReason::Cancelled));

    assert_eq!(
        failed.get(key::LABLET_RUN_STOP_REASON),
        Some(&text("retries_exhausted"))
    );
    assert_eq!(
        failed.get(key::ERROR_TYPE),
        Some(&text("retries_exhausted"))
    );
    assert_eq!(
        failed.get(key::LABLET_RUN_ERROR),
        Some(&text("529 overloaded"))
    );
    assert_eq!(stopped.get(key::ERROR_TYPE), Some(&text("cancelled")));
    assert_eq!(
        stopped.get(key::LABLET_RUN_ERROR),
        None,
        "no error ended a run that was cancelled"
    );
}

#[test]
fn the_result_is_held_only_by_the_wide_event_of_a_run_that_captures_content() {
    let kept = wide(&every_part_of_a_context(), &every_part_of_a_summary());
    let left_out = wide(
        &RunContext {
            capture_content: false,
            ..every_part_of_a_context()
        },
        &every_part_of_a_summary(),
    );

    assert_eq!(kept.get(key::LABLET_RESULT_TEXT), Some(&text("Forty-two.")));
    assert_eq!(left_out.get(key::LABLET_RESULT_TEXT), None);
    assert_eq!(left_out.get(key::LABLET_RESULT_STRUCTURED), None);
    assert_eq!(
        left_out.get(key::LABLET_RESULT_TEXT_BYTES),
        Some(&Value::Int(10))
    );
    assert_eq!(
        left_out.get(key::LABLET_RESULT_HAS_STRUCTURED),
        Some(&Value::Flag(true))
    );
}

#[test]
fn a_run_without_a_structured_result_holds_none_though_it_captures_content() {
    let context = RunContext {
        capture_content: true,
        ..context()
    };

    let attributes = wide(&context, &completed(&context, two_counts()));

    assert_eq!(
        attributes.get(key::LABLET_RESULT_TEXT),
        Some(&text("Done."))
    );
    assert_eq!(attributes.get(key::LABLET_RESULT_STRUCTURED), None);
    assert_eq!(
        attributes.get(key::LABLET_RESULT_HAS_STRUCTURED),
        Some(&Value::Flag(false))
    );
}

#[test]
fn a_tool_has_its_keys_only_when_the_run_offered_it_and_called_it() {
    let mut summary = every_part_of_a_summary();
    summary.tools = vec![tool_name("read_file"), tool_name("grep")];

    let attributes = wide(&context(), &summary);

    let per_tool: Vec<&str> = attributes
        .keys()
        .map(String::as_str)
        .filter(|key| key.starts_with("lablet.tool."))
        .collect();
    assert_eq!(
        per_tool,
        [
            "lablet.tool.calls.read_file",
            "lablet.tool.errors.read_file",
            "lablet.tool.latency_ms.read_file"
        ],
        "grep was offered and never called, and bash has a share and wasn't offered"
    );
}

#[test]
fn thinking_is_spelled_as_its_mode_and_the_tokens_of_a_budget() {
    let spelled = [
        Thinking::ProviderDefault,
        Thinking::Adaptive,
        Thinking::Budget(NonZeroU32::new(2_048).unwrap()),
        Thinking::Disabled,
    ]
    .map(thinking);

    assert_eq!(
        spelled,
        ["provider_default", "adaptive", "budget:2048", "disabled"]
    );
}

#[cfg(unix)]
#[test]
fn a_transcript_path_that_is_not_utf_8_is_written_with_what_is_not_replaced() {
    use std::os::unix::ffi::OsStringExt as _;

    let context = RunContext {
        transcript_path: Some(PathBuf::from(OsString::from_vec(
            b"runs/\xff/transcript.json".to_vec(),
        ))),
        ..context()
    };

    let attributes = wide(&context, &completed(&context, two_counts()));

    assert_eq!(
        attributes.get(key::LABLET_RUN_TRANSCRIPT_PATH),
        Some(&text("runs/\u{fffd}/transcript.json"))
    );
}

#[test]
fn the_model_named_is_the_one_the_run_called() {
    let attributes = wide(&context(), &completed(&context(), two_counts()));

    assert_eq!(
        attributes.get(key::GEN_AI_REQUEST_MODEL),
        Some(&text(MODEL))
    );
    assert_eq!(
        attributes.get(key::GEN_AI_PROVIDER_NAME),
        Some(&text("fake"))
    );
    assert_eq!(
        attributes.get(key::LABLET_REQUEST_API),
        Some(&text("script"))
    );
}
