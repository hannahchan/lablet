//! Lablet reads and sets none of OpenTelemetry's globals, which this
//! binary's one test sets and reads: a global is one per process, so its
//! steps run in sequence in a binary of their own, where no other test sees
//! what they set.

#![cfg(test)]

use std::collections::HashMap;

use lablet::{Config, FinishedRun, Format, Lablet, Otel, RunRequest};
use lablet_conformance::host::Host;
use lablet_model::ToolResultContent;
use lablet_test_support::Scratch;
use opentelemetry::logs::NoopLoggerProvider;
use opentelemetry::propagation::text_map_propagator::FieldIter;
use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry::trace::{Span as _, TraceContextExt as _, Tracer as _, TracerProvider as _};
use opentelemetry::{Context, global};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use serde_json::json;

/// What a command prints of the context variables a step looks for.
const PRINTS: &str = "printf '%s|%s' \"${TRACEPARENT-unset}\" \"${X_GLOBAL-unset}\"";

/// A config of the fake provider playing a run that calls `bash` with
/// [`PRINTS`] and then ends.
fn printing(scratch: &Scratch) -> Config {
    scratch.create_dir("work");
    let script = scratch.write(
        "printing.yaml",
        format!(
            "- response:\n    content:\n      - tool_use: {{ id: call_1, name: bash, input: {{ json: {{ command: {} }} }} }}\n    finish: tool_use\n- response:\n    content:\n      - text: Done.\n    finish: end_turn\n",
            json!(PRINTS)
        ),
    );
    let config = json!({
        "model": { "provider": "fake", "script": script, "name": "scripted-1" },
        "prompt": { "system": "You fix failing tests." },
        "tools": { "builtin": { "root": scratch.at("work"), "enabled": ["bash"] } },
    });
    Config::from_str(&config.to_string(), Format::Json).unwrap()
}

/// What the run's one command printed of `TRACEPARENT` and `X_GLOBAL`.
fn printed(finished: &FinishedRun) -> (String, String) {
    let call = &finished.transcript.turns()[0].tool_calls()[0];
    let text: String = call
        .content
        .iter()
        .map(|ToolResultContent::Text(text)| text.as_str())
        .collect();
    let shown = text.strip_suffix("\nexit code: 0").unwrap();
    let (traceparent, global_key) = shown.split_once('|').unwrap();
    (traceparent.to_owned(), global_key.to_owned())
}

/// A propagator that injects one key of its own, the current span's id.
#[derive(Debug)]
struct Named(Vec<String>);

impl Named {
    fn new(key: &str) -> Self {
        Self(vec![key.to_owned()])
    }
}

impl TextMapPropagator for Named {
    fn inject_context(&self, cx: &Context, injector: &mut dyn Injector) {
        injector.set(&self.0[0], cx.span().span_context().span_id().to_string());
    }

    fn extract_with_context(&self, cx: &Context, _: &dyn Extractor) -> Context {
        cx.clone()
    }

    fn fields(&self) -> FieldIter<'_> {
        FieldIter::new(&self.0)
    }
}

/// A host's propagator that asks OpenTelemetry's global one at each
/// inject and extract, and names the fields of the global one the host set.
#[derive(Debug)]
struct TheGlobal(Vec<String>);

impl TextMapPropagator for TheGlobal {
    fn inject_context(&self, cx: &Context, injector: &mut dyn Injector) {
        #[expect(
            clippy::disallowed_methods,
            reason = "the test is a host that hands lablet the global propagator"
        )]
        global::get_text_map_propagator(|propagator| propagator.inject_context(cx, injector));
    }

    fn extract_with_context(&self, cx: &Context, extractor: &dyn Extractor) -> Context {
        #[expect(
            clippy::disallowed_methods,
            reason = "the test is a host that hands lablet the global propagator"
        )]
        global::get_text_map_propagator(|propagator| propagator.extract_with_context(cx, extractor))
    }

    fn fields(&self) -> FieldIter<'_> {
        FieldIter::new(&self.0)
    }
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

/// The `Lablet` of `config` on `otel`, and its one run.
async fn run_on(config: Config, otel: Otel) -> FinishedRun {
    let mut lablet = Lablet::builder(config, otel).build().await.unwrap();
    let finished = lablet.run(request()).await;
    lablet.shutdown().await;
    finished
}

#[tokio::test]
async fn lablet_sets_no_global_and_a_global_set_before_the_build_receives_nothing() {
    let scratch = Scratch::new("globals");

    // The library sets no global: after a build and a run on the host's
    // pieces, the global tracer provider and the global propagator are
    // still the no-op ones, and the host's tracer provider holds the run.
    let host = Host::new();
    let finished = run_on(
        printing(&scratch),
        Otel::new(
            host.tracer_provider(),
            host.logger_provider(),
            TraceContextPropagator::new(),
        ),
    )
    .await;
    let (traceparent, _) = printed(&finished);
    assert!(traceparent.starts_with("00-"), "{traceparent}");
    assert_eq!(host.exported().spans_of("invoke_agent").len(), 1);
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
    let within = Context::current_with_span(host.tracer_provider().tracer("probe").start("probe"));
    let mut carrier = HashMap::new();
    #[expect(
        clippy::disallowed_methods,
        reason = "the test reads the global propagator to show that nothing set it"
    )]
    global::get_text_map_propagator(|propagator| propagator.inject_context(&within, &mut carrier));
    assert!(
        carrier.is_empty(),
        "a global propagator was set: {carrier:?}"
    );

    // With a global tracer provider and a global propagator set before the
    // build, a `Lablet` given the host's pieces sends its run to the tracer
    // provider handed in and none to the global one, and its command's
    // context is injected through the propagator handed in.
    let (global_tracer_provider, global_spans) = provider();
    #[expect(
        clippy::disallowed_methods,
        reason = "the test sets the global tracer provider to show that lablet doesn't read it"
    )]
    global::set_tracer_provider(global_tracer_provider);
    #[expect(
        clippy::disallowed_methods,
        reason = "the test sets the global propagator to show that lablet doesn't read it"
    )]
    global::set_text_map_propagator(Named::new("x-global"));
    // The test keeps a handle on the provider: when the last one goes, with
    // the `Lablet`, the provider shuts down and its exporter forgets its
    // spans.
    let (handed_in, handed_in_spans) = provider();
    let finished = run_on(
        printing(&scratch),
        Otel::new(
            handed_in.clone(),
            NoopLoggerProvider::new(),
            TraceContextPropagator::new(),
        ),
    )
    .await;
    let (traceparent, global_key) = printed(&finished);
    assert!(traceparent.starts_with("00-"), "{traceparent}");
    assert_eq!(global_key, "unset", "the global propagator injected");
    assert_eq!(runs(&handed_in_spans), 1);
    assert_eq!(runs(&global_spans), 0, "the global provider heard the run");

    // On the API's no-ops, nothing reaches a global either.
    let finished = run_on(printing(&scratch), Otel::noop()).await;
    assert_eq!(
        printed(&finished),
        ("unset".to_owned(), "unset".to_owned()),
        "a context was injected"
    );
    assert_eq!(runs(&global_spans), 0, "the global provider heard the run");

    // A host that hands the global ones in gets them: its run reaches the
    // global tracer provider, and its command's context is injected through
    // the global propagator.
    #[expect(
        clippy::disallowed_methods,
        reason = "the test is a host that hands lablet the global tracer provider"
    )]
    let global_provider = global::tracer_provider();
    let finished = run_on(
        printing(&scratch),
        Otel::new(
            global_provider,
            NoopLoggerProvider::new(),
            TheGlobal(vec!["x-global".to_owned()]),
        ),
    )
    .await;
    let (traceparent, global_key) = printed(&finished);
    assert_eq!(traceparent, "unset");
    assert_eq!(
        global_key.len(),
        16,
        "the global propagator's key: {global_key}"
    );
    assert_eq!(runs(&global_spans), 1);
}
