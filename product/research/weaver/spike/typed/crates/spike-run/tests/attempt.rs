//! The generated types against the real SDK: what reaches an exporter.

use std::time::{Duration, SystemTime};

use opentelemetry::global::BoxedTracer;
use opentelemetry::logs::AnyValue;
use opentelemetry::trace::{SpanKind, TraceContextExt as _, Tracer as _, TracerProvider as _};
use opentelemetry::{Context, Key, Value};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tracing_subscriber::layer::SubscriberExt as _;

use spike_run::{FailedAttempt, failed_attempt};

#[test]
fn a_failed_attempt_reaches_the_exporters_as_the_registry_declares() {
    let spans = InMemorySpanExporter::default();
    let tracer_provider = SdkTracerProvider::builder()
        .with_simple_exporter(spans.clone())
        .build();
    let logs = InMemoryLogExporter::default();
    let logger_provider = SdkLoggerProvider::builder()
        .with_simple_exporter(logs.clone())
        .build();
    let subscriber =
        tracing_subscriber::registry().with(OpenTelemetryTracingBridge::new(&logger_provider));

    let tracer = BoxedTracer::new(Box::new(tracer_provider.tracer("lablet")));
    let root = tracer.start("invoke_agent lablet");
    let parent = Context::current_with_span(root);
    let started = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    tracing::subscriber::with_default(subscriber, || {
        failed_attempt(
            &tracer,
            &parent,
            &FailedAttempt {
                run_id: "run-1".to_owned(),
                digest: "sha256:abc".to_owned(),
                model: "claude-opus-5-5".to_owned(),
                turn: 1,
                attempt: 2,
                started,
                latency: Duration::from_millis(1500),
                backoff: Duration::from_millis(400),
                message: "overloaded".to_owned(),
            },
        );
    });

    let finished = spans.get_finished_spans().expect("spans");
    let chat = finished.iter().find(|s| s.name == "chat claude-opus-5-5").expect("chat span");
    assert_eq!(chat.span_kind, SpanKind::Client);
    assert_eq!(chat.start_time, started);
    assert_eq!(chat.end_time, started + Duration::from_millis(1500));
    let attribute = |key: &str| {
        chat.attributes
            .iter()
            .find(|kv| kv.key == Key::from(key.to_owned()))
            .map(|kv| kv.value.clone())
    };
    assert_eq!(attribute("gen_ai.provider.name"), Some(Value::from("anthropic")));
    assert_eq!(attribute("lablet.attempt"), Some(Value::I64(2)));
    assert_eq!(attribute("error.type"), Some(Value::from("retryable")));
    assert_eq!(attribute("gen_ai.usage.input_tokens"), None);
    assert_eq!(chat.events.events.len(), 1);
    assert_eq!(chat.events.events[0].name, "lablet.retry");
    assert_eq!(chat.parent_span_id, parent.span().span_context().span_id());

    let emitted = logs.get_emitted_logs().expect("logs");
    assert_eq!(emitted.len(), 1);
    let record = &emitted[0].record;
    assert_eq!(record.event_name(), Some("gen_ai.client.operation.exception"));
    assert_eq!(record.target().map(|t| t.as_ref()), Some("lablet"));
    assert_eq!(record.severity_text(), Some("WARN"));
    let trace = record.trace_context().expect("the record is in a span's context");
    assert_eq!(trace.span_id, chat.span_context.span_id());
    let record_attribute = |key: &str| {
        record
            .attributes_iter()
            .find(|(k, _)| k.as_str() == key)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(
        record_attribute("exception.message"),
        Some(AnyValue::from("overloaded".to_owned()))
    );
    assert_eq!(record_attribute("lablet.attempt"), Some(AnyValue::Int(2)));
    assert_eq!(record_attribute("lablet.trial"), Some(AnyValue::from("3".to_owned())));
    assert_eq!(record_attribute("lablet.task.id"), None);
    // The appender's scope can carry no schema URL; lablet's export crate would restore it.
    println!("scope: {:?}", emitted[0].instrumentation);
}
