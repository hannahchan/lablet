use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;

use super::*;
use crate::config::Format;

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

/// `text` substituted in the environment `held`, with the names dropped.
fn substituted(text: &str, held: &dyn Fn(&str) -> Option<OsString>) -> Result<String, String> {
    replaced(text, held, &mut Vec::new())
}

#[test]
fn a_reference_is_replaced_by_what_its_variable_holds() {
    let held = env(&[("HOME", "/home/lab"), ("A", "1"), ("B_2", ""), ("_x", "x")]);

    for (text, written) in [
        ("${HOME}/work", "/home/lab/work"),
        ("${A}${A}-${B_2}-${_x}", "11--x"),
        ("no reference", "no reference"),
        ("", ""),
    ] {
        assert_eq!(substituted(text, &held), Ok(written.to_owned()), "{text}");
    }
}

#[test]
fn a_dollar_that_begins_no_reference_is_text_and_a_doubled_one_writes_a_reference_as_text() {
    let held = env(&[("A", "1")]);

    for (text, written) in [
        ("$A and $", "$A and $"),
        ("costs $5", "costs $5"),
        ("$${A}", "${A}"),
        ("$$${A}", "$${A}"),
        ("$$A", "$$A"),
        ("${A}$", "1$"),
    ] {
        assert_eq!(substituted(text, &held), Ok(written.to_owned()), "{text}");
    }
}

#[test]
fn a_variable_that_is_not_set_or_is_no_text_is_refused_by_its_name_alone() {
    assert_eq!(
        substituted("x ${MISSING} y", &nothing),
        Err("the variable `MISSING` isn't set".to_owned())
    );
    let binary = |_: &str| Some(OsStr::from_bytes(b"sk-\xff").to_owned());
    let refused = substituted("${KEY}", &binary).unwrap_err();
    assert_eq!(refused, "the variable `KEY` holds what isn't UTF-8");
}

#[test]
fn a_reference_that_names_no_variable_is_refused_with_how_to_write_one() {
    let held = env(&[("A", "1")]);
    for text in [
        "${",
        "${A",
        "${}",
        "${1A}",
        "${A-B}",
        "${A B}",
        "${a.b}",
        "x ${A:-y}",
    ] {
        let refused = substituted(text, &held).unwrap_err();

        assert!(
            refused.starts_with("`${` begins a variable, written `${NAME}`")
                && refused.ends_with("write `$${` for the text `${`"),
            "{text}: {refused}"
        );
    }
}

/// The config of `text`, substituted in the environment `held`.
fn config(text: &str, held: &dyn Fn(&str) -> Option<OsString>) -> Result<Substituted, Refusal> {
    Config::from_str(text, Format::Yaml)
        .unwrap()
        .substituted(held)
}

#[test]
fn the_variables_replaced_are_kept_by_the_setting_they_were_replaced_in() {
    let held = env(&[("A", "1"), ("B", "2")]);
    let text = "
model: { name: '${A}-${B}-${A}' }
tools: { builtin: { env: { X: '${B}', Y: literal } } }
telemetry: { otlp: { headers: { H: '${A}' } } }
";
    let real = config(text, &held).unwrap();
    let within =
        |key: KeyPath| -> Vec<String> { real.replaced_within(&key).map(str::to_owned).collect() };

    assert_eq!(within(KeyPath::of("model.name")), ["A", "B"], "once each");
    assert_eq!(within(KeyPath::of("tools.builtin.env")), ["B"]);
    assert_eq!(
        within(KeyPath::of("tools.builtin.env").key("Y")),
        Vec::<String>::new()
    );
    assert_eq!(
        within(KeyPath::of("telemetry.otlp.headers").key("H")),
        ["A"]
    );
    assert_eq!(within(KeyPath::of("telemetry")), ["A"]);
    assert_eq!(within(KeyPath::of("prompt")), Vec::<String>::new());
}

/// The substituted config holds what a variable holds, so it's never
/// formatted: its `Debug` form names the type and nothing in it.
#[test]
fn a_substituted_config_is_shown_as_nothing_of_what_it_holds() {
    let secret = "sk-0123456789abcdef-no-log-holds";
    let real = config("model: { name: '${SECRET}' }", &env(&[("SECRET", secret)])).unwrap();

    assert_eq!(format!("{real:?}"), "Substituted { .. }");
    assert_eq!(real.model.name, secret);
}

#[test]
fn every_setting_that_holds_text_is_substituted_and_no_other_is() {
    let held = env(&[("V", "value")]);
    let text = "
run:
  transcript_path: ${V}/t.json
  completion_schema: { type: object, description: '${V}', items: ['${V}'] }
model:
  provider: openai
  name: ${V}
  api_key_env: ${V}
  base_url: http://${V}
prompt:
  system: ${V}
  skills: ['${V}/SKILL.md']
tools:
  builtin: { root: '${V}', env: { KEPT: '${V}' } }
  mcp:
    - { name: '${V}', transport: stdio, command: '${V}', args: ['${V}'], env: { E: '${V}' } }
    - { name: http, transport: http, url: 'http://${V}', headers: { H: '${V}' } }
  allow: ['${V}']
  deny: ['${V}']
telemetry:
  otlp: { endpoint: 'http://${V}', headers: { H: '${V}' } }
  file: { path: '${V}.jsonl' }
  resource: { team: '${V}' }
";

    let real = config(text, &held).unwrap();

    assert_eq!(real.run.transcript_path, Some("value/t.json".into()));
    assert_eq!(
        real.run.completion_schema,
        Some(serde_json::json!({ "type": "object", "description": "value", "items": ["value"] }))
    );
    assert_eq!(real.model.name, "value");
    assert_eq!(real.model.api_key_env.as_deref(), Some("value"));
    assert_eq!(real.model.base_url.as_deref(), Some("http://value"));
    assert_eq!(real.prompt.system.as_deref(), Some("value"));
    assert_eq!(real.prompt.skills, [PathBuf::from("value/SKILL.md")]);
    assert_eq!(real.tools.builtin.root, Some("value".into()));
    assert_eq!(real.tools.builtin.env["KEPT"], "value");
    assert_eq!(
        real.tools.mcp[0],
        McpServer::Stdio {
            name: "value".to_owned(),
            command: "value".to_owned(),
            args: vec!["value".to_owned()],
            env: BTreeMap::from([("E".to_owned(), "value".to_owned())]),
            startup_timeout: std::time::Duration::from_secs(30),
            call_timeout: std::time::Duration::from_secs(300),
            names: super::super::McpNames::Prefixed,
            instructions: true,
        }
    );
    assert!(matches!(
        &real.tools.mcp[1],
        McpServer::Http { url, headers, .. }
            if url == "http://value" && headers["H"] == "value"
    ));
    assert_eq!(real.tools.allow, Some(vec!["value".to_owned()]));
    assert_eq!(real.tools.deny, ["value"]);
    assert_eq!(
        real.telemetry.otlp.endpoint.as_deref(),
        Some("http://value")
    );
    assert_eq!(
        real.telemetry
            .otlp
            .headers
            .as_ref()
            .map(|headers| &headers["H"]),
        Some(&"value".to_owned())
    );
    assert_eq!(real.telemetry.file.path, Some("value.jsonl".into()));
    assert_eq!(real.telemetry.resource["team"], "value");

    let written = Config::from_str(text, Format::Yaml).unwrap();
    assert_eq!(
        written.model.name, "${V}",
        "the config keeps what's written"
    );
}

#[test]
fn a_setting_whose_variable_is_not_set_is_refused_by_its_key() {
    for (text, key) in [
        ("model: { name: '${UNSET}' }", KeyPath::of("model.name")),
        (
            "prompt: { skills: [a, '${UNSET}'] }",
            KeyPath::of("prompt.skills").index(1),
        ),
        (
            "tools: { builtin: { env: { A: '${UNSET}' } } }",
            KeyPath::of("tools.builtin.env").key("A"),
        ),
        (
            "tools: { mcp: [{ name: a, transport: stdio, command: a, args: [x, '${UNSET}'] }] }",
            KeyPath::of("tools.mcp").index(0).key("args").index(1),
        ),
        (
            "run: { completion_schema: { a: [{ b: '${UNSET}' }] } }",
            KeyPath::of("run.completion_schema")
                .key("a")
                .index(0)
                .key("b"),
        ),
    ] {
        assert_eq!(
            config(text, &nothing).map(drop),
            Err(Refusal::Invalid {
                key,
                reason: "the variable `UNSET` isn't set".to_owned()
            }),
            "{text}"
        );
    }
}

#[test]
fn a_path_that_is_not_utf_8_is_left_as_it_is() {
    let mut written = Config::default();
    let path = PathBuf::from(OsStr::from_bytes(b"${V}/\xff"));
    written.tools.builtin.root = Some(path.clone());

    let real = written.substituted(&nothing).unwrap();

    assert_eq!(real.tools.builtin.root, Some(path));
}
