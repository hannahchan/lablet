//! The wide event of a run the loop made, read back from the file: O11,
//! O12, O14 and O15, and the clauses of O1 and O2 that are of it.

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::path::PathBuf;

use lablet_conformance::observer::{
    assert_the_wide_event_is_declared, assert_the_wide_event_sums_its_steps,
};
use lablet_conformance::otlp::Attributes;
use lablet_model::{
    CacheScope, Effort, McpLifetime, McpServer, McpServers, Rates, RequestParams, RunLabels,
    StopReason, Thinking,
};
use lablet_policy::Pricing;
use lablet_telemetry_registry::attribute as key;
use lablet_telemetry_registry::signals::{
    EVENT_LABLET_RUN_KEYS, EVENT_LABLET_RUN_REQUIRED, EventLabletRunKey,
};
use serde_json::{Value, json};

use crate::harness::{
    BASH_SAYS, CONFIG_DIGEST, FAILS_CALLS_ENDS, MODEL, PROMPT, READ_FILE_SAYS, RUN, SYSTEM,
    Settings, Traced, VERSION, count, traced,
};

/// A run of one response, from a provider that reports two counts.
const ENDS: &str = r"
- response:
    content: [{ text: Done. }]
    usage: { input_tokens: 1200, output_tokens: 30 }
    finish: end_turn
    latency: 120ms
";

/// A run of two calls, from a provider that reports every count.
const REPORTS_EVERY_COUNT: &str = r"
- error:
    kind: retryable
    message: 529 overloaded
    usage: { input_tokens: 800, output_tokens: 3, cache_read_tokens: 600, cache_write_tokens: 50 }
    latency: 40ms
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: cargo test --quiet } } }
    usage:
      input_tokens: 1000
      output_tokens: 50
      reasoning_output_tokens: 20
      cache_read_tokens: 200
      cache_write_tokens: 90
    finish: tool_use
    latency: 250ms
- response:
    content: [{ text: Done. }]
    usage: { input_tokens: 1200, output_tokens: 30 }
    finish: end_turn
    latency: 120ms
";

fn keys(attributes: &Attributes) -> BTreeSet<&str> {
    attributes.keys().map(String::as_str).collect()
}

fn labelled() -> RunLabels {
    RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: Some("terse-tool-descriptions".to_owned()),
        trial: Some("3".to_owned()),
    }
}

fn docs_and_search() -> McpServers {
    let server = |name: &str, version: &str| McpServer {
        name: name.to_owned(),
        version: version.to_owned(),
    };
    McpServers::new(
        McpLifetime::Lablet,
        vec![server("docs", "1.4.0"), server("search", "0.9.2")],
    )
    .unwrap()
}

/// Everything a run may be without.
fn everything(settings: &mut Settings) {
    settings.capture_content = true;
    settings.labels = labelled();
    settings.request = RequestParams {
        max_tokens: 4_096,
        temperature: Some(0.7),
        thinking: Thinking::Budget(NonZeroU32::new(2_048).unwrap()),
        effort: Some(Effort::High),
        seed: Some(-42),
        cache_scope: CacheScope::Run,
    };
    settings.max_turns = NonZeroU32::new(30);
    settings.pricing = Some(Pricing::new(Rates::new(3.0, 15.0, 0.3, 3.75).unwrap()));
    settings.transcript_path = Some(PathBuf::from("runs/01K5F3Z8/transcript.json"));
    settings.skills_count = 2;
    settings.mcp = Some(docs_and_search());
}

fn assert_holds(wide: &Attributes, expected: &[(&str, Value)]) {
    for (key, held) in expected {
        assert_eq!(wide.get(*key), Some(held), "{key}");
    }
}

fn assert_lacks(wide: &Attributes, keys: &[&str]) {
    for key in keys {
        assert_eq!(wide.get(*key), None, "{key}");
    }
}

// What a run may be without

#[tokio::test(start_paused = true)]
async fn a_run_with_nothing_it_may_be_without_has_the_keys_the_registry_requires_and_no_other() {
    let traced = traced("wide-least", ENDS, |_| {}).await;

    let wide = traced.wide();
    assert_the_wide_event_is_declared(wide);
    assert_eq!(
        keys(&wide.attributes),
        EVENT_LABLET_RUN_REQUIRED.iter().copied().collect()
    );
    assert_lacks(
        &wide.attributes,
        &[
            key::LABLET_RUN_MAX_TURNS,
            key::LABLET_RUN_COST_USD,
            key::LABLET_PRICING_INPUT_USD_PER_MTOK,
            key::LABLET_PRICING_OUTPUT_USD_PER_MTOK,
            key::LABLET_PRICING_CACHE_READ_USD_PER_MTOK,
            key::LABLET_PRICING_CACHE_WRITE_USD_PER_MTOK,
            key::LABLET_MCP_SERVERS,
            key::LABLET_MCP_SERVER_VERSIONS,
            key::LABLET_MCP_LIFETIME,
            key::LABLET_RUN_TRANSCRIPT_PATH,
            key::LABLET_PROVIDER_FAILED_INPUT_TOKENS,
            key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS,
            key::ERROR_TYPE,
            key::LABLET_RUN_ERROR,
            key::LABLET_RESULT_TEXT,
        ],
    );
    assert_holds(
        &wide.attributes,
        &[
            (key::LABLET_TOOL_CALLS_TOTAL, json!(0)),
            (key::LABLET_PROVIDER_RETRIES, json!(0)),
            (key::LABLET_SKILLS_COUNT, json!(0)),
            (key::GEN_AI_RESPONSE_FINISH_REASONS, json!(["end_turn"])),
        ],
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_with_everything_has_every_key_a_run_that_completed_over_no_network_may_hold() {
    let traced = traced("wide-most", REPORTS_EVERY_COUNT, everything).await;

    let wide = traced.wide();
    assert_the_wide_event_is_declared(wide);
    let absent: Vec<_> = EventLabletRunKey::ALL
        .iter()
        .map(|key| key.name())
        .filter(|key| !wide.attributes.contains_key(*key))
        .collect();
    assert_eq!(
        absent,
        [
            key::ERROR_TYPE,
            key::LABLET_RESULT_STRUCTURED,
            key::LABLET_RUN_ERROR,
            key::SERVER_ADDRESS,
            key::SERVER_PORT,
        ],
        "the run completed, in natural mode, and the script is reached over no network"
    );
    let per_tool: Vec<_> = wide
        .attributes
        .keys()
        .filter(|key| !EVENT_LABLET_RUN_KEYS.contains(&key.as_str()))
        .collect();
    assert_eq!(
        per_tool,
        [
            "lablet.tool.calls.bash",
            "lablet.tool.errors.bash",
            "lablet.tool.latency_ms.bash"
        ]
    );
}

// What the wide event holds

#[tokio::test(start_paused = true)]
async fn the_wide_event_says_what_the_run_was_set_up_with() {
    let traced = traced("wide-setup", REPORTS_EVERY_COUNT, everything).await;

    assert_holds(
        &traced.wide().attributes,
        &[
            (key::GEN_AI_CONVERSATION_ID, json!(RUN)),
            (key::SESSION_ID, json!(RUN)),
            (key::GEN_AI_AGENT_NAME, json!("lablet")),
            (key::GEN_AI_AGENT_VERSION, json!(VERSION)),
            (key::LABLET_CONFIG_DIGEST, json!(CONFIG_DIGEST)),
            (key::GEN_AI_PROVIDER_NAME, json!("fake")),
            (key::GEN_AI_REQUEST_MODEL, json!(MODEL)),
            (key::GEN_AI_REQUEST_MAX_TOKENS, json!(4_096)),
            (key::GEN_AI_REQUEST_SEED, json!(-42)),
            (key::GEN_AI_REQUEST_REASONING_LEVEL, json!("high")),
            (key::GEN_AI_REQUEST_TEMPERATURE, json!(0.7)),
            (key::LABLET_REQUEST_THINKING, json!("budget:2048")),
            (key::LABLET_RUN_COMPLETION_MODE, json!("natural")),
            (key::LABLET_RUN_MAX_TURNS, json!(30)),
            (key::LABLET_RUN_TIMEOUT_MS, json!(3_600_000)),
            (
                key::LABLET_TOOLS_NAMES,
                json!(["bash", "read_file", "write"]),
            ),
            (key::LABLET_TOOLS_COUNT, json!(3)),
            (key::LABLET_PROMPT_SYSTEM_BYTES, json!(SYSTEM.len())),
            (key::LABLET_PROMPT_USER_BYTES, json!(PROMPT.len())),
            (key::LABLET_SKILLS_COUNT, json!(2)),
            (
                key::LABLET_RUN_TRANSCRIPT_PATH,
                json!("runs/01K5F3Z8/transcript.json"),
            ),
        ],
    );
    assert_ne!(SYSTEM.len(), PROMPT.len());
}

#[tokio::test(start_paused = true)]
async fn the_wide_event_says_what_came_of_the_run() {
    let traced = traced("wide-outcome", FAILS_CALLS_ENDS, everything).await;

    let wide = &traced.wide().attributes;
    assert_the_wide_event_sums_its_steps(&traced.exported, RUN);
    let said_of_the_unknown_call =
        traced.finished.transcript.turns()[1].tool_calls()[0].output_bytes();
    let text = "The parser test passes now.";
    assert_holds(
        wide,
        &[
            (key::LABLET_RUN_STOP_REASON, json!("completed")),
            (key::LABLET_RUN_DURATION_MS, json!(3_810)),
            (key::LABLET_RUN_TURNS, json!(3)),
            (key::LABLET_RESULT_TEXT_BYTES, json!(text.len())),
            (key::LABLET_RESULT_TEXT, json!(text)),
            (key::LABLET_RESULT_HAS_STRUCTURED, json!(false)),
            (key::LABLET_TELEMETRY_DROPPED_RECORDS, json!(0)),
            (key::LABLET_PROVIDER_RETRIES, json!(1)),
            (key::LABLET_PROVIDER_LATENCY_MS_TOTAL, json!(510)),
            (key::LABLET_PROVIDER_LATENCY_MS_MAX, json!(250)),
            (key::GEN_AI_USAGE_INPUT_TOKENS, json!(3_300)),
            (key::GEN_AI_USAGE_OUTPUT_TOKENS, json!(100)),
            (key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS, json!(5)),
            (key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS, json!(200)),
            (key::LABLET_PROVIDER_FAILED_INPUT_TOKENS, json!(800)),
            (key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS, json!(0)),
            (
                key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
                json!(600),
            ),
            (
                key::GEN_AI_RESPONSE_FINISH_REASONS,
                json!(["tool_use", "tool_use", "end_turn"]),
            ),
            (key::LABLET_TOOL_CALLS_TOTAL, json!(3)),
            (key::LABLET_TOOL_CALLS_ERRORS, json!(1)),
            (key::LABLET_TOOL_CALLS_UNKNOWN, json!(1)),
            (key::LABLET_TOOL_CALLS_TRUNCATED, json!(0)),
            (key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL, json!(1_300)),
            (
                key::LABLET_TOOL_CALLS_INPUT_BYTES_TOTAL,
                json!(
                    r#"{"command":"cargo test --quiet"}"#.len()
                        + r#"{"path":"src/parser.rs"}"#.len()
                        + "{}".len()
                ),
            ),
            (
                key::LABLET_TOOL_CALLS_OUTPUT_BYTES_TOTAL,
                json!((BASH_SAYS.len() + READ_FILE_SAYS.len()) as u64 + said_of_the_unknown_call),
            ),
            ("lablet.tool.calls.bash", json!(1)),
            ("lablet.tool.errors.bash", json!(0)),
            ("lablet.tool.latency_ms.bash", json!(1_000)),
            ("lablet.tool.calls.read_file", json!(1)),
            ("lablet.tool.errors.read_file", json!(0)),
            ("lablet.tool.latency_ms.read_file", json!(300)),
        ],
    );
    assert_lacks(
        wide,
        &[
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
            key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
            "lablet.tool.calls.no_such_tool",
            "lablet.tool.calls.write",
        ],
    );
    assert!(said_of_the_unknown_call > 0);
}

#[tokio::test(start_paused = true)]
async fn the_wide_event_of_a_run_that_failed_names_the_reason_and_the_error() {
    let traced = traced(
        "wide-failed",
        "- error: { kind: fatal, message: unknown model }",
        |_| {},
    )
    .await;

    let wide = traced.wide();
    assert_eq!(
        traced.finished.summary.outcome.stop_reason(),
        StopReason::ProviderError
    );
    assert_the_wide_event_is_declared(wide);
    assert_the_wide_event_sums_its_steps(&traced.exported, RUN);
    assert_holds(
        &wide.attributes,
        &[
            (key::LABLET_RUN_STOP_REASON, json!("provider_error")),
            (key::ERROR_TYPE, json!("provider_error")),
            (key::LABLET_RUN_ERROR, json!("unknown model")),
            (key::LABLET_RUN_TURNS, json!(0)),
            (key::LABLET_PROVIDER_RETRIES, json!(0)),
            (key::GEN_AI_USAGE_INPUT_TOKENS, json!(0)),
            (key::GEN_AI_RESPONSE_FINISH_REASONS, json!([])),
            (key::LABLET_RESULT_TEXT_BYTES, json!(0)),
        ],
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_that_is_priced_reports_its_cost_beside_the_rates_it_was_priced_at() {
    let traced = traced("wide-priced", REPORTS_EVERY_COUNT, everything).await;

    // What the failed attempt reported is priced with what the turns used.
    let uncached = (2_200 - 200 - 90) + (800 - 600 - 50);
    let cost = (f64::from(uncached) * 3.0
        + f64::from(50 + 30 + 3) * 15.0
        + f64::from(200 + 600) * 0.3
        + f64::from(90 + 50) * 3.75)
        / 1_000_000.0;
    assert_holds(
        &traced.wide().attributes,
        &[
            (key::LABLET_RUN_COST_USD, json!(cost)),
            (key::LABLET_PRICING_INPUT_USD_PER_MTOK, json!(3.0)),
            (key::LABLET_PRICING_OUTPUT_USD_PER_MTOK, json!(15.0)),
            (key::LABLET_PRICING_CACHE_READ_USD_PER_MTOK, json!(0.3)),
            (key::LABLET_PRICING_CACHE_WRITE_USD_PER_MTOK, json!(3.75)),
        ],
    );
}

// O4, as far as it's of the wide event

#[tokio::test(start_paused = true)]
async fn the_result_is_on_the_wide_event_only_when_the_run_captures_content() {
    let kept = traced("wide-captured", ENDS, |settings| {
        settings.capture_content = true;
    })
    .await;
    let left_out = traced("wide-not-captured", ENDS, |_| {}).await;

    assert_eq!(kept.wide().attributes[key::LABLET_RESULT_TEXT], "Done.");
    assert_lacks(
        &left_out.wide().attributes,
        &[key::LABLET_RESULT_TEXT, key::LABLET_RESULT_STRUCTURED],
    );
    assert_eq!(
        count(&left_out.wide().attributes, key::LABLET_RESULT_TEXT_BYTES),
        5
    );
    assert!(!left_out.written.contains("Done."));
}

// O11

#[tokio::test(start_paused = true)]
async fn the_labels_a_request_named_are_on_the_wide_event_and_in_the_outcome() {
    let traced = traced("o11-wide", ENDS, |settings| settings.labels = labelled()).await;

    assert_holds(
        &traced.wide().attributes,
        &[
            (key::LABLET_TASK_ID, json!("fix-failing-test")),
            (key::LABLET_EXPERIMENT_ID, json!("terse-tool-descriptions")),
            (key::LABLET_TRIAL, json!("3")),
        ],
    );
    assert_eq!(traced.finished.summary.outcome.labels, labelled());
}

#[tokio::test(start_paused = true)]
async fn a_request_that_named_one_label_leaves_the_others_off_the_wide_event() {
    let traced = traced("o11-wide-one", ENDS, |settings| {
        settings.labels.experiment = Some("terse-tool-descriptions".to_owned());
    })
    .await;

    let wide = &traced.wide().attributes;
    assert_eq!(wide[key::LABLET_EXPERIMENT_ID], "terse-tool-descriptions");
    assert_lacks(wide, &[key::LABLET_TASK_ID, key::LABLET_TRIAL]);
    assert_eq!(
        traced.finished.summary.outcome.labels,
        RunLabels {
            experiment: Some("terse-tool-descriptions".to_owned()),
            ..RunLabels::default()
        }
    );
}

// O12

fn digests(traced: &Traced) -> (&str, &str) {
    let wide = &traced.wide().attributes;
    (
        wide[key::LABLET_TOOLS_DIGEST].as_str().unwrap(),
        wide[key::LABLET_PROMPT_SYSTEM_DIGEST].as_str().unwrap(),
    )
}

fn is_a_sha_256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .chars()
            .all(|digit| digit.is_ascii_digit() || ('a'..='f').contains(&digit))
}

#[tokio::test(start_paused = true)]
async fn two_tool_sets_that_differ_in_one_description_differ_in_their_digest() {
    let first = traced("o12-tools-first", ENDS, |_| {}).await;
    let again = traced("o12-tools-again", ENDS, |_| {}).await;
    let changed = traced("o12-tools-changed", ENDS, |settings| {
        settings.bash_does = "Runs a command in a shell.".to_owned();
    })
    .await;

    let (tools, system) = digests(&first);
    assert!(is_a_sha_256(tools), "{tools}");
    assert_eq!(digests(&again), (tools, system));
    assert_ne!(digests(&changed).0, tools);
    assert_eq!(digests(&changed).1, system);
    assert_ne!(tools, system);
    assert!(
        count(&changed.wide().attributes, key::LABLET_PROMPT_TOOLS_BYTES)
            > count(&first.wide().attributes, key::LABLET_PROMPT_TOOLS_BYTES)
    );
}

#[tokio::test(start_paused = true)]
async fn two_system_prompts_differ_in_their_digest() {
    let first = traced("o12-system-first", ENDS, |_| {}).await;
    let again = traced("o12-system-again", ENDS, |_| {}).await;
    let changed = traced("o12-system-changed", ENDS, |settings| {
        settings.system = "You fix tests, at length.".to_owned();
    })
    .await;

    let (tools, system) = digests(&first);
    assert!(is_a_sha_256(system), "{system}");
    assert_eq!(digests(&again), (tools, system));
    assert_ne!(digests(&changed).1, system);
    assert_eq!(digests(&changed).0, tools);
}

// O14

#[tokio::test(start_paused = true)]
async fn a_count_the_provider_did_not_report_is_not_on_the_wide_event() {
    let traced = traced("o14-wide", ENDS, |_| {}).await;

    let wide = &traced.wide().attributes;
    assert_lacks(
        wide,
        &[
            key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
        ],
    );
    assert_eq!(count(wide, key::GEN_AI_USAGE_INPUT_TOKENS), 1_200);
    assert_eq!(count(wide, key::GEN_AI_USAGE_OUTPUT_TOKENS), 30);
    let usage = traced.finished.summary.outcome.usage;
    assert_eq!(
        (
            usage.reasoning_output_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens
        ),
        (None, None, None)
    );
}

#[tokio::test(start_paused = true)]
async fn a_count_of_zero_that_the_provider_reported_is_on_the_wide_event() {
    let traced = traced(
        "o14-wide-zero",
        r"
- error: { kind: retryable, usage: { input_tokens: 0, cache_write_tokens: 0 } }
- response:
    content: [{ text: Done. }]
    usage: { input_tokens: 1200, output_tokens: 30, cache_read_tokens: 0 }
    finish: end_turn
",
        |_| {},
    )
    .await;

    let wide = &traced.wide().attributes;
    assert_holds(
        wide,
        &[
            (key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS, json!(0)),
            (key::LABLET_PROVIDER_FAILED_INPUT_TOKENS, json!(0)),
            (key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS, json!(0)),
            (
                key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
                json!(0),
            ),
        ],
    );
    assert_lacks(
        wide,
        &[
            key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
            key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
        ],
    );
}

// O15

#[tokio::test(start_paused = true)]
async fn the_wide_event_says_how_the_model_was_reached() {
    let shared = traced("o15-shared", ENDS, |_| {}).await;
    let apart = traced("o15-run", ENDS, |settings| {
        settings.request.cache_scope = CacheScope::Run;
    })
    .await;

    assert_holds(
        &shared.wide().attributes,
        &[
            (key::LABLET_REQUEST_API, json!("script")),
            (key::LABLET_REQUEST_REASONING_REPLAYED, json!(false)),
            (key::LABLET_REQUEST_CACHE_SCOPE, json!("shared")),
            (key::LABLET_REQUEST_THINKING, json!("provider_default")),
        ],
    );
    assert_eq!(
        apart.wide().attributes[key::LABLET_REQUEST_CACHE_SCOPE],
        "run"
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_with_mcp_servers_names_them_with_the_version_each_gave_and_their_lifetime() {
    let served = traced("o15-mcp", ENDS, |settings| {
        settings.mcp = Some(docs_and_search());
    })
    .await;
    let alone = traced("o15-no-mcp", ENDS, |_| {}).await;

    assert_holds(
        &served.wide().attributes,
        &[
            (key::LABLET_MCP_SERVERS, json!(["docs", "search"])),
            (key::LABLET_MCP_SERVER_VERSIONS, json!(["1.4.0", "0.9.2"])),
            (key::LABLET_MCP_LIFETIME, json!("lablet")),
        ],
    );
    assert_lacks(
        &alone.wide().attributes,
        &[
            key::LABLET_MCP_SERVERS,
            key::LABLET_MCP_SERVER_VERSIONS,
            key::LABLET_MCP_LIFETIME,
        ],
    );
}
