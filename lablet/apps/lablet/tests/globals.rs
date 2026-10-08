//! The library root's fallback to OpenTelemetry's global tracer provider,
//! which this binary's one test sets: a global is one per process, so its
//! steps run in sequence in a binary of their own, where no other test sees
//! what they set.

#![cfg(test)]

use lablet::{Config, Format, Lablet, RunRequest};
use lablet_conformance::host::Host;
use lablet_test_support::Scratch;
use opentelemetry::global;
use opentelemetry::logs::NoopLoggerProvider;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use serde_json::json;

/// A config of the fake provider playing a run that ends at once.
fn config(scratch: &Scratch) -> Config {
    let script = scratch.write(
        "script.yaml",
        "- response:\n    content:\n      - text: Done.\n    finish: end_turn\n",
    );
    let config = json!({
        "model": { "provider": "fake", "script": script, "name": "scripted-1" },
        "prompt": { "system": "You fix failing tests." },
    });
    Config::from_str(&config.to_string(), Format::Json).unwrap()
}

/// A tracer provider of the SDK's over an in-memory exporter, and the
/// exporter.
fn provider() -> (SdkTracerProvider, InMemorySpanExporter) {
    let spans = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(spans.clone())
        .build();
    (provider, spans)
}

/// How many root spans of a run `spans` holds.
fn runs(spans: &InMemorySpanExporter) -> usize {
    spans
        .get_finished_spans()
        .unwrap()
        .iter()
        .filter(|span| span.name.starts_with("invoke_agent"))
        .count()
}

fn request() -> RunRequest {
    RunRequest::new("Fix the failing test.").unwrap()
}

#[tokio::test]
async fn the_fallback_reads_the_globals_and_a_piece_handed_in_wins() {
    let scratch = Scratch::new("globals");

    // The library sets no global: after a build and a run with a tracer
    // provider handed in, the global tracer provider is still the no-op one.
    let host = Host::new();
    let mut lablet = Lablet::builder(config(&scratch), host.logger_provider())
        .with_tracer_provider(host.tracer_provider())
        .build()
        .await
        .unwrap();
    lablet.run(request()).await;
    lablet.shutdown().await;
    #[expect(
        clippy::disallowed_methods,
        reason = "the test reads the global tracer provider to show that nothing set it"
    )]
    let global_provider = global::tracer_provider();
    let span = global_provider.tracer("probe").start("probe");
    assert!(
        !span.span_context().is_valid(),
        "a global tracer provider was set"
    );
    assert_eq!(host.exported().spans_of("invoke_agent").len(), 1);

    // A tracer provider handed in wins over a global one.
    let (global_tracer_provider, global_spans) = provider();
    #[expect(
        clippy::disallowed_methods,
        reason = "the test sets the global tracer provider to check the fallback"
    )]
    global::set_tracer_provider(global_tracer_provider);
    let (handed_in, handed_in_spans) = provider();
    let mut lablet = Lablet::builder(config(&scratch), NoopLoggerProvider::new())
        .with_tracer_provider(handed_in)
        .build()
        .await
        .unwrap();
    lablet.run(request()).await;
    assert_eq!(runs(&handed_in_spans), 1);
    assert_eq!(runs(&global_spans), 0, "the global provider heard the run");

    // With none handed in, the global tracer provider set before the build
    // receives the run, and one set after the build receives none of the
    // next run, since the global one is read once, at the build.
    let mut lablet = lablet::build(config(&scratch), NoopLoggerProvider::new())
        .await
        .unwrap();
    lablet.run(request()).await;
    assert_eq!(runs(&global_spans), 1);
    let (later, later_spans) = provider();
    #[expect(
        clippy::disallowed_methods,
        reason = "the test sets the global tracer provider to check the fallback"
    )]
    global::set_tracer_provider(later);
    lablet.run(request()).await;
    assert_eq!(
        runs(&global_spans),
        2,
        "the next run went to the global of its build"
    );
    assert_eq!(
        runs(&later_spans),
        0,
        "a global set after the build heard the run"
    );
}
