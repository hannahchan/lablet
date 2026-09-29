use std::time::{Duration, UNIX_EPOCH};

use lablet_conformance::otlp::{Exported, SpanKind as ReadKind, Status as ReadStatus};
use lablet_telemetry_registry::SCHEMA_URL;
use opentelemetry::logs::{LoggerProvider as _, Severity};
use opentelemetry::trace::{SpanId, SpanKind, TraceId};
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use serde_json::json;

use super::*;
use crate::attributes::Attributes;
use crate::signal::{Ended, Happened, Record, Span};
use crate::testing::Scratch;

const FIRST: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
const SECOND: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNED";
const TRACE: TraceId = TraceId::from_bytes([0xab; 16]);
const STARTED_UNIX_MS: u64 = 1_790_000_000_000;

fn id(run: &str) -> RunId {
    RunId::new(run).unwrap()
}

fn resource() -> Resource {
    Resource::builder_empty()
        .with_attributes([
            KeyValue::new("service.name", "lablet"),
            KeyValue::new("team", "evals"),
        ])
        .build()
}

fn scope() -> InstrumentationScope {
    InstrumentationScope::builder("lablet")
        .with_version("0.1.0")
        .with_schema_url(SCHEMA_URL)
        .build()
}

fn after(ms: u64) -> std::time::SystemTime {
    UNIX_EPOCH + Duration::from_millis(STARTED_UNIX_MS + ms)
}

fn span(name: &str) -> SpanData {
    Span {
        name: name.to_owned(),
        kind: SpanKind::Client,
        id: SpanId::from_bytes([0xcd; 8]),
        parent: Some(SpanId::from_bytes([0xef; 8])),
        start: after(5),
        end: after(255),
        attributes: Attributes::default()
            .with("lablet.test.turn", 2_u32)
            .with("lablet.test.model", "scripted-1")
            .with("lablet.test.ratio", 0.5)
            .with("lablet.test.flag", true)
            .with("lablet.test.reasons", vec!["end_turn".to_owned()]),
        events: vec![Happened {
            name: "lablet.retry",
            at: after(255),
            attributes: Attributes::default().with("lablet.test.attempt", 1_u32),
        }],
        ended: Ended::Badly("529 overloaded".to_owned()),
    }
    .into_data(TRACE, &scope())
}

fn spans_to(sink: &Sink) -> FileSpanExporter {
    let mut exporter = FileSpanExporter::new(sink.clone());
    exporter.set_resource(&resource());
    exporter
}

fn records_to(sink: &Sink) -> FileLogExporter {
    let mut exporter = FileLogExporter::new(sink.clone());
    exporter.set_resource(&resource());
    exporter
}

async fn export_records(exporter: &FileLogExporter, names: &[&'static str]) -> OTelSdkResult {
    let provider = SdkLoggerProvider::builder().build();
    let logger = provider.logger_with_scope(scope());
    let scope = scope();
    let records: Vec<_> = names
        .iter()
        .map(|name| {
            Record {
                name,
                severity: Severity::Warn,
                at: after(255),
                span: SpanId::from_bytes([0xcd; 8]),
                attributes: Attributes::default().with("lablet.test.turn", 2_u32),
            }
            .into_sdk(TRACE, &logger)
        })
        .collect();
    let batch: Vec<_> = records.iter().map(|record| (record, &scope)).collect();
    exporter.export(LogBatch::new(&batch)).await
}

fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

// Where the lines go

#[tokio::test]
async fn each_run_has_a_file_of_its_own_named_for_the_run() {
    let scratch = Scratch::new("each-run");
    let sink = Sink::new(FileTarget::EachRun {
        directory: scratch.directory(),
    });
    let exporter = spans_to(&sink);

    sink.start(&id(FIRST));
    exporter.export(vec![span("chat first")]).await.unwrap();
    exporter.export(vec![span("chat first")]).await.unwrap();
    sink.start(&id(SECOND));
    exporter.export(vec![span("chat second")]).await.unwrap();

    let first = Exported::read(&scratch.path(&format!("lablet-{FIRST}.otlp.jsonl"))).unwrap();
    let second = Exported::read(&scratch.path(&format!("lablet-{SECOND}.otlp.jsonl"))).unwrap();
    assert_eq!(first.lines, 2);
    assert!(first.spans.iter().all(|span| span.name == "chat first"));
    assert_eq!(second.lines, 1);
    assert_eq!(second.spans[0].name, "chat second");
    assert_eq!(std::fs::read_dir(scratch.directory()).unwrap().count(), 2);
}

#[tokio::test]
async fn one_path_is_appended_to_by_every_run_and_keeps_what_it_held() {
    let scratch = Scratch::new("one-path");
    let path = scratch.path("runs.otlp.jsonl");
    std::fs::write(&path, "{\"resourceSpans\":[]}\n").unwrap();
    let sink = Sink::new(FileTarget::Path(path.clone()));
    let (spans, records) = (spans_to(&sink), records_to(&sink));

    sink.start(&id(FIRST));
    spans.export(vec![span("chat first")]).await.unwrap();
    sink.start(&id(SECOND));
    export_records(&records, &["lablet.run"]).await.unwrap();

    let exported = Exported::read(&path).unwrap();
    assert_eq!(exported.lines, 3);
    assert_eq!(exported.spans[0].line, 2);
    assert_eq!(exported.records[0].line, 3);
    assert_eq!(std::fs::read_dir(scratch.directory()).unwrap().count(), 1);
}

#[tokio::test]
async fn one_path_needs_no_run_to_have_started() {
    let scratch = Scratch::new("no-run-yet");
    let path = scratch.path("runs.otlp.jsonl");
    let exporter = spans_to(&Sink::new(FileTarget::Path(path.clone())));

    exporter.export(vec![span("chat first")]).await.unwrap();

    assert_eq!(lines(&path).len(), 1);
}

#[tokio::test]
async fn a_file_for_each_run_has_no_name_until_a_run_starts() {
    let scratch = Scratch::new("unnamed");
    let exporter = spans_to(&Sink::new(FileTarget::EachRun {
        directory: scratch.directory(),
    }));

    let refused = exporter.export(vec![span("chat first")]).await;

    assert_eq!(
        refused.unwrap_err().to_string(),
        "Operation failed: no run has started, so the telemetry has no file to go to"
    );
    assert_eq!(std::fs::read_dir(scratch.directory()).unwrap().count(), 0);
}

#[tokio::test]
async fn a_run_id_that_would_be_a_path_has_no_file_and_the_run_after_it_has_its_own() {
    let scratch = Scratch::new("separator");
    let inner = scratch.path("inner");
    std::fs::create_dir_all(&inner).unwrap();
    let sink = Sink::new(FileTarget::EachRun {
        directory: inner.clone(),
    });
    let exporter = spans_to(&sink);

    let mut refused = Vec::new();
    for run in ["../escaped", "with\0nul"] {
        sink.start(&id(run));
        refused.push(
            exporter
                .export(vec![span("chat first")])
                .await
                .unwrap_err()
                .to_string(),
        );
    }
    sink.start(&id(".."));
    exporter.export(vec![span("chat dots")]).await.unwrap();

    assert_eq!(
        refused,
        [
            "Operation failed: the run id \"../escaped\" can't be part of the name of the \
             run's telemetry file: it holds a `/`",
            "Operation failed: the run id \"with\\0nul\" can't be part of the name of the \
             run's telemetry file: it holds a NUL",
        ]
    );
    assert_eq!(
        std::fs::read_dir(scratch.directory()).unwrap().count(),
        1,
        "nothing was written beside the directory the target names"
    );
    assert_eq!(
        lines(&inner.join("lablet-...otlp.jsonl")).len(),
        1,
        "a name that's the run id between two others is a file's, whatever the id"
    );
}

#[tokio::test]
async fn a_file_that_cannot_be_written_is_an_error_that_names_it_and_is_tried_again() {
    let scratch = Scratch::new("missing-directory");
    let directory = scratch.path("not-made-yet");
    let sink = Sink::new(FileTarget::EachRun {
        directory: directory.clone(),
    });
    let exporter = spans_to(&sink);
    sink.start(&id(FIRST));
    let path = directory.join(format!("lablet-{FIRST}.otlp.jsonl"));

    let refused = exporter.export(vec![span("chat first")]).await;
    std::fs::create_dir_all(&directory).unwrap();
    let written = exporter.export(vec![span("chat second")]).await;

    let refused = refused.unwrap_err().to_string();
    assert!(
        refused.starts_with(&format!(
            "Operation failed: {} couldn't be written: ",
            path.display()
        )),
        "{refused}"
    );
    written.unwrap();
    assert_eq!(Exported::read(&path).unwrap().spans[0].name, "chat second");
}

#[tokio::test]
async fn standard_error_takes_a_line_as_a_file_does() {
    let sink = Sink::new(FileTarget::Stderr);
    sink.start(&id(FIRST));

    spans_to(&sink).export(Vec::new()).await.unwrap();
}

// What a line holds

#[tokio::test]
async fn an_export_of_spans_is_one_line_of_otlp_json() {
    let scratch = Scratch::new("spans-line");
    let path = scratch.path("runs.otlp.jsonl");
    let exporter = spans_to(&Sink::new(FileTarget::Path(path.clone())));

    exporter
        .export(vec![span("chat scripted-1"), span("chat scripted-2")])
        .await
        .unwrap();

    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        written.matches('\n').count(),
        1,
        "two spans of one export are one line"
    );
    assert!(written.ends_with("}\n"));
    assert!(written.starts_with("{\"resourceSpans\":[{"), "{written}");
    assert!(!written.contains(": "), "the JSON is compact");
    for as_the_mapping_writes_it in [
        "\"traceId\":\"abababababababababababababababab\"",
        "\"spanId\":\"cdcdcdcdcdcdcdcd\"",
        "\"parentSpanId\":\"efefefefefefefef\"",
        "\"startTimeUnixNano\":\"1790000000005000000\"",
        "\"endTimeUnixNano\":\"1790000000255000000\"",
        "{\"key\":\"lablet.test.turn\",\"value\":{\"intValue\":\"2\"}}",
        "{\"key\":\"lablet.test.ratio\",\"value\":{\"doubleValue\":0.5}}",
        "{\"key\":\"lablet.test.flag\",\"value\":{\"boolValue\":true}}",
    ] {
        assert!(
            written.contains(as_the_mapping_writes_it),
            "{as_the_mapping_writes_it} isn't in {written}"
        );
    }
}

#[tokio::test]
async fn a_span_is_read_back_as_it_was_exported_with_its_resource_and_its_scope() {
    let scratch = Scratch::new("spans-read");
    let path = scratch.path("runs.otlp.jsonl");
    let spans = spans_to(&Sink::new(FileTarget::Path(path.clone())));

    spans.export(vec![span("chat scripted-1")]).await.unwrap();

    let exported = Exported::read(&path).unwrap();
    assert!(exported.records.is_empty());
    assert_eq!(exported.spans.len(), 1);
    let read = &exported.spans[0];
    assert_eq!(
        serde_json::to_value(&read.resource).unwrap(),
        json!({ "service.name": "lablet", "team": "evals" })
    );
    assert_eq!(read.scope.name, "lablet");
    assert_eq!(read.scope.version, "0.1.0");
    assert_eq!(read.scope.schema_url, SCHEMA_URL);
    assert_eq!(read.trace_id, "abababababababababababababababab");
    assert_eq!(read.span_id, "cdcdcdcdcdcdcdcd");
    assert_eq!(read.parent_span_id.as_deref(), Some("efefefefefefefef"));
    assert_eq!(read.flags & 0xff, 1, "the span says it was sampled");
    assert_eq!(read.name, "chat scripted-1");
    assert_eq!(read.kind, ReadKind::Client);
    assert_eq!(read.start_unix_nano, (STARTED_UNIX_MS + 5) * 1_000_000);
    assert_eq!(read.end_unix_nano, (STARTED_UNIX_MS + 255) * 1_000_000);
    assert_eq!(read.duration_ms(), 250);
    assert_eq!(
        serde_json::to_value(&read.attributes).unwrap(),
        json!({
            "lablet.test.turn": 2,
            "lablet.test.model": "scripted-1",
            "lablet.test.ratio": 0.5,
            "lablet.test.flag": true,
            "lablet.test.reasons": ["end_turn"],
        })
    );
    assert_eq!(read.events.len(), 1);
    assert_eq!(read.events[0].name, "lablet.retry");
    assert_eq!(
        read.events[0].time_unix_nano,
        (STARTED_UNIX_MS + 255) * 1_000_000
    );
    assert_eq!(
        serde_json::to_value(&read.events[0].attributes).unwrap(),
        json!({ "lablet.test.attempt": 1 })
    );
    assert_eq!(read.status, ReadStatus::Error("529 overloaded".to_owned()));
}

#[tokio::test]
async fn an_export_of_log_records_is_one_line_and_is_read_back_as_it_was_exported() {
    let scratch = Scratch::new("records-line");
    let path = scratch.path("runs.otlp.jsonl");
    let records = records_to(&Sink::new(FileTarget::Path(path.clone())));

    export_records(
        &records,
        &["gen_ai.client.operation.exception", "lablet.run"],
    )
    .await
    .unwrap();

    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(written.matches('\n').count(), 1);
    assert!(written.starts_with("{\"resourceLogs\":[{"), "{written}");
    let exported = Exported::read(&path).unwrap();
    assert!(exported.spans.is_empty());
    let names: Vec<_> = exported
        .records
        .iter()
        .map(|record| record.event_name.as_str())
        .collect();
    assert_eq!(names, ["gen_ai.client.operation.exception", "lablet.run"]);
    let read = &exported.records[0];
    assert_eq!(
        serde_json::to_value(&read.resource).unwrap(),
        json!({ "service.name": "lablet", "team": "evals" })
    );
    assert_eq!(read.scope.name, "lablet");
    assert_eq!(read.scope.schema_url, SCHEMA_URL);
    assert_eq!(read.severity_number, 13);
    assert_eq!(read.severity_text, "WARN");
    assert_eq!(read.time_unix_nano, (STARTED_UNIX_MS + 255) * 1_000_000);
    assert_eq!(read.observed_time_unix_nano, read.time_unix_nano);
    assert_eq!(read.trace_id, "abababababababababababababababab");
    assert_eq!(read.span_id, "cdcdcdcdcdcdcdcd");
    assert_eq!(read.flags, 1);
    assert_eq!(
        serde_json::to_value(&read.attributes).unwrap(),
        json!({ "lablet.test.turn": 2 })
    );
    assert_eq!(read.body, None);
}
