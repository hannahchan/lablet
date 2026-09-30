//! `lablet check`: the config checked whole without a call to the provider,
//! a refusal as a `config:` message that names the key, where it was
//! written and its value as written, and what a config that passed comes
//! to.

use lablet::{Config, Format};
use serde_json::{Value, json};

use super::harness::{CONFIG, ENDS, Lab, Ran, ran};

/// The config of spec §7, whose defaults spec §7 quotes.
const SPEC: &str = include_str!("../../../src/config/tests/spec.yaml");

/// A variable nothing sets, which each check below removes from lablet's
/// environment too.
const UNSET: &str = "LABLET_TEST_A_VARIABLE_NOTHING_SETS";

/// The variable a key is read from in the checks below that need one set.
const KEY: &str = "LABLET_TEST_KEY";

/// Checks `lablet.yaml`, which holds `text`, with `args` after the config,
/// with [`KEY`] set to what no provider would take and [`UNSET`] unset.
fn check(lab: &Lab, text: &str, args: &[&str]) -> Ran {
    lab.write("lablet.yaml", text);
    let mut command = lab.lablet(&[&["check", "--config", "lablet.yaml"][..], args].concat());
    command.env(KEY, "placeholder").env_remove(UNSET);
    ran(command, "")
}

/// A check that passed: its exit code, and the one line it ends with.
fn passed(run: &Ran, tools: &str) {
    assert_eq!(run.code, Some(0), "{run:?}");
    assert_eq!(run.stderr, format!("passed: {tools}\n"), "{run:?}");
}

/// A check that was refused with the one line `line`, and nothing printed
/// on standard output.
fn refused(run: &Ran, line: &str) {
    assert_eq!(run.code, Some(1), "{run:?}");
    assert_eq!(run.stdout, "", "{run:?}");
    assert_eq!(run.stderr, format!("{line}\n"), "{run:?}");
}

/// What `lablet check --resolved` printed, read back as a config.
fn resolved(run: &Ran) -> Config {
    Config::from_str(&run.stdout, Format::Yaml).unwrap()
}

#[test]
fn an_unknown_key_is_refused_by_the_key_and_its_line() {
    // C1
    let lab = Lab::new("check-unknown-key");

    let run = check(&lab, "prompt:\n  system: Hi.\nrun:\n  max_turn: 3\n", &[]);

    assert_eq!(run.code, Some(1), "{run:?}");
    assert!(
        run.stderr.starts_with(
            "config: run.max_turn (line 4) is refused: no setting has the key; the keys beside \
             it are `completion`, `max_turns`, "
        ),
        "{run:?}"
    );
    assert_eq!(run.stderr_lines().len(), 1, "{run:?}");
}

#[test]
fn an_invalid_value_is_refused_by_the_key_the_value_and_the_values_it_takes() {
    // C2
    let lab = Lab::new("check-invalid-value");

    let run = check(
        &lab,
        "prompt:\n  system: Hi.\nrun:\n  completion: implicit\n",
        &[],
    );

    refused(
        &run,
        "config: run.completion (line 4): \"implicit\" is refused: the accepted values are \
         `natural`, `explicit`",
    );
}

#[test]
fn an_override_is_in_the_resolved_config_and_a_refused_one_is_named_as_an_override() {
    // C3
    let lab = Lab::new("check-override");
    lab.write_config(ENDS, json!({}));

    let run = ran(
        lab.lablet(&[
            "check",
            "--config",
            CONFIG,
            "--resolved",
            "--set",
            "run.max_turns=5",
        ]),
        "",
    );

    passed(&run, "0 tools");
    assert!(run.stdout.contains("\n  max_turns: 5\n"), "{run:?}");
    assert_eq!(resolved(&run).run.max_turns, std::num::NonZeroU32::new(5));

    let run = ran(
        lab.lablet(&[
            "check",
            "--config",
            CONFIG,
            "--set",
            "run.completion=implicit",
        ]),
        "",
    );
    refused(
        &run,
        "config: run.completion (an override): \"implicit\" is refused: the accepted values are \
         `natural`, `explicit`",
    );
    let run = ran(
        lab.lablet(&["check", "--config", CONFIG, "--set", "run"]),
        "",
    );
    refused(
        &run,
        "config: the override is refused: an override is written `key=value`, and this one \
         holds no `=`",
    );
}

#[test]
fn an_override_is_named_where_the_refusal_is_of_the_section_it_set_a_key_in() {
    let lab = Lab::new("check-override-within");
    lab.write_config(ENDS, json!({}));
    let set = |key_value: &str| {
        ran(
            lab.lablet(&["check", "--config", CONFIG, "--set", key_value]),
            "",
        )
    };

    refused(
        &set("tools.allow.0=bash"),
        "config: tools.allow.0: the override is refused: no list is written at tools.allow, \
         so `0` is no place in one",
    );
    let run = set("model.thinking.budget=64000");
    assert_eq!(run.code, Some(1), "{run:?}");
    assert!(
        run.stderr
            .starts_with("config: model.thinking (an override): {\"budget\":64000} is refused: "),
        "{run:?}"
    );
    assert_eq!(run.stderr_lines().len(), 1, "{run:?}");
}

#[test]
fn an_unset_key_variable_is_named_only_when_it_is_written_in_capitals_and_fake_needs_none() {
    // C4
    let lab = Lab::new("check-key");
    let anthropic =
        |variable: &str| format!("model:\n  api_key_env: {variable}\nprompt:\n  system: Hi.\n");

    refused(
        &check(&lab, &anthropic(UNSET), &[]),
        &format!(
            "config: model.api_key_env (line 2) is refused: `{UNSET}`, the variable it names, \
             isn't set, and the provider `anthropic` needs a key"
        ),
    );
    let lower = UNSET.to_lowercase();
    let run = check(&lab, &anthropic(&lower), &[]);
    refused(
        &run,
        "config: model.api_key_env (line 2) is refused: the variable it names isn't set, and \
         the provider `anthropic` needs a key",
    );

    lab.write("script.yaml", ENDS);
    let fake = format!(
        "model:\n  provider: fake\n  script: script.yaml\n  api_key_env: {UNSET}\nprompt:\n  \
         system: Hi.\n"
    );
    passed(&check(&lab, &fake, &[]), "0 tools");
}

#[test]
fn a_script_the_fake_model_cannot_read_is_refused_as_a_run_refuses_it() {
    let lab = Lab::new("check-missing-script");

    let run = check(
        &lab,
        "model:\n  provider: fake\n  script: missing.yaml\nprompt:\n  system: Hi.\n",
        &[],
    );

    refused(
        &run,
        "config: model.script (line 3): \"missing.yaml\" is refused: No such file or directory \
         (os error 2)",
    );
}

#[test]
fn a_setting_the_provider_cant_apply_is_refused_and_its_default_left_out_of_the_resolved_config() {
    // C13
    let lab = Lab::new("check-not-applied");
    let anthropic = format!("model:\n  api_key_env: {KEY}\nprompt:\n  system: Hi.\n");
    let chat = "model:\n  provider: openai\n  base_url: http://localhost:11434/v1\nprompt:\n  \
                system: Hi.\n";

    refused(
        &check(
            &lab,
            &anthropic.replace("prompt:", "  seed: 7\nprompt:"),
            &[],
        ),
        "config: model.seed (line 3): 7 is refused: the provider `anthropic` can't apply it",
    );
    refused(
        &check(
            &lab,
            &chat.replace("prompt:", "  thinking: adaptive\nprompt:"),
            &[],
        ),
        "config: model.thinking (line 4): \"adaptive\" is refused: the provider `openai` over \
         its API `chat_completions` can't apply it",
    );

    for (text, left_out) in [(anthropic.as_str(), "seed"), (chat, "thinking")] {
        let run = check(&lab, text, &["--resolved"]);
        passed(&run, "0 tools");
        let printed: Value = serde_saphyr::from_str(&run.stdout).unwrap();
        let model = printed["model"].as_object().unwrap();
        assert!(!model.contains_key(left_out), "{left_out}: {model:?}");
    }
}

#[test]
fn a_value_a_variable_gives_is_shown_as_the_config_writes_it() {
    // C16
    let lab = Lab::new("check-variable-value");
    let held = "not a URL, and not for stderr";
    lab.write(
        "lablet.yaml",
        "model:\n  provider: openai\n  base_url: '${LABLET_TEST_BASE_URL}'\nprompt:\n  system: \
         Hi.\n",
    );
    let mut command = lab.lablet(&["check", "--config", "lablet.yaml"]);
    command.env("LABLET_TEST_BASE_URL", held);

    let run = ran(command, "");

    refused(
        &run,
        "config: model.base_url (line 3): \"${LABLET_TEST_BASE_URL}\" is refused: a URL begins \
         `http://` or `https://` and names a host, as `http://localhost:11434/v1` does",
    );
    assert!(!run.stderr.contains(held), "{run:?}");
}

#[test]
fn a_variable_that_is_not_set_is_refused_by_the_key_it_is_in() {
    // C10
    let lab = Lab::new("check-unset-variable");

    let run = check(
        &lab,
        &format!(
            "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: You fix ${{{UNSET}}} tests.\n"
        ),
        &[],
    );

    refused(
        &run,
        &format!(
            "config: prompt.system (line 5): \"You fix ${{{UNSET}}} tests.\" is refused: the \
             variable `{UNSET}` isn't set"
        ),
    );
}

#[test]
fn every_default_the_resolved_config_holds_is_the_one_spec_7_gives() {
    // C17: the spec's config states every setting, and its example servers
    // are the one thing it states that isn't a default.
    let lab = Lab::new("check-defaults");
    let mut spec = Config::from_str(SPEC, Format::Yaml).unwrap();
    spec.tools.mcp.clear();
    lab.write("lablet.yaml", "prompt:\n  system: You are ...\n");
    let mut command = lab.lablet(&["check", "--config", "lablet.yaml", "--resolved"]);
    command.env("ANTHROPIC_API_KEY", "placeholder");

    let run = ran(command, "");

    passed(&run, "0 tools");
    let printed: Value = serde_saphyr::from_str(&run.stdout).unwrap();
    assert_eq!(printed, serde_json::to_value(spec.resolved()).unwrap());
    assert_eq!(resolved(&run).digest(), spec.digest());
}

#[test]
fn a_check_lists_the_tools_a_run_is_offered_and_starts_no_run() {
    let lab = Lab::new("check-tools");
    lab.write_config(
        ENDS,
        json!({ "tools": {
            "builtin": lab.builtin(&["write_file", "bash", "read_file"]),
            "deny": ["write_file"],
        } }),
    );

    let run = lab.run(&["check", "--config", CONFIG]);

    passed(&run, "2 tools");
    assert_eq!(run.stdout, "bash\nread_file\n");
    assert!(!lab.telemetry().exists(), "a run started");
}
