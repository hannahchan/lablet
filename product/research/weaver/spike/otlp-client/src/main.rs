//! Sends one correct span + one flawed span, and one correct wide-event log record
//! + one flawed one, to a `weaver registry live-check` OTLP/gRPC listener.
use opentelemetry::logs::{LogRecord, Logger, LoggerProvider as _, Severity};
use opentelemetry::trace::{Span, SpanKind, Tracer, TracerProvider as _};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{LogExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;

#[tokio::main]
async fn main() {
    let endpoint = std::env::var("OTLP").unwrap_or_else(|_| "http://127.0.0.1:4317".into());
    let resource = Resource::builder().with_service_name("lablet").build();

    let span_exporter = SpanExporter::builder().with_tonic().with_endpoint(&endpoint).build().unwrap();
    let tp = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_simple_exporter(span_exporter)
        .build();
    let tracer = tp.tracer("lablet");

    // 1. A conforming root span.
    let mut ok = tracer
        .span_builder("invoke_agent lablet")
        .with_kind(SpanKind::Internal)
        .with_attributes(vec![
            KeyValue::new("gen_ai.operation.name", "invoke_agent"),
            KeyValue::new("gen_ai.agent.name", "lablet"),
            KeyValue::new("lablet.run.stop_reason", "completed"),
            KeyValue::new("lablet.run.turns", 7i64),
            KeyValue::new("lablet.tool.calls.bash", 3i64),
        ])
        .start(&tracer);
    ok.end();

    // 2. A flawed span: wrong type, unknown enum value, undeclared key, template with wrong type.
    let mut bad = tracer
        .span_builder("invoke_agent lablet")
        .with_kind(SpanKind::Internal)
        .with_attributes(vec![
            KeyValue::new("gen_ai.operation.name", "invoke_agent"),
            KeyValue::new("lablet.run.stop_reason", "gave_up"),
            KeyValue::new("lablet.run.turns", "seven"),
            KeyValue::new("lablet.undeclared", true),
            KeyValue::new("lablet.tool.calls.bash", "three"),
        ])
        .start(&tracer);
    bad.end();
    tp.shutdown().unwrap();

    let log_exporter = LogExporter::builder().with_tonic().with_endpoint(&endpoint).build().unwrap();
    let lp = SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_simple_exporter(log_exporter)
        .build();
    let logger = lp.logger("lablet");

    // 3. A conforming wide event.
    let mut rec = logger.create_log_record();
    rec.set_event_name("lablet.run");
    rec.set_severity_number(Severity::Info);
    rec.set_body("run summary".into());
    rec.add_attribute("gen_ai.agent.name", "lablet");
    rec.add_attribute("gen_ai.usage.input_tokens", 1234i64);
    rec.add_attribute("lablet.run.stop_reason", "max_turns");
    rec.add_attribute("lablet.run.turns", 7i64);
    rec.add_attribute("lablet.tool.calls.bash", 3i64);
    logger.emit(rec);

    // 4. A flawed wide event: required attributes missing, wrong type, unknown event name.
    let mut rec = logger.create_log_record();
    rec.set_event_name("lablet.run");
    rec.set_severity_number(Severity::Info);
    rec.add_attribute("lablet.run.turns", "seven");
    logger.emit(rec);

    let mut rec = logger.create_log_record();
    rec.set_event_name("lablet.unknown_event");
    rec.add_attribute("lablet.run.turns", 1i64);
    logger.emit(rec);

    lp.shutdown().unwrap();
    eprintln!("sent 2 spans, 3 log records to {endpoint}");
}
