//! Library mode: what configured an OpenTelemetry SDK does nothing and says
//! nothing, no environment variable is read, and content capture and
//! lablet's credentials still apply.
//!
//! Each case runs in a child process of this binary, since what it pins is
//! that lablet reads nothing of the process's environment, which `cargo
//! xtask test` strips and a test can't set for its own process.

use std::process::{Command, Stdio};

use lablet::{FinishedRun, RunRequest, Secret, StopReason};
use lablet_model::ToolResultContent;
use lablet_test_support::conformance::host::SERVICE;
use opentelemetry::Context;
use opentelemetry::trace::{
    FutureExt as _, Span as _, TraceContextExt as _, Tracer as _, TracerProvider as _,
};
use serde_json::{Value, json};

use crate::harness::{Diagnostics, ENDS, Lab, Traced, request};
use crate::key;

/// Set in the environment of a child process a case below starts, so the
/// child's side runs.
const CHILD: &str = "LABLET_TEST_LIBRARY_MODE_CHILD";

/// A value as long as a key is, and so one that's cut.
const TOKEN: &str = "otlp-token-0123456789abcdef";

/// Runs the test `name` of this module in a child process whose
/// environment adds `env`, and holds that it ran and passed.
fn in_a_child(name: &str, env: &[(&str, &str)]) {
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("library_mode::{name}"),
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&child.stdout);
    assert!(
        child.status.success() && stdout.contains("1 passed"),
        "the child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&child.stderr)
    );
}

fn in_the_child() -> bool {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test reads the variable its parent set, to know which side it's on"
    )]
    let child = std::env::var_os(CHILD).is_some();
    child
}

/// A script in which `bash` runs `command` and a last turn ends the run.
fn running(command: &str) -> String {
    format!(
        "
- response:
    content:
      - tool_use: {{ id: call_1, name: bash, input: {{ json: {{ command: {} }} }} }}
    finish: tool_use
- response:
    content:
      - text: Done.
    finish: end_turn
",
        json!(command)
    )
}

/// What the model was sent of the run's one tool call.
fn sent(finished: &FinishedRun) -> String {
    finished.transcript.turns()[0].tool_calls()[0]
        .content
        .iter()
        .map(|ToolResultContent::Text(text)| text.as_str())
        .collect()
}

/// The child's side: a run whose config names a telemetry file, an
/// endpoint that doesn't parse and a resource, in an environment that turns
/// the SDK off, names a parent and holds what the command line warns of.
#[tokio::test]
async fn library_mode_ignores_child() {
    if !in_the_child() {
        return;
    }
    let lab = Lab::new("library-mode-ignores");
    let file = lab.at("telemetry.otlp.jsonl");
    let config = lab.config(
        ENDS,
        json!({ "telemetry": {
            "file": { "path": file },
            "otlp": { "endpoint": "notaurl" },
            "resource": { "team": "evals" },
        } }),
    );
    let diagnostics = Diagnostics::capture();

    lab.builder(config.clone()).check().await.unwrap();
    let mut lablet = lab.build(config).await.unwrap();
    let operation = lab
        .host()
        .tracer_provider()
        .tracer("host")
        .start("host operation");
    let host = operation.span_context().clone();
    let within = Context::current_with_span(operation);
    let finished = lablet.run(request()).with_context(within.clone()).await;
    within.span().end();
    lablet.shutdown().await;

    assert_eq!(diagnostics.lines(), [""; 0], "library mode says nothing");
    assert_eq!(
        finished.summary.outcome.stop_reason(),
        StopReason::Completed
    );
    assert!(!file.exists(), "the config's file is the host's to write");
    let exported = lab.exported();
    let traced = Traced::of(&exported, finished.summary.outcome.run_id.as_str());
    let root = traced.root();
    assert_eq!(
        root.parent_span_id.as_deref(),
        Some(host.span_id().to_string().as_str()),
        "the run's parent is the current context, not `TRACEPARENT`"
    );
    assert_eq!(root.trace_id, host.trace_id().to_string());
    assert_eq!(traced.wide().trace_id, root.trace_id);
    for resource in exported
        .spans
        .iter()
        .map(|span| &span.resource)
        .chain(exported.records.iter().map(|record| &record.resource))
    {
        assert_eq!(resource[key::SERVICE_NAME], json!(SERVICE));
        assert!(!resource.contains_key("team"), "{resource:?}");
    }
}

/// O29 and C15 don't apply to a library: what configured the SDK, the
/// config's telemetry section and the SDK's variables, does nothing, and
/// lablet says nothing of it.
#[test]
fn library_mode_ignores_the_telemetry_section_and_the_sdk_s_variables_and_says_nothing() {
    in_a_child(
        "library_mode_ignores_child",
        &[
            ("OTEL_SDK_DISABLED", "true"),
            (
                "TRACEPARENT",
                "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
            ),
            ("TRACESTATE", "vendor=value"),
            ("BAGGAGE", "%%%;;=,=bad"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "notaurl"),
            ("OTEL_EXPORTER_OTLP_CERTIFICATE", "/no/such/file.pem"),
            ("OTEL_BSP_MAX_QUEUE_SIZE", "abc"),
            ("OTEL_RESOURCE_ATTRIBUTES", "team=from-the-environment"),
        ],
    );
}

/// Whether the content record of the run's first call holds what the
/// model was sent.
async fn captured(lab: &Lab, more: Value) -> bool {
    let mut lablet = lab.build(lab.config(ENDS, more)).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;
    let exported = lab.exported();
    Traced::of(&exported, finished.summary.outcome.run_id.as_str())
        .records
        .iter()
        .any(|record| record.attributes.contains_key(key::GEN_AI_INPUT_MESSAGES))
}

/// The child's side: the capture variable is `true`, and nothing reads it.
#[tokio::test]
async fn capture_child() {
    if !in_the_child() {
        return;
    }
    assert!(
        !captured(&Lab::new("capture-variable"), json!({})).await,
        "the variable isn't read, so nothing is captured"
    );
    assert!(
        captured(
            &Lab::new("capture-stated"),
            json!({ "telemetry": { "capture_content": true } })
        )
        .await,
        "the config's setting alone decides"
    );
}

#[test]
fn capture_content_alone_decides_in_library_mode_and_its_variable_is_not_read() {
    in_a_child(
        "capture_child",
        &[("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT", "true")],
    );
}

/// The child's side: a command prints the OTLP header variable it inherits
/// from what the host handed in, uncut, since the library reads no variable
/// and the config names none.
#[tokio::test]
async fn header_variable_child() {
    if !in_the_child() {
        return;
    }
    let lab = Lab::new("library-header-variable");
    let script = running(&format!(
        "echo \"$OTEL_EXPORTER_OTLP_HEADERS\"; echo Bearer {TOKEN}"
    ));
    let config = lab.config(&script, json!({ "tools": lab.builtin_tools(&["bash"]) }));

    let checked = lab.builder(config.clone()).check().await.unwrap();
    let mut lablet = lab.build(config).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    assert!(checked.own().is_empty(), "the config names no credential");
    assert!(
        checked
            .inherited()
            .contains(&"OTEL_EXPORTER_OTLP_HEADERS".to_owned()),
        "{:?}",
        checked.inherited()
    );
    assert_eq!(
        sent(&finished),
        format!("authorization=Bearer%20{TOKEN}\nBearer {TOKEN}\nexit code: 0")
    );
}

#[test]
fn an_otlp_header_variable_the_host_hands_in_reaches_bash_uncut_in_library_mode() {
    let value = format!("authorization=Bearer%20{TOKEN}");
    in_a_child(
        "header_variable_child",
        &[("OTEL_EXPORTER_OTLP_HEADERS", &value)],
    );
}

/// A variable of the host's that holds the header's token.
const HOLDS_THE_TOKEN: &str = "LABLET_TEST_TELEMETRY_TOKEN";

/// The child's side: a header value the host set in code is cut, and the
/// variable of the handed-in environment holding its token is dropped.
#[tokio::test]
async fn header_in_code_child() {
    if !in_the_child() {
        return;
    }
    let lab = Lab::new("library-header-in-code");
    let script = running(&format!(
        "echo \"tok=${{{HOLDS_THE_TOKEN}-dropped}}\"; echo Bearer {TOKEN}"
    ));
    let mut config = lab.config(&script, json!({ "tools": lab.builtin_tools(&["bash"]) }));
    config.telemetry.otlp.headers = Some(
        [(
            "authorization".to_owned(),
            Secret::new(format!("Bearer {TOKEN}")),
        )]
        .into(),
    );

    let checked = lab.builder(config.clone()).check().await.unwrap();
    let mut lablet = lab.build(config).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    assert_eq!(
        checked.own()[0].as_own(),
        format!(
            "telemetry.otlp.headers.authorization (set in code), or {HOLDS_THE_TOKEN}, which \
             holds it; no process inherits it"
        )
    );
    assert_eq!(
        sent(&finished),
        "tok=dropped\n[secret withheld]\nexit code: 0",
        "the variable holding the token is dropped, and the value is cut"
    );
}

#[test]
fn a_header_set_in_code_is_cut_and_a_handed_in_variable_holding_it_is_dropped() {
    in_a_child("header_in_code_child", &[(HOLDS_THE_TOKEN, TOKEN)]);
}

/// A host that wants spans and no records hands in the API's no-op logger
/// provider.
#[tokio::test]
async fn a_lablet_built_with_the_no_op_logger_provider_emits_its_spans_and_no_record() {
    let lab = Lab::new("no-op-logger");
    let otel = lablet::Otel::new(
        &lab.host().tracer_provider(),
        &opentelemetry::logs::NoopLoggerProvider::new(),
        opentelemetry::trace::noop::NoopTextMapPropagator::new(),
    );
    let mut lablet = lablet::Lablet::builder(
        lab.config(ENDS, json!({ "telemetry": { "capture_content": true } })),
        otel,
    )
    .build()
    .await
    .unwrap();

    let finished = lablet.run(RunRequest::new("Fix it.").unwrap()).await;
    lablet.shutdown().await;

    let exported = lab.exported();
    let traced = Traced::of(&exported, finished.summary.outcome.run_id.as_str());
    assert_eq!(traced.root().name, format!("{} lablet", key::INVOKE_AGENT));
    assert_eq!(traced.chats().len(), 1);
    assert!(exported.records.is_empty(), "{:?}", exported.records);
}

/// A `Lablet` is built with `bash` only when its host handed in the
/// environment commands start from, and its check says so.
#[tokio::test]
async fn library_mode_refuses_bash_without_an_environment_handed_in() {
    let lab = Lab::new("library-no-environment");
    let config = lab.config(ENDS, json!({ "tools": lab.builtin_tools(&["bash"]) }));

    let refused = lablet::Lablet::builder(config, lab.otel())
        .build()
        .await
        .map(drop)
        .unwrap_err();

    assert!(
        refused
            .to_string()
            .starts_with("tools.builtin.enabled[0] (line 1): \"bash\" is refused: no process environment was handed in"),
        "{refused}"
    );
}
