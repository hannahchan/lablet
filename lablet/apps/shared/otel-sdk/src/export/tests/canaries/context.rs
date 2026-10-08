//! The canaries of the gaps lablet fills in the resource and the inbound
//! context, and of the SDK's warning about a sampler it reads for itself,
//! which the command line's diagnostic log leaves out.

use std::collections::HashMap;
use std::io;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use lablet_test_support::Scratch;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{Context, Key, Value, global};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::resource::{EnvResourceDetector, ResourceDetector as _};
use opentelemetry_sdk::trace::SdkTracerProvider;
use serde_json::json;

use lablet::{Config, Format};

/// Set in the environment of a child process a canary below starts, so the
/// child's side runs.
const CHILD: &str = "LABLET_TEST_CONTEXT_CANARY_CHILD";

/// Runs the test `name` of this module in a child process whose
/// environment adds `env`, and holds that it ran and passed.
fn in_a_child(name: &str, env: &[(&str, &str)]) {
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("export::tests::canaries::context::{name}"),
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

/// The child's side: the SDK's own detector of `OTEL_RESOURCE_ATTRIBUTES`
/// gives a value as it's written.
#[test]
fn env_resource_detector_child() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let detected = EnvResourceDetector::new().detect();
    assert_eq!(detected.get(&Key::new("team")), Some(Value::from("a%2Cb")));
}

/// Lablet decodes the pairs itself, from the seam, rather than decoding what
/// this detector gives. When the SDK decodes them, this fails, and lablet's
/// own detector can give way to the SDK's once
/// `canary_resource_builder_puts_the_pairs_over_otel_service_name` fails too.
#[test]
fn canary_env_resource_detector_leaves_an_escape_undecoded() {
    in_a_child(
        "env_resource_detector_child",
        &[("OTEL_RESOURCE_ATTRIBUTES", "team=a%2Cb")],
    );
}

/// The child's side: the SDK's own resource, with both `OTEL_SERVICE_NAME`
/// and a `service.name` among the pairs, takes the pairs'.
#[test]
fn resource_builder_precedence_child() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let resource = Resource::builder().build();
    assert_eq!(
        resource.get(&Key::new("service.name")),
        Some(Value::from("from-the-pairs"))
    );
}

/// Lablet puts `OTEL_SERVICE_NAME` over the pairs' `service.name`, as the
/// specification orders them, where the SDK's builder merges the pairs
/// last. A release that decodes the pairs but keeps this order would fail
/// the decoding canary alone, and giving way to the SDK's detector then
/// would let the pairs name the service. When the SDK takes the variable
/// over the pairs, this fails.
#[test]
fn canary_resource_builder_puts_the_pairs_over_otel_service_name() {
    in_a_child(
        "resource_builder_precedence_child",
        &[
            ("OTEL_SERVICE_NAME", "from-the-variable"),
            ("OTEL_RESOURCE_ATTRIBUTES", "service.name=from-the-pairs"),
        ],
    );
}

/// The child's side: a `Lablet` built with `OTEL_PROPAGATORS` in the
/// process's environment, and then the global propagator asked to inject a
/// sampled span's context.
#[tokio::test]
async fn global_propagator_child() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let scratch = Scratch::new("canary-global-propagator");
    let config = json!({
        "model": {
            "provider": "fake",
            "script": scratch.write(
                "script.yaml",
                "- response:\n    content:\n      - text: Done.\n    finish: end_turn\n",
            ),
            "name": "scripted-1",
        },
        "prompt": { "system": "You fix failing tests." },
        "telemetry": {
            "otlp": { "enabled": false },
            "file": { "path": scratch.at("telemetry.otlp.jsonl") },
        },
    });
    let lablet = lablet::build(Config::from_str(&config.to_string(), Format::Json).unwrap())
        .await
        .unwrap();
    let parent = Context::new().with_remote_span_context(SpanContext::new(
        TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap(),
        SpanId::from_hex("b7ad6b7169203331").unwrap(),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    ));
    let mut injected = HashMap::new();
    #[expect(
        clippy::disallowed_methods,
        reason = "the canary reads the global propagator to show that nothing set it"
    )]
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&parent, &mut injected);
    });
    lablet.shutdown().await;

    assert_eq!(injected, HashMap::new());
}

/// Lablet extracts through propagators of its own and never sets the
/// global one. When building the providers, or anything else a build
/// calls, comes to install a global propagator from `OTEL_PROPAGATORS`,
/// this fails, and lablet's must be held apart from it.
#[test]
fn canary_the_global_propagator_is_a_no_op_after_a_lablet_is_built() {
    in_a_child(
        "global_propagator_child",
        &[("OTEL_PROPAGATORS", "tracecontext,baggage")],
    );
}

/// What a subscriber writes, kept.
#[derive(Debug, Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Written {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The child's side: the SDK's tracer provider builder, started under a
/// sampler variable that names no sampler, which no reading of the variable
/// accepts, so only a builder that stops reading it stops warning.
#[test]
fn sdk_sampler_warning_child() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let written = Written::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(written.clone())
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, || drop(SdkTracerProvider::builder()));

    let text = String::from_utf8(written.0.lock().unwrap().clone()).unwrap();
    assert!(
        text.contains("TracerProvider.Config.InvalidSamplerType"),
        "{text}"
    );
}

/// The command line's diagnostic log leaves out the SDK's warnings named
/// `TracerProvider.Config.*`, since lablet states the sampler it read
/// itself. When the SDK stops reading the variable, or stops warning of it,
/// this fails, and the filter can go.
#[test]
fn canary_the_sdk_warns_of_a_sampler_it_reads_for_itself() {
    in_a_child(
        "sdk_sampler_warning_child",
        &[("OTEL_TRACES_SAMPLER", "not_a_sampler")],
    );
}
