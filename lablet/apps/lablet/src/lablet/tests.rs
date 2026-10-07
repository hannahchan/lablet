//! What one run of a `Lablet` leaves for the next, and how its end orders
//! the transcript, the wide event and the flush.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::future::{Future, ready};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lablet_conformance::otlp::Exported;
use lablet_conformance::receiver::{Mode, Receiver};
use lablet_test_support::Scratch;
use opentelemetry::InstrumentationScope;
use opentelemetry::logs::AnyValue;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogBatch, LogExporter};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use serde_json::{Value, json};

use crate::build::build_to;
use crate::export::{FileTarget, Telemetry};
use crate::otel_env::OtelEnv;
use crate::telemetry::generated::{LabletInvokeAgent, LabletRun, key};
use crate::{CancelHandle, Config, Format, RunId, RunRequest};

/// A response that comes at once.
const ENDS: &str = "
- response:
    content:
      - text: Nothing to fix.
    finish: end_turn
";

/// A response that comes after an hour, which no run here waits for.
const STALLS: &str = "
- response:
    content:
      - text: Never heard.
    finish: end_turn
    latency: 1h
";

/// A config of the fake provider playing `script`, with `more` merged in.
fn config_of(scratch: &Scratch, script: &str, more: &Value) -> Config {
    let mut config = json!({
        "model": {
            "provider": "fake",
            "script": scratch.write("script.yaml", script),
            "name": "scripted-1",
        },
        "prompt": { "system": "You fix failing tests." },
    });
    for (key, value) in more.as_object().unwrap() {
        config[key] = value.clone();
    }
    Config::from_str(&config.to_string(), Format::Json).unwrap()
}

fn request(run_id: &str) -> RunRequest {
    RunRequest::new("Fix the failing test.")
        .unwrap()
        .run_id(RunId::new(run_id).unwrap())
        .unwrap()
}

/// The trace ids of everything `exported` holds.
fn traces(exported: &Exported) -> BTreeSet<&str> {
    exported
        .spans
        .iter()
        .map(|span| span.trace_id.as_str())
        .chain(
            exported
                .records
                .iter()
                .map(|record| record.trace_id.as_str()),
        )
        .collect()
}

#[tokio::test]
async fn a_run_whose_future_is_dropped_leaves_its_spans_in_its_own_file_and_none_in_the_next_run_s()
{
    let scratch = Scratch::new("dropped-run");
    let config = Config::from_str(
        &json!({
            "model": {
                "provider": "fake",
                "script": scratch.write("script.yaml", STALLS),
                "name": "scripted-1",
            },
            "prompt": { "system": "You fix failing tests." },
        })
        .to_string(),
        Format::Json,
    )
    .unwrap();
    let directory = scratch.path().to_owned();
    let mut lablet = build_to(config, FileTarget::EachRun { directory })
        .await
        .unwrap();

    let dropped = tokio::time::timeout(Duration::from_millis(50), lablet.run(request("dropped")));
    assert!(dropped.await.is_err(), "the run never answers");
    let fired = CancelHandle::new();
    fired.cancel();
    lablet.run(request("next").cancellation(fired)).await;
    lablet.shutdown().await;

    let read = |run: &str| {
        Exported::read(&scratch.path().join(format!("lablet-{run}.otlp.jsonl"))).unwrap()
    };
    let (abandoned, next) = (read("dropped"), read("next"));
    let roots = abandoned.spans_of(LabletInvokeAgent::GEN_AI_OPERATION_NAME);
    assert_eq!(roots.len(), 1, "{abandoned:?}");
    assert!(
        roots[0].attributes.is_empty(),
        "the abandoned root span was never filled"
    );
    assert!(abandoned.records_of(LabletRun::NAME).is_empty());
    assert_eq!(next.records_of(LabletRun::NAME).len(), 1);
    let (abandoned, next) = (traces(&abandoned), traces(&next));
    assert_eq!(abandoned.len(), 1, "{abandoned:?}");
    assert_eq!(next.len(), 1, "{next:?}");
    assert!(abandoned.is_disjoint(&next));
}

/// A run's end waits on a collector that accepts and never answers for one
/// flush bound of the SDK's, since it flushes once, and the file holds the
/// run and its wide event by then.
#[tokio::test]
async fn a_run_end_with_a_collector_that_never_answers_waits_one_flush_bound() {
    let receiver = Receiver::start(Mode::NeverAnswers).await;
    let scratch = Scratch::new("run-end-never-answered");
    let config = config_of(
        &scratch,
        ENDS,
        &json!({ "telemetry": { "otlp": { "endpoint": receiver.grpc_endpoint() } } }),
    );
    let path = scratch.at("runs.otlp.jsonl");
    let mut lablet = build_to(config, FileTarget::Path(path.clone()))
        .await
        .unwrap();

    let began = Instant::now();
    lablet.run(request("run-a")).await;
    let waited = began.elapsed();

    assert!(
        waited < Duration::from_millis(6_500),
        "the run's end waited {waited:?}, past one flush bound of five seconds"
    );
    let exported = Exported::read(&path).unwrap();
    assert_eq!(
        exported
            .spans_of(LabletInvokeAgent::GEN_AI_OPERATION_NAME)
            .len(),
        1
    );
    assert_eq!(exported.records_of(LabletRun::NAME).len(), 1);
    lablet.shutdown().await;
}

/// Whether the file at `path` holds a whole transcript.
fn whole_at(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|document| document["turns"].is_array())
}

/// A log exporter that keeps, for each wide event it's handed, the
/// transcript path the event names and whether that transcript was whole
/// at that moment.
#[derive(Debug, Clone, Default)]
struct Looking(Arc<Mutex<Vec<(String, bool)>>>);

impl LogExporter for Looking {
    fn export(&self, batch: LogBatch<'_>) -> impl Future<Output = OTelSdkResult> + Send {
        for (record, _) in batch.iter() {
            if record.event_name() != Some(LabletRun::NAME) {
                continue;
            }
            let named = record
                .attributes_iter()
                .find(|(name, _)| name.as_str() == key::LABLET_RUN_TRANSCRIPT_PATH)
                .map(|(_, value)| match value {
                    AnyValue::String(path) => path.as_str().to_owned(),
                    other => panic!("{other:?}"),
                })
                .unwrap();
            let whole = whole_at(Path::new(&named));
            self.0.lock().unwrap().push((named, whole));
        }
        ready(Ok(()))
    }
}

/// The wide event is emitted once the transcript it names is written. The
/// telemetry its end goes through exports each record the moment it's
/// emitted, so a wide event emitted before the transcript's write would
/// reach the exporter then, and not wait for the flush.
#[tokio::test]
async fn the_transcript_a_wide_event_names_is_whole_when_an_exporter_is_handed_the_wide_event() {
    let scratch = Scratch::new("transcript-then-wide");
    let transcript = scratch.at("transcript.json");
    let config = config_of(
        &scratch,
        ENDS,
        &json!({ "run": { "transcript_path": transcript } }),
    );
    let directory = scratch.path().to_owned();
    let mut lablet = build_to(config, FileTarget::EachRun { directory })
        .await
        .unwrap();
    let one_at_a_time =
        |name: &str| (name == "OTEL_BLRP_MAX_EXPORT_BATCH_SIZE").then(|| OsString::from("1"));
    let looking = Looking::default();
    lablet.telemetry = Telemetry::builder(InstrumentationScope::builder("lablet").build())
        .sdk(OtelEnv::read(&one_at_a_time).sdk)
        .exporting_to(InMemorySpanExporter::default(), looking.clone())
        .build()
        .unwrap();

    lablet.run(request("run-a")).await;
    lablet.shutdown().await;

    assert_eq!(
        *looking.0.lock().unwrap(),
        [(transcript.display().to_string(), true)],
        "the wide event reached the exporter once, after its transcript was written"
    );
}
