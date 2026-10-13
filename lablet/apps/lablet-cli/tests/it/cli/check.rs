//! `lablet check`: the config checked whole without a call to the provider,
//! a refusal as a `config:` message that names the key, where it was
//! written and its value as written, and what a config that passed comes
//! to: its credentials, what `bash` inherits, what's set and read no more,
//! where a run's telemetry goes, and its digest.

use std::ffi::OsString;

use lablet::{Config, Format, RawConfig};
use serde_json::{Value, json};

use super::harness::{CONFIG, ENDS, Lab, PROMPT, Ran, ran};
use crate::harness::json_of;

/// The config of spec §7, whose defaults spec §7 quotes.
const SPEC: &str = include_str!("../../../../shared/config/src/tests/spec.yaml");

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

/// What `unread:` and `telemetry:` say for a config written as text with
/// no `telemetry` section, under an environment that sets no retired
/// variable.
const NOTHING_NAMED: &str =
    "unread:    none\ntelemetry: none named, so a run sends and writes nothing";

/// The lines a check of a config with no credential, no `env` entry, no
/// `bash` and no `telemetry` section ends with before the digest.
const NONE: &str = "own:       none\ngiven:     none\nenv:       none\nunread:    none\n\
                    telemetry: none named, so a run sends and writes nothing";

/// The same for the lab's config, [`CONFIG`], written on one line of JSON,
/// which names the lab's telemetry file.
const NONE_WITH_FILE: &str =
    "own:       none\ngiven:     none\nenv:       none\nunread:    none\ntelemetry: file (line 1)";

/// The same for the lab's config with `bash`, whose `inherited:` line names
/// what the binary's environment holds.
const NONE_WITH_BASH: &str = "own:       none\ngiven:     none\nenv:       none\ninherited: \
                              <inherited>, as set; BAGGAGE, TRACEPARENT, TRACESTATE, its tool \
                              span's\nunread:    none\ntelemetry: file (line 1)";

/// What `limit:` says of `bash` on this platform.
const LIMIT: &str = if cfg!(target_os = "macos") {
    "bash can read any file you can, and your shell's environment and lablet's"
} else {
    "bash can read any file you can, and your shell's environment"
};

/// `stderr` with the digest's 64 hex digits as `<digest>` and the names
/// `inherited:` lists as set as `<inherited>`, which are the binary's
/// environment's, so a summary compares whole.
fn normalised(stderr: &str) -> String {
    let mut text = stderr.to_owned();
    if let Some(start) = text.find("digest:    ") {
        let digits = start + "digest:    ".len();
        let end = text[digits..]
            .find('\n')
            .map_or(text.len(), |offset| digits + offset);
        let digest = &text[digits..end];
        assert!(
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "{stderr}"
        );
        text.replace_range(digits..end, "<digest>");
    }
    if let Some(start) = text.find("inherited: ") {
        let names = start + "inherited: ".len();
        if let Some(offset) = text[names..].find(", as set") {
            text.replace_range(names..names + offset, "<inherited>");
        }
    }
    text
}

/// A check that passed: its exit code, and the lines it ends with, `names`
/// before the digest, `limit:` with `bash`, then `passed:`.
fn passed(run: &Ran, names: &str, tools: &str) {
    assert_eq!(run.code, Some(0), "{run:?}");
    let limit = if names.contains("inherited:") {
        format!("\nlimit:     {LIMIT}")
    } else {
        String::new()
    };
    assert_eq!(
        normalised(&run.stderr),
        format!("{names}\ndigest:    <digest>{limit}\npassed:    {tools}\n"),
        "{run:?}"
    );
}

/// A check that was refused with the one line `line`, and nothing printed
/// on standard output.
fn refused(run: &Ran, line: &str) {
    assert_eq!(run.code, Some(1), "{run:?}");
    assert_eq!(run.stdout, "", "{run:?}");
    assert_eq!(run.stderr, format!("{line}\n"), "{run:?}");
}

/// What `lablet check --resolved` printed, read back as a config, with every
/// `${VAR}` it kept as written filled with a placeholder.
fn resolved(run: &Ran) -> Config {
    RawConfig::from_str(&run.stdout, Format::Yaml)
        .unwrap()
        .substituted(|_: &str| Some(OsString::from("placeholder")))
        .unwrap()
        .config()
        .unwrap()
}

/// The one line of `stderr` that begins `label`.
fn line_of<'a>(run: &'a Ran, label: &str) -> &'a str {
    run.stderr_lines()
        .into_iter()
        .find(|line| line.starts_with(label))
        .unwrap_or_else(|| panic!("no {label} line: {run:?}"))
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

    passed(&run, NONE_WITH_FILE, "0 tools");
    assert!(run.stdout.contains("\n  max_turns: 5 # --set\n"), "{run:?}");
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
fn an_unset_key_variable_is_named_where_the_provider_needs_a_key_and_fake_needs_none() {
    // C4, as the caller owning the config rewords it: the key is a `${VAR}`
    // under `api_key`, and `api_key_env` is a key no setting has.
    let lab = Lab::new("check-key");
    let anthropic = |key: &str| format!("model:\n  api_key: {key}\nprompt:\n  system: Hi.\n");

    refused(
        &check(&lab, &anthropic(&format!("${{{UNSET}}}")), &[]),
        &format!(
            "config: model.api_key (line 2): \"${{{UNSET}}}\" is refused: `{UNSET}` isn't set, \
             and the provider `anthropic` needs a key; set it"
        ),
    );
    refused(
        &check(&lab, "prompt:\n  system: Hi.\n", &[]),
        "config: model.api_key isn't set: the provider `anthropic` needs a key; add api_key: \
         ${ANTHROPIC_API_KEY}",
    );
    let bare = check(&lab, &anthropic(UNSET), &[]);
    refused(
        &bare,
        &format!(
            "config: model.api_key (line 2): \"{UNSET}\" is refused: it's a variable's name, not \
             a ${{VAR}}; write api_key: ${{{UNSET}}}"
        ),
    );

    let old = check(
        &lab,
        &format!("model:\n  api_key_env: {UNSET}\nprompt:\n  system: Hi.\n"),
        &[],
    );
    assert_eq!(old.code, Some(1), "{old:?}");
    assert!(
        old.stderr.starts_with(
            "config: model.api_key_env (line 2) is refused: no setting has the key; the keys \
             beside it are "
        ) && old.stderr.contains("`api_key`"),
        "{old:?}"
    );

    lab.write("script.yaml", ENDS);
    let fake = format!(
        "model:\n  provider: fake\n  script: script.yaml\n  api_key: ${{{UNSET}}}\nprompt:\n  \
         system: Hi.\n"
    );
    passed(
        &check(&lab, &fake, &[]),
        &format!(
            "own:       {UNSET} (line 4) (unset, not cut) -> model.api_key; no process inherits \
             it\ngiven:     none\nenv:       none\n{NOTHING_NAMED}"
        ),
        "0 tools",
    );
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
    let anthropic = format!("model:\n  api_key: ${{{KEY}}}\nprompt:\n  system: Hi.\n");
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

    let key_named = format!(
        "own:       {KEY} (line 2) (under 16 bytes, not cut) -> model.api_key; no process \
         inherits it\ngiven:     none\nenv:       none\n{NOTHING_NAMED}"
    );
    for (text, left_out, names) in [
        (anthropic.as_str(), "seed", key_named.as_str()),
        (chat, "thinking", NONE),
    ] {
        let run = check(&lab, text, &["--resolved"]);
        passed(&run, names, "0 tools");
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
fn a_variable_that_is_not_set_is_refused_by_the_key_it_is_in_and_a_prompt_is_text() {
    // C10, with R10's fix, and the prompt left as the file wrote it: a
    // `${VAR}` in it is text, so an unset variable there refuses nothing.
    let lab = Lab::new("check-unset-variable");
    lab.write("script.yaml", ENDS);

    let run = check(
        &lab,
        &format!(
            "model:\n  provider: fake\n  script: script.yaml\n  name: ${{{UNSET}}}\nprompt:\n  \
             system: Hi.\n"
        ),
        &[],
    );

    refused(
        &run,
        &format!(
            "config: model.name (line 4): \"${{{UNSET}}}\" is refused: `{UNSET}` isn't set; set \
             it, or give a default, as ${{{UNSET}:-default}}"
        ),
    );

    let prompt = check(
        &lab,
        &format!(
            "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: You fix ${{{UNSET}}} tests.\n"
        ),
        &["--resolved"],
    );
    passed(&prompt, NONE, "0 tools");
    assert!(
        prompt
            .stdout
            .contains(&format!("You fix ${{{UNSET}}} tests.")),
        "{prompt:?}"
    );
}

#[test]
fn a_variable_that_is_not_set_in_the_telemetry_file_or_resource_is_refused() {
    // The command line's SDK is configured from them, so an unset variable
    // there is refused as one in any setting is.
    let lab = Lab::new("check-unset-telemetry");

    for (section, key) in [
        ("file:\n    path", "telemetry.file.path"),
        ("resource:\n    team", "telemetry.resource.team"),
    ] {
        let run = check(
            &lab,
            &format!(
                "model:\n  provider: fake\n  script: script.yaml\ntelemetry:\n  {section}: '${{{UNSET}}}'\n"
            ),
            &[],
        );

        refused(
            &run,
            &format!(
                "config: {key} (line 6): \"${{{UNSET}}}\" is refused: `{UNSET}` isn't set; set it, \
                 or give a default, as ${{{UNSET}:-default}}"
            ),
        );
    }
}

/// R12: a `${` that begins no variable's name is refused by its key with
/// the fix, and the text isn't shown, since what follows may be a value.
#[test]
fn a_dollar_brace_that_begins_no_variable_is_refused_without_its_value() {
    let lab = Lab::new("check-dollar-brace");
    lab.write("script.yaml", ENDS);

    let run = check(
        &lab,
        "model:\n  provider: fake\n  script: script.yaml\n  name: ${price-0123456789abcdef\nprompt:\n  system: Hi.\n",
        &[],
    );

    refused(
        &run,
        "config: model.name (line 4) is refused: its ${ begins no variable's name; write $${ for ${",
    );
    assert!(!run.stderr.contains("price"), "{run:?}");
}

/// R11: a `credentials` entry whose variable is unset is refused with the
/// `${VAR:-}` fix, and with that fix passes, the entry unset and named so
/// on `given:`, where `bash` doesn't get it; `--resolved` writes it as the
/// file did, and reads back to the same digest with the variable set.
#[test]
fn an_unset_credential_variable_is_refused_with_its_fix_and_an_empty_default_leaves_it_unset() {
    let lab = Lab::new("check-unset-credential");
    lab.write("script.yaml", ENDS);
    lab.write("work/notes.md", "Nothing yet.");
    let config = |entry: &str| {
        format!(
            "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\ntools:\n  \
             builtin:\n    root: work\n    enabled: [bash]\n    credentials:\n      GITHUB_TOKEN: \
             {entry}\n"
        )
    };

    refused(
        &check(&lab, &config(&format!("${{{UNSET}}}")), &[]),
        &format!(
            "config: tools.builtin.credentials.GITHUB_TOKEN (line 11): \"${{{UNSET}}}\" is \
             refused: `{UNSET}` isn't set; set it, or write ${{{UNSET}:-}} to give it only when set"
        ),
    );

    let unset = check(&lab, &config(&format!("${{{UNSET}:-}}")), &["--resolved"]);
    passed(
        &unset,
        &format!(
            "own:       none\ngiven:     none: {UNSET} (line 11) is unset\nenv:       none\n\
             inherited: <inherited>, as set; BAGGAGE, TRACEPARENT, TRACESTATE, its tool span's\n\
             {NOTHING_NAMED}"
        ),
        "1 tool",
    );
    assert!(
        unset.stdout.contains(&format!(
            "    credentials:\n      GITHUB_TOKEN: ${{{UNSET}:-}} # environment\n"
        )),
        "an unset entry is written as the file wrote it: {unset:?}"
    );
    assert_eq!(
        resolved(&unset).digest().to_string(),
        line_of(&unset, "digest:    ")["digest:    ".len()..]
    );
}

#[test]
fn every_default_the_resolved_config_holds_is_the_one_spec_7_gives() {
    // C17: the spec's config states every setting, and its example servers
    // are the one thing it states that isn't a default.
    let lab = Lab::new("check-defaults");
    let mut spec = RawConfig::from_str(SPEC, Format::Yaml)
        .unwrap()
        .substituted(|_: &str| Some(OsString::from("placeholder")))
        .unwrap()
        .config()
        .unwrap();
    spec.tools.mcp.clear();
    lab.write(
        "lablet.yaml",
        "model:\n  api_key: ${ANTHROPIC_API_KEY}\nprompt:\n  system: You are ...\n",
    );
    let mut command = lab.lablet(&["check", "--config", "lablet.yaml", "--resolved"]);
    command.env("ANTHROPIC_API_KEY", "placeholder");

    let run = ran(command, "");

    passed(
        &run,
        &format!(
            "own:       ANTHROPIC_API_KEY (line 2) (under 16 bytes, not cut) -> model.api_key; no \
             process inherits it\ngiven:     none\nenv:       none\n{NOTHING_NAMED}"
        ),
        "0 tools",
    );
    let printed: Value = serde_saphyr::from_str(&run.stdout).unwrap();
    assert_eq!(printed, serde_json::to_value(spec.resolved()).unwrap());
    assert_eq!(resolved(&run).digest(), spec.digest());
}

/// `--resolved` says where each written setting came from, after it: `#
/// line N` for one the file wrote, a nested key and a list item alike, `#
/// --set` for one an override set, and `# environment` for one a variable
/// filled, whose value is written as the file wrote it; a default has no
/// comment, and the whole reads back with the same digest. An item of a
/// list the config holds sorted, or once where the file wrote it twice, is
/// told by its value, so its comment is its own and not the comment of the
/// item the file wrote at its index.
#[test]
fn the_resolved_config_comments_each_setting_with_its_line_its_override_or_the_environment() {
    let lab = Lab::new("check-resolved-comments");
    lab.write("script.yaml", ENDS);
    lab.write("work/notes.md", "Nothing yet.");
    lab.write(
        "lablet.yaml",
        "model:\n  provider: fake\n  script: script.yaml\n  name: ${LABLET_TEST_MODEL}\nprompt:\n  \
         system: Hi.\nrun:\n  max_turns: 3\ntools:\n  builtin:\n    root: work\n    enabled:\n      \
         - write_file\n      - read_file\n  deny:\n    - write_file\n    - ${LABLET_TEST_DENIED}\n\
         telemetry:\n  propagators:\n    - b3\n    - B3\n    - tracecontext\n",
    );
    let held = [
        ("LABLET_TEST_MODEL", "scripted-9"),
        ("LABLET_TEST_DENIED", "read_file"),
    ];
    let mut command = lab.lablet(&[
        "check",
        "--config",
        "lablet.yaml",
        "--resolved",
        "--set",
        "run.completion=explicit",
    ]);
    command.envs(held);

    let run = ran(command, "");

    passed(&run, NONE, "1 tool");
    for line in [
        "  provider: fake # line 2\n",
        "  script: script.yaml # line 3\n",
        "  name: ${LABLET_TEST_MODEL} # environment\n",
        "  system: Hi. # line 6\n",
        "  max_turns: 3 # line 8\n",
        "  completion: explicit # --set\n",
        "    root: work # line 11\n",
        "    enabled:\n    - read_file # line 14\n    - write_file # line 13\n",
        "  deny:\n  - ${LABLET_TEST_DENIED} # environment\n  - write_file # line 16\n",
        "  propagators:\n  - b3 # line 20\n  - tracecontext # line 22\n",
        "  max_tokens: ",
    ] {
        assert!(run.stdout.contains(line), "{line:?}: {run:?}");
    }
    let max_tokens = run
        .stdout
        .lines()
        .find(|line| line.starts_with("  max_tokens: "))
        .unwrap();
    assert!(
        !max_tokens.contains('#'),
        "a default has no source: {max_tokens}"
    );
    assert!(!run.stdout.contains("scripted-9"), "{run:?}");
    let read_back = RawConfig::from_str(&run.stdout, Format::Yaml)
        .unwrap()
        .substituted(|name: &str| {
            held.iter()
                .find(|(held, _)| *held == name)
                .map(|(_, value)| OsString::from(value))
        })
        .unwrap()
        .config()
        .unwrap();
    assert_eq!(
        read_back.digest().to_string(),
        line_of(&run, "digest:    ")["digest:    ".len()..]
    );
}

/// `--resolved` comments each propagator of a list written as one text,
/// `tracecontext,b3`, with where the text was written, since no item has a
/// line of its own: the key's line, or `# --set` where an override set it.
/// A text a variable filled is written back whole, as the file wrote it,
/// with `# environment`.
#[test]
fn the_resolved_config_comments_each_propagator_written_as_one_text_with_where_the_text_was_written()
 {
    let lab = Lab::new("check-resolved-propagators-text");
    lab.write("script.yaml", ENDS);
    let items = |comment: &str| {
        format!("  propagators:\n  - tracecontext # {comment}\n  - b3 # {comment}\n")
    };
    for (written, set, expected) in [
        ("'tracecontext, B3'", None, items("line 7")),
        (
            "baggage",
            Some("telemetry.propagators=tracecontext,b3"),
            items("--set"),
        ),
        (
            "${LABLET_TEST_PROPAGATORS}",
            None,
            "  propagators: ${LABLET_TEST_PROPAGATORS} # environment\n".to_owned(),
        ),
    ] {
        lab.write(
            "lablet.yaml",
            &format!(
                "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\n\
                 telemetry:\n  propagators: {written}\n"
            ),
        );
        let mut args = vec!["check", "--config", "lablet.yaml", "--resolved"];
        if let Some(set) = set {
            args.extend(["--set", set]);
        }
        let mut command = lab.lablet(&args);
        command.env("LABLET_TEST_PROPAGATORS", "tracecontext,b3");

        let run = ran(command, "");

        assert_eq!(run.code, Some(0), "{run:?}");
        assert!(run.stdout.contains(&expected), "{written}: {run:?}");
    }
}

/// A relative path the file writes starts at the file's directory, from
/// whichever directory lablet runs in, and one `--set` writes where lablet
/// runs; `--resolved` keeps each as written.
#[test]
fn a_relative_root_in_the_file_starts_at_the_files_directory_and_one_set_on_the_command_line_where_lablet_runs()
 {
    let lab = Lab::new("check-relative-root");
    lab.write("repo/notes.md", "Nothing yet.");
    lab.write("conf/script.yaml", ENDS);
    let config = |root: &str| {
        format!(
            "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\ntools:\n  \
             builtin:\n    root: {root}\n    enabled: [read_file]\n"
        )
    };

    lab.write("conf/lablet.yaml", &config("../repo"));
    let from_the_file = lab.run(&["check", "--config", "conf/lablet.yaml", "--resolved"]);
    passed(&from_the_file, NONE, "1 tool");
    assert!(
        from_the_file
            .stdout
            .contains("    root: ../repo # line 8\n"),
        "{from_the_file:?}"
    );
    assert!(
        from_the_file
            .stdout
            .contains("  script: script.yaml # line 3\n"),
        "{from_the_file:?}"
    );

    lab.write("conf/lablet.yaml", &config("repo"));
    let beside_the_file = lab.run(&["check", "--config", "conf/lablet.yaml"]);
    assert_eq!(beside_the_file.code, Some(1), "{beside_the_file:?}");
    assert!(
        beside_the_file
            .stderr
            .starts_with("config: tools.builtin.root (line 8): \"repo\" is refused: "),
        "conf/repo is no directory: {beside_the_file:?}"
    );

    let where_lablet_runs = lab.run(&[
        "check",
        "--config",
        "conf/lablet.yaml",
        "--set",
        "tools.builtin.root=repo",
        "--resolved",
    ]);
    passed(&where_lablet_runs, NONE, "1 tool");
    assert!(
        where_lablet_runs
            .stdout
            .contains("    root: repo # --set\n"),
        "{where_lablet_runs:?}"
    );
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

    passed(&run, NONE_WITH_BASH, "2 tools");
    assert_eq!(run.stdout, "bash\nread_file\n");
    assert!(!lab.telemetry().exists(), "a run started");
}

/// T16 at the command line: the names a check prints, each own credential
/// with the field it fills and a copy of its value the environment holds
/// under another name, each given credential with the name `bash` gets it
/// under, each `env` entry, and `inherited:` naming none of them; a value
/// a variable gave in no line of it; and `--resolved` printing each
/// `${VAR}` as written, with where it came from.
#[test]
fn a_check_prints_the_credentials_and_what_bash_inherits_and_no_value() {
    let lab = Lab::new("check-secrets");
    lab.write("script.yaml", ENDS);
    lab.write("work/notes.md", "Nothing yet.");
    lab.write(
        "lablet.yaml",
        "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\ntools:\n  \
         builtin:\n    root: work\n    enabled: [bash]\n    env:\n      PATH: /usr/bin\n    \
         credentials:\n      GITHUB_TOKEN: ${LABLET_TEST_GH}\ntelemetry:\n  otlp:\n    \
         headers:\n      Authorization: Bearer ${LABLET_TEST_TOKEN}\n      X-Short: \
         ${LABLET_TEST_SHORT}\n",
    );
    let token = "tok-0123456789abcdef-no-stderr";
    let github = "ghp-0123456789abcdef-no-stderr";
    let check = |args: &[&str]| {
        let mut command = lab.lablet(&[&["check", "--config", "lablet.yaml"][..], args].concat());
        command
            .env("LABLET_TEST_TOKEN", token)
            .env("LABLET_TEST_COPY", token)
            .env("LABLET_TEST_SHORT", "short")
            .env("LABLET_TEST_GH", github);
        ran(command, "")
    };

    let plain = check(&[]);
    passed(
        &plain,
        &format!(
            "own:       LABLET_TEST_TOKEN (line 17) -> telemetry.otlp.headers.Authorization, or \
             LABLET_TEST_COPY, which holds it; no process inherits it\n\
             own:       LABLET_TEST_SHORT (line 18) (under 16 bytes, not cut) -> \
             telemetry.otlp.headers.X-Short; no process inherits it\n\
             given:     LABLET_TEST_GH (line 13) -> bash as GITHUB_TOKEN\n\
             env:       PATH (line 11) -> bash\n\
             inherited: <inherited>, as set; BAGGAGE, TRACEPARENT, TRACESTATE, its tool span's\n\
             {NOTHING_NAMED}"
        ),
        "1 tool",
    );
    for value in [token, github] {
        assert!(!plain.stderr.contains(value), "{plain:?}");
    }
    let inherited = line_of(&plain, "inherited:");
    for name in [
        "LABLET_TEST_TOKEN",
        "LABLET_TEST_COPY",
        "LABLET_TEST_SHORT",
        "LABLET_TEST_GH",
    ] {
        assert!(!inherited.contains(name), "{inherited}");
    }

    let resolved = check(&["--resolved"]);
    assert_eq!(resolved.code, Some(0), "{resolved:?}");
    for written in [
        "Authorization: Bearer ${LABLET_TEST_TOKEN} # environment",
        "GITHUB_TOKEN: ${LABLET_TEST_GH} # environment",
        "X-Short: ${LABLET_TEST_SHORT} # environment",
    ] {
        assert!(resolved.stdout.contains(written), "{written}: {resolved:?}");
    }
    for value in [token, github] {
        assert!(
            !resolved.stdout.contains(value) && !resolved.stderr.contains(value),
            "{resolved:?}"
        );
    }
}

/// R62 at the command line: a header value written with no `${VAR}` is a
/// token written out, refused by its name in the map form and by its pair in
/// the text form, from `check` and from `run`, never shown.
#[test]
fn a_header_value_with_no_variable_is_refused_and_never_shown() {
    let lab = Lab::new("check-literal-header");
    lab.write("script.yaml", ENDS);
    let literal = "literal-0123456789abcdef";
    let config = |headers: &str| {
        format!(
            "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\ntelemetry:\n  \
             otlp:\n    endpoint: http://127.0.0.1:1\n    headers: {headers}\n"
        )
    };

    for (headers, said) in [
        (
            format!("\n      Authorization: Bearer ${{{KEY}}}\n      X-Literal: {literal}"),
            "config: telemetry.otlp.headers.X-Literal (line 11) is refused: it holds no ${VAR}, \
             and what it holds isn't shown; write X-Literal: ${X_LITERAL}, and rotate it if this \
             was a token"
                .to_owned(),
        ),
        (
            format!("authorization=Bearer%20${{{KEY}}},x-literal={literal}"),
            "config: telemetry.otlp.headers (line 9) is refused: its pair `x-literal` holds no \
             ${VAR}, and what it holds isn't shown; write x-literal=${X_LITERAL}, and rotate it if \
             this was a token"
                .to_owned(),
        ),
    ] {
        let checked = check(&lab, &config(&headers), &[]);
        refused(&checked, &said);
        assert!(!checked.stderr.contains(literal), "{checked:?}");

        let mut command = lab.lablet(&["run", "--config", "lablet.yaml", "--prompt", PROMPT]);
        command.env(KEY, "placeholder");
        let run = ran(command, "");
        refused(&run, &said);
        assert!(!lab.telemetry().exists(), "a run began: {run:?}");
    }
}

/// R63: a URL's user and password are each their own `${VAR}`, so a
/// variable that holds a whole URL with a password in it is refused by its
/// name, at `model.base_url` and at `telemetry.otlp.endpoint`, by `check`
/// and by `run`, and nothing of what it holds is shown; written as their
/// own variables, the user and the password are two own credentials.
#[test]
fn a_whole_url_variable_that_holds_a_user_or_password_is_refused_by_its_name() {
    let lab = Lab::new("check-url-credentials");
    lab.write("script.yaml", ENDS);
    let password = "gwsecretpassword1234567";
    let (url, user, pass) = (
        "LABLET_TEST_GATEWAY_URL",
        "LABLET_TEST_GW_USER",
        "LABLET_TEST_GW_PASS",
    );
    let with = |subcommand: &[&str], text: &str| {
        lab.write("lablet.yaml", text);
        let mut command = lab.lablet(&[subcommand, &["--config", "lablet.yaml"][..]].concat());
        command
            .env(KEY, "placeholder")
            .env(url, format!("https://gwuser:{password}@gw.example/v1"))
            .env(user, "gwuser")
            .env(pass, password);
        ran(command, "")
    };
    let gateway = |base_url: &str| {
        format!(
            "model:\n  provider: anthropic\n  api_key: ${{{KEY}}}\n  base_url: {base_url}\n\
             prompt:\n  system: Hi.\n"
        )
    };
    let collector = |endpoint: &str| {
        format!(
            "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\ntelemetry:\n  \
             otlp:\n    endpoint: {endpoint}\n"
        )
    };
    let fix = "holds a user or password, which isn't shown; write each as its own ${VAR}, as \
               https://${GW_USER}:${GW_PASS}@gw/v1";

    let whole = with(&["check"], &gateway(&format!("${{{url}}}")));
    refused(
        &whole,
        &format!("config: model.base_url (line 4) is refused: `{url}` {fix}"),
    );
    for (subcommand, args) in [("check", &[][..]), ("run", &["--prompt", PROMPT][..])] {
        let refusal = with(
            &[&[subcommand][..], args].concat(),
            &collector(&format!("${{{url}:-}}")),
        );
        refused(
            &refusal,
            &format!("config: telemetry.otlp.endpoint (line 8) is refused: `{url}` {fix}"),
        );
        assert!(!refusal.stderr.contains(password), "{refusal:?}");
    }
    assert!(!whole.stderr.contains(password), "{whole:?}");

    let own = with(
        &["check"],
        &gateway(&format!("https://${{{user}}}:${{{pass}}}@gw.example/v1")),
    );
    assert_eq!(own.code, Some(0), "{own:?}");
    for variable in [user, pass] {
        assert!(
            own.stderr.contains(&format!(
                "own:       {variable} (line 4) -> model.base_url; no process inherits it\n"
            )),
            "{own:?}"
        );
    }
    assert!(!own.stderr.contains(password), "{own:?}");
}

/// R17's URL form: a password a URL's text writes out, in the file, in a
/// `${VAR:-default}`'s default or beside a `${VAR}`, or by `--set`, is
/// refused by `check` and by `run` by its line or as an override, and
/// nothing of what the URL holds is shown; a user and a password each a
/// `${VAR}` passes.
#[test]
fn a_url_s_password_written_out_is_refused_by_check_and_by_run_and_never_shown() {
    let lab = Lab::new("check-url-written-out");
    lab.write("script.yaml", ENDS);
    let password = "gwsecretpassword1234567";
    let (url, user) = ("LABLET_TEST_GATEWAY_URL", "LABLET_TEST_GW_USER");
    let with = |subcommand: &[&str], text: &str, set: Option<&str>| {
        lab.write("lablet.yaml", text);
        let mut args = [subcommand, &["--config", "lablet.yaml"][..]].concat();
        if let Some(set) = set {
            args.extend(["--set", set]);
        }
        let mut command = lab.lablet(&args);
        command
            .env(KEY, "placeholder")
            .env(user, "gwuser-0123456789abcdef")
            .env_remove(url);
        ran(command, "")
    };
    let collector = |endpoint: &str| {
        format!(
            "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\ntelemetry:\n  \
             otlp:\n    endpoint: {endpoint}\n"
        )
    };
    let why = "is refused: its user or password isn't a ${VAR}, and what it holds isn't shown";
    let rotate = "and rotate it if this was one";
    let in_file = format!(
        "config: telemetry.otlp.endpoint (line 8) {why}; write \
         https://${{GW_USER}}:${{GW_PASS}}@gw/v1, {rotate}"
    );
    let by_override = format!(
        "config: telemetry.otlp.endpoint (an override) {why}; write --set \
         'telemetry.otlp.endpoint=https://${{GW_USER}}:${{GW_PASS}}@gw/v1', {rotate}"
    );
    let literal = format!("https://gwuser:{password}@127.0.0.1:1");
    for (subcommand, args) in [("check", &[][..]), ("run", &["--prompt", PROMPT][..])] {
        let subcommand = [&[subcommand][..], args].concat();
        for (text, set, said) in [
            (collector(&format!("'{literal}'")), None, &in_file),
            (
                collector(&format!("'${{{url}:-{literal}}}'")),
                None,
                &in_file,
            ),
            (
                collector(&format!("'https://${{{user}}}:{password}@127.0.0.1:1'")),
                None,
                &in_file,
            ),
            (
                collector("null"),
                Some(format!("telemetry.otlp.endpoint={literal}")),
                &by_override,
            ),
        ] {
            let refusal = with(&subcommand, &text, set.as_deref());
            refused(&refusal, said);
            assert!(!refusal.stderr.contains(password), "{refusal:?}");
            assert!(!lab.telemetry().exists(), "a run began: {refusal:?}");
        }
    }

    let own = with(
        &["check"],
        &collector(&format!("'https://${{{user}}}@127.0.0.1:1'")),
        None,
    );
    assert_eq!(own.code, Some(0), "{own:?}");
    assert!(
        own.stderr.contains(&format!(
            "own:       {user} (line 8) -> telemetry.otlp.endpoint; no process inherits it\n"
        )),
        "{own:?}"
    );
}

/// R40 and R38 at the command line, from `check` and from `run`: an `env`
/// entry that names its own variable where `bash` inherits it already is
/// refused, and so is one filled from the variable that fills
/// `model.api_key`; moved under `credentials`, a token is given knowingly.
#[test]
fn an_env_entry_that_names_a_credential_is_refused_by_check_and_by_run() {
    let lab = Lab::new("check-env-credential");
    lab.write("script.yaml", ENDS);
    lab.write("work/notes.md", "Nothing yet.");
    let github = "LABLET_TEST_GH";
    let config = |entry: &str| {
        format!(
            "model:\n  provider: fake\n  script: script.yaml\n  api_key: ${{{KEY}}}\nprompt:\n  \
             system: Hi.\ntools:\n  builtin:\n    root: work\n    enabled: [bash]\n    env:\n      \
             {entry}\n"
        )
    };
    let with = |subcommand: &[&str], text: &str| {
        lab.write("lablet.yaml", text);
        let mut command = lab.lablet(&[subcommand, &["--config", "lablet.yaml"][..]].concat());
        command
            .env(KEY, "sk-test-0123456789abcdef")
            .env(github, "ghp-0123456789abcdef");
        ran(command, "")
    };
    let check: &[&str] = &["check"];
    let run: &[&str] = &["run", "--prompt", "Hi."];

    let same_name = config(&format!("{github}: ${{{github}}}"));
    let r40 = format!(
        "config: tools.builtin.env.{github} (line 12): \"${{{github}}}\" is refused: bash inherits \
         `{github}` as set already, uncut; remove the line, or move it under \
         `tools.builtin.credentials` to have it cut"
    );
    refused(&with(check, &same_name), &r40);
    refused(&with(run, &same_name), &r40);

    let from_the_key = config(&format!("T: ${{{KEY}}}"));
    let r38 = format!(
        "config: tools.builtin.env.T (line 12): \"${{{KEY}}}\" is refused: `{KEY}` fills \
         `model.api_key`, and no process inherits an own credential; to give bash the key \
         knowingly, list T under `tools.builtin.credentials`"
    );
    refused(&with(check, &from_the_key), &r38);
    refused(&with(run, &from_the_key), &r38);
    assert!(!lab.telemetry().exists(), "a run started");

    let given = with(
        check,
        &config(&format!(
            "PATH: /usr/bin\n    credentials:\n      GITHUB_TOKEN: ${{{github}}}"
        )),
    );
    passed(
        &given,
        &format!(
            "own:       {KEY} (line 4) -> model.api_key; no process inherits it\n\
             given:     {github} (line 14) -> bash as GITHUB_TOKEN\n\
             env:       PATH (line 12) -> bash\n\
             inherited: <inherited>, as set; BAGGAGE, TRACEPARENT, TRACESTATE, its tool span's\n\
             {NOTHING_NAMED}"
        ),
        "1 tool",
    );
    assert!(!given.stderr.contains("0123456789abcdef"), "{given:?}");
}

/// W66: lablet's own key listed under `credentials` passes, with a warning
/// that `bash` has it, and `own:` says who gets it.
#[test]
fn an_own_credential_given_to_bash_passes_with_a_warning() {
    let lab = Lab::new("check-own-given");
    lab.write("script.yaml", ENDS);
    lab.write("work/notes.md", "Nothing yet.");
    lab.write(
        "lablet.yaml",
        &format!(
            "model:\n  provider: fake\n  script: script.yaml\n  api_key: ${{{KEY}}}\nprompt:\n  \
             system: Hi.\ntools:\n  builtin:\n    root: work\n    enabled: [bash]\n    \
             credentials:\n      {KEY}: ${{{KEY}}}\n"
        ),
    );
    let mut command = lab.lablet(&["check", "--config", "lablet.yaml"]);
    command.env(KEY, "sk-test-0123456789abcdef");

    let run = ran(command, "");

    assert_eq!(run.code, Some(0), "{run:?}");
    assert!(
        run.stderr.contains(&format!(
            "tools.builtin.credentials.{KEY} (line 12): it's lablet's own key, given to bash; \
             a task that needs the key gets it only when the config says so"
        )),
        "{run:?}"
    );
    assert!(
        normalised(&run.stderr).contains(&format!(
            "own:       {KEY} (line 4) -> model.api_key; given to bash by credentials (line \
             12)\ngiven:     {KEY} (line 12) -> bash\n"
        )),
        "{run:?}"
    );
    assert!(!run.stderr.contains("0123456789abcdef"), "{run:?}");
}

/// The script of a run whose one command prints the header variable as
/// `bash` sees it.
const PRINTS_HEADERS: &str = "
- response:
    content:
      - tool_use:
          id: call_1
          name: bash
          input: { json: { command: \"env | grep '^OTEL_EXPORTER_OTLP_HEADERS=' || echo unset\" } }
    finish: tool_use
- response:
    content:
      - text: The variable is printed.
    finish: end_turn
";

/// `unread:` names what lablet read before the caller owned the config, a
/// name no field takes yet marked so, and no metrics variable, since
/// lablet never read one.
#[test]
fn unread_names_a_variable_lablet_read_before_and_no_metrics_variable() {
    let lab = Lab::new("check-unread");
    lab.write("script.yaml", ENDS);
    lab.write(
        "lablet.yaml",
        "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\n",
    );
    let mut command = lab.lablet(&["check", "--config", "lablet.yaml"]);
    command
        .env("OTEL_TRACES_SAMPLER", "always_off")
        .env("OTEL_METRICS_EXPORTER", "otlp")
        .env("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", "http://127.0.0.1:1");

    let run = ran(command, "");

    passed(
        &run,
        "own:       none\ngiven:     none\nenv:       none\nunread:    OTEL_TRACES_SAMPLER (no \
         field yet): set, and read only where the file writes ${NAME}\ntelemetry: none named, so \
         a run sends and writes nothing",
        "0 tools",
    );
}

/// C19, inverted, as the caller owning the config rewords it: a header
/// variable the file doesn't map is the framework's: `bash` inherits it
/// uncut, `own:` doesn't name it and `unread:` does, with no line holding
/// its value; `--resolved` under an endpoint variable prints `null` for the
/// endpoint, as written, and nothing the environment holds, with no
/// warning about a protocol variable, since neither is read; and `enabled`
/// is a key no setting has.
#[test]
fn an_unmapped_otlp_header_variable_reaches_bash_uncut_and_is_named_on_unread_and_enabled_is_an_unknown_key()
 {
    let lab = Lab::new("check-otel");
    lab.write("work/notes.md", "Nothing yet.");
    lab.write_config(
        PRINTS_HEADERS,
        json!({
            "tools": { "builtin": lab.builtin(&["bash"]) },
            "run": { "transcript_path": "transcript.json" },
        }),
    );
    let token = "tok-0123456789abcdef-no-stderr";
    let headers = format!("authorization=Bearer%20{token}");
    let with = |args: &[&str], env: &[(&str, &str)]| {
        let mut command = lab.lablet(args);
        command.envs(env.iter().copied());
        ran(command, "")
    };

    let named = with(
        &["check", "--config", CONFIG],
        &[("OTEL_EXPORTER_OTLP_HEADERS", &headers)],
    );
    passed(
        &named,
        "own:       none\ngiven:     none\nenv:       none\ninherited: <inherited>, as set; \
         BAGGAGE, TRACEPARENT, TRACESTATE, its tool span's\nunread:    OTEL_EXPORTER_OTLP_HEADERS: \
         set, and read only where the file writes ${NAME}\ntelemetry: file (line 1)",
        "1 tool",
    );
    assert!(
        line_of(&named, "inherited:").contains("OTEL_EXPORTER_OTLP_HEADERS"),
        "{named:?}"
    );
    assert!(!named.stderr.contains(token), "{named:?}");

    let run = with(
        &["run", "--config", CONFIG, "--prompt", PROMPT],
        &[("OTEL_EXPORTER_OTLP_HEADERS", &headers)],
    );
    assert_eq!(run.code, Some(0), "{run:?}");
    let transcript = json_of(&lab.at("transcript.json"));
    let printed: String = transcript["turns"][0]["tool_calls"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        printed,
        format!("OTEL_EXPORTER_OTLP_HEADERS={headers}\nexit code: 0"),
        "uncut: the variable is the framework's, not lablet's"
    );

    let text = "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\n";
    lab.write("script.yaml", ENDS);
    lab.write("lablet.yaml", text);
    let printed = with(
        &["check", "--config", "lablet.yaml", "--resolved"],
        &[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:1"),
            ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/thrift"),
        ],
    );
    passed(
        &printed,
        "own:       none\ngiven:     none\nenv:       none\nunread:    \
         OTEL_EXPORTER_OTLP_PROTOCOL, OTEL_EXPORTER_OTLP_ENDPOINT: set, and read only where the \
         file writes ${NAME}\ntelemetry: none named, so a run sends and writes nothing",
        "0 tools",
    );
    assert!(
        printed.stdout.contains(
            "  otlp:\n    endpoint: null\n    protocol: null\n    headers: null\n    tls:\n      \
             ca_file: null\n      client_cert: null\n      client_key: null\n"
        ),
        "{printed:?}"
    );
    for held in ["127.0.0.1", "thrift"] {
        assert!(
            !printed.stdout.contains(held) && !printed.stderr.contains(held),
            "the environment's value is nowhere: {printed:?}"
        );
    }
    assert_eq!(
        resolved(&printed).digest(),
        Config::from_str(text, Format::Yaml).unwrap().digest()
    );

    lab.write(
        "lablet.yaml",
        &format!(
            "{text}telemetry:\n  otlp:\n    enabled: false\n    endpoint: http://localhost:4317\n"
        ),
    );
    let unknown = with(&["check", "--config", "lablet.yaml"], &[]);
    assert_eq!(unknown.code, Some(1), "{unknown:?}");
    assert!(
        unknown.stderr.starts_with(
            "config: telemetry.otlp.enabled (line 8) is refused: no setting has the key; the keys \
             beside it are `endpoint`, `protocol`, `headers`, `tls`"
        ),
        "{unknown:?}"
    );
    assert_eq!(unknown.stderr_lines().len(), 1, "{unknown:?}");
}

/// `telemetry:` names where a run writes and sends, each with its line,
/// the endpoint less anything from a `?` on, and, with an endpoint named,
/// a compression variable the exporter reads for itself; a file alone is
/// named alone.
#[test]
fn the_telemetry_line_names_the_file_the_endpoint_and_a_compression_variable_the_exporter_reads() {
    let lab = Lab::new("check-telemetry-line");
    lab.write("script.yaml", ENDS);
    let both = "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\ntelemetry:\n  \
                file:\n    path: out.jsonl\n  otlp:\n    endpoint: http://127.0.0.1:1/v1?team=a\n    \
                protocol: grpc\n";

    let plain = check(&lab, both, &[]);
    passed(
        &plain,
        "own:       none\ngiven:     none\nenv:       none\nunread:    none\ntelemetry: file (line \
         8); OTLP grpc to http://127.0.0.1:1/v1 (line 10)",
        "0 tools",
    );

    lab.write("lablet.yaml", both);
    let mut command = lab.lablet(&["check", "--config", "lablet.yaml"]);
    command.env("OTEL_EXPORTER_OTLP_COMPRESSION", "gzip");
    let compressed = ran(command, "");
    passed(
        &compressed,
        "own:       none\ngiven:     none\nenv:       none\nunread:    none\ntelemetry: file (line \
         8); OTLP grpc to http://127.0.0.1:1/v1 (line 10); OTEL_EXPORTER_OTLP_COMPRESSION is set, \
         and the exporter reads it itself",
        "0 tools",
    );

    let file_alone = check(
        &lab,
        "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\ntelemetry:\n  \
         file:\n    path: out.jsonl\n",
        &[],
    );
    passed(
        &file_alone,
        "own:       none\ngiven:     none\nenv:       none\nunread:    none\ntelemetry: file (line 8)",
        "0 tools",
    );
    assert!(!lab.at("out.jsonl").exists(), "a check writes no telemetry");
}
