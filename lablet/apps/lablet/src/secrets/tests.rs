//! Each field of the config that defines a secret, against an environment
//! the test states, since a test can't set a variable of its own process.

use std::ffi::OsString;

use lablet_conformance::receiver::{Mode, Receiver};
use lablet_test_support::Scratch;

use super::*;
use crate::config::Format;
use crate::otel_env::OtelEnv;

/// A key as long as a key is, and so one that's cut.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

const FAKE: &str = "model: { provider: fake, script: run.yaml }\n";

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

/// An environment that holds what `held` says, and nothing else.
fn env(held: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    move |asked| {
        held.iter()
            .find(|(name, _)| name == asked)
            .map(|(_, value)| value.into())
    }
}

fn nothing(_: &str) -> Option<OsString> {
    None
}

/// What `derived` makes of the config `text` in the environment `held`,
/// for a run that has an executor.
fn derived_from(text: &str, held: &dyn Fn(&str) -> Option<OsString>) -> Derived {
    let written = config(text);
    let real = written.substituted(held).unwrap();
    derived(&written, &real, held, None, true)
}

fn variable(name: &str, held: Held) -> Named {
    Named {
        source: Source::Variable(name.to_owned()),
        held,
    }
}

fn key(path: KeyPath, held: Held) -> Named {
    Named {
        source: Source::Key(path),
        held,
    }
}

fn names(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

fn secrets(values: &[&str]) -> Secrets {
    Secrets::new(values.iter().map(|&value| value.to_owned()))
}

#[test]
fn the_variable_lablet_reads_its_key_from_is_withheld_and_what_it_holds_is_cut() {
    for (text, name) in [
        (
            "model: { provider: fake, script: run.yaml, api_key_env: WORK_KEY }",
            "WORK_KEY",
        ),
        ("", "ANTHROPIC_API_KEY"),
    ] {
        let derived = derived_from(text, &env(&[(name, KEY)]));

        assert_eq!(
            derived,
            Derived {
                withheld: names(&[name]),
                cut: vec![variable(name, Held::Cut)],
                values: secrets(&[KEY]),
            }
        );
        assert!(!format!("{derived:?}").contains(KEY), "{derived:?}");
    }
}

/// The OTLP header and endpoint variables are the one exception: read for
/// every config, since the exporter reads them whatever the config names.
#[test]
fn a_config_that_names_no_secret_reads_nothing_but_the_otlp_variables_and_holds_nothing() {
    let never = |name: &str| -> Option<OsString> {
        assert!(
            Exporter::HEADER_VARIABLES.contains(&name)
                || Exporter::ENDPOINT_VARIABLES.contains(&name),
            "the environment was read: {name}"
        );
        None
    };

    let derived = derived_from(FAKE, &never);

    assert_eq!(
        derived,
        Derived {
            withheld: BTreeSet::new(),
            cut: Vec::new(),
            values: Secrets::default(),
        }
    );
}

#[test]
fn a_variable_that_is_unset_empty_or_short_is_withheld_and_named_with_why_it_is_not_cut() {
    let text = "model: { provider: fake, script: run.yaml, api_key_env: WORK_KEY }";

    for (value, expected, shown) in [
        (None, Held::Unset, "WORK_KEY (not set, not cut)"),
        (Some("  "), Held::Empty, "WORK_KEY (empty, not cut)"),
        (
            Some("short"),
            Held::Short,
            "WORK_KEY (under 16 bytes, not cut)",
        ),
    ] {
        let held = move |asked: &str| {
            (asked == "WORK_KEY")
                .then(|| value.map(OsString::from))
                .flatten()
        };

        let derived = derived_from(text, &held);

        assert_eq!(derived.withheld, names(&["WORK_KEY"]));
        assert_eq!(derived.cut, [variable("WORK_KEY", expected)]);
        assert_eq!(derived.cut[0].to_string(), shown);
        assert!(derived.values.is_empty(), "{expected:?}");
    }
    assert_eq!(variable("K", Held::Cut).to_string(), "K");
}

#[test]
fn every_header_value_is_a_secret_as_substituted_and_a_variable_in_one_is_withheld() {
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

    let derived = derived_from(text, &env(&[("T", t), ("U", u)]));

    let headers = KeyPath::of("telemetry.otlp.headers");
    assert_eq!(
        derived,
        Derived {
            withheld: names(&["T", "U"]),
            cut: vec![
                variable("T", Held::Cut),
                variable("U", Held::Cut),
                key(headers.key("X-Literal"), Held::Cut),
                key(headers.key("X-Short"), Held::Short),
            ],
            values: secrets(&[
                &format!("Bearer {t}"),
                t,
                "literal-0123456789abcdef",
                "short",
                &format!("v-{t}-{u}"),
                u,
            ]),
        }
    );
    assert_eq!(
        derived.cut[3].to_string(),
        "telemetry.otlp.headers.X-Short (under 16 bytes, not cut)"
    );
}

#[test]
fn the_user_information_of_a_url_is_a_secret_in_each_of_its_forms_and_its_host_is_not() {
    let text = "
model: { provider: openai, base_url: 'https://hannah:${P}@gw.example/v1' }
telemetry: { otlp: { endpoint: 'https://${HOST}:4317' } }
tools:
  mcp:
    - { name: s, transport: http, url: 'https://user-0123456789ab:pw%40-0123456789abc@h.example/mcp' }
";
    let password = "pass-0123456789abcdef";

    let derived = derived_from(text, &env(&[("P", password), ("HOST", "otel.example")]));

    assert_eq!(
        derived,
        Derived {
            withheld: names(&["P"]),
            cut: vec![
                variable("P", Held::Cut),
                key(KeyPath::of("tools.mcp").index(0).key("url"), Held::Cut),
            ],
            values: secrets(&[
                &format!("hannah:{password}"),
                "hannah",
                password,
                "user-0123456789ab:pw%40-0123456789abc",
                "user-0123456789ab:pw@-0123456789abc",
                "user-0123456789ab",
                "pw%40-0123456789abc",
                "pw@-0123456789abc",
            ]),
        }
    );
    assert_eq!(derived.cut[1].to_string(), "tools.mcp[0].url");
}

#[test]
fn a_variable_that_gives_a_url_its_user_information_from_outside_the_written_text_is_withheld() {
    let base = "https://u-0123456789abcdef:p-0123456789abcdef@h";
    let text = "model: { provider: openai, base_url: '${BASE}/v1' }";

    let derived = derived_from(text, &env(&[("BASE", base)]));
    assert_eq!(
        derived,
        Derived {
            withheld: names(&["BASE"]),
            cut: vec![variable("BASE", Held::Cut)],
            values: secrets(&[
                base,
                "u-0123456789abcdef:p-0123456789abcdef",
                "u-0123456789abcdef",
                "p-0123456789abcdef",
            ]),
        }
    );

    let no_information = derived_from(text, &env(&[("BASE", "https://h.example")]));
    assert_eq!(no_information, derived_from(FAKE, &nothing));
}

#[test]
fn a_variable_substituted_into_an_env_is_a_secret_and_a_value_written_out_is_not() {
    let text = "
model: { provider: fake, script: run.yaml }
tools:
  builtin: { root: ., enabled: [bash], env: { GITHUB_TOKEN: '${GITHUB_TOKEN}', PATH: /usr/bin:/bin, MIXED: 'x-${A}' } }
  mcp:
    - { name: t, transport: stdio, command: tracker-mcp, env: { TRACKER_TOKEN: '${TRACKER_TOKEN}' } }
";
    let held = env(&[
        ("GITHUB_TOKEN", "ghp_0123456789abcdef"),
        ("A", "a-0123456789abcdef"),
        ("TRACKER_TOKEN", "trk-0123456789abcdef"),
    ]);

    let derived = derived_from(text, &held);

    assert_eq!(
        derived,
        Derived {
            withheld: names(&["A", "GITHUB_TOKEN", "TRACKER_TOKEN"]),
            cut: vec![
                variable("A", Held::Cut),
                variable("GITHUB_TOKEN", Held::Cut),
                variable("TRACKER_TOKEN", Held::Cut),
            ],
            values: secrets(&[
                "a-0123456789abcdef",
                "ghp_0123456789abcdef",
                "trk-0123456789abcdef",
            ]),
        }
    );
}

#[test]
fn the_values_are_held_only_for_a_run_that_has_an_executor() {
    let text = "model: { provider: fake, script: run.yaml, api_key_env: WORK_KEY }";
    let held = env(&[("WORK_KEY", KEY)]);
    let written = config(text);
    let real = written.substituted(&held).unwrap();

    let without = derived(&written, &real, &held, None, false);

    assert_eq!(without.values, Secrets::default());
    assert_eq!(without.withheld, names(&["WORK_KEY"]));
    assert_eq!(
        without.cut,
        [variable("WORK_KEY", Held::Cut)],
        "the value is still read, to say whether it would be cut"
    );
}

#[test]
fn a_variable_in_two_fields_is_withheld_and_named_once() {
    let text = "
model: { provider: fake, script: run.yaml, api_key_env: K }
telemetry: { otlp: { headers: { Authorization: 'Bearer ${K}' } } }
tools: { builtin: { root: ., enabled: [bash], env: { K: '${K}' } } }
";

    let derived = derived_from(text, &env(&[("K", KEY)]));

    assert_eq!(derived.withheld, names(&["K"]));
    assert_eq!(derived.cut, [variable("K", Held::Cut)]);
    assert_eq!(
        derived.values,
        secrets(&[KEY, &format!("Bearer {KEY}")]),
        "the header's whole value beside the key"
    );
}

#[test]
fn the_user_information_of_a_url_is_what_stands_before_the_at_of_its_authority() {
    assert_eq!(user_information("https://u:p@h/x"), Some("u:p"));
    assert_eq!(user_information("https://u@h?q=a@b"), Some("u"));
    assert_eq!(user_information("https://u:p%40q@h#a@b"), Some("u:p%40q"));
    assert_eq!(user_information("http://h/x@y"), None);
    assert_eq!(user_information("https://h"), None);
    assert_eq!(user_information("${BASE}/v1"), None);

    assert_eq!(without_user_information("https://u:p@h/x"), "https://h/x");
    assert_eq!(without_user_information("https://h/x"), "https://h/x");
}

/// A value without a scheme starts with its authority, as the gRPC
/// exporter reads it, so its user information is a URL's.
#[test]
fn a_value_without_a_scheme_starts_with_its_authority() {
    assert_eq!(user_information("u:p@h:4317"), Some("u:p"));
    assert_eq!(user_information("no-scheme:u@h"), Some("no-scheme:u"));
    assert_eq!(user_information("u@h/x://y"), Some("u"));
    assert_eq!(
        user_information("u:p@h://x"),
        Some("u:p"),
        "an `@` ends no scheme"
    );
    assert_eq!(user_information("h:4317/x@y"), None);
    assert_eq!(
        user_information("h:4317/p://u@q"),
        None,
        "a `/` ends no scheme"
    );
    assert_eq!(user_information("h:4317?p://u@q"), None);
    assert_eq!(user_information("h:4317#p://u@q"), None);
    assert_eq!(user_information("h:4317"), None);

    assert_eq!(without_user_information("u:p@h:4317"), "h:4317");
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

/// The OTLP header variables the exporter reads are the one secret found
/// rather than told: cut whole, as `env` prints a variable, and each
/// decoded header with it, and named without being withheld, since the
/// exporter in every command reads them too.
#[test]
fn an_otlp_header_variable_is_cut_whole_and_by_each_header_and_withheld_from_no_command() {
    const TOKEN: &str = "Bearer sk-0123456789abcdef";
    let value = "x-env=1,authorization=Bearer%20sk-0123456789abcdef";

    for name in Exporter::HEADER_VARIABLES {
        let derived = derived_from(FAKE, &env(&[(name, value)]));

        assert_eq!(
            derived,
            Derived {
                withheld: BTreeSet::new(),
                cut: vec![variable(name, Held::Cut)],
                values: secrets(&[value, TOKEN]),
            },
            "{name}"
        );
        assert!(!format!("{derived:?}").contains("sk-0123"), "{derived:?}");
    }
}

/// The OTLP endpoint variables the exporter reads are the other secret
/// found rather than told, for the credentials one carries: cut in each
/// form the config's endpoint's are, and named without being withheld,
/// since the exporter in every command reads them too. The host is no
/// secret, so a variable that names only a host isn't listed.
#[test]
fn an_otlp_endpoint_variable_s_user_information_is_cut_in_each_form_and_withheld_from_no_command() {
    const PASSWORD: &str = "pw%40-0123456789abcdef";
    let value = format!("https://collector-user:{PASSWORD}@collector.internal:4317/v1/traces");

    for name in Exporter::ENDPOINT_VARIABLES {
        let derived = derived_from(FAKE, &env(&[(name, &value)]));

        assert_eq!(
            derived,
            Derived {
                withheld: BTreeSet::new(),
                cut: vec![variable(name, Held::Cut)],
                values: secrets(&[
                    &format!("collector-user:{PASSWORD}"),
                    "collector-user:pw@-0123456789abcdef",
                    "collector-user",
                    PASSWORD,
                    "pw@-0123456789abcdef",
                ]),
            },
            "{name}"
        );
        assert!(
            !format!("{derived:?}").contains("0123456789"),
            "{derived:?}"
        );
    }

    let short = derived_from(
        FAKE,
        &env(&[("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", "https://u:p@h")]),
    );
    assert_eq!(
        short.cut,
        vec![variable("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", Held::Short)]
    );
    assert!(short.withheld.is_empty());
    assert_eq!(short.values, Secrets::default());

    let host_only = derived_from(
        FAKE,
        &env(&[(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "https://collector.internal:4317",
        )]),
    );
    assert_eq!(host_only, derived_from(FAKE, &nothing));
}

/// An endpoint without a scheme, which the gRPC exporter gives one, carries
/// its credentials where a URL does, so they're cut alike, from the config
/// and from the environment.
#[test]
fn the_user_information_of_an_endpoint_without_a_scheme_is_cut_as_a_urls_is() {
    const INFORMATION: &str = "collector-user:pw-0123456789abcdef";
    let endpoint = format!("{INFORMATION}@collector.internal:4317");
    let cut = secrets(&[INFORMATION, "collector-user", "pw-0123456789abcdef"]);

    let stated = derived_from(
        &format!("{FAKE}telemetry: {{ otlp: {{ endpoint: '{endpoint}' }} }}"),
        &nothing,
    );
    assert_eq!(
        stated.cut,
        [key(KeyPath::of("telemetry.otlp.endpoint"), Held::Cut)]
    );
    assert_eq!(stated.values, cut);

    let inherited = derived_from(FAKE, &env(&[("OTEL_EXPORTER_OTLP_ENDPOINT", &endpoint)]));
    assert_eq!(
        inherited.cut,
        [variable("OTEL_EXPORTER_OTLP_ENDPOINT", Held::Cut)]
    );
    assert!(inherited.withheld.is_empty());
    assert_eq!(inherited.values, cut);
}

#[test]
fn an_otlp_header_variable_that_is_empty_or_short_is_named_with_why_it_is_not_cut() {
    for (value, held) in [("  ", Held::Empty), ("a=b", Held::Short)] {
        let derived = derived_from(FAKE, &env(&[("OTEL_EXPORTER_OTLP_LOGS_HEADERS", value)]));

        assert_eq!(
            derived.cut,
            vec![variable("OTEL_EXPORTER_OTLP_LOGS_HEADERS", held)],
            "{value:?}"
        );
        assert!(derived.withheld.is_empty());
        assert_eq!(derived.values, Secrets::default());
    }
}

#[test]
fn an_otlp_header_variable_is_named_without_an_executor_and_its_value_held_only_with_one() {
    let held = env(&[(
        "OTEL_EXPORTER_OTLP_HEADERS",
        "authorization=Bearer%20sk-0123456789abcdef",
    )]);
    let written = config(FAKE);
    let real = written.substituted(&held).unwrap();

    let without = derived(&written, &real, &held, None, false);

    assert_eq!(
        without.cut,
        vec![variable("OTEL_EXPORTER_OTLP_HEADERS", Held::Cut)]
    );
    assert_eq!(without.values, Secrets::default());
}

/// A client key is read into the TLS identity when its signal's exporter
/// speaks TLS, and what its file holds is cut from what a tool returns,
/// each line too, as a value with several lines is. Its variable holds a
/// path, which is no secret: it's named among what's cut and withheld from
/// no command.
#[tokio::test]
async fn a_client_keys_contents_are_cut_from_a_tools_result_and_its_path_is_not() {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = Scratch::new("secrets-client-key");
    let key_file = scratch.write("client.key", receiver.client_key());
    let certificate_file = scratch.write("client.pem", receiver.client_certificate());
    let (key_path, certificate_path) = (
        key_file.display().to_string(),
        certificate_file.display().to_string(),
    );
    let named = [
        ("OTEL_EXPORTER_OTLP_CLIENT_KEY", key_path.as_str()),
        (
            "OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE",
            certificate_path.as_str(),
        ),
    ];
    let held = env(&named);
    let written = config(&format!(
        "{FAKE}telemetry: {{ otlp: {{ endpoint: 'https://collector.internal:4318' }} }}"
    ));
    let real = written.substituted(&held).unwrap();
    let otlp = crate::otlp::settings(&written, &real, &OtelEnv::read(&held).exporter)
        .unwrap()
        .unwrap();

    let derived = derived(&written, &real, &held, Some(&otlp), true);

    assert_eq!(
        derived.cut,
        vec![variable("OTEL_EXPORTER_OTLP_CLIENT_KEY", Held::Cut)]
    );
    assert!(derived.withheld.is_empty());
    let key = receiver.client_key();
    let line = key.lines().nth(1).unwrap();
    let result = format!("cat said:\n{key}\nand then: {line}\nfrom {key_path}");
    let cut = derived.values.redacted(&result);
    assert!(!cut.contains(line), "{cut}");
    assert!(cut.contains(&key_path), "the path is no secret: {cut}");
    assert!(!format!("{derived:?}").contains(line));
}
