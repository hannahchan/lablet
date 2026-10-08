//! Library mode: what configured an OpenTelemetry SDK does nothing and says
//! nothing, while content capture and lablet's secrets still apply.
//!
//! Each case runs in a child process of this binary, since what it pins is
//! what lablet reads of the process's environment, which `cargo xtask test`
//! strips and a test can't set for its own process.

use std::process::{Command, Stdio};

use lablet::{FinishedRun, RunRequest, StopReason};
use lablet_conformance::host::SERVICE;
use lablet_model::ToolResultContent;
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
    std::env::var_os(CHILD).is_some()
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
            "otlp": { "enabled": true, "endpoint": "notaurl" },
            "resource": { "team": "evals" },
        } }),
    );
    let diagnostics = Diagnostics::capture();

    lablet::check(&config).await.unwrap();
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

/// The child's side: the capture variable is `true`.
#[tokio::test]
async fn capture_child() {
    if !in_the_child() {
        return;
    }
    assert!(captured(&Lab::new("capture-variable"), json!({})).await);
    assert!(
        !captured(
            &Lab::new("capture-stated"),
            json!({ "telemetry": { "capture_content": false } })
        )
        .await,
        "the config's setting wins over the variable"
    );
}

#[test]
fn capture_content_and_its_variable_still_decide_in_library_mode() {
    in_a_child(
        "capture_child",
        &[("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT", "true")],
    );
}

/// The child's side: a command prints the OTLP header variable it
/// inherits, and its value decoded.
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

    let checked = lablet::check(&config).await.unwrap();
    let mut lablet = lab.build(config).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    assert!(checked.withheld().is_empty(), "every command inherits it");
    assert_eq!(checked.cut(), ["OTEL_EXPORTER_OTLP_HEADERS"]);
    assert_eq!(
        sent(&finished),
        "[secret withheld]\n[secret withheld]\nexit code: 0"
    );
}

#[test]
fn an_otlp_header_variable_s_value_is_cut_from_a_tool_result_in_library_mode() {
    let value = format!("authorization=Bearer%20{TOKEN}");
    in_a_child(
        "header_variable_child",
        &[("OTEL_EXPORTER_OTLP_HEADERS", &value)],
    );
}

/// The variable a child process gives `telemetry.otlp.headers`.
const SUBSTITUTED: &str = "LABLET_TEST_TELEMETRY_TOKEN";

/// The child's side: a command prints the variable substituted into the
/// config's header, and the header's value.
#[tokio::test]
async fn substituted_header_child() {
    if !in_the_child() {
        return;
    }
    let lab = Lab::new("library-substituted-header");
    let script = running(&format!("echo \"tok=${SUBSTITUTED}\"; echo Bearer {TOKEN}"));
    let config = lab.config(
        &script,
        json!({
            "tools": lab.builtin_tools(&["bash"]),
            "telemetry": { "otlp": { "headers": {
                "authorization": format!("Bearer ${{{SUBSTITUTED}}}"),
            } } },
        }),
    );

    let checked = lablet::check(&config).await.unwrap();
    let mut lablet = lab.build(config).await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    assert_eq!(checked.withheld(), &[SUBSTITUTED.to_owned()].into());
    assert_eq!(checked.cut(), [SUBSTITUTED]);
    assert_eq!(
        sent(&finished),
        "tok=\n[secret withheld]\nexit code: 0",
        "the variable is withheld from the command, and its value is cut"
    );
}

#[test]
fn a_substituted_telemetry_header_is_withheld_and_cut_in_library_mode() {
    in_a_child("substituted_header_child", &[(SUBSTITUTED, TOKEN)]);
}

/// A host that wants spans and no records hands in the API's no-op logger
/// provider.
#[tokio::test]
async fn a_lablet_built_with_the_no_op_logger_provider_emits_its_spans_and_no_record() {
    let lab = Lab::new("no-op-logger");
    let mut lablet = lablet::Lablet::builder(
        lab.config(ENDS, json!({ "telemetry": { "capture_content": true } })),
        opentelemetry::logs::NoopLoggerProvider::new(),
    )
    .with_tracer_provider(lab.host().tracer_provider())
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

/// A variable nothing sets.
const UNSET: &str = "LABLET_TEST_A_VARIABLE_NOTHING_SETS";

/// The telemetry file and the resource only configured an SDK, so a
/// variable in them that isn't set is no refusal in library mode, as it is
/// on the command line.
#[tokio::test]
async fn an_unset_variable_in_the_telemetry_file_or_resource_is_not_refused_in_library_mode() {
    let lab = Lab::new("library-unset-telemetry");
    let config = lab.config(
        ENDS,
        json!({ "telemetry": {
            "file": { "path": format!("${{{UNSET}}}/telemetry.otlp.jsonl") },
            "resource": { "team": format!("${{{UNSET}}}") },
        } }),
    );

    lablet::check(&config).await.unwrap();
    lab.build(config).await.unwrap().shutdown().await;
}
