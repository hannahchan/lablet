//! The exit code: 0 for a run that completed, 2 for a run that stopped any
//! other way, and 1 for a run that never started (spec §1), with the
//! outcome printed for every run that started.

use serde_json::json;

use crate::harness::{CONFIG, ENDS, Lab};

#[test]
fn a_run_that_completed_exits_0() {
    let lab = Lab::new("exit-completed");
    lab.config(ENDS, json!({}));

    let run = lab.run_config(&[]);

    assert_eq!(run.code, Some(0), "{run:?}");
    assert_eq!(run.outcome()["stop_reason"], json!("completed"));
}

#[test]
fn a_refused_run_exits_2_whether_the_model_or_a_filter_refused() {
    // L9: a refusal is never a completed run, and a withheld response's
    // tool call doesn't run.
    for (test, script) in [
        (
            "exit-refusal",
            "
- response:
    content:
      - text: I can't help with that.
    finish: refusal
",
        ),
        (
            "exit-content-filter",
            "
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: ls } } }
    finish: content_filter
",
        ),
    ] {
        let lab = Lab::new(test);
        lab.config(script, json!({}));

        let run = lab.run_config(&[]);

        assert_eq!(run.code, Some(2), "{run:?}");
        let outcome = run.outcome();
        assert_eq!(outcome["stop_reason"], json!("refused"));
        assert_eq!(outcome["tool_calls"], json!(0));
    }
}

#[test]
fn a_run_whose_key_the_provider_rejected_exits_2_with_its_outcome_printed() {
    // E15: the run began, so it has an outcome, and the rejection is its
    // error.
    let lab = Lab::new("exit-auth");
    lab.config(
        "- error: { kind: auth, message: 401 invalid x-api-key }\n",
        json!({}),
    );

    let run = lab.run_config(&[]);

    assert_eq!(run.code, Some(2), "{run:?}");
    let outcome = run.outcome();
    assert_eq!(outcome["stop_reason"], json!("provider_error"));
    assert_eq!(outcome["turns"], json!(0));
    assert_eq!(outcome["error"], json!("401 invalid x-api-key"));
    assert_eq!(lab.exported().records_of("lablet.run").len(), 1);
}

#[test]
fn a_config_that_isnt_read_exits_1_with_a_config_message_and_no_outcome() {
    let lab = Lab::new("exit-unknown-key");
    lab.config(ENDS, json!({ "run": { "max_turn": 3 } }));

    let run = lab.run_config(&[]);

    assert_eq!(run.code, Some(1), "{run:?}");
    assert!(run.stdout.is_empty(), "{run:?}");
    let lines = run.stderr_lines();
    assert_eq!(lines.len(), 1, "{run:?}");
    assert!(lines[0].starts_with("config: "), "{run:?}");
    assert!(lines[0].contains("max_turn"), "{run:?}");
    assert!(!lab.telemetry().exists(), "a run started");
}

#[test]
fn a_config_the_library_wont_build_exits_1_with_a_config_message() {
    let lab = Lab::new("exit-unsupported");
    lab.config(ENDS, json!({ "run": { "transcript_format": "atif" } }));

    let run = lab.run_config(&[]);

    assert_eq!(run.code, Some(1), "{run:?}");
    assert!(run.stdout.is_empty(), "{run:?}");
    assert!(
        run.stderr
            .starts_with("config: `run.transcript_format: atif` isn't supported yet"),
        "{run:?}"
    );
    assert_eq!(run.stderr_lines().len(), 1, "{run:?}");
}

#[test]
fn a_config_file_that_isnt_there_exits_1() {
    let lab = Lab::new("exit-no-config");

    let run = lab.run(&["run", "--config", CONFIG, "--prompt", "Say hello."]);

    assert_eq!(run.code, Some(1), "{run:?}");
    assert!(
        run.stderr
            .starts_with("config: the config lablet.json couldn't be read: "),
        "{run:?}"
    );
}

#[test]
fn arguments_the_command_line_refuses_exit_1_and_its_help_exits_0() {
    let lab = Lab::new("exit-usage");

    for refused in [
        &["run", "--prompt", "Say hello."][..],
        &[
            "run",
            "--config",
            CONFIG,
            "--prompt",
            "a",
            "--prompt-file",
            "b",
        ],
        &["run", "--config", CONFIG, "--max-turns", "3"],
        &["init", "--provider", "ollama"],
        &[],
    ] {
        let run = lab.run(refused);
        assert_eq!(run.code, Some(1), "{refused:?}: {run:?}");
        assert!(run.stdout.is_empty(), "{refused:?}: {run:?}");
    }
    let help = lab.run(&["run", "--help"]);
    assert_eq!(help.code, Some(0), "{help:?}");
    assert!(help.stdout.contains("--prompt-file"), "{help:?}");
}
