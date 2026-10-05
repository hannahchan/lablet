//! Every trace is sampled and every span keeps what the loop gives it,
//! whatever the environment says of sampling and span limits, held from a
//! child process whose environment says otherwise, since a test can't set
//! a variable of its own process.

use std::process::{Command, Stdio};

use lablet_conformance::otlp::Exported;
use lablet_test_support::Scratch;
use opentelemetry::trace::{Span as _, Tracer as _};
use opentelemetry::{Context, KeyValue};
use serde_json::json;

use super::harness::{RUN, Records, Settings, WIDE, after, built, emit_run};
use crate::export::FileTarget;

/// Set in the environment of the child process the test below starts, to
/// the file the child exports to.
const CHILD: &str = "LABLET_TEST_LIMITS_CHILD";

/// The child's side, which does nothing unless the test below started it:
/// one run, and beside it a span with several attributes and events,
/// exported to the file the parent named.
#[tokio::test]
async fn a_child_process_exports_a_span_with_several_attributes_and_events() {
    let Some(path) = std::env::var_os(CHILD) else {
        return;
    };
    let scratch = Scratch::new("limits-child");
    let telemetry = built(Settings {
        target: Some(FileTarget::Path(path.into())),
        ..Settings::in_scratch(&scratch)
    });
    let tracer = telemetry.tracer();

    let mut span = tracer
        .span_builder("chat scripted-1")
        .with_start_time(after(0))
        .with_attributes((0..6).map(|n| KeyValue::new(format!("lablet.test.{n}"), i64::from(n))))
        .start_with_context(&tracer, &Context::new());
    span.add_event_with_timestamp("lablet.retry", after(1), Vec::new());
    span.add_event_with_timestamp("lablet.retry", after(2), Vec::new());
    span.end_with_timestamp(after(3));
    let wide = emit_run(&telemetry, RUN, Records::Exception);
    telemetry.flush(wide).await.unwrap();
    telemetry.shutdown().await.unwrap();
}

/// The sampler is always on and the limits are set as lablet means them,
/// after the SDK has read its environment, so a sampler the environment
/// turns off and limits it lowers change nothing: the span is exported
/// whole, and so is the run. The reader refuses a span the SDK dropped
/// anything from, so a limit that held would fail the read.
#[test]
fn the_environments_sampler_and_span_limits_cannot_drop_a_span_or_cut_it() {
    let scratch = Scratch::new("limits-environment");
    let path = scratch.at("runs.otlp.jsonl");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "export::tests::limits::a_child_process_exports_a_span_with_several_attributes_and_events",
            "--test-threads=1",
        ])
        .env(CHILD, &path)
        .env("OTEL_TRACES_SAMPLER", "always_off")
        .env("OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT", "1")
        .env("OTEL_SPAN_EVENT_COUNT_LIMIT", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "the child failed:\n{}\n{}",
        String::from_utf8_lossy(&child.stdout),
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
    for span in &exported.spans {
        assert_eq!(span.flags & 0xff, 1, "sampled: {span:?}");
    }
    assert_eq!(exported.spans_of("invoke_agent").len(), 1, "the run whole");
    assert_eq!(exported.records_of(WIDE).len(), 1);
}
