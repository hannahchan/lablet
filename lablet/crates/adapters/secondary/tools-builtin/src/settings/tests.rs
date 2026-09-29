use std::time::Duration;

use super::*;

fn held(name: &str) -> Option<OsString> {
    match name {
        "PATH" => Some("/usr/bin:/bin".into()),
        "HOME" => Some("/home/lablet".into()),
        "ANTHROPIC_API_KEY" => Some("a key of lablet's".into()),
        _ => None,
    }
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
fn a_command_starts_with_the_variables_of_the_list_that_are_set_and_no_other() {
    let environment = environment(held, BTreeMap::new()).unwrap();

    assert_eq!(names(&environment), ["HOME", "PATH"]);
    assert_eq!(environment[&OsString::from("PATH")], "/usr/bin:/bin");
}

#[test]
fn a_variable_the_settings_add_is_there_and_replaces_one_of_the_list() {
    let environment = environment(held, added(&[("CI", "true"), ("PATH", "/opt/bin")])).unwrap();

    assert_eq!(names(&environment), ["CI", "HOME", "PATH"]);
    assert_eq!(environment[&OsString::from("CI")], "true");
    assert_eq!(environment[&OsString::from("PATH")], "/opt/bin");
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
            environment(held, added(&[(name, value)])),
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
    };

    let shown = format!("{settings:?}");

    assert_eq!(
        shown,
        "Settings { root: \"/work\", enabled: {Bash}, timeout: 120s, env: [\"NPM_TOKEN\"] }"
    );
}
