//! What one run of a `Lablet` leaves for the next.

use std::collections::BTreeSet;
use std::time::Duration;

use lablet_conformance::otlp::Exported;
use lablet_test_support::Scratch;
use serde_json::json;

use crate::build::build_to;
use crate::export::FileTarget;
use crate::telemetry::generated::{LabletInvokeAgent, LabletRun};
use crate::{CancelHandle, Config, Format, RunId, RunRequest};

/// A response that comes after an hour, which no run here waits for.
const STALLS: &str = "
- response:
    content:
      - text: Never heard.
    finish: end_turn
    latency: 1h
";

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
