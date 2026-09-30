//! The diagnostic log, which names a file as the config writes it, so that
//! nothing a variable holds reaches standard error (spec §7).

use serde_json::json;

use super::harness::{CONFIG, ENDS, Lab, PROMPT, ran};

/// A call of a tool the run doesn't offer, and nothing after it, so the
/// run asks the script for more than it holds.
const RUNS_OUT: &str = "
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: ls } } }
    finish: tool_use
";

/// A failure the script injects, with no message of its own, so the
/// provider's error names the script.
const FAILS: &str = "
- error: { kind: fatal }
";

/// The variable the config's paths are read from.
const DIRECTORY: &str = "LABLET_TEST_DIRECTORY";

#[test]
fn a_file_that_cannot_be_written_is_warned_of_with_nothing_the_variable_in_its_path_holds() {
    let lab = Lab::new("diagnostics-variable");
    let blocked = lab.write("blocked", "a file, where the variable names a directory");
    let held = blocked.join("held-by-the-variable");
    lab.write_config(
        ENDS,
        json!({
            "run": { "transcript_path": format!("${{{DIRECTORY}}}/transcript.json") },
            "telemetry": { "file": { "path": format!("${{{DIRECTORY}}}/telemetry.jsonl") } },
        }),
    );
    let mut command = lab.lablet(&["run", "--config", CONFIG, "--prompt", PROMPT]);
    command.env(DIRECTORY, &held).env("RUST_LOG", "warn");

    let run = ran(command, "");

    assert_eq!(run.code, Some(0), "{run:?}");
    assert!(
        run.stderr.contains(&format!(
            "the transcript couldn't be written to ${{{DIRECTORY}}}/transcript.json: "
        )),
        "{run:?}"
    );
    assert!(
        run.stderr
            .contains("the telemetry file couldn't be written: "),
        "{run:?}"
    );
    assert!(!run.stderr.contains("held-by-the-variable"), "{run:?}");
}

#[test]
fn a_script_a_variable_leads_to_is_named_as_the_config_writes_it_in_everything_a_run_leaves() {
    let lab = Lab::new("diagnostics-script");
    let held = lab.at("held-by-the-variable");
    let transcript = lab.at("transcript.json");
    let written = format!("${{{DIRECTORY}}}/script.yaml");
    for (script, error) in [
        (RUNS_OUT, format!("provider: script {written:?} ran out: ")),
        (
            FAILS,
            format!("provider: entry 1 of script {written:?} injects a failure of kind fatal"),
        ),
    ] {
        lab.write("held-by-the-variable/script.yaml", script);
        let mut config = lab.tree(ENDS, json!({ "run": { "transcript_path": transcript } }));
        config["model"]["script"] = json!(written);
        lab.write(CONFIG, &config.to_string());
        let mut command = lab.lablet(&["run", "--config", CONFIG, "--prompt", PROMPT]);
        command.env(DIRECTORY, &held);

        let run = ran(command, "");

        assert_eq!(run.code, Some(2), "{run:?}");
        let outcome = run.outcome();
        assert_eq!(outcome["stop_reason"], json!("provider_error"));
        assert!(run.stderr.starts_with(&error), "{run:?}");
        let telemetry = std::fs::read_to_string(lab.telemetry()).unwrap();
        for with_the_error in [&outcome["error"].to_string(), &telemetry] {
            assert!(with_the_error.contains(&written), "{with_the_error}");
        }
        let left = [
            ("stdout", run.stdout),
            ("stderr", run.stderr),
            ("transcript", std::fs::read_to_string(&transcript).unwrap()),
            ("telemetry", telemetry),
        ];
        for (what, text) in left {
            assert!(!text.contains("held-by-the-variable"), "{what}: {text}");
        }
    }
}
