//! What the command line's export does with a run, through its
//! composition: a telemetry file of lablet's own the root may not hold, a
//! file that can't be written, a collector that never answers, and the
//! order of a run's end as a collector sees it.

use std::os::unix::fs::symlink;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lablet::{BuildError, OwnFile, Place, RunId, RunRequest, StopReason};
use lablet_cli::compose;
use lablet_conformance::otlp::Exported;
use lablet_conformance::receiver::{Mode, Receiver};
use lablet_run::telemetry::generated::{LabletInvokeAgent, LabletRun};
use serde_json::{Value, json};

use crate::harness::{ENDS, Lab, read, refusal, request};
use crate::key;

/// The telemetry file the command line writes is one of lablet's own, so a
/// root that holds it, through a link that leads under the root, is
/// refused, while standard error is no file.
#[tokio::test]
async fn a_root_that_holds_the_telemetry_file_is_refused_with_the_file_it_holds() {
    let scratch = Lab::new("root-holds-telemetry");
    let root = scratch.root();
    let tools = json!({ "tools": { "builtin": scratch.builtin(&["read_file"]) } });
    symlink(&root, scratch.at("linked")).unwrap();
    let telemetry = scratch.at("linked/telemetry.otlp.jsonl");
    let mut tree = scratch.tree(ENDS, tools.clone());
    tree["telemetry"]["file"]["path"] = json!(telemetry);
    assert_eq!(
        refusal(read(&tree)).await,
        BuildError::RootHolds {
            place: Some(Place::Line(1)),
            root: root.display().to_string(),
            holds: OwnFile::Telemetry,
            path: telemetry.display().to_string(),
        }
    );

    let mut tree = scratch.tree(ENDS, tools);
    tree["telemetry"]["file"]["path"] = json!("-");
    compose::build(read(&tree)).await.unwrap();
}

/// With no file named, a run writes none, so a root that holds the
/// working directory holds no telemetry of lablet's.
#[tokio::test]
async fn a_root_that_holds_the_working_directory_holds_no_telemetry_when_no_file_is_named() {
    let here = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
    let scratch = Lab::new("root-holds-here");
    let mut tree = scratch.tree(
        ENDS,
        json!({ "tools": { "builtin": { "root": here, "enabled": ["read_file"] } } }),
    );
    tree["telemetry"]["file"]["path"] = json!(null);

    let built = compose::build(read(&tree)).await;

    built.unwrap().shutdown().await;
}

#[tokio::test]
async fn telemetry_that_cannot_be_written_leaves_the_outcome_alone() {
    let scratch = Lab::new("unwritable");
    let nowhere = scratch.at("no-such-directory/telemetry.otlp.jsonl");
    let config = scratch.config(
        ENDS,
        json!({ "telemetry": { "file": { "path": nowhere } } }),
    );
    let mut lablet = compose::build(config).await.unwrap();
    let diagnostics = crate::harness::Diagnostics::capture();

    let finished = lablet.run(request()).await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!(outcome.result().text, "Nothing to fix.");
    assert!(!nowhere.exists());
    let lines = diagnostics.lines();
    let warned: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("WARN") && line.contains(outcome.run_id.as_str()))
        .collect();
    assert_eq!(warned.len(), 1, "{lines:?}");
    assert!(
        warned[0].contains("the run's telemetry wasn't exported whole"),
        "{lines:?}"
    );
    assert_eq!(warned[0].matches("exported whole").count(), 1, "{lines:?}");
    lablet.shutdown().await;
}

/// A run's end waits on a collector that accepts and never answers for one
/// flush bound of the SDK's, since it flushes once, and the file holds the
/// run and its wide event by then.
#[tokio::test]
async fn a_run_end_with_a_collector_that_never_answers_waits_one_flush_bound() {
    let receiver = Receiver::start(Mode::NeverAnswers).await;
    let scratch = Lab::new("run-end-never-answered");
    let path = scratch.telemetry();
    let config = scratch.config(
        ENDS,
        json!({ "telemetry": {
            "otlp": { "enabled": true, "endpoint": receiver.grpc_endpoint(), "protocol": "grpc" },
        } }),
    );
    let mut lablet = compose::build(config).await.unwrap();

    let began = Instant::now();
    lablet
        .run(
            RunRequest::new("Fix it.")
                .unwrap()
                .run_id(RunId::new("run-a").unwrap())
                .unwrap(),
        )
        .await;
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
fn whole_at(path: &str) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|document| document["turns"].is_array())
}

#[tokio::test]
async fn the_transcript_a_wide_event_names_is_whole_when_the_wide_event_arrives() {
    let scratch = Lab::new("transcript-before-wide");
    let transcript = scratch.at("transcript.json");
    let found = Arc::new(Mutex::new(Vec::new()));
    let finding = Arc::clone(&found);
    let receiver = Receiver::watching(Mode::Answers, move |received| {
        let exported = received.exported().unwrap();
        for wide in exported.records_of(key::WIDE_EVENT) {
            let named = wide.attributes[key::LABLET_RUN_TRANSCRIPT_PATH]
                .as_str()
                .unwrap()
                .to_owned();
            let whole = whole_at(&named);
            finding.lock().unwrap().push((named, whole));
        }
    })
    .await;
    let config = scratch.config(
        ENDS,
        json!({
            "run": { "transcript_path": transcript },
            "telemetry": { "otlp": {
                "enabled": true,
                "endpoint": receiver.grpc_endpoint(),
                "protocol": "grpc",
            } },
        }),
    );
    let mut lablet = compose::build(config).await.unwrap();

    lablet.run(request()).await;
    lablet.shutdown().await;

    assert_eq!(
        *found.lock().unwrap(),
        [(transcript.display().to_string(), true)],
        "the wide event reached the collector once, after its transcript was written"
    );
    assert_eq!(
        scratch.exported().records_of(key::WIDE_EVENT).len(),
        1,
        "and the file holds it"
    );
}
