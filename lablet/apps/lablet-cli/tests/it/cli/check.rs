//! `lablet check`: the config checked whole without a call to the provider,
//! a refusal as a `config:` message that names the key, where it was
//! written and its value as written, and what a config that passed comes
//! to: its credentials, what `bash` inherits, and its digest.

use std::ffi::OsString;

use lablet::{Config, Format, RawConfig};
use serde_json::{Value, json};

use super::harness::{CONFIG, ENDS, Lab, Ran, ran};

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

/// The lines a check of a config with no credential, no `env` entry, no
/// `bash` and nothing cut ends with before the digest.
const NONE: &str = "own:       none\ngiven:     none\nenv:       none\ncut:       none";

/// The same for a config with `bash`, whose `inherited:` line names what
/// the binary's environment holds.
const NONE_WITH_BASH: &str = "own:       none\ngiven:     none\nenv:       none\ninherited: \
                              <inherited>, as set; BAGGAGE, TRACEPARENT, TRACESTATE, its tool \
                              span's\ncut:       none";

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

    passed(&run, NONE, "0 tools");
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
             it\ngiven:     none\nenv:       none\ncut:       none"
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
         inherits it\ngiven:     none\nenv:       none\ncut:       none"
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
    // C10, with the prompt left as the file wrote it: a `${VAR}` in it is
    // text, so an unset variable there refuses nothing.
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
            "config: model.name (line 4): \"${{{UNSET}}}\" is refused: the variable `{UNSET}` \
             isn't set"
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
    // The command line configures its SDK from them, as a library doesn't.
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
                "config: {key} (line 6): \"${{{UNSET}}}\" is refused: the variable `{UNSET}` isn't \
                 set"
            ),
        );
    }
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
        "own:       ANTHROPIC_API_KEY (line 2) (under 16 bytes, not cut) -> model.api_key; no \
         process inherits it\ngiven:     none\nenv:       none\ncut:       none",
        "0 tools",
    );
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

    passed(&run, NONE_WITH_BASH, "2 tools");
    assert_eq!(run.stdout, "bash\nread_file\n");
    assert!(!lab.telemetry().exists(), "a run started");
}

/// T16 at the command line: the names a check prints, each own credential
/// with the field it fills and a copy of its value the environment holds
/// under another name, each given credential with the name `bash` gets it
/// under, each `env` entry, and `inherited:` naming none of them; a value
/// a variable gave in no line of it; and `--resolved` printing each
/// `${VAR}` as written and a credential written out as `[secret]`.
#[test]
fn a_check_prints_the_credentials_and_what_bash_inherits_and_no_value() {
    let lab = Lab::new("check-secrets");
    lab.write("script.yaml", ENDS);
    lab.write(
        "lablet.yaml",
        "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\ntools:\n  \
         builtin:\n    root: work\n    enabled: [bash]\n    env:\n      PATH: /usr/bin\n    \
         credentials:\n      GITHUB_TOKEN: ${LABLET_TEST_GH}\ntelemetry:\n  otlp:\n    \
         headers:\n      Authorization: Bearer ${LABLET_TEST_TOKEN}\n      X-Literal: \
         literal-0123456789abcdef\n      X-Short: ${LABLET_TEST_SHORT}\n",
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
        "own:       LABLET_TEST_TOKEN (line 17) -> telemetry.otlp.headers.Authorization, or \
         LABLET_TEST_COPY, which holds it; no process inherits it\n\
         own:       telemetry.otlp.headers.X-Literal (line 18) holds its value; no process \
         inherits it\n\
         own:       LABLET_TEST_SHORT (line 19) (under 16 bytes, not cut) -> \
         telemetry.otlp.headers.X-Short; no process inherits it\n\
         given:     LABLET_TEST_GH (line 13) -> bash as GITHUB_TOKEN\n\
         env:       PATH (line 11) -> bash\n\
         inherited: <inherited>, as set; BAGGAGE, TRACEPARENT, TRACESTATE, its tool span's\n\
         cut:       none",
        "1 tool",
    );
    for value in [token, github, "literal-0123456789abcdef"] {
        assert!(!plain.stderr.contains(value), "{plain:?}");
    }
    let inherited = plain
        .stderr_lines()
        .into_iter()
        .find(|line| line.starts_with("inherited:"))
        .unwrap()
        .to_owned();
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
        "Authorization: Bearer ${LABLET_TEST_TOKEN}",
        "GITHUB_TOKEN: ${LABLET_TEST_GH}",
        "X-Short: ${LABLET_TEST_SHORT}",
        "[secret]",
    ] {
        assert!(resolved.stdout.contains(written), "{written}: {resolved:?}");
    }
    for value in [token, github, "literal-0123456789abcdef"] {
        assert!(
            !resolved.stdout.contains(value) && !resolved.stderr.contains(value),
            "{resolved:?}"
        );
    }
}

/// R40 and R38 at the command line, from `check` and from `run`: an `env`
/// entry that names its own variable where `bash` inherits it already is
/// refused, and so is one filled from the variable that fills
/// `model.api_key`; moved under `credentials`, a token is given knowingly.
#[test]
fn an_env_entry_that_names_a_credential_is_refused_by_check_and_by_run() {
    let lab = Lab::new("check-env-credential");
    lab.write("script.yaml", ENDS);
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
             cut:       none"
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

/// C19: a header variable the exporter reads is named among what's cut and
/// not among what's own, with no line holding its value; `--resolved`
/// prints `null` for every field left to the environment, as written, and
/// nothing the environment holds; a protocol from the environment lablet
/// doesn't know passes with a warning naming the variable and the value;
/// and `enabled: false` beside an endpoint is refused as a setting without
/// effect.
#[test]
fn an_otlp_header_variable_is_cut_and_not_own_and_what_the_environment_cannot_set_is_refused() {
    let lab = Lab::new("check-otel");
    lab.write("script.yaml", ENDS);
    let text = "model:\n  provider: fake\n  script: script.yaml\nprompt:\n  system: Hi.\n";
    lab.write("lablet.yaml", text);
    let token = "tok-0123456789abcdef-no-stderr";
    let check = |args: &[&str], env: &[(&str, &str)]| {
        let mut command = lab.lablet(&[&["check", "--config", "lablet.yaml"][..], args].concat());
        command.envs(env.iter().copied());
        ran(command, "")
    };

    let headers = format!("authorization=Bearer%20{token}");
    let named = check(&[], &[("OTEL_EXPORTER_OTLP_HEADERS", &headers)]);
    passed(
        &named,
        "own:       none\ngiven:     none\nenv:       none\ncut:       OTEL_EXPORTER_OTLP_HEADERS",
        "0 tools",
    );
    assert!(!named.stderr.contains(token), "{named:?}");

    let printed = check(
        &["--resolved"],
        &[("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:1")],
    );
    passed(&printed, NONE, "0 tools");
    assert!(
        printed.stdout.contains(
            "telemetry:\n  capture_content: null\n  otlp:\n    enabled: null\n    endpoint: \
             null\n    protocol: null\n    headers: null\n"
        ),
        "{printed:?}"
    );
    assert!(
        !printed.stdout.contains("127.0.0.1"),
        "the environment's endpoint is nowhere in the resolved config: {printed:?}"
    );
    assert_eq!(
        resolved(&printed).digest(),
        Config::from_str(text, Format::Yaml).unwrap().digest()
    );

    let unknown = check(
        &[],
        &[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:1"),
            ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/thrift"),
        ],
    );
    assert_eq!(unknown.code, Some(0), "{unknown:?}");
    assert!(
        normalised(&unknown.stderr).ends_with(&format!(
            "{NONE}\ndigest:    <digest>\npassed:    0 tools\n"
        )),
        "{unknown:?}"
    );
    assert!(
        unknown.stderr.contains(
            "`OTEL_EXPORTER_OTLP_PROTOCOL` holds `http/thrift`, which isn't one lablet serves, so \
             it's ignored"
        ),
        "{unknown:?}"
    );

    lab.write(
        "lablet.yaml",
        &format!(
            "{text}telemetry:\n  otlp:\n    enabled: false\n    endpoint: http://localhost:4317\n"
        ),
    );
    refused(
        &check(&[], &[]),
        "config: telemetry.otlp.enabled (line 8): false is refused: it turns the network \
         exporter off, and `telemetry.otlp.endpoint: http://localhost:4317` names where it \
         sends; a config that wants it off states no endpoint",
    );
}
