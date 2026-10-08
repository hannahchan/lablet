//! How a run's end orders the transcript and the wide event, as the
//! logger provider a host hands in sees it.

use std::future::{Future, ready};
use std::path::Path;
use std::sync::{Arc, Mutex};

use lablet_run::telemetry::generated::{LabletRun, key};
use lablet_test_support::Scratch;
use opentelemetry::logs::AnyValue;
use opentelemetry::trace::noop::{NoopTextMapPropagator, NoopTracerProvider};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogBatch, LogExporter, SdkLoggerProvider};
use serde_json::{Value, json};

use crate::{Config, Format, Otel, RunId, RunRequest};

/// A response that comes at once.
const ENDS: &str = "
- response:
    content:
      - text: Nothing to fix.
    finish: end_turn
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
/// logger provider the host hands in exports each record the moment it's
/// emitted, so a wide event emitted before the transcript's write would
/// reach the exporter then.
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
    let looking = Looking::default();
    let otel = Otel::new(
        NoopTracerProvider::new(),
        SdkLoggerProvider::builder()
            .with_simple_exporter(looking.clone())
            .build(),
        NoopTextMapPropagator::new(),
    );
    let mut lablet = crate::build(config, otel).await.unwrap();

    lablet.run(request("run-a")).await;
    lablet.shutdown().await;

    assert_eq!(
        *looking.0.lock().unwrap(),
        [(transcript.display().to_string(), true)],
        "the wide event reached the exporter once, after its transcript was written"
    );
}
