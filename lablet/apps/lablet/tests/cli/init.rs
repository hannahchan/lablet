//! `lablet init`, and a run of what it wrote, as it wrote it (C7).

use std::path::PathBuf;

use lablet_conformance::otlp::Exported;
use lablet_telemetry_registry::attribute as key;
use serde_json::json;

use crate::harness::{Lab, ran};

/// The telemetry files a run left in `lab`, each named for its run.
fn telemetry_files(lab: &Lab) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(lab.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("lablet-") && name.ends_with(".otlp.jsonl"))
        })
        .collect();
    files.sort();
    files
}

#[test]
fn a_fake_starter_runs_as_init_wrote_it_and_leaves_its_telemetry_in_a_file() {
    let lab = Lab::new("init-run");

    let init = lab.run(&["init", "--provider", "fake"]);
    assert_eq!(init.code, Some(0), "{init:?}");
    assert!(init.stdout.is_empty(), "{init:?}");
    let run = lab.run(&["run", "--config", "lablet.yaml", "--prompt", "Say hello."]);

    assert_eq!(run.code, Some(0), "{run:?}");
    let outcome = run.outcome();
    assert_eq!(outcome["stop_reason"], json!("completed"));
    assert_eq!(
        outcome["result"]["text"],
        json!("Hello from the scripted model. Change what I say in lablet-script.yaml.")
    );
    let run_id = outcome["run_id"].as_str().unwrap();
    let files = telemetry_files(&lab);
    assert_eq!(files, [lab.at(&format!("lablet-{run_id}.otlp.jsonl"))]);
    let exported = Exported::read(&files[0]).unwrap();
    let wide = exported.records_of("lablet.run");
    assert_eq!(wide.len(), 1);
    assert_eq!(
        wide[0].attributes[key::GEN_AI_CONVERSATION_ID],
        json!(run_id)
    );
    assert_eq!(exported.spans_of("invoke_agent").len(), 1);
    assert_eq!(exported.spans_of("chat").len(), 1);
}

#[test]
fn init_writes_into_the_directory_it_names_and_the_config_runs_from_there() {
    let lab = Lab::new("init-directory");

    let init = lab.run(&["init", "--provider", "fake", "trials/first"]);
    assert_eq!(init.code, Some(0), "{init:?}");
    assert!(lab.at("trials/first/lablet.yaml").is_file());
    assert!(lab.at("trials/first/lablet-script.yaml").is_file());
    assert!(
        init.stderr
            .contains("cd trials/first && lablet run --config lablet.yaml"),
        "{init:?}"
    );

    let mut run = lab.lablet(&["run", "--config", "lablet.yaml", "--prompt", "Say hello."]);
    run.current_dir(lab.at("trials/first"));
    let run = ran(run, "");
    assert_eq!(run.code, Some(0), "{run:?}");
    assert_eq!(run.outcome()["stop_reason"], json!("completed"));
}

#[test]
fn init_over_a_config_that_is_there_writes_nothing_and_exits_1() {
    let lab = Lab::new("init-twice");
    lab.write("lablet.yaml", "model: { provider: fake }\n");

    let init = lab.run(&["init", "--provider", "fake"]);

    assert_eq!(init.code, Some(1), "{init:?}");
    assert_eq!(
        init.stderr,
        "config: ./lablet.yaml is there already, and init writes over nothing\n"
    );
    assert_eq!(
        std::fs::read_to_string(lab.at("lablet.yaml")).unwrap(),
        "model: { provider: fake }\n"
    );
    assert!(!lab.at("lablet-script.yaml").exists());
}
