use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use lablet_model::{
    CacheScope, CompletionMode, Cost, Endpoint, Latency, McpLifetime, McpServer, McpServers,
    ModelRef, OutcomeParts, PromptSizes, ProviderApi, ProviderTotals, Rates, RequestParams,
    RunContext, RunOutcome, RunSummary, TaskResult, ToolCallTotals, Usage,
};
use lablet_telemetry_registry::attribute as key;
use lablet_telemetry_registry::signals::{EVENT_LABLET_RUN_KEYS, EVENT_LABLET_RUN_REQUIRED};
use opentelemetry::trace::{SpanId, TraceId};
use serde_json::json;

use super::*;
use crate::testing::{
    CONFIG_DIGEST, MODEL, RUN, completed, context, every_parameter, labelled, run_id, stopped,
    text, tool_name, two_counts,
};

const TOOLS_DIGEST: &str = "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a";
const SYSTEM_DIGEST: &str = "dbc1b4c900ffe48d575b5da5c638040125f65db0fe3e24494b76ea986457d986";

fn closed(context: RunContext, summary: RunSummary) -> Closed {
    Closed {
        trace: TraceId::from(7_u128),
        root: SpanId::from(9_u64),
        at: UNIX_EPOCH + Duration::from_millis(1_790_000_012_345),
        context,
        summary,
    }
}

/// The attributes of the wide event of a run, of which nothing was lost.
fn wide(context: RunContext, summary: RunSummary) -> Attributes {
    wide_event(&closed(context, summary), 0).attributes
}

fn texts_of(texts: &[&str]) -> Held {
    Held::Texts(texts.iter().map(|text| (*text).to_owned()).collect())
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

/// Holds the keys of a wide event to the registry: no key is there twice,
/// every key the registry requires is there, and every key that's there is
/// one the registry declares, a template's for one of `tools`.
fn assert_declared(attributes: &Attributes, tools: &[&str]) {
    let keys = attributes.keys();
    let distinct: BTreeSet<&str> = keys.iter().copied().collect();
    assert_eq!(keys.len(), distinct.len(), "a key is there twice: {keys:?}");
    for required in EVENT_LABLET_RUN_REQUIRED {
        assert!(distinct.contains(required), "{required} is missing");
    }
    let per_tool: Vec<String> = Template::ALL
        .iter()
        .flat_map(|template| {
            tools
                .iter()
                .map(move |tool| format!("{}.{tool}", template.prefix()))
        })
        .collect();
    for key in distinct {
        assert!(
            Key::ALL.iter().any(|declared| declared.name() == key)
                || per_tool.iter().any(|declared| declared == key),
            "{key} isn't declared"
        );
    }
}

#[test]
fn the_keys_of_the_enums_are_the_keys_the_registry_lists_for_the_event() {
    let listed: BTreeSet<&str> = EVENT_LABLET_RUN_KEYS.iter().copied().collect();
    let plain: Vec<&str> = Key::ALL.iter().map(|key| key.name()).collect();
    let templates: Vec<&str> = Template::ALL
        .iter()
        .map(|template| template.prefix())
        .collect();

    let of_the_enums: BTreeSet<&str> = plain.iter().chain(&templates).copied().collect();
    assert_eq!(of_the_enums, listed);
    assert_eq!(plain.len() + templates.len(), listed.len());
    assert_eq!(
        templates,
        [
            key::LABLET_TOOL_CALLS,
            key::LABLET_TOOL_ERRORS,
            key::LABLET_TOOL_LATENCY_MS
        ]
    );
}

#[test]
fn the_wide_event_is_an_info_record_in_the_context_of_the_root_span_at_the_run_s_end() {
    let closed = closed(context(), completed(&context(), two_counts(), None));

    let event = wide_event(&closed, 0);

    assert_eq!(event.name, "lablet.run");
    assert_eq!(event.severity, Severity::Info);
    assert_eq!(event.span, closed.root);
    assert_eq!(event.at, closed.at);
}

/// What the wide event of [`every_part_of_a_context`] and
/// [`every_part_of_a_summary`] holds to say which run it's of and what the
/// run was set up with, when 901 records were lost.
fn held_of_the_setup() -> Vec<(&'static str, Held)> {
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
        (key::SERVER_PORT, Held::Int(443)),
        (key::GEN_AI_REQUEST_MAX_TOKENS, Held::Int(4_096)),
        (key::GEN_AI_REQUEST_SEED, Held::Int(-42)),
        (key::GEN_AI_REQUEST_REASONING_LEVEL, text("high")),
        (key::GEN_AI_REQUEST_TEMPERATURE, Held::Float(0.7)),
        (key::LABLET_REQUEST_THINKING, text("adaptive")),
        (key::LABLET_REQUEST_API, text("messages")),
        (key::LABLET_REQUEST_REASONING_REPLAYED, Held::Flag(true)),
        (key::LABLET_REQUEST_CACHE_SCOPE, text("run")),
        (key::LABLET_RUN_COMPLETION_MODE, text("explicit")),
        (key::LABLET_RUN_MAX_TURNS, Held::Int(30)),
        (key::LABLET_RUN_TIMEOUT_MS, Held::Int(600_000)),
        (
            key::LABLET_TOOLS_NAMES,
            texts_of(&["bash", "read_file", "task_complete"]),
        ),
        (key::LABLET_TOOLS_COUNT, Held::Int(3)),
        (key::LABLET_MCP_SERVERS, texts_of(&["docs", "search"])),
        (
            key::LABLET_MCP_SERVER_VERSIONS,
            texts_of(&["1.4.0", "0.9.2"]),
        ),
        (key::LABLET_MCP_LIFETIME, text("lablet")),
        (key::LABLET_PROMPT_SYSTEM_BYTES, Held::Int(101)),
        (key::LABLET_PROMPT_USER_BYTES, Held::Int(102)),
        (key::LABLET_PROMPT_TOOLS_BYTES, Held::Int(103)),
        (key::LABLET_SKILLS_COUNT, Held::Int(801)),
    ]
}

/// What the same wide event holds to say what came of the run.
fn held_of_the_outcome() -> Vec<(&'static str, Held)> {
    vec![
        (key::LABLET_RUN_STOP_REASON, text("completed")),
        (key::LABLET_RUN_DURATION_MS, Held::Int(603)),
        (key::LABLET_RUN_TURNS, Held::Int(601)),
        (key::LABLET_RESULT_TEXT_BYTES, Held::Int(10)),
        (key::LABLET_RESULT_HAS_STRUCTURED, Held::Flag(true)),
        (
            key::LABLET_RUN_TRANSCRIPT_PATH,
            text("runs/01K5F3Z8/transcript.json"),
        ),
        (key::LABLET_TELEMETRY_DROPPED_RECORDS, Held::Int(901)),
        (key::LABLET_RESULT_TEXT, text("Forty-two.")),
        (key::LABLET_RESULT_STRUCTURED, text(r#"{"answer":42}"#)),
        (key::LABLET_PROVIDER_RETRIES, Held::Int(301)),
        (key::LABLET_PROVIDER_LATENCY_MS_TOTAL, Held::Int(605)),
        (key::LABLET_PROVIDER_LATENCY_MS_MAX, Held::Int(303)),
        (key::GEN_AI_USAGE_INPUT_TOKENS, Held::Int(701)),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, Held::Int(702)),
        (key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS, Held::Int(703)),
        (key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS, Held::Int(704)),
        (key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS, Held::Int(705)),
        (key::LABLET_PROVIDER_FAILED_INPUT_TOKENS, Held::Int(201)),
        (key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS, Held::Int(202)),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
            Held::Int(204),
        ),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
            Held::Int(205),
        ),
        (
            key::GEN_AI_RESPONSE_FINISH_REASONS,
            texts_of(&["tool_use", "end_turn"]),
        ),
        (key::LABLET_RUN_COST_USD, Held::Float(0.0421)),
        (key::LABLET_PRICING_INPUT_USD_PER_MTOK, Held::Float(3.0)),
        (key::LABLET_PRICING_OUTPUT_USD_PER_MTOK, Held::Float(15.0)),
        (
            key::LABLET_PRICING_CACHE_READ_USD_PER_MTOK,
            Held::Float(0.3),
        ),
        (
            key::LABLET_PRICING_CACHE_WRITE_USD_PER_MTOK,
            Held::Float(3.75),
        ),
        (key::LABLET_TOOL_CALLS_TOTAL, Held::Int(602)),
        (key::LABLET_TOOL_CALLS_ERRORS, Held::Int(401)),
        (key::LABLET_TOOL_CALLS_UNKNOWN, Held::Int(402)),
        (key::LABLET_TOOL_CALLS_TRUNCATED, Held::Int(403)),
        (key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL, Held::Int(404)),
        (key::LABLET_TOOL_CALLS_INPUT_BYTES_TOTAL, Held::Int(405)),
        (key::LABLET_TOOL_CALLS_OUTPUT_BYTES_TOTAL, Held::Int(406)),
        ("lablet.tool.calls.bash", Held::Int(501)),
        ("lablet.tool.errors.bash", Held::Int(502)),
        ("lablet.tool.latency_ms.bash", Held::Int(503)),
        ("lablet.tool.calls.read_file", Held::Int(511)),
        ("lablet.tool.errors.read_file", Held::Int(512)),
        ("lablet.tool.latency_ms.read_file", Held::Int(513)),
    ]
}

#[test]
fn every_key_holds_what_the_run_s_context_and_its_summary_hold_for_it() {
    let attributes = wide_event(
        &closed(every_part_of_a_context(), every_part_of_a_summary()),
        901,
    )
    .attributes;

    let expected = [held_of_the_setup(), held_of_the_outcome()].concat();
    let held: BTreeMap<&str, Held> = attributes
        .pairs()
        .into_iter()
        .map(|(key, held)| (key, held.clone()))
        .collect();
    assert_eq!(held, expected.iter().cloned().collect());
    assert_eq!(attributes.keys().len(), expected.len());
    assert_declared(&attributes, &["bash", "read_file"]);
    // Every key a run that completed may hold is there, so none was left
    // without a value to be held to.
    assert_eq!(
        expected.len(),
        EVENT_LABLET_RUN_KEYS.len() - 2 - 3 + 6,
        "every key but the two of a run that didn't complete, and each template twice"
    );
    assert_eq!(attributes.held(key::ERROR_TYPE), None);
    assert_eq!(attributes.held(key::LABLET_RUN_ERROR), None);
}

#[test]
fn the_keys_are_in_the_order_of_the_registry_and_then_each_tool_s_in_the_order_offered() {
    let attributes = wide(every_part_of_a_context(), every_part_of_a_summary());

    let keys = attributes.keys();
    let (plain, per_tool) = keys.split_at(keys.len() - 6);
    assert!(plain.is_sorted(), "{plain:?}");
    assert_eq!(
        per_tool,
        [
            "lablet.tool.calls.bash",
            "lablet.tool.errors.bash",
            "lablet.tool.latency_ms.bash",
            "lablet.tool.calls.read_file",
            "lablet.tool.errors.read_file",
            "lablet.tool.latency_ms.read_file",
        ]
    );
}

#[test]
fn a_run_with_nothing_it_may_be_without_holds_the_keys_the_registry_requires_and_no_other() {
    let mut summary = completed(&context(), two_counts(), None);
    summary.max_turns = None;

    let attributes = wide(context(), summary);

    assert_declared(&attributes, &[]);
    let mut keys = attributes.keys();
    keys.sort_unstable();
    assert_eq!(keys, EVENT_LABLET_RUN_REQUIRED);
}

#[test]
fn a_key_a_run_has_nothing_for_is_left_out_and_a_count_of_nothing_is_written() {
    let mut summary = completed(&context(), two_counts(), None);
    summary.max_turns = None;
    summary.tools = Vec::new();

    let attributes = wide(context(), summary);

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
        assert_eq!(attributes.held(key), None, "{key}");
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
        assert_eq!(attributes.held(key), Some(&Held::Int(0)), "{key}");
    }
    assert_eq!(
        attributes.held(key::LABLET_TOOLS_NAMES),
        Some(&texts_of(&[]))
    );
}

#[test]
fn a_count_a_failed_attempt_did_not_report_is_left_out_beside_the_two_it_always_has() {
    let mut summary = completed(&context(), two_counts(), None);
    summary.failed_usage = Some(Usage {
        input_tokens: 800,
        cache_write_tokens: Some(0),
        ..Usage::default()
    });

    let attributes = wide(context(), summary);

    assert_eq!(
        attributes.held(key::LABLET_PROVIDER_FAILED_INPUT_TOKENS),
        Some(&Held::Int(800))
    );
    assert_eq!(
        attributes.held(key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS),
        Some(&Held::Int(0))
    );
    assert_eq!(
        attributes.held(key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS),
        None
    );
    assert_eq!(
        attributes.held(key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS),
        Some(&Held::Int(0))
    );
}

#[test]
fn a_run_that_did_not_complete_names_its_stop_reason_as_its_error_type() {
    let failed = wide(context(), stopped(&context(), StopReason::RetriesExhausted));
    let stopped = wide(context(), stopped(&context(), StopReason::Cancelled));

    assert_eq!(
        failed.held(key::LABLET_RUN_STOP_REASON),
        Some(&text("retries_exhausted"))
    );
    assert_eq!(
        failed.held(key::ERROR_TYPE),
        Some(&text("retries_exhausted"))
    );
    assert_eq!(
        failed.held(key::LABLET_RUN_ERROR),
        Some(&text("529 overloaded"))
    );
    assert_eq!(stopped.held(key::ERROR_TYPE), Some(&text("cancelled")));
    assert_eq!(
        stopped.held(key::LABLET_RUN_ERROR),
        None,
        "no error ended a run that was cancelled"
    );
    assert_declared(&failed, &[]);
    assert_declared(&stopped, &[]);
}

#[test]
fn the_result_is_held_only_by_the_wide_event_of_a_run_that_captures_content() {
    let kept = wide(every_part_of_a_context(), every_part_of_a_summary());
    let left_out = wide(
        RunContext {
            capture_content: false,
            ..every_part_of_a_context()
        },
        every_part_of_a_summary(),
    );

    assert_eq!(
        kept.held(key::LABLET_RESULT_TEXT),
        Some(&text("Forty-two."))
    );
    assert_eq!(left_out.held(key::LABLET_RESULT_TEXT), None);
    assert_eq!(left_out.held(key::LABLET_RESULT_STRUCTURED), None);
    assert_eq!(
        left_out.held(key::LABLET_RESULT_TEXT_BYTES),
        Some(&Held::Int(10))
    );
    assert_eq!(
        left_out.held(key::LABLET_RESULT_HAS_STRUCTURED),
        Some(&Held::Flag(true))
    );
}

#[test]
fn a_run_without_a_structured_result_holds_none_though_it_captures_content() {
    let context = RunContext {
        capture_content: true,
        ..context()
    };

    let attributes = wide(context.clone(), completed(&context, two_counts(), None));

    assert_eq!(
        attributes.held(key::LABLET_RESULT_TEXT),
        Some(&text("Done."))
    );
    assert_eq!(attributes.held(key::LABLET_RESULT_STRUCTURED), None);
    assert_eq!(
        attributes.held(key::LABLET_RESULT_HAS_STRUCTURED),
        Some(&Held::Flag(false))
    );
}

#[test]
fn a_tool_has_its_keys_only_when_the_run_offered_it_and_called_it() {
    let mut summary = every_part_of_a_summary();
    summary.tools = vec![tool_name("read_file"), tool_name("grep")];

    let attributes = wide(context(), summary);

    let per_tool: Vec<&str> = attributes
        .keys()
        .into_iter()
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
    assert_declared(&attributes, &["read_file"]);
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

    let attributes = wide(context.clone(), completed(&context, two_counts(), None));

    assert_eq!(
        attributes.held(key::LABLET_RUN_TRANSCRIPT_PATH),
        Some(&text("runs/\u{fffd}/transcript.json"))
    );
}

#[test]
fn the_model_named_is_the_one_the_run_called() {
    let attributes = wide(context(), completed(&context(), two_counts(), None));

    assert_eq!(
        attributes.held(key::GEN_AI_REQUEST_MODEL),
        Some(&text(MODEL))
    );
    assert_eq!(
        attributes.held(key::GEN_AI_PROVIDER_NAME),
        Some(&text("fake"))
    );
    assert_eq!(
        attributes.held(key::LABLET_REQUEST_API),
        Some(&text("script"))
    );
}
