use std::time::Duration;

use super::*;

/// lablet's environment, as a test says it is.
fn inherited() -> Vec<(OsString, OsString)> {
    [
        ("PATH", "/usr/bin:/bin"),
        ("JAVA_HOME", "/opt/java"),
        ("GITHUB_TOKEN", "a framework's own"),
        ("ANTHROPIC_API_KEY", "a key of lablet's"),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), value.into()))
    .collect()
}

fn withheld(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

fn added(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn names(environment: &BTreeMap<OsString, OsString>) -> Vec<&str> {
    environment
        .keys()
        .map(|name| name.to_str().unwrap())
        .collect()
}

#[test]
fn a_command_inherits_every_variable_of_lablet_s_but_the_ones_withheld() {
    let environment = environment(
        inherited(),
        &withheld(&["ANTHROPIC_API_KEY", "NOT_SET"]),
        BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(names(&environment), ["GITHUB_TOKEN", "JAVA_HOME", "PATH"]);
    assert_eq!(environment[&OsString::from("PATH")], "/usr/bin:/bin");
    assert_eq!(
        environment[&OsString::from("GITHUB_TOKEN")],
        "a framework's own"
    );
}

#[test]
fn a_variable_whose_name_is_no_text_is_inherited() {
    use std::os::unix::ffi::OsStringExt as _;
    let name = OsString::from_vec(b"NOT_\xFF_TEXT".to_vec());

    let environment = environment(
        [(name.clone(), "1".into())],
        &withheld(&["NOT_\u{FFFD}_TEXT"]),
        BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(environment.keys().collect::<Vec<_>>(), [&name]);
}

#[test]
fn a_variable_the_settings_add_is_there_and_replaces_one_that_was_inherited() {
    let environment = environment(
        inherited(),
        &withheld(&[]),
        added(&[("CI", "true"), ("PATH", "/opt/bin")]),
    )
    .unwrap();

    assert_eq!(
        names(&environment),
        [
            "ANTHROPIC_API_KEY",
            "CI",
            "GITHUB_TOKEN",
            "JAVA_HOME",
            "PATH"
        ]
    );
    assert_eq!(environment[&OsString::from("CI")], "true");
    assert_eq!(environment[&OsString::from("PATH")], "/opt/bin");
}

#[test]
fn a_withheld_variable_the_settings_name_is_passed_on_with_their_value() {
    let environment = environment(
        inherited(),
        &withheld(&["ANTHROPIC_API_KEY"]),
        added(&[("ANTHROPIC_API_KEY", "the task's own")]),
    )
    .unwrap();

    assert_eq!(
        environment[&OsString::from("ANTHROPIC_API_KEY")],
        "the task's own"
    );
}

#[test]
fn a_variable_no_process_can_be_started_with_is_refused_by_name() {
    for (name, value, reason) in [
        ("", "1", "its name is empty"),
        ("A=B", "1", "its name holds `=` or a NUL"),
        ("A\x00B", "1", "its name holds `=` or a NUL"),
        ("A", "1\x002", "its value holds a NUL"),
    ] {
        assert_eq!(
            environment(inherited(), &withheld(&[]), added(&[(name, value)])),
            Err(SettingsError::Variable {
                name: name.to_owned(),
                reason
            })
        );
    }
}

#[test]
fn the_tools_are_offered_in_one_order_under_the_names_the_model_calls() {
    let enabled: BTreeSet<Tool> = [Tool::WriteFile, Tool::Bash, Tool::ReadFile].into();

    let names: Vec<String> = enabled.iter().map(ToString::to_string).collect();

    assert_eq!(names, ["bash", "read_file", "write_file"]);
    assert_eq!(enabled.into_iter().collect::<Vec<_>>(), Tool::ALL);
}

#[test]
fn settings_are_shown_with_the_names_of_their_variables_and_none_of_the_values() {
    let settings = Settings {
        root: "/work".into(),
        enabled: [Tool::Bash].into(),
        timeout: Duration::from_secs(120),
        env: added(&[("NPM_TOKEN", "a value no log is to hold")]),
        withheld: Withheld {
            variables: withheld(&["ANTHROPIC_API_KEY"]),
            values: lablet_model::Secrets::new(["a key no log is to hold".to_owned()]),
        },
    };

    let shown = format!("{settings:?}");

    assert_eq!(
        shown,
        "Settings { root: \"/work\", enabled: {Bash}, timeout: 120s, env: [\"NPM_TOKEN\"], \
         withheld: Withheld { variables: {\"ANTHROPIC_API_KEY\"}, values: Secrets { values: 1 } } }"
    );
}
