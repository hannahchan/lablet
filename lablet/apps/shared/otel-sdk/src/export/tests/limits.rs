//! The sampler, the span limits and the batch processors are what the
//! environment the seam read says, and only that: the SDK's own reading of
//! the process environment decides none of them, which a child process
//! whose environment says otherwise holds, since a test can't set a
//! variable of its own process.

use std::collections::BTreeSet;
use std::process::{Command, Stdio};
use std::time::Duration;

use lablet_conformance::otlp::{Exported, ReadError};
use lablet_test_support::Scratch;
use opentelemetry::trace::{Span as _, Tracer as _};
use opentelemetry::{Context, KeyValue};
use serde_json::json;

use super::harness::{
    CONTENT, CONTENT_PER_RUN, RUN, Records, SPANS_PER_RUN, Settings, WIDE, after, built, emit_run,
    run, sdk_of,
};
use crate::export::{FileTarget, Telemetry};
use crate::otel_env::Sdk;

/// A telemetry exporting to `path`, with the SDK's settings `sdk`.
fn to_file(scratch: &Scratch, path: &std::path::Path, sdk: Sdk) -> Telemetry {
    built(Settings {
        target: Some(FileTarget::Path(path.to_owned())),
        sdk,
        ..Settings::in_scratch(scratch)
    })
}

/// Emits beside a run a span with six attributes and two events.
fn several(telemetry: &Telemetry) {
    let tracer = telemetry.tracer();
    let mut span = tracer
        .span_builder("chat scripted-1")
        .with_start_time(after(0))
        .with_attributes((0..6).map(|n| KeyValue::new(format!("lablet.test.{n}"), i64::from(n))))
        .start_with_context(&tracer, &Context::new());
    span.add_event_with_timestamp("lablet.retry", after(1), Vec::new());
    span.add_event_with_timestamp("lablet.retry", after(2), Vec::new());
    span.end_with_timestamp(after(3));
}

#[tokio::test]
async fn the_environments_sampler_decides_which_spans_are_kept() {
    let scratch = Scratch::new("limits-sampler");
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = to_file(
        &scratch,
        &path,
        sdk_of(&[("OTEL_TRACES_SAMPLER", "always_off")]),
    );

    run(&telemetry, RUN, Records::Captured).await.unwrap();
    telemetry.shutdown().await.unwrap();

    let exported = Exported::read(&path).unwrap();
    assert!(exported.spans.is_empty(), "{:?}", exported.spans);
    assert_eq!(exported.records_of(CONTENT).len(), CONTENT_PER_RUN);
    assert_eq!(exported.records_of(WIDE).len(), 1);
}

#[tokio::test]
async fn the_environments_span_limits_cut_a_span() {
    let scratch = Scratch::new("limits-cut");
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = to_file(
        &scratch,
        &path,
        sdk_of(&[("OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT", "1")]),
    );

    several(&telemetry);
    telemetry.flush_leftovers().await.unwrap();

    // The reader refuses a span the SDK dropped anything from, and names
    // what was dropped, so its refusal is what says the cut happened.
    let refused = Exported::read(&path).unwrap_err();
    let ReadError::Line { reason, .. } = &refused else {
        panic!("{refused}");
    };
    assert_eq!(
        reason,
        "the span `chat scripted-1` has a count of dropped attributes, which the reader doesn't \
         carry"
    );
    telemetry.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_environments_batch_size_reaches_the_processors() {
    let scratch = Scratch::new("limits-batch");
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = to_file(
        &scratch,
        &path,
        sdk_of(&[
            ("OTEL_BSP_MAX_EXPORT_BATCH_SIZE", "1"),
            ("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE", "1"),
        ]),
    );

    run(&telemetry, RUN, Records::Captured).await.unwrap();
    telemetry.shutdown().await.unwrap();

    let exported = Exported::read(&path).unwrap();
    assert_eq!(exported.spans.len(), SPANS_PER_RUN);
    assert_eq!(exported.records.len(), 1 + CONTENT_PER_RUN + 1);
    assert_eq!(
        exported.lines,
        exported.spans.len() + exported.records.len(),
        "one span or one record to a line"
    );
}

/// Set in the environment of the child process the test below starts, to
/// the file the child exports to.
const CHILD: &str = "LABLET_TEST_LIMITS_CHILD";

/// What the child's process environment sets, each to what would change
/// the export if the SDK read it: no span sampled, every limit 1, and each
/// processor holding one span or record and exporting them one at a time.
const PROCESS_ENVIRONMENT: [(&str, &str); 13] = [
    ("OTEL_TRACES_SAMPLER", "always_off"),
    ("OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT", "1"),
    ("OTEL_SPAN_EVENT_COUNT_LIMIT", "1"),
    ("OTEL_SPAN_LINK_COUNT_LIMIT", "1"),
    ("OTEL_EVENT_ATTRIBUTE_COUNT_LIMIT", "1"),
    ("OTEL_LINK_ATTRIBUTE_COUNT_LIMIT", "1"),
    ("OTEL_ATTRIBUTE_COUNT_LIMIT", "1"),
    ("OTEL_BSP_MAX_QUEUE_SIZE", "1"),
    ("OTEL_BSP_MAX_EXPORT_BATCH_SIZE", "1"),
    ("OTEL_BSP_SCHEDULE_DELAY", "1"),
    ("OTEL_BLRP_MAX_QUEUE_SIZE", "1"),
    ("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE", "1"),
    ("OTEL_BLRP_SCHEDULE_DELAY", "1"),
];

/// What the child's seam reads, beside nothing: delays no run reaches, so
/// its processors export only when they're flushed, and a delay the SDK
/// read from the process environment in their place would split the
/// export across the pauses the child makes.
const SEAM: [(&str, &str); 2] = [
    ("OTEL_BSP_SCHEDULE_DELAY", "3600000"),
    ("OTEL_BLRP_SCHEDULE_DELAY", "3600000"),
];

/// The child's side, which does nothing unless the test below started it:
/// one run, and beside it a span with several attributes and events,
/// exported to the file the parent named, with the SDK's settings of an
/// environment that sets the delays alone, and a pause before the run and
/// one before its end.
#[tokio::test]
async fn a_child_process_exports_a_run_with_the_settings_of_an_environment_that_sets_the_delays_alone()
 {
    let Some(path) = std::env::var_os(CHILD) else {
        return;
    };
    let scratch = Scratch::new("limits-child");
    let telemetry = to_file(&scratch, path.as_ref(), sdk_of(&SEAM));

    several(&telemetry);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let wide = emit_run(&telemetry, RUN, Records::Captured);
    tokio::time::sleep(Duration::from_millis(50)).await;
    telemetry.flush(wide).await.unwrap();
    telemetry.shutdown().await.unwrap();
}

/// Every setting is stated on the SDK, so the process environment, which
/// the SDK's builders read into their defaults, decides none: the span is
/// exported whole, the run's spans are sampled, and each processor exports
/// everything in one batch, at the flush. The reader refuses a span the SDK
/// dropped anything from, so a limit that held would fail the read.
#[test]
fn the_crates_own_reading_of_the_process_environment_decides_nothing() {
    let scratch = Scratch::new("limits-environment");
    let path = scratch.at("runs.otlp.jsonl");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "export::tests::limits::a_child_process_exports_a_run_with_the_settings_of_an_environment_that_sets_the_delays_alone",
            "--test-threads=1",
        ])
        .env(CHILD, &path)
        .envs(PROCESS_ENVIRONMENT)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&child.stdout);
    assert!(
        child.status.success() && stdout.contains("test result: ok. 1 passed;"),
        "the child failed or ran other than one test:\n{stdout}\n{}",
        String::from_utf8_lossy(&child.stderr)
    );

    let exported = Exported::read(&path).unwrap();
    let chats = exported.spans_of("chat");
    assert_eq!(chats.len(), 3, "the span beside the run's two chats");
    let several = chats
        .iter()
        .find(|span| span.attributes.contains_key("lablet.test.5"))
        .expect("the span with several attributes");
    assert_eq!(
        serde_json::to_value(&several.attributes).unwrap(),
        json!({
            "lablet.test.0": 0,
            "lablet.test.1": 1,
            "lablet.test.2": 2,
            "lablet.test.3": 3,
            "lablet.test.4": 4,
            "lablet.test.5": 5,
        }),
        "every attribute, whatever the count limit says"
    );
    assert_eq!(several.events.len(), 2, "every event, whatever the limit");
    assert_eq!(
        exported.spans.len(),
        1 + SPANS_PER_RUN,
        "every span, sampled"
    );
    assert_eq!(exported.records.len(), 1 + CONTENT_PER_RUN + 1);
    let spans: BTreeSet<usize> = exported.spans.iter().map(|span| span.line).collect();
    let records: BTreeSet<usize> = exported.records.iter().map(|record| record.line).collect();
    assert_eq!(spans.len(), 1, "every span in one batch: {spans:?}");
    assert_eq!(records.len(), 1, "every record in one batch: {records:?}");
}
