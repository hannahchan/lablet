//! The config digest of a run of the binary, as its wide event holds it:
//! taken after `--set` and before `${VAR}` substitution, over the settings
//! that say what a run does (C5, C6, C12).

use serde_json::json;

use super::harness::{CONFIG, ENDS, Lab, ran};
use crate::harness::Traced;
use crate::key;

/// The variable the system prompt of the runs below holds.
const STYLE: &str = "LABLET_TEST_STYLE";

/// Runs [`CONFIG`] under the id `run`, with `args` after it and `style` as
/// the value of [`STYLE`], and returns the config digest and the system
/// prompt's digest its wide event holds, from the telemetry file at
/// `telemetry`.
fn digests(lab: &Lab, run: &str, args: &[&str], style: &str, telemetry: &str) -> [String; 2] {
    let mut command = lab.lablet(
        &[
            &[
                "run", "--config", CONFIG, "--prompt", "Fix it.", "--run-id", run,
            ][..],
            args,
        ]
        .concat(),
    );
    command.env(STYLE, style);
    let ran = ran(command, "");
    assert_eq!(ran.code, Some(0), "{run}: {ran:?}");

    let exported = lablet_conformance::otlp::Exported::read(&lab.at(telemetry)).unwrap();
    let wide = &Traced::of(&exported, run).wide().attributes;
    [key::LABLET_CONFIG_DIGEST, key::LABLET_PROMPT_SYSTEM_DIGEST]
        .map(|key| wide[key].as_str().unwrap().to_owned())
}

#[test]
fn two_runs_that_differ_only_in_what_a_variable_holds_share_a_config_digest() {
    // C5: the variable's value is in the system prompt's digest, and never
    // in the config's.
    let lab = Lab::new("digest-variable");
    lab.write_config(
        ENDS,
        json!({ "prompt": { "system": format!("You fix ${{{STYLE}}} tests.") } }),
    );
    let telemetry = "telemetry.otlp.jsonl";

    let [config, system] = digests(&lab, "flaky", &[], "flaky", telemetry);
    let [config_again, system_again] = digests(&lab, "slow", &[], "slow", telemetry);

    assert_eq!(config, config_again);
    assert_ne!(system, system_again);
}

#[test]
fn an_override_that_changes_a_setting_changes_the_digest_and_one_that_states_a_default_doesnt() {
    // C6, with each config stated by `--set`, which the digest is taken
    // after.
    let lab = Lab::new("digest-overrides");
    lab.write_config(ENDS, json!({}));
    let telemetry = "telemetry.otlp.jsonl";
    let run = |id: &str, args: &[&str]| digests(&lab, id, args, "any", telemetry)[0].clone();

    let plain = run("plain", &[]);
    let capped = run("capped", &["--set", "run.max_turns=5"]);
    let recapped = run("recapped", &["--set", "run.max_turns=7"]);
    let spelled_out = run(
        "spelled-out",
        &["--set", "run.timeout=10m", "--set", "run.max_turns=null"],
    );
    let other_words = run("other-words", &["--set", "run.timeout=600s"]);

    assert_ne!(plain, capped);
    assert_ne!(capped, recapped);
    assert_eq!(plain, spelled_out);
    assert_eq!(plain, other_words);
}

#[test]
fn two_runs_that_differ_only_in_where_they_write_share_a_config_digest() {
    // C12
    let lab = Lab::new("digest-output");
    let output = |at: &str| {
        json!({
            "run": { "transcript_path": format!("{at}-{{run_id}}.json") },
            "telemetry": {
                "file": { "path": format!("{at}.otlp.jsonl") },
                "capture_content": at == "second",
                "resource": { "written.by": at },
            },
        })
    };

    lab.write_config(ENDS, output("first"));
    let [first, _] = digests(&lab, "first", &[], "any", "first.otlp.jsonl");
    lab.write_config(ENDS, output("second"));
    let [second, _] = digests(&lab, "second", &[], "any", "second.otlp.jsonl");

    assert_eq!(first, second);
    assert!(lab.at("first-first.json").is_file());
    assert!(lab.at("second-second.json").is_file());
}
