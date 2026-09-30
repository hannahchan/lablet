//! What names a run on the command line: its id and its labels, in its
//! outcome and on its wide event (C18).

use lablet_telemetry_registry::attribute as key;
use serde_json::json;

use super::harness::{ENDS, Lab};

#[test]
fn the_run_id_and_the_labels_given_are_in_the_outcome_and_on_the_wide_event() {
    let lab = Lab::new("labels");
    lab.write_config(ENDS, json!({}));

    let run = lab.run_config(&[
        "--run-id",
        "r",
        "--task",
        "t",
        "--experiment",
        "e",
        "--trial",
        "3",
    ]);

    assert_eq!(run.code, Some(0), "{run:?}");
    let outcome = run.outcome();
    assert_eq!(outcome["run_id"], json!("r"));
    assert_eq!(
        outcome["labels"],
        json!({ "task": "t", "experiment": "e", "trial": "3" })
    );
    let exported = lab.exported();
    let wide = exported.records_of("lablet.run");
    assert_eq!(wide.len(), 1);
    for (key, value) in [
        (key::GEN_AI_CONVERSATION_ID, "r"),
        (key::LABLET_TASK_ID, "t"),
        (key::LABLET_EXPERIMENT_ID, "e"),
        (key::LABLET_TRIAL, "3"),
    ] {
        assert_eq!(wide[0].attributes[key], json!(value), "{key}");
    }
}

#[test]
fn a_run_without_names_has_a_fresh_id_and_no_labels() {
    let lab = Lab::new("labels-none");
    lab.write_config(ENDS, json!({}));

    let first = lab.run_config(&[]).outcome();
    let second = lab.run_config(&[]).outcome();

    assert_ne!(first["run_id"], second["run_id"]);
    assert_eq!(
        first["labels"],
        json!({ "task": null, "experiment": null, "trial": null })
    );
}

#[test]
fn a_run_id_the_library_refuses_exits_1_before_the_run() {
    let lab = Lab::new("labels-refused");
    lab.write_config(ENDS, json!({}));

    let run = lab.run_config(&["--run-id", " r"]);

    assert_eq!(run.code, Some(1), "{run:?}");
    assert!(run.stdout.is_empty(), "{run:?}");
    assert!(run.stderr.starts_with("config: --run-id: "), "{run:?}");
    assert!(!lab.telemetry().exists(), "a run started");
}
