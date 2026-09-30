//! The diagnostic log, which names a file as the config writes it, so that
//! nothing a variable holds reaches standard error (spec §7).

use serde_json::json;

use super::harness::{CONFIG, ENDS, Lab, PROMPT, ran};

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
