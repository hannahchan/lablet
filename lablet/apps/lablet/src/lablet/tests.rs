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
use lablet_run::telemetry::generated::{LabletInvokeAgent, LabletRun, key};
use lablet_test_support::Scratch;
use opentelemetry::InstrumentationScope;
use opentelemetry::logs::AnyValue;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogBatch, LogExporter};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use serde_json::{Value, json};

use crate::{CancelHandle, Config, Format, RunId, RunRequest, build};
use lablet_otel_sdk::export::Telemetry;
use lablet_otel_sdk::otel_env::OtelEnv;

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

/// What the `telemetry` section states for a run's telemetry to go to the
/// file at `path` alone.
fn to_file(path: &Path) -> Value {
    json!({ "telemetry": { "file": { "path": path }, "otlp": { "enabled": false } } })
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

/// The spans a dropped run had open are exported before the next run
/// starts, in an export of their own: what the processor holds is flushed
/// before the next run reads the clock, rather than going out with the
/// next run's spans.
#[tokio::test]
async fn a_dropped_runs_spans_are_written_before_the_next_run_reads_its_clock() {
    let scratch = Scratch::new("dropped-run");
    let path = scratch.at("runs.otlp.jsonl");
    let config = config_of(&scratch, STALLS, &to_file(&path));
    let mut lablet = crate::build(config).await.unwrap();

    let dropped = tokio::time::timeout(Duration::from_millis(50), lablet.run(request("dropped")));
    assert!(dropped.await.is_err(), "the run never answers");
    let fired = CancelHandle::new();
    fired.cancel();
    lablet.run(request("next").cancellation(fired)).await;
    lablet.shutdown().await;

    let exported = Exported::read(&path).unwrap();
    let roots = exported.spans_of(LabletInvokeAgent::GEN_AI_OPERATION_NAME);
    assert_eq!(roots.len(), 2, "{exported:?}");
    let abandoned = roots
        .iter()
        .find(|root| root.attributes.is_empty())
        .unwrap();
    let (of_abandoned, of_next): (Vec<_>, Vec<_>) = exported
        .spans
        .iter()
        .partition(|span| span.trace_id == abandoned.trace_id);
    assert!(!of_next.is_empty());
    let last_abandoned = of_abandoned.iter().map(|span| span.line).max().unwrap();
    let first_next = of_next.iter().map(|span| span.line).min().unwrap();
    assert!(
        last_abandoned < first_next,
        "the abandoned run's spans went out with the next run's: lines {last_abandoned} and \
         {first_next}"
    );
    assert_eq!(exported.records_of(LabletRun::NAME).len(), 1);
    assert_eq!(traces(&exported).len(), 2);
}

/// A run's end waits on a collector that accepts and never answers for one
/// flush bound of the SDK's, since it flushes once, and the file holds the
/// run and its wide event by then.
#[tokio::test]
async fn a_run_end_with_a_collector_that_never_answers_waits_one_flush_bound() {
    let receiver = Receiver::start(Mode::NeverAnswers).await;
    let scratch = Scratch::new("run-end-never-answered");
    let path = scratch.at("runs.otlp.jsonl");
    let config = config_of(
        &scratch,
        ENDS,
        &json!({ "telemetry": {
            "otlp": { "endpoint": receiver.grpc_endpoint(), "protocol": "grpc" },
            "file": { "path": path },
        } }),
    );
    let mut lablet = crate::build(config).await.unwrap();

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
        &json!({
            "run": { "transcript_path": transcript },
            "telemetry": { "otlp": { "enabled": false } },
        }),
    );
    let prepared = lablet_prepare::prepare(&config, &lablet_config::environment).unwrap();
    let tools = lablet_tools_wiring::tools(&config, &prepared)
        .await
        .unwrap();
    let (_, wiring) = prepared.split().unwrap();
    let one_at_a_time =
        |name: &str| (name == "OTEL_BLRP_MAX_EXPORT_BATCH_SIZE").then(|| OsString::from("1"));
    let looking = Looking::default();
    let telemetry = Telemetry::builder(InstrumentationScope::builder("lablet").build())
        .sdk(OtelEnv::read(&one_at_a_time).sdk)
        .exporting_to(InMemorySpanExporter::default(), looking.clone())
        .build()
        .unwrap();
    let mut lablet = build::wire(&config, wiring, tools, telemetry);

    lablet.run(request("run-a")).await;
    lablet.shutdown().await;

    assert_eq!(
        *looking.0.lock().unwrap(),
        [(transcript.display().to_string(), true)],
        "the wide event reached the exporter once, after its transcript was written"
    );
}
