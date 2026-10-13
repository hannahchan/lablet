//! Each field of the config that defines a credential, with which variable
//! filled it read off the record, and what's handed in to be cut.

use std::ffi::OsString;

use lablet_conformance::receiver::{Mode, Receiver};

use super::*;
use lablet_config::{Format, RawConfig};

/// A key as long as a key is, and so one that's cut.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

const FAKE: &str = "model: { provider: fake, script: run.yaml }\n";

/// The config `text` holds, with each `${VAR}` filled from `held`.
fn filled(text: &str, held: &[(&str, &str)]) -> Config {
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RawConfig::from_str(text, Format::Yaml)
        .unwrap()
        .substituted(move |asked: &str| -> Option<OsString> {
            held.iter()
                .find(|(name, _)| name == asked)
                .map(|(_, value)| value.into())
        })
        .unwrap()
        .config()
        .unwrap()
}

/// What `derived` makes of the config `text` filled from `held`, for a run
/// that has an executor and nothing handed in.
fn derived_from(text: &str, held: &[(&str, &str)]) -> Derived {
    derived(&filled(text, held), None, true)
}

fn names(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

fn secrets(values: &[&str]) -> Secrets {
    Secrets::new(values.iter().map(|&value| value.to_owned()))
}

/// The own credentials as `check` prints them.
fn own(derived: &Derived) -> Vec<String> {
    derived.own.iter().map(Credential::as_own).collect()
}

/// The given credentials as `check` prints them.
fn given(derived: &Derived) -> Vec<String> {
    derived.given.iter().map(Credential::as_given).collect()
}

#[test]
fn the_variable_that_fills_the_key_is_own_dropped_and_its_value_cut() {
    let derived = derived_from(
        "model: { provider: fake, script: run.yaml, api_key: '${WORK_KEY}' }",
        &[("WORK_KEY", KEY)],
    );

    assert_eq!(
        own(&derived),
        ["WORK_KEY (line 1) -> model.api_key; no process inherits it"]
    );
    assert!(derived.given.is_empty());
    assert_eq!(derived.dropped, names(&["WORK_KEY"]));
    assert_eq!(derived.values, secrets(&[KEY]));
    assert_eq!(derived.dropping, secrets(&[KEY]));
    assert!(!format!("{derived:?}").contains(KEY), "{derived:?}");
}

/// A key set in code is own by its field alone, and nothing is dropped by
/// name for it: the drop by value is its guard.
#[test]
fn a_key_set_in_code_is_own_and_names_no_variable() {
    let mut config = Config::default();
    config.model.api_key = Some(Secret::new(KEY));

    let derived = derived(&config, None, true);

    assert_eq!(
        own(&derived),
        ["model.api_key (set in code); no process inherits it"]
    );
    assert!(derived.dropped.is_empty());
    assert_eq!(derived.dropping, secrets(&[KEY]));
    assert_eq!(derived.own[0].source, Source::Code);
    assert!(derived.own[0].holds(KEY));
    assert!(derived.own[0].holds(&format!("Bearer%20{KEY}")));
    assert!(!derived.own[0].holds("another-0123456789abcdef"));
}

#[test]
fn a_config_that_names_no_credential_holds_nothing() {
    let derived = derived_from(FAKE, &[]);

    assert_eq!(
        derived,
        Derived {
            own: Vec::new(),
            given: Vec::new(),
            dropped: BTreeSet::new(),
            values: Secrets::default(),
            dropping: Secrets::default(),
        }
    );
    assert_eq!(variables(&filled(FAKE, &[])), Variables::default());
}

/// A key's variable that's unset, empty or short is own and named with why
/// its value isn't cut.
#[test]
fn a_key_variable_that_is_unset_empty_or_short_is_named_with_why_it_is_not_cut() {
    let text = "model: { provider: fake, script: run.yaml, api_key: '${WORK_KEY}' }";

    for (held, expected, shown) in [
        (vec![], Held::Unset, "WORK_KEY (line 1) (unset, not cut)"),
        (
            vec![("WORK_KEY", "  ")],
            Held::Empty,
            "WORK_KEY (line 1) (empty, not cut)",
        ),
        (
            vec![("WORK_KEY", "short")],
            Held::Short,
            "WORK_KEY (line 1) (under 16 bytes, not cut)",
        ),
    ] {
        let derived = derived_from(text, &held);

        assert_eq!(derived.own[0].held, expected);
        assert_eq!(
            derived.own[0].as_own(),
            format!("{shown} -> model.api_key; no process inherits it")
        );
        assert_eq!(derived.dropped, names(&["WORK_KEY"]));
        assert!(derived.values.is_empty(), "{expected:?}");
    }
}

#[test]
fn every_header_value_is_own_from_its_variable_or_as_written() {
    let text = "
model: { provider: fake, script: run.yaml }
telemetry:
  otlp:
    headers: { Authorization: 'Bearer ${T}', X-Literal: literal-0123456789abcdef, X-Short: short }
tools:
  mcp:
    - { name: s, transport: http, url: 'http://h/mcp', headers: { H: 'v-${T}-${U}' } }
";
    let (t, u) = ("tok-0123456789abcdef", "u-0123456789abcdef0");

    let derived = derived_from(text, &[("T", t), ("U", u)]);

    assert_eq!(
        own(&derived),
        [
            "T (line 5) -> telemetry.otlp.headers.Authorization; no process inherits it",
            "telemetry.otlp.headers.X-Literal (line 5) holds its value; no process inherits it",
            "telemetry.otlp.headers.X-Short (line 5) holds its value (under 16 bytes, not cut); \
             no process inherits it",
            "T (line 8) -> tools.mcp[0].headers.H; no process inherits it",
            "U (line 8) -> tools.mcp[0].headers.H; no process inherits it",
        ]
    );
    assert_eq!(derived.dropped, names(&["T", "U"]));
    // The field's whole value is cut, and `Bearer` bounds the token inside
    // it; a variable's value alone isn't known to a library that reads none.
    assert_eq!(
        derived.values,
        secrets(&[
            &format!("Bearer {t}"),
            "literal-0123456789abcdef",
            "short",
            &format!("v-{t}-{u}"),
        ])
    );
}

#[test]
fn the_user_information_of_a_url_is_own_in_each_of_its_forms_and_its_host_is_not() {
    let text = "
model: { provider: openai, base_url: 'https://hannah:${P}@gw.example/v1' }
telemetry: { otlp: { endpoint: 'https://${HOST}:4317' } }
tools:
  mcp:
    - { name: s, transport: http, url: 'https://user-0123456789ab:pw%40-0123456789abc@h.example/mcp' }
";
    let password = "pass-0123456789abcdef";

    let derived = derived_from(text, &[("P", password), ("HOST", "otel.example")]);

    assert_eq!(
        own(&derived),
        [
            "P (line 2) -> model.base_url; no process inherits it",
            "tools.mcp[0].url (line 6) holds its value; no process inherits it",
        ]
    );
    assert_eq!(derived.dropped, names(&["P"]));
    assert_eq!(
        derived.values,
        secrets(&[
            &format!("hannah:{password}"),
            "hannah",
            password,
            "user-0123456789ab:pw%40-0123456789abc",
            "user-0123456789ab:pw@-0123456789abc",
            "user-0123456789ab",
            "pw%40-0123456789abc",
            "pw@-0123456789abc",
        ])
    );
}

#[test]
fn a_variable_that_gives_a_url_its_user_information_from_outside_the_written_text_is_own() {
    let base = "https://u-0123456789abcdef:p-0123456789abcdef@h";
    let text = "model: { provider: openai, base_url: '${BASE}/v1' }";

    let derived = derived_from(text, &[("BASE", base)]);
    assert_eq!(
        own(&derived),
        ["BASE (line 1) -> model.base_url; no process inherits it"]
    );
    assert_eq!(derived.dropped, names(&["BASE"]));
    assert_eq!(
        derived.values,
        secrets(&[
            "u-0123456789abcdef:p-0123456789abcdef",
            "u-0123456789abcdef",
            "p-0123456789abcdef",
        ])
    );

    let no_information = derived_from(text, &[("BASE", "https://h.example")]);
    assert_eq!(no_information, derived_from(FAKE, &[]));
}

/// An `env` entry is no credential, filled or not; a `credentials` entry is
/// given, dropped from what a command inherits and cut.
#[test]
fn a_credentials_entry_is_given_and_an_env_entry_is_no_credential() {
    let text = "
model: { provider: fake, script: run.yaml }
tools:
  builtin:
    root: .
    enabled: [bash]
    env: { PATH: /usr/bin:/bin, MIXED: 'x-${A}' }
    credentials: { GITHUB_TOKEN: '${GITHUB_TOKEN}', TOK: '${TRACKER_TOKEN}' }
";
    let held = [
        ("GITHUB_TOKEN", "ghp_0123456789abcdef"),
        ("A", "a-0123456789abcdef"),
        ("TRACKER_TOKEN", "trk-0123456789abcdef"),
    ];

    let derived = derived_from(text, &held);

    assert!(derived.own.is_empty());
    assert_eq!(
        given(&derived),
        [
            "GITHUB_TOKEN (line 8) -> bash",
            "TRACKER_TOKEN (line 8) -> bash as TOK",
        ]
    );
    assert_eq!(derived.given[0].given_as(), Some("GITHUB_TOKEN"));
    assert_eq!(derived.dropped, names(&["GITHUB_TOKEN", "TRACKER_TOKEN"]));
    assert_eq!(
        derived.values,
        secrets(&["ghp_0123456789abcdef", "trk-0123456789abcdef"]),
        "the `env` entry's variable is no secret"
    );
    let variables = variables(&filled(text, &held));
    assert_eq!(variables.own, []);
    assert_eq!(
        variables.given_entry("TRACKER_TOKEN"),
        Some(&KeyPath::of("tools.builtin.credentials").key("TOK"))
    );
    assert_eq!(variables.given_entry("A"), None);
}

/// Q5: an own credential listed under `credentials` too is shown under
/// both, and `own:` says who gets it.
#[test]
fn an_own_credential_given_under_credentials_is_shown_on_both_sides() {
    let text = "
model: { provider: fake, script: run.yaml, api_key: '${ANTHROPIC_API_KEY}' }
tools: { builtin: { root: ., enabled: [bash], credentials: { ANTHROPIC_API_KEY: '${ANTHROPIC_API_KEY}' } } }
";

    let derived = derived_from(text, &[("ANTHROPIC_API_KEY", KEY)]);

    assert_eq!(
        own(&derived),
        ["ANTHROPIC_API_KEY (line 2) -> model.api_key; given to bash by credentials (line 3)"]
    );
    assert_eq!(given(&derived), ["ANTHROPIC_API_KEY (line 3) -> bash"]);
    assert_eq!(derived.dropped, names(&["ANTHROPIC_API_KEY"]));
    assert_eq!(derived.values, secrets(&[KEY]), "one value, held once");
}

#[test]
fn the_values_are_held_only_for_a_run_that_has_an_executor() {
    let config = filled(
        "model: { provider: fake, script: run.yaml, api_key: '${WORK_KEY}' }",
        &[("WORK_KEY", KEY)],
    );

    let without = derived(&config, None, false);

    assert_eq!(without.values, Secrets::default());
    assert_eq!(without.dropping, Secrets::default());
    assert_eq!(without.dropped, names(&["WORK_KEY"]));
    assert_eq!(
        without.own[0].held,
        Held::Cut,
        "the value is still read, to say whether it would be cut"
    );
}

#[test]
fn a_variable_in_two_fields_is_dropped_once_and_each_field_names_it() {
    let text = "
model: { provider: fake, script: run.yaml, api_key: '${K}' }
telemetry: { otlp: { headers: { Authorization: 'Bearer ${K}' } } }
";

    let derived = derived_from(text, &[("K", KEY)]);

    assert_eq!(derived.dropped, names(&["K"]));
    assert_eq!(derived.own.len(), 2);
    assert_eq!(
        derived.values,
        secrets(&[KEY, &format!("Bearer {KEY}")]),
        "the header's whole value beside the key"
    );
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

/// What the file `telemetry.otlp.tls.client_key` names holds, where the
/// root read it, is own: cut whole and by each line, named under `own:` as
/// the file's contents, dropped by no name, since no variable gave it, and
/// among the values a variable of the environment is dropped by.
#[tokio::test]
async fn a_client_keys_contents_are_own_cut_and_named_and_no_variable_is_dropped_for_them() {
    let receiver = Receiver::start(Mode::Answers).await;
    let key = receiver.client_key();
    let config = filled(
        "model: { provider: fake, script: run.yaml }\ntelemetry: { otlp: { endpoint: 'https://c:4318', tls: { client_cert: c.pem, client_key: '${KEY_FILE}' } } }\n",
        &[("KEY_FILE", "/keys/client.key")],
    );

    let derived = derived(&config, Some(key), true);

    assert_eq!(
        own(&derived),
        ["telemetry.otlp.tls.client_key (line 2) -> its file's contents; cut"]
    );
    assert!(
        derived.dropped.is_empty(),
        "the variable holds a path, which is no secret"
    );
    let line = key.lines().nth(1).unwrap();
    for value in [key, line] {
        assert_ne!(derived.values.redacted(value), value, "{value} isn't cut");
        assert_ne!(
            derived.dropping.redacted(value),
            value,
            "{value} isn't dropped by value"
        );
    }
    assert!(!format!("{derived:?}").contains(line), "{derived:?}");

    let without = derived_from_key(&config, Some(key), false);
    assert_eq!(without.values, Secrets::default());
    assert_eq!(
        own(&without),
        own(&derived),
        "named without an executor too"
    );
}

/// `derived` with a client key, as a root hands one in.
fn derived_from_key(config: &Config, key: Option<&str>, with_values: bool) -> Derived {
    derived(config, key, with_values)
}

/// `telemetry.otlp.headers` as OpenTelemetry's `k=v` text is one setting: the
/// variable that filled it is own at the field, what's cut is the text and
/// each value it decodes to, and a text that comes out unset is named so.
#[test]
fn a_headers_text_is_own_by_its_variable_and_cut_as_the_text_and_as_each_value() {
    let text = "
model: { provider: fake, script: run.yaml }
telemetry:
  otlp:
    headers: '${OTEL_EXPORTER_OTLP_HEADERS:-}'
";
    let token = "tok-0123456789abcdef";
    let value = format!("authorization=Bearer%20{token},x-env=1");

    let derived = derived_from(text, &[("OTEL_EXPORTER_OTLP_HEADERS", &value)]);
    assert_eq!(
        own(&derived),
        ["OTEL_EXPORTER_OTLP_HEADERS (line 5) -> telemetry.otlp.headers; no process inherits it"]
    );
    assert_eq!(derived.dropped, names(&["OTEL_EXPORTER_OTLP_HEADERS"]));
    assert_eq!(
        derived.values,
        secrets(&[&value, &format!("Bearer {token}"), "1"]),
        "the text whole, and each decoded value"
    );

    let unset = derived_from(text, &[]);
    assert_eq!(
        own(&unset),
        [
            "OTEL_EXPORTER_OTLP_HEADERS (line 5) (unset, not cut) -> telemetry.otlp.headers; no \
             process inherits it"
        ]
    );
    assert_eq!(unset.values, Secrets::default());
}

/// A map entry or a credentials entry whose `${VAR:-}` came out empty isn't
/// in its map, and is still named: own as unset, and given as `none:`.
#[test]
fn an_entry_whose_variable_came_out_empty_is_named_as_unset() {
    let text = "
model: { provider: fake, script: run.yaml }
telemetry: { otlp: { headers: { Authorization: '${T:-}', X-Env: '${E}' } } }
tools: { builtin: { root: ., enabled: [bash], credentials: { OTEL_EXPORTER_OTLP_HEADERS: '${COMMAND_OTLP_HEADERS:-}', G: '${G}' } } }
";

    let derived = derived_from(
        text,
        &[("E", "e-0123456789abcdef"), ("G", "g-0123456789abcdef")],
    );

    assert_eq!(
        own(&derived),
        [
            "E (line 3) -> telemetry.otlp.headers.X-Env; no process inherits it",
            "T (line 3) (unset, not cut) -> telemetry.otlp.headers.Authorization; no process \
             inherits it",
        ],
        "the entries that are stated, then those left out"
    );
    assert_eq!(
        given(&derived),
        [
            "G (line 4) -> bash",
            "none: COMMAND_OTLP_HEADERS (line 4) is unset",
        ]
    );
    assert_eq!(
        derived.dropped,
        names(&["T", "E", "G", "COMMAND_OTLP_HEADERS"])
    );
}

/// A client key's armour, `-----BEGIN PRIVATE KEY-----` and its end, is
/// every PEM's: it's neither cut nor what a variable of the environment is
/// dropped by, so another key or a certificate isn't taken for this one.
/// A variable that holds this key is named under its `own:` line.
#[tokio::test]
async fn a_client_keys_armour_is_neither_cut_nor_dropped_by() {
    let receiver = Receiver::start(Mode::Answers).await;
    let key = receiver.client_key();
    let config = filled(
        "model: { provider: fake, script: run.yaml }\ntelemetry: { otlp: { tls: { client_key: /keys/client.key } } }\n",
        &[],
    );

    let mut derived = derived(&config, Some(key), true);

    let armour: Vec<&str> = key
        .lines()
        .filter(|line| line.starts_with("-----"))
        .collect();
    assert_eq!(armour.len(), 2, "{armour:?}");
    for line in armour {
        assert_eq!(derived.values.redacted(line), line);
        assert_eq!(derived.dropping.redacted(line), line);
    }
    assert_ne!(derived.dropping.redacted(key), key);
    derived.own[0].copies = vec!["MY_KEY".to_owned()];
    assert_eq!(
        own(&derived),
        [
            "telemetry.otlp.tls.client_key (line 2) -> its file's contents, or MY_KEY, which holds it; cut"
        ]
    );
}

/// A key set in code after the file's `${K:-}` or `${K}` came out unset,
/// `K` unset or set to nothing, is the code's, and is named so, not as
/// filled from `K`.
#[test]
fn a_key_set_in_code_after_its_variable_came_out_unset_is_named_set_in_code() {
    for (api_key, variables) in [
        ("${K:-}", &[][..]),
        ("${K:-}", &[("K", "")][..]),
        ("${K}", &[("K", "")][..]),
    ] {
        let mut config = filled(
            &format!("model: {{ provider: fake, script: run.yaml, api_key: '{api_key}' }}"),
            variables,
        );
        config.model.api_key = Some(Secret::new(KEY));

        let derived = derived(&config, None, true);

        assert_eq!(
            own(&derived),
            ["model.api_key (set in code); no process inherits it"],
            "{api_key} {variables:?}"
        );
        assert!(derived.dropped.is_empty(), "{:?}", derived.dropped);
    }
}
