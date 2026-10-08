//! `lablet init`, and a run of what it wrote, as it wrote it (C7).

use std::path::PathBuf;

use lablet_conformance::otlp::Exported;
use serde_json::json;

use super::harness::{Lab, ran};
use crate::key;

/// The files a run left in `lab` that hold telemetry.
fn telemetry_files(lab: &Lab) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(lab.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".otlp.jsonl"))
        })
        .collect();
    files.sort();
    files
}

/// `lablet` with `args` in `lab`, with both exporter selectors `none`, so
/// a config that states nothing of the network sends to no collector.
fn without_a_collector(lab: &Lab, args: &[&str]) -> super::harness::Ran {
    let mut command = lab.lablet(args);
    command
        .env("OTEL_TRACES_EXPORTER", "none")
        .env("OTEL_LOGS_EXPORTER", "none");
    ran(command, "")
}

/// The starter names no file, so a run of it as it's written leaves none;
/// one that names a file finds the run there.
#[test]
fn a_fake_starter_runs_as_init_wrote_it_and_writes_no_telemetry_file() {
    let lab = Lab::new("init-run");

    let init = lab.run(&["init", "--provider", "fake"]);
    assert_eq!(init.code, Some(0), "{init:?}");
    assert!(init.stdout.is_empty(), "{init:?}");
    let run = without_a_collector(
        &lab,
        &["run", "--config", "lablet.yaml", "--prompt", "Say hello."],
    );

    assert_eq!(run.code, Some(0), "{run:?}");
    let outcome = run.outcome();
    assert_eq!(outcome["stop_reason"], json!("completed"));
    assert_eq!(
        outcome["result"]["text"],
        json!("Hello from the scripted model. Change what I say in lablet-script.yaml.")
    );
    assert_eq!(telemetry_files(&lab), Vec::<PathBuf>::new());

    let run = without_a_collector(
        &lab,
        &[
            "run",
            "--config",
            "lablet.yaml",
            "--prompt",
            "Say hello.",
            "--set",
            "telemetry.file.path=starter.otlp.jsonl",
        ],
    );

    assert_eq!(run.code, Some(0), "{run:?}");
    let run_id = run.outcome()["run_id"].as_str().unwrap().to_owned();
    let files = telemetry_files(&lab);
    assert_eq!(files, [lab.at("starter.otlp.jsonl")]);
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
    run.current_dir(lab.at("trials/first"))
        .env("OTEL_TRACES_EXPORTER", "none")
        .env("OTEL_LOGS_EXPORTER", "none");
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
