//! The smoke test: a scripted provider, the built-in tools and the file
//! exporter, through `build` and `run`, and the spans and records the run
//! left in its file.

use lablet::{FinishedRun, RunId, StopReason};
use lablet_conformance::otlp::{Attributes, Exported, LogRecord, SpanKind, Status};
use lablet_telemetry_registry::attribute as key;
use lablet_telemetry_registry::signals::{
    EVENT_LABLET_RUN_KEYS, EVENT_LABLET_RUN_REQUIRED, EventLabletRunTemplate,
    SPAN_LABLET_CHAT_KEYS, SPAN_LABLET_CHAT_REQUIRED, SPAN_LABLET_EXECUTE_TOOL_KEYS,
    SPAN_LABLET_EXECUTE_TOOL_REQUIRED, SPAN_LABLET_INVOKE_AGENT_KEYS,
    SPAN_LABLET_INVOKE_AGENT_REQUIRED,
};
use serde_json::json;

use crate::harness::{Lab, MODEL, Traced, request};

const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";

/// A failed attempt, a response that writes a file and reads it back, and
/// a response that ends the run.
const WRITES_READS_ENDS: &str = "
- error: { kind: retryable, message: 529 overloaded, usage: { input_tokens: 40 } }
- response:
    content:
      - text: I'll note the fix and read it back.
      - tool_use: { id: call_1, name: write_file, input: { json: { path: notes/fix.md, content: Off by one. } } }
      - tool_use: { id: call_2, name: read_file, input: { json: { path: notes/fix.md } } }
    usage: { input_tokens: 1000, output_tokens: 50, cache_read_tokens: 200 }
    finish: tool_use
    response_id: msg_01
    response_model: scripted-2026-09
- response:
    content:
      - tool_use: { id: call_3, name: bash, input: { json: { command: cat notes/fix.md; exit 3 } } }
    usage: { input_tokens: 1100, output_tokens: 20 }
    finish: tool_use
- response:
    content:
      - text: The parser was off by one.
    usage: { input_tokens: 1200, output_tokens: 30 }
    finish: end_turn
";

/// Holds `attributes` to the registry's lists for their signal: every key
/// the signal always carries is there, and none is there that the signal
/// doesn't declare, by name or as one of `templates` with a tool's name
/// after it.
fn assert_declared(
    signal: &str,
    attributes: &Attributes,
    required: &[&str],
    declared: &[&str],
    templates: &[EventLabletRunTemplate],
) {
    for key in required {
        assert!(
            attributes.contains_key(*key),
            "{signal} lacks `{key}`, which the registry requires of it"
        );
    }
    for key in attributes.keys() {
        let of_a_tool = templates.iter().any(|template| {
            key.strip_prefix(template.prefix())
                .is_some_and(|tool| tool.starts_with('.'))
        });
        assert!(
            declared.contains(&key.as_str()) || of_a_tool,
            "{signal} holds `{key}`, which the registry doesn't declare for it"
        );
    }
}

/// One run of the script, and what was in its file when `run` returned.
struct Smoke {
    scratch: Lab,
    finished: FinishedRun,
    exported: Exported,
    config_digest: String,
}

impl Smoke {
    async fn run(test: &str) -> Self {
        let scratch = Lab::new(test);
        let config = scratch.config(
            WRITES_READS_ENDS,
            json!({
                "run": { "retry_backoff_base": "1ms", "retry_backoff_max": "1ms" },
                "tools": { "builtin": scratch.builtin(&["bash", "read_file", "write_file"]) },
            }),
        );
        let config_digest = config.digest().to_string();
        let mut lablet = lablet::build(config).await.unwrap();

        let finished = lablet
            .run(request().run_id(RunId::new(RUN).unwrap()).unwrap())
            .await;
        // Read before the shutdown: the file is whole when `run` returns.
        let exported = scratch.exported();
        lablet.shutdown().await;

        Self {
            scratch,
            finished,
            exported,
            config_digest,
        }
    }

    fn traced(&self) -> Traced<'_> {
        Traced::of(&self.exported, RUN)
    }
}

#[tokio::test]
async fn a_run_completes_and_leaves_a_root_span_over_its_calls() {
    let smoke = Smoke::run("smoke-root").await;

    let outcome = &smoke.finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!(outcome.result().text, "The parser was off by one.");
    assert_eq!((outcome.turns, outcome.tool_calls), (3, 3));
    assert_eq!(
        std::fs::read_to_string(smoke.scratch.root().join("notes/fix.md")).unwrap(),
        "Off by one.",
        "the tools worked under the root"
    );

    assert_eq!(
        smoke.exported.spans.len(),
        8,
        "a root, four attempts and three calls"
    );
    let traced = smoke.traced();
    assert_eq!(traced.spans.len(), 8, "every span names its run");
    assert_eq!(traced.records.len(), smoke.exported.records.len());

    let root = traced.root();
    assert_eq!(root.name, "invoke_agent lablet");
    assert_eq!(root.kind, SpanKind::Internal);
    assert_eq!(root.parent_span_id, None);
    assert_eq!(root.status, Status::Unset);
    assert_declared(
        "the root span",
        &root.attributes,
        SPAN_LABLET_INVOKE_AGENT_REQUIRED,
        SPAN_LABLET_INVOKE_AGENT_KEYS,
        &[],
    );
    assert_eq!(
        root.attributes[key::GEN_AI_AGENT_VERSION],
        json!(lablet::VERSION)
    );
    assert_eq!(root.attributes[key::GEN_AI_REQUEST_MODEL], json!(MODEL));
    assert_eq!(
        root.attributes[key::LABLET_RUN_STOP_REASON],
        json!("completed")
    );
    assert_eq!(root.attributes[key::LABLET_RUN_TURNS], json!(3));
    assert_eq!(root.attributes[key::LABLET_TOOL_CALLS_TOTAL], json!(3));

    for span in &traced.spans {
        assert_eq!(span.trace_id, root.trace_id);
        assert_eq!(span.attributes[key::SESSION_ID], json!(RUN));
        assert_eq!(
            span.attributes[key::LABLET_CONFIG_DIGEST],
            json!(smoke.config_digest)
        );
        assert_eq!(span.flags & 1, 1, "every span is sampled");
    }
    for child in traced
        .spans
        .iter()
        .filter(|span| span.span_id != root.span_id)
    {
        assert_eq!(child.parent_span_id.as_ref(), Some(&root.span_id));
    }
}

#[tokio::test]
async fn every_attempt_of_a_provider_call_leaves_a_chat_span() {
    let smoke = Smoke::run("smoke-chat").await;
    let traced = smoke.traced();

    let chats = traced.chats();
    assert_eq!(chats.len(), 4);
    for chat in &chats {
        assert_eq!(chat.name, format!("chat {MODEL}"));
        assert_eq!(chat.kind, SpanKind::Client);
        assert_declared(
            "a chat span",
            &chat.attributes,
            SPAN_LABLET_CHAT_REQUIRED,
            SPAN_LABLET_CHAT_KEYS,
            &[],
        );
        assert_eq!(chat.attributes[key::GEN_AI_PROVIDER_NAME], json!("fake"));
    }
    assert_eq!(chats[0].attributes[key::ERROR_TYPE], json!("retryable"));
    assert_eq!(
        chats
            .iter()
            .map(|chat| (
                &chat.attributes[key::LABLET_TURN],
                &chat.attributes[key::LABLET_ATTEMPT]
            ))
            .collect::<Vec<_>>(),
        [
            (&json!(1), &json!(1)),
            (&json!(1), &json!(2)),
            (&json!(2), &json!(1)),
            (&json!(3), &json!(1)),
        ]
    );
    assert_eq!(
        chats[1].attributes[key::GEN_AI_RESPONSE_MODEL],
        json!("scripted-2026-09")
    );
    assert_eq!(
        chats[1].attributes[key::GEN_AI_USAGE_INPUT_TOKENS],
        json!(1000)
    );

    let exceptions = smoke
        .exported
        .records_of("gen_ai.client.operation.exception");
    assert_eq!(exceptions.len(), 1);
    assert_eq!(exceptions[0].span_id, chats[0].span_id);
    assert_eq!(
        smoke
            .exported
            .records_of("gen_ai.client.inference.operation.details"),
        Vec::<&LogRecord>::new(),
        "content is captured only when the config says so"
    );
}

#[tokio::test]
async fn every_tool_call_leaves_a_tool_span() {
    let smoke = Smoke::run("smoke-tools").await;
    let traced = smoke.traced();

    let tools = traced.tools();
    assert_eq!(
        tools
            .iter()
            .map(|tool| (
                tool.name.as_str(),
                &tool.attributes[key::LABLET_TOOL_STATUS]
            ))
            .collect::<Vec<_>>(),
        [
            ("execute_tool write_file", &json!("ok")),
            ("execute_tool read_file", &json!("ok")),
            ("execute_tool bash", &json!("ok")),
        ],
        "a command that exits with a code of its own is a result like any other"
    );
    for tool in &tools {
        assert_eq!(tool.kind, SpanKind::Internal);
        assert_declared(
            "a tool span",
            &tool.attributes,
            SPAN_LABLET_EXECUTE_TOOL_REQUIRED,
            SPAN_LABLET_EXECUTE_TOOL_KEYS,
            &[],
        );
        assert_eq!(tool.attributes[key::LABLET_TOOL_SOURCE], json!("builtin"));
        assert_eq!(tool.attributes[key::GEN_AI_TOOL_TYPE], json!("function"));
    }
    assert_eq!(
        tools
            .iter()
            .map(|tool| (
                &tool.attributes[key::LABLET_TURN],
                &tool.attributes[key::GEN_AI_TOOL_CALL_ID]
            ))
            .collect::<Vec<_>>(),
        [
            (&json!(1), &json!("call_1")),
            (&json!(1), &json!("call_2")),
            (&json!(2), &json!("call_3")),
        ]
    );
}

#[tokio::test]
async fn the_wide_event_is_the_last_line_of_the_run_and_holds_what_the_run_came_to() {
    let smoke = Smoke::run("smoke-wide").await;
    let traced = smoke.traced();

    let (root, wide) = (traced.root(), traced.wide());
    assert_eq!(wide.trace_id, root.trace_id);
    assert_eq!(wide.span_id, root.span_id);
    assert_eq!(wide.line, smoke.exported.lines);
    assert_declared(
        "the wide event",
        &wide.attributes,
        EVENT_LABLET_RUN_REQUIRED,
        EVENT_LABLET_RUN_KEYS,
        EventLabletRunTemplate::ALL,
    );
    for (key, holds) in [
        (key::GEN_AI_CONVERSATION_ID, json!(RUN)),
        (key::GEN_AI_AGENT_VERSION, json!(lablet::VERSION)),
        (key::LABLET_CONFIG_DIGEST, json!(smoke.config_digest)),
        (key::GEN_AI_PROVIDER_NAME, json!("fake")),
        (key::GEN_AI_REQUEST_MODEL, json!(MODEL)),
        (key::GEN_AI_REQUEST_MAX_TOKENS, json!(32_000)),
        (key::LABLET_REQUEST_API, json!("script")),
        (key::LABLET_REQUEST_CACHE_SCOPE, json!("shared")),
        (key::LABLET_REQUEST_THINKING, json!("provider_default")),
        (key::LABLET_RUN_COMPLETION_MODE, json!("natural")),
        (key::LABLET_RUN_TIMEOUT_MS, json!(600_000)),
        (key::LABLET_RUN_STOP_REASON, json!("completed")),
        (key::LABLET_RUN_TURNS, json!(3)),
        (key::LABLET_PROVIDER_RETRIES, json!(1)),
        (key::GEN_AI_USAGE_INPUT_TOKENS, json!(3300)),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, json!(100)),
        (key::LABLET_PROVIDER_FAILED_INPUT_TOKENS, json!(40)),
        (key::LABLET_TOOL_CALLS_TOTAL, json!(3)),
        (key::LABLET_TOOL_CALLS_ERRORS, json!(0)),
        (
            key::LABLET_TOOLS_NAMES,
            json!(["bash", "read_file", "write_file"]),
        ),
        (key::LABLET_TELEMETRY_DROPPED_RECORDS, json!(0)),
        (key::LABLET_SKILLS_COUNT, json!(0)),
    ] {
        assert_eq!(wide.attributes[key], holds, "{key}");
    }
    assert_eq!(wide.attributes["lablet.tool.calls.bash"], json!(1));
    for absent in [
        key::LABLET_RUN_TRANSCRIPT_PATH,
        key::LABLET_MCP_SERVERS,
        key::LABLET_RUN_MAX_TURNS,
        key::LABLET_RUN_COST_USD,
    ] {
        assert!(!wide.attributes.contains_key(absent), "{absent}");
    }
}

#[tokio::test]
async fn every_export_carries_the_resource_the_config_states() {
    let scratch = Lab::new("resource");
    let config = scratch.config(
        crate::harness::ENDS,
        json!({ "telemetry": { "resource": { "team": "evals", "service.name": "mine" } } }),
    );
    let mut lablet = lablet::build(config).await.unwrap();

    lablet.run(request()).await;
    lablet.shutdown().await;

    let exported = scratch.exported();
    let resources = exported
        .spans
        .iter()
        .map(|span| &span.resource)
        .chain(exported.records.iter().map(|record| &record.resource));
    let mut exports = 0;
    for resource in resources {
        exports += 1;
        assert_eq!(resource["team"], json!("evals"));
        assert_eq!(
            resource[key::SERVICE_NAME],
            json!("lablet"),
            "a key of lablet's own keeps lablet's value"
        );
        assert_eq!(resource[key::SERVICE_VERSION], json!(lablet::VERSION));
    }
    assert_eq!(exports, 3, "a root span, a chat span and the wide event");
}

#[tokio::test]
async fn content_reaches_telemetry_when_the_config_says_so() {
    let scratch = Lab::new("content");
    let config = scratch.config(
        crate::harness::ENDS,
        json!({ "telemetry": { "capture_content": true } }),
    );
    let mut lablet = lablet::build(config).await.unwrap();

    lablet.run(request()).await;
    lablet.shutdown().await;

    let exported = scratch.exported();
    assert!(
        !exported
            .records_of("gen_ai.client.inference.operation.details")
            .is_empty()
    );
    let wide = exported.records_of("lablet.run")[0];
    assert_eq!(
        wide.attributes[key::LABLET_RESULT_TEXT],
        json!("Nothing to fix.")
    );
    assert_eq!(wide.line, exported.lines);
}
