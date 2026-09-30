//! What a run request names the run's task, experiment and trial: on every
//! record of the run, and in its outcome.

use lablet::{OutcomeDocument, RunId, RunLabels};
use lablet_telemetry_registry::attribute as key;
use serde_json::json;

use crate::harness::{Lab, Traced, request};

const LABELS: [&str; 3] = [
    key::LABLET_TASK_ID,
    key::LABLET_EXPERIMENT_ID,
    key::LABLET_TRIAL,
];

/// A failed attempt and a tool call, so the run has a record of every
/// kind: an exception, content, and the wide event. Its namesakes in other
/// crates differ on purpose: this one states no usage or latency, which the
/// labels don't depend on.
const FAILS_CALLS_ENDS: &str = "
- error: { kind: retryable, message: 529 overloaded }
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: echo one test fails } } }
    finish: tool_use
- response:
    content:
      - text: One test fails.
    finish: end_turn
";

#[tokio::test]
async fn the_labels_of_a_request_are_on_every_record_of_its_run_and_of_no_other() {
    let scratch = Lab::new("labels");
    let config = scratch.config(
        FAILS_CALLS_ENDS,
        json!({
            "run": { "retry_backoff_base": "1ms", "retry_backoff_max": "1ms" },
            "tools": { "builtin": scratch.builtin(&["bash"]) },
            "telemetry": { "capture_content": true },
        }),
    );
    let mut lablet = lablet::build(config).await.unwrap();
    let labels = RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: Some("terse-tool-descriptions".to_owned()),
        trial: Some("3".to_owned()),
    };

    let labelled = lablet
        .run(
            request()
                .run_id(RunId::new("labelled").unwrap())
                .labels(labels.clone()),
        )
        .await;
    let bare = lablet
        .run(request().run_id(RunId::new("bare").unwrap()))
        .await;
    lablet.shutdown().await;

    let exported = scratch.exported();
    let traced = Traced::of(&exported, "labelled");
    assert_eq!(traced.spans.len(), 5, "a root, three attempts and a call");
    let events: Vec<_> = traced
        .records
        .iter()
        .map(|record| record.event_name.as_str())
        .collect();
    for event in [
        "gen_ai.client.operation.exception",
        "gen_ai.client.inference.operation.details",
        "lablet.run",
    ] {
        assert!(events.contains(&event), "{events:?}");
    }
    let of_the_run = traced
        .spans
        .iter()
        .map(|span| (span.name.as_str(), &span.attributes))
        .chain(
            traced
                .records
                .iter()
                .map(|record| (record.event_name.as_str(), &record.attributes)),
        );
    for (signal, attributes) in of_the_run {
        assert_eq!(
            attributes[key::LABLET_TASK_ID],
            json!("fix-failing-test"),
            "{signal}"
        );
        assert_eq!(
            attributes[key::LABLET_EXPERIMENT_ID],
            json!("terse-tool-descriptions"),
            "{signal}"
        );
        assert_eq!(attributes[key::LABLET_TRIAL], json!("3"), "{signal}");
    }
    assert_eq!(labelled.summary.outcome.labels, labels);
    assert_eq!(
        serde_json::to_value(OutcomeDocument::from(labelled.summary.outcome)).unwrap()["labels"],
        json!({
            "task": "fix-failing-test",
            "experiment": "terse-tool-descriptions",
            "trial": "3",
        })
    );

    let traced = Traced::of(&exported, "bare");
    assert_eq!(traced.spans.len(), 5);
    assert_eq!(traced.records.len(), events.len());
    for attributes in traced
        .spans
        .iter()
        .map(|span| &span.attributes)
        .chain(traced.records.iter().map(|record| &record.attributes))
    {
        for label in LABELS {
            assert!(!attributes.contains_key(label), "{label}: {attributes:?}");
        }
    }
    assert_eq!(bare.summary.outcome.labels, RunLabels::default());
    assert_eq!(
        serde_json::to_value(OutcomeDocument::from(bare.summary.outcome)).unwrap()["labels"],
        json!({ "task": null, "experiment": null, "trial": null })
    );
    // Every span and every record of the file is one run's or the
    // other's, so none was left out of the two checks.
    assert_eq!(exported.spans.len(), 10);
    assert_eq!(exported.records.len(), 2 * events.len());
}

#[tokio::test]
async fn a_request_that_names_one_label_has_that_one_on_its_records() {
    let scratch = Lab::new("one-label");
    let mut lablet = lablet::build(scratch.config(crate::harness::ENDS, json!({})))
        .await
        .unwrap();

    let finished = lablet
        .run(request().labels(RunLabels {
            experiment: Some("terse-tool-descriptions".to_owned()),
            ..RunLabels::default()
        }))
        .await;
    lablet.shutdown().await;

    let exported = scratch.exported();
    let traced = Traced::of(&exported, finished.summary.outcome.run_id.as_str());
    for attributes in traced
        .spans
        .iter()
        .map(|span| &span.attributes)
        .chain([&traced.wide().attributes])
    {
        assert_eq!(
            attributes[key::LABLET_EXPERIMENT_ID],
            json!("terse-tool-descriptions")
        );
        assert!(!attributes.contains_key(key::LABLET_TASK_ID));
        assert!(!attributes.contains_key(key::LABLET_TRIAL));
    }
}
