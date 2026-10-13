use std::ffi::OsString;

use lablet_cli::compose::Report;
use lablet_config::{Config, Format, RawConfig};
use lablet_test_support::Scratch;
use serde_json::json;

use super::{answer, summary, written};

/// A variable cargo sets for every test, and so one that's set. Its value
/// is a path, as long as a key.
const SET: &str = "CARGO_MANIFEST_DIR";

/// A variable cargo sets for every test, whose value is under the floor.
const SHORT: &str = "CARGO_PKG_NAME";

/// A script the fake model can play, which a check reads as a run would.
const ENDS: &str = "- response: { content: [{ text: Done. }], finish: end_turn }\n";

/// The context variables a command is given, as `inherited:` ends.
const CONTEXT: &str = "BAGGAGE, TRACEPARENT, TRACESTATE, its tool span's";

/// What `limit:` says of `bash` on this platform.
const LIMIT: &str = if cfg!(target_os = "macos") {
    "bash can read any file you can, and your shell's environment and lablet's"
} else {
    "bash can read any file you can, and your shell's environment"
};

/// The value of [`SET`], which a test reads only to hold that it's nowhere.
fn held() -> String {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test reads the variable cargo set, to hold that its value is nowhere"
    )]
    let held = std::env::var(SET).unwrap();
    held
}

/// A config of the fake model with `tools`, whose system prompt holds a
/// `${VAR}`, which a prompt keeps as text.
fn tree(scratch: &Scratch, tools: &serde_json::Value) -> serde_json::Value {
    scratch.create_dir("work");
    json!({
        "run": { "max_turns": 5, "timeout": "2m" },
        "model": { "provider": "fake", "script": scratch.write("script.yaml", ENDS), "name": "scripted-1" },
        "prompt": { "system": format!("You fix the tests in ${{{SET}}}.") },
        "tools": tools,
    })
}

/// The config `tree` holds, filled from the test's environment as the
/// command line fills one.
fn config(tree: &serde_json::Value) -> Config {
    RawConfig::from_str(&tree.to_string(), Format::Json)
        .unwrap()
        .substituted(lablet_cli::input::variable)
        .unwrap()
        .config()
        .unwrap()
}

/// An environment handed in as the base every command starts from: `HOME`
/// and `PATH`, and `more`.
fn environment(more: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    [("HOME", "/home/t"), ("PATH", "/usr/bin")]
        .iter()
        .chain(more)
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

/// What `lablet_cli::compose::check` makes of the config `tree`, with
/// `environment` handed in, in a process environment where none of the
/// retired roster is set.
fn checked_tree(tree: &serde_json::Value, environment: Vec<(OsString, OsString)>) -> Report {
    let config = config(tree);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime
        .block_on(lablet_cli::compose::check_in(
            &config,
            environment,
            &|_: &str| false,
        ))
        .unwrap()
}

/// What `lablet_cli::compose::check` makes of a config of the fake model
/// with `tools`, with `HOME` and `PATH` handed in.
fn checked(scratch: &Scratch, tools: &serde_json::Value) -> Report {
    checked_tree(&tree(scratch, tools), environment(&[]))
}

/// The summary of `report`, with the digest it holds as `<digest>`.
fn summarised(report: &Report) -> String {
    summary(report).replace(&report.checked().digest().to_string(), "<digest>")
}

/// What the summary says of the telemetry and the roster for every config
/// here: nothing named, and nothing set.
const NOTHING: &str = "unread:    none\ntelemetry: none named, so a run sends and writes nothing";

#[test]
fn a_check_lists_the_tools_a_run_is_offered_a_line_each_in_their_order() {
    let scratch = Scratch::new("answer-tools");
    let checked = checked(
        &scratch,
        &json!({
            "builtin": { "root": scratch.at("work"), "enabled": ["write_file", "bash", "read_file"] },
            "deny": ["write_file"],
        }),
    );

    assert_eq!(answer(checked.checked(), None).unwrap(), "bash\nread_file");
    assert_eq!(
        summarised(&checked),
        format!(
            "own:       none\ngiven:     none\nenv:       none\ninherited: HOME, PATH, as set; \
             {CONTEXT}\n{NOTHING}\ndigest:    <digest>\nlimit:     {LIMIT}\npassed:    2 tools"
        )
    );
}

#[test]
fn a_check_of_a_config_with_no_tools_lists_nothing() {
    let scratch = Scratch::new("answer-no-tools");
    let checked = checked(&scratch, &json!({}));

    assert_eq!(answer(checked.checked(), None).unwrap(), "");
    assert_eq!(
        summarised(&checked),
        format!(
            "own:       none\ngiven:     none\nenv:       none\n{NOTHING}\ndigest:    \
             <digest>\npassed:    0 tools"
        )
    );
}

/// T16: the summary names each own credential by its variable and line and
/// the field it fills, with a copy of its value the environment holds under
/// another name, and a note on a value under the floor; `env:` names what's
/// passed; `inherited:` names no own variable and no copy; and no line
/// holds a value.
#[test]
fn a_check_names_the_credentials_and_what_bash_inherits_and_never_a_value() {
    let scratch = Scratch::new("answer-secrets");
    let mut tree = tree(
        &scratch,
        &json!({ "builtin": {
            "root": scratch.at("work"),
            "enabled": ["bash"],
            "env": { "PATH": "/usr/bin" },
        } }),
    );
    tree["telemetry"] = json!({ "otlp": { "headers": {
        "Authorization": format!("Bearer ${{{SET}}}"),
        "X-Short": format!("${{{SHORT}}}"),
    } } });
    let held = held();

    let checked = checked_tree(
        &tree,
        environment(&[(SET, &held), ("MY_COPY", &held), (SHORT, "lablet-cli")]),
    );
    let summary = summarised(&checked);

    assert_eq!(
        summary,
        format!(
            "own:       {SET} (line 1) -> telemetry.otlp.headers.Authorization, or MY_COPY, \
             which holds it; no process inherits it\n\
             own:       {SHORT} (line 1) (under 16 bytes, not cut) -> \
             telemetry.otlp.headers.X-Short; no process inherits it\n\
             given:     none\n\
             env:       PATH (line 1) -> bash\n\
             inherited: HOME, as set; {CONTEXT}\n\
             {NOTHING}\n\
             digest:    <digest>\n\
             limit:     {LIMIT}\n\
             passed:    1 tool"
        )
    );
    assert!(!summary.contains(&held), "{summary}");
}

#[test]
fn the_resolved_config_is_yaml_that_reads_back_as_the_config_it_resolves() {
    let scratch = Scratch::new("answer-resolved");
    let tree = tree(
        &scratch,
        &json!({ "builtin": {
            "root": scratch.at("work"),
            "enabled": ["bash"],
            "env": { "WHERE": format!("${{{SET}}}") },
        } }),
    );
    let checked = checked_tree(&tree, environment(&[]));

    let yaml = answer(checked.checked(), Some(&written(&tree))).unwrap();
    assert_eq!(
        summarised(&checked),
        format!(
            "own:       none\ngiven:     none\nenv:       WHERE (line 1) reaches bash uncut; if \
             it's a credential, move it under tools.builtin.credentials\ninherited: HOME, PATH, \
             as set; {CONTEXT}\n{NOTHING}\ndigest:    <digest>\nlimit:     \
             {LIMIT}\npassed:    1 tool"
        )
    );

    assert!(yaml.starts_with("run:\n"), "{yaml}");
    assert!(!yaml.ends_with('\n'), "{yaml:?}");
    let read_back = config(&serde_saphyr::from_str(&yaml).unwrap());
    assert_eq!(&read_back.resolved(), checked.checked().resolved());
    assert_eq!(read_back.digest(), checked.checked().resolved().digest());
    assert_eq!(read_back.run.max_turns, std::num::NonZeroU32::new(5));
    // The config as it's written, the prompt's text and the filled entry,
    // and nothing a variable holds.
    assert_eq!(yaml.matches(&format!("${{{SET}}}")).count(), 2, "{yaml}");
    assert!(!yaml.contains(&held()), "{yaml}");
    // Each setting the file wrote carries its line, and the filled entry
    // its source; a default carries nothing.
    assert!(yaml.contains("  max_turns: 5 # line 1\n"), "{yaml}");
    assert!(
        yaml.contains(&format!("      WHERE: ${{{SET}}} # environment\n")),
        "{yaml}"
    );
    assert!(yaml.contains("  completion: natural\n"), "{yaml}");
}
