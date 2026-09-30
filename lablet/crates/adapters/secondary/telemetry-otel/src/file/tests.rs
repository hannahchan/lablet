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
            at: after(250),
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
        directory: scratch.path().to_owned(),
    });
    let exporter = spans_to(&sink);

    sink.start(&id(FIRST));
    exporter.export(vec![span("chat first")]).await.unwrap();
    exporter.export(vec![span("chat first")]).await.unwrap();
    sink.start(&id(SECOND));
    exporter.export(vec![span("chat second")]).await.unwrap();

    let first = Exported::read(&scratch.at(&format!("lablet-{FIRST}.otlp.jsonl"))).unwrap();
    let second = Exported::read(&scratch.at(&format!("lablet-{SECOND}.otlp.jsonl"))).unwrap();
    assert_eq!(first.lines, 2);
    let names: Vec<_> = first.spans.iter().map(|span| span.name.as_str()).collect();
    assert_eq!(names, ["chat first", "chat first"]);
    assert_eq!(second.lines, 1);
    assert_eq!(second.spans[0].name, "chat second");
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn one_path_is_appended_to_by_every_run_and_keeps_what_it_held() {
    let scratch = Scratch::new("one-path");
    let path = scratch.at("runs.otlp.jsonl");
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
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn one_path_needs_no_run_to_have_started() {
    let scratch = Scratch::new("no-run-yet");
    let path = scratch.at("runs.otlp.jsonl");
    let exporter = spans_to(&Sink::new(FileTarget::Path(path.clone())));

    exporter.export(vec![span("chat first")]).await.unwrap();

    assert_eq!(lines(&path).len(), 1);
}

#[tokio::test]
async fn a_file_for_each_run_has_no_name_until_a_run_starts() {
    let scratch = Scratch::new("unnamed");
    let exporter = spans_to(&Sink::new(FileTarget::EachRun {
        directory: scratch.path().to_owned(),
    }));

    let refused = exporter.export(vec![span("chat first")]).await;

    assert_eq!(
        refused.unwrap_err().to_string(),
        "Operation failed: no run has started, so the telemetry has no file to go to"
    );
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn a_run_id_that_would_be_a_path_has_no_file_and_the_run_after_it_has_its_own() {
    let scratch = Scratch::new("separator");
    let inner = scratch.at("inner");
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
        std::fs::read_dir(scratch.path()).unwrap().count(),
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
async fn a_file_that_cannot_be_written_is_an_error_that_names_no_path_and_is_tried_again() {
    let scratch = Scratch::new("missing-directory");
    let directory = scratch.at("not-made-yet");
    let sink = Sink::new(FileTarget::EachRun {
        directory: directory.clone(),
    });
    let exporter = spans_to(&sink);
    sink.start(&id(FIRST));
    let path = directory.join(format!("lablet-{FIRST}.otlp.jsonl"));

    let refused = exporter.export(vec![span("chat first")]).await;
    std::fs::create_dir_all(&directory).unwrap();
    let written = exporter.export(vec![span("chat second")]).await;

    assert_eq!(
        refused.unwrap_err().to_string(),
        "Operation failed: the telemetry file couldn't be written: No such file or directory \
         (os error 2)"
    );
    written.unwrap();
    assert_eq!(Exported::read(&path).unwrap().spans[0].name, "chat second");
}

#[tokio::test]
async fn a_file_that_was_moved_between_two_runs_is_not_written_to_by_the_second() {
    let scratch = Scratch::new("moved");
    let (path, moved) = (scratch.at("runs.otlp.jsonl"), scratch.at("first.jsonl"));
    let sink = Sink::new(FileTarget::Path(path.clone()));
    let exporter = spans_to(&sink);

    sink.start(&id(FIRST));
    exporter.export(vec![span("chat first")]).await.unwrap();
    std::fs::rename(&path, &moved).unwrap();
    sink.start(&id(SECOND));
    exporter.export(vec![span("chat second")]).await.unwrap();
    exporter.export(vec![span("chat second")]).await.unwrap();

    let (at_the_path, moved) = (
        Exported::read(&path).unwrap(),
        Exported::read(&moved).unwrap(),
    );
    assert_eq!(moved.lines, 1);
    assert_eq!(moved.spans[0].name, "chat first");
    assert_eq!(at_the_path.lines, 2);
    assert!(
        at_the_path
            .spans
            .iter()
            .all(|span| span.name == "chat second")
    );
}

// A write that fails partway

/// What a file holds, which outlives the writers that were opened to it.
type Written = Arc<Mutex<Vec<u8>>>;

/// A file that fails once it has taken `room` bytes, and takes a write
/// that's longer than its room for as much as it has room for.
struct Fills {
    written: Written,
    room: usize,
}

impl io::Write for Fills {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let taken = bytes.len().min(self.room);
        if taken == 0 {
            return Err(io::Error::other("no space left on the device"));
        }
        self.room -= taken;
        self.written
            .lock()
            .unwrap()
            .extend_from_slice(&bytes[..taken]);
        Ok(taken)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A file at `runs.otlp.jsonl` as the sink holds one, and what it holds.
struct Appended {
    held: Option<Held<Fills>>,
    written: Written,
}

impl Appended {
    fn new() -> Self {
        Self {
            held: None,
            written: Written::default(),
        }
    }

    /// Appends `line`. A file that has to be opened for it has `room`.
    fn append_to(&mut self, path: &str, line: &str, room: usize) -> io::Result<()> {
        let written = Arc::clone(&self.written);
        Held::append(&mut self.held, Path::new(path), line, |_| {
            Ok(Fills { written, room })
        })
    }

    fn append(&mut self, line: &str, room: usize) -> io::Result<()> {
        self.append_to("runs.otlp.jsonl", line, room)
    }

    fn text(&self) -> String {
        String::from_utf8(self.written.lock().unwrap().clone()).unwrap()
    }
}

const ONE: &str = "{\"resourceSpans\":[1]}\n";
const TWO: &str = "{\"resourceSpans\":[2]}\n";
const THREE: &str = "{\"resourceSpans\":[3]}\n";

#[test]
fn a_line_that_follows_part_of_another_begins_a_line_of_its_own_and_reads_back() {
    let mut file = Appended::new();

    file.append(ONE, ONE.len() + 9).unwrap();
    let failed = file.append(TWO, 0).unwrap_err();
    file.append(THREE, usize::MAX).unwrap();
    file.append(ONE, 0).unwrap();

    assert_eq!(failed.to_string(), "no space left on the device");
    assert_eq!(
        file.text(),
        format!("{ONE}{{\"resourc\n{THREE}{ONE}"),
        "what was written of the line that failed is a line of its own, and the only one"
    );
    let text = file.text();
    let read: Vec<_> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect();
    assert_eq!(
        read,
        [
            Some(json!({ "resourceSpans": [1] })),
            None,
            Some(json!({ "resourceSpans": [3] })),
            Some(json!({ "resourceSpans": [1] })),
        ]
    );
}

#[test]
fn a_write_that_failed_before_it_wrote_anything_leaves_no_line_behind() {
    let mut file = Appended::new();

    file.append(ONE, ONE.len()).unwrap();
    file.append(TWO, 0).unwrap_err();
    file.append(THREE, usize::MAX).unwrap();

    assert_eq!(file.text(), format!("{ONE}{THREE}"));
}

#[test]
fn a_file_that_failed_is_opened_again_and_one_that_did_not_is_kept_open() {
    let mut file = Appended::new();

    file.append(ONE, ONE.len() + TWO.len()).unwrap();
    file.append(TWO, 0).unwrap();
    assert!(
        file.append(THREE, 0).is_err(),
        "the file that's open is full"
    );
    file.append(THREE, THREE.len()).unwrap();

    assert_eq!(file.text(), format!("{ONE}{TWO}{THREE}"));
}

#[test]
fn a_newline_that_could_not_be_written_is_written_before_the_line_after() {
    let mut file = Appended::new();

    file.append(ONE, 4).unwrap_err();
    file.append(TWO, 0).unwrap_err();
    file.append(THREE, usize::MAX).unwrap();

    assert_eq!(file.text(), format!("{{\"re\n{THREE}"));
}

#[test]
fn a_file_that_was_let_go_of_still_ends_in_the_part_of_a_line_it_was_left_with() {
    let mut file = Appended::new();

    file.append(ONE, 4).unwrap_err();
    file.held.as_mut().unwrap().let_go();
    file.append(TWO, usize::MAX).unwrap();

    assert_eq!(file.text(), format!("{{\"re\n{TWO}"));
}

#[test]
fn part_of_a_line_in_one_file_puts_no_newline_in_another() {
    let mut file = Appended::new();

    file.append_to("first.otlp.jsonl", ONE, 4).unwrap_err();
    file.append_to("second.otlp.jsonl", TWO, usize::MAX)
        .unwrap();

    assert_eq!(file.text(), format!("{{\"re{TWO}"));
}

#[test]
fn a_file_that_takes_nothing_and_says_so_without_an_error_is_an_error() {
    struct TakesNothing;

    impl io::Write for TakesNothing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Ok(0)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut held = None;

    let failed = Held::append(&mut held, Path::new("runs.otlp.jsonl"), ONE, |_| {
        Ok(TakesNothing)
    });

    assert_eq!(failed.unwrap_err().kind(), io::ErrorKind::WriteZero);
}

#[test]
fn a_write_that_was_interrupted_is_made_again() {
    struct Interrupted {
        written: Vec<u8>,
        interrupts: bool,
    }

    impl io::Write for Interrupted {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.interrupts = !self.interrupts;
            if !self.interrupts {
                return Err(io::ErrorKind::Interrupted.into());
            }
            self.written.extend_from_slice(&bytes[..1]);
            Ok(1)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut file = Interrupted {
        written: Vec::new(),
        interrupts: true,
    };
    let mut torn = false;

    whole(&mut file, ONE, &mut torn).unwrap();

    assert_eq!(file.written, ONE.as_bytes());
    assert!(!torn);
}

/// Standard error can't be handed a writer of the test's, so what it
/// writes is held here, through the function both destinations write with.
#[test]
fn a_line_put_after_part_of_another_begins_a_line_of_its_own() {
    let written = Written::default();
    let mut to = Fills {
        written: Arc::clone(&written),
        room: ONE.len() + 4,
    };
    let mut torn = false;

    put(&mut to, ONE, &mut torn).unwrap();
    put(&mut to, TWO, &mut torn).unwrap_err();
    assert!(torn);
    put(&mut to, THREE, &mut torn).unwrap_err();
    assert!(torn, "a newline that couldn't be written is still owed");
    to.room = usize::MAX;
    put(&mut to, THREE, &mut torn).unwrap();

    assert!(!torn);
    assert_eq!(
        String::from_utf8(written.lock().unwrap().clone()).unwrap(),
        format!("{ONE}{{\"re\n{THREE}")
    );
}

/// What reaches standard error can't be read back here, so this holds only
/// what the sink says of it.
#[tokio::test]
async fn standard_error_stays_the_destination_when_a_run_starts_and_is_left_whole() {
    let sink = Sink::new(FileTarget::Stderr);
    sink.start(&id(FIRST));

    spans_to(&sink).export(Vec::new()).await.unwrap();

    assert!(!stderr_is_torn(&sink));
}

fn stderr_is_torn(sink: &Sink) -> bool {
    match sink.open.lock().unwrap().destination {
        Destination::Stderr { torn } => torn,
        ref other => panic!("{other:?} isn't standard error"),
    }
}

/// Standard error is one stream across runs, so part of a line one run
/// left there is ended by the next run's first line.
#[tokio::test]
async fn standard_error_ends_the_part_of_a_line_a_failed_write_left_whatever_run_writes_next() {
    let sink = Sink::new(FileTarget::Stderr);
    sink.open.lock().unwrap().destination = Destination::Stderr { torn: true };

    sink.start(&id(SECOND));

    assert!(stderr_is_torn(&sink), "a run's start leaves it as it was");
    spans_to(&sink).export(Vec::new()).await.unwrap();
    assert!(!stderr_is_torn(&sink));
}

// What a line holds

#[tokio::test]
async fn an_export_of_spans_is_one_line_of_otlp_json() {
    let scratch = Scratch::new("spans-line");
    let path = scratch.at("runs.otlp.jsonl");
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
    let path = scratch.at("runs.otlp.jsonl");
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
        (STARTED_UNIX_MS + 250) * 1_000_000,
        "the event is timed as it happened, not as the span ended"
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
    let path = scratch.at("runs.otlp.jsonl");
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
