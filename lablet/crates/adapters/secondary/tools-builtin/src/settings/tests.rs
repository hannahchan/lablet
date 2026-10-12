use std::time::Duration;

use super::*;

/// A key as long as a key is, and so one the cut takes.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

/// The environment a command starts from, as a test says it is.
fn base() -> Vec<(OsString, OsString)> {
    [
        ("PATH", "/usr/bin:/bin"),
        ("JAVA_HOME", "/opt/java"),
        ("GITHUB_TOKEN", "a framework's own"),
        ("ANTHROPIC_API_KEY", KEY),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), value.into()))
    .collect()
}

fn names(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

fn added(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn in_order(environment: &BTreeMap<OsString, OsString>) -> Vec<&str> {
    environment
        .keys()
        .map(|name| name.to_str().unwrap())
        .collect()
}

fn secrets(values: &[&str]) -> Secrets {
    Secrets::new(values.iter().map(|&value| value.to_owned()))
}

#[test]
fn a_command_inherits_every_variable_of_the_base_but_the_ones_dropped_by_name() {
    let dropped = environment(
        base(),
        &names(&["ANTHROPIC_API_KEY", "NOT_SET"]),
        &Secrets::default(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(
        in_order(&dropped.environment),
        ["GITHUB_TOKEN", "JAVA_HOME", "PATH"]
    );
    assert_eq!(
        dropped.environment[&OsString::from("PATH")],
        "/usr/bin:/bin"
    );
    assert_eq!(
        dropped.environment[&OsString::from("GITHUB_TOKEN")],
        "a framework's own"
    );
    assert_eq!(dropped.by_name, names(&["ANTHROPIC_API_KEY"]));
    assert!(dropped.by_value.is_empty());
}

/// The drop by value: a variable holding a credential's value is dropped
/// under any name, as it is and percent-encoded, as the cut would take it,
/// and a value under the cut's floor is never dropped. The key holds `+`,
/// `/` and `=`, which encoding changes, so the encoded copy is caught only
/// through the decoder.
#[test]
fn a_variable_holding_a_credential_s_value_is_dropped_under_any_name_and_a_short_one_is_not() {
    let key = "sk+ant/0123456789abcdef0123456789==";
    let short = "shortvalue12";
    let base: Vec<(OsString, OsString)> = [
        ("PATH", "/usr/bin:/bin"),
        ("MY_COPY", key),
        ("AUTH", &format!("Bearer {key}")),
        (
            "ENCODED",
            "authorization=Bearer%20sk%2Bant%2F0123456789abcdef0123456789%3D%3D",
        ),
        ("SHORT_COPY", short),
        ("SHLVL", "1"),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), value.into()))
    .collect();

    let dropped = environment(
        base,
        &BTreeSet::new(),
        &secrets(&[key, short]),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(
        in_order(&dropped.environment),
        ["PATH", "SHLVL", "SHORT_COPY"]
    );
    assert_eq!(dropped.by_value, names(&["AUTH", "ENCODED", "MY_COPY"]));
    assert!(dropped.by_name.is_empty());
    assert_eq!(short.len(), 12, "under the floor of {}", Secrets::MIN_BYTES);
}

/// A variable dropped by name isn't counted among those dropped by value,
/// though it holds the value too.
#[test]
fn a_variable_that_filled_a_credential_is_dropped_by_name_alone() {
    let dropped = environment(
        base(),
        &names(&["ANTHROPIC_API_KEY"]),
        &secrets(&[KEY]),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(dropped.by_name, names(&["ANTHROPIC_API_KEY"]));
    assert!(dropped.by_value.is_empty());
}

#[test]
fn a_variable_whose_name_is_no_text_is_inherited() {
    use std::os::unix::ffi::OsStringExt as _;
    let name = OsString::from_vec(b"NOT_\xFF_TEXT".to_vec());

    let dropped = environment(
        [(name.clone(), "1".into())],
        &names(&["NOT_\u{FFFD}_TEXT"]),
        &Secrets::default(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(dropped.environment.keys().collect::<Vec<_>>(), [&name]);
}

#[test]
fn a_variable_the_config_adds_is_there_and_replaces_one_that_was_inherited() {
    let dropped = environment(
        base(),
        &BTreeSet::new(),
        &Secrets::default(),
        added(&[("CI", "true"), ("PATH", "/opt/bin")]),
        BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(
        in_order(&dropped.environment),
        [
            "ANTHROPIC_API_KEY",
            "CI",
            "GITHUB_TOKEN",
            "JAVA_HOME",
            "PATH"
        ]
    );
    assert_eq!(dropped.environment[&OsString::from("CI")], "true");
    assert_eq!(dropped.environment[&OsString::from("PATH")], "/opt/bin");
}

/// A credential reaches the command under the name the config wrote, and
/// nowhere else: the variable that held its value is dropped.
#[test]
fn a_credential_is_given_under_its_own_name_after_the_drops() {
    let dropped = environment(
        base(),
        &names(&["ANTHROPIC_API_KEY"]),
        &secrets(&[KEY]),
        BTreeMap::new(),
        added(&[("THE_KEY", KEY), ("GITHUB_TOKEN", "the task's own")]),
    )
    .unwrap();

    assert_eq!(
        in_order(&dropped.environment),
        ["GITHUB_TOKEN", "JAVA_HOME", "PATH", "THE_KEY"]
    );
    assert_eq!(dropped.environment[&OsString::from("THE_KEY")], KEY);
    assert_eq!(
        dropped.environment[&OsString::from("GITHUB_TOKEN")],
        "the task's own",
        "a credential wins over what was inherited"
    );
}

#[test]
fn a_variable_or_credential_no_process_can_be_started_with_is_refused_by_name() {
    for (name, value, reason) in [
        ("", "1", "its name is empty"),
        ("A=B", "1", "its name holds `=` or a NUL"),
        ("A\x00B", "1", "its name holds `=` or a NUL"),
        ("A", "1\x002", "its value holds a NUL"),
    ] {
        assert_eq!(
            environment(
                base(),
                &BTreeSet::new(),
                &Secrets::default(),
                added(&[(name, value)]),
                BTreeMap::new()
            ),
            Err(SettingsError::Variable {
                name: name.to_owned(),
                reason
            })
        );
        assert_eq!(
            environment(
                base(),
                &BTreeSet::new(),
                &Secrets::default(),
                BTreeMap::new(),
                added(&[(name, value)])
            ),
            Err(SettingsError::Credential {
                name: name.to_owned(),
                reason
            })
        );
    }
}

#[test]
fn a_percent_escape_is_decoded_and_what_is_no_escape_is_kept() {
    assert_eq!(percent_decoded("p%40ss"), "p@ss");
    assert_eq!(percent_decoded("Bearer%20tok"), "Bearer tok");
    assert_eq!(percent_decoded("100%"), "100%");
    assert_eq!(percent_decoded("%zz%4"), "%zz%4");
    assert_eq!(percent_decoded("caf%C3%A9"), "café");
    assert_eq!(percent_decoded("%ff"), "\u{FFFD}");
    assert_eq!(percent_decoded("plain"), "plain");
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
        environment: [("NPM_TOKEN".into(), "a value no log is to hold".into())].into(),
        kept: names(&["ANTHROPIC_API_KEY", "NPM_TOKEN"]),
    };

    let shown = format!("{settings:?}");

    assert_eq!(
        shown,
        "Settings { root: \"/work\", enabled: {Bash}, timeout: 120s, environment: \
         [\"NPM_TOKEN\"], kept: {\"ANTHROPIC_API_KEY\", \"NPM_TOKEN\"} }"
    );
}
