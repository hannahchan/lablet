use lablet::{Checked, Config, Format};
use lablet_test_support::Scratch;
use serde_json::json;

use super::{answer, summary};

/// A variable cargo sets for every test, and so one that's set. Its value
/// is a path, as long as a key.
const SET: &str = "CARGO_MANIFEST_DIR";

/// A variable cargo sets for every test, whose value is under the floor.
const SHORT: &str = "CARGO_PKG_NAME";

/// A script the fake model can play, which a check reads as a run would.
const ENDS: &str = "- response: { content: [{ text: Done. }], finish: end_turn }\n";

/// A config of the fake model with `tools`, whose system prompt holds a
/// variable.
fn tree(scratch: &Scratch, tools: &serde_json::Value) -> serde_json::Value {
    scratch.create_dir("work");
    json!({
        "run": { "max_turns": 5, "timeout": "2m" },
        "model": { "provider": "fake", "script": scratch.write("script.yaml", ENDS), "name": "scripted-1" },
        "prompt": { "system": format!("You fix the tests in ${{{SET}}}.") },
        "tools": tools,
    })
}

/// What `lablet::check` makes of the config `tree`.
fn checked_tree(tree: &serde_json::Value) -> Checked {
    let config = Config::from_str(&tree.to_string(), Format::Json).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(lablet::check(&config)).unwrap()
}

/// What `lablet::check` makes of a config of the fake model with `tools`.
fn checked(scratch: &Scratch, tools: &serde_json::Value) -> Checked {
    checked_tree(&tree(scratch, tools))
}

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

    assert_eq!(answer(&checked, false).unwrap(), "bash\nread_file");
    assert_eq!(
        summary(&checked),
        "withheld: none\ncut: none\npassed: 2 tools"
    );
}

#[test]
fn a_check_of_a_config_with_no_tools_lists_nothing() {
    let scratch = Scratch::new("answer-no-tools");
    let checked = checked(&scratch, &json!({}));

    assert_eq!(answer(&checked, false).unwrap(), "");
    assert_eq!(
        summary(&checked),
        "withheld: none\ncut: none\npassed: 0 tools"
    );
}

/// T16: the summary names what's withheld and cut, a variable by its name
/// and a written value by its key, with a note on a value under the floor,
/// and holds no value.
#[test]
fn a_check_names_the_variables_withheld_and_the_names_cut_and_never_a_value() {
    let scratch = Scratch::new("answer-secrets");
    let mut tree = tree(
        &scratch,
        &json!({ "builtin": {
            "root": scratch.at("work"),
            "enabled": ["bash"],
            "env": { "TOKEN": format!("${{{SET}}}"), "PATH": "/usr/bin" },
        } }),
    );
    tree["telemetry"] = json!({ "otlp": { "headers": {
        "Authorization": format!("Bearer ${{{SET}}}"),
        "X-Literal": "literal-0123456789abcdef",
        "X-Short": format!("${{{SHORT}}}"),
    } } });

    let summary = summary(&checked_tree(&tree));

    assert_eq!(
        summary,
        format!(
            "withheld: {SET}, {SHORT}\ncut: {SET}, {SHORT} (under 16 bytes, not cut), \
             telemetry.otlp.headers.X-Literal\npassed: 1 tool"
        )
    );
    assert!(!summary.contains(&std::env::var(SET).unwrap()), "{summary}");
}

#[test]
fn the_resolved_config_is_yaml_that_reads_back_as_the_config_it_resolves() {
    let scratch = Scratch::new("answer-resolved");
    let checked = checked(
        &scratch,
        &json!({ "builtin": { "root": scratch.at("work"), "enabled": ["bash"] } }),
    );

    let yaml = answer(&checked, true).unwrap();
    assert_eq!(
        summary(&checked),
        "withheld: none\ncut: none\npassed: 1 tool"
    );

    assert!(yaml.starts_with("run:\n"), "{yaml}");
    assert!(!yaml.ends_with('\n'), "{yaml:?}");
    let read_back = Config::from_str(&yaml, Format::Yaml).unwrap();
    assert_eq!(&read_back.resolved(), checked.resolved());
    assert_eq!(read_back.digest(), checked.resolved().digest());
    assert_eq!(read_back.run.max_turns, std::num::NonZeroU32::new(5));
    // The config as it's written, and nothing a variable holds.
    assert!(yaml.contains(&format!("${{{SET}}}")), "{yaml}");
    assert!(!yaml.contains(&std::env::var(SET).unwrap()), "{yaml}");
}
