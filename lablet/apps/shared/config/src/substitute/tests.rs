use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::PathBuf;

use serde_json::json;

use super::*;
use crate::{Config, ConfigError, Format, McpServer, Place, RawConfig, Secret};

/// An environment that holds what `held` says, and nothing else.
fn env(held: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
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

/// `text` filled in the environment `held`, as a text of its own.
fn filled_text(text: &str, held: &dyn Fn(&str) -> Option<OsString>) -> Result<String, String> {
    match replaced(text, held, false)? {
        Replaced::Unchanged => Ok(text.to_owned()),
        Replaced::Text(filled, _) => Ok(filled),
        Replaced::Unset(_) => unreachable!("only the key leaves a setting unset"),
    }
}

/// The raw config of `text`, filled in the environment `held`.
fn raw(text: &str, held: &dyn Fn(&str) -> Option<OsString>) -> Result<RawConfig, ConfigError> {
    RawConfig::from_str(text, Format::Yaml)
        .unwrap()
        .substituted(held)
}

/// The config of `text`, filled in the environment `held`.
fn config(text: &str, held: &dyn Fn(&str) -> Option<OsString>) -> Result<Config, ConfigError> {
    raw(text, held)?.config()
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
        assert_eq!(filled_text(text, &held), Ok(written.to_owned()), "{text}");
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
        assert_eq!(filled_text(text, &held), Ok(written.to_owned()), "{text}");
    }
}

/// `${VAR:-default}` takes the default when the variable is unset or
/// empty, and the variable's value otherwise; `${VAR:-}` takes nothing.
#[test]
fn a_reference_with_a_default_takes_it_when_the_variable_is_unset_or_empty() {
    let held = env(&[("SET", "value"), ("EMPTY", "")]);

    for (text, written) in [
        ("${SET:-other}", "value"),
        ("${EMPTY:-other}", "other"),
        ("${UNSET:-other}", "other"),
        ("${UNSET:-}", ""),
        ("${EMPTY:-}", ""),
        ("${UNSET:-a:-b}", "a:-b"),
        ("x${UNSET:-}y", "xy"),
    ] {
        assert_eq!(filled_text(text, &held), Ok(written.to_owned()), "{text}");
    }
}

#[test]
fn a_variable_that_is_not_set_or_is_no_text_is_refused_by_its_name_alone() {
    assert_eq!(
        filled_text("x ${MISSING} y", &nothing),
        Err("the variable `MISSING` isn't set".to_owned())
    );
    let binary = |_: &str| Some(OsStr::from_bytes(b"sk-\xff").to_owned());
    let refused = filled_text("${KEY}", &binary).unwrap_err();
    assert_eq!(refused, "the variable `KEY` holds what isn't UTF-8");
}

#[test]
fn a_reference_that_names_no_variable_is_refused_with_how_to_write_one() {
    let held = env(&[("A", "1")]);
    for text in [
        "${", "${A", "${}", "${1A}", "${A-B}", "${A B}", "${a.b}", "${:-x}",
    ] {
        let refused = filled_text(text, &held).unwrap_err();

        assert!(
            refused.starts_with("`${` begins a variable, written `${NAME}`")
                && refused.ends_with("write `$${` for the text `${`"),
            "{text}: {refused}"
        );
    }
}

/// The record holds each key the fill changed, with the text as written and
/// the variables it named, once each and in order.
#[test]
fn the_fill_records_each_changed_key_with_its_written_text_and_its_variables() {
    let held = env(&[("A", "1"), ("B", "2")]);
    let text = "
model: { name: '${A}-${B}-${A}', api_key: '${A}' }
tools: { builtin: { env: { X: '${B}', Y: literal, Z: 'costs $${A}' } } }
telemetry: { otlp: { headers: { H: '${UNSET:-x}' } } }
";
    let config = config(text, &held).unwrap();
    let filled = config.filled();

    assert!(filled.ran());
    let variables = |key: &str| -> Vec<(String, bool)> {
        filled
            .at(&KeyPath::of(key))
            .unwrap()
            .variables
            .iter()
            .map(|variable| (variable.name.clone(), variable.set))
            .collect()
    };
    assert_eq!(config.model.name, "1-2-1");
    assert_eq!(
        filled.at(&KeyPath::of("model.name")).unwrap().written,
        "${A}-${B}-${A}"
    );
    assert_eq!(
        variables("model.name"),
        [("A".to_owned(), true), ("B".to_owned(), true)],
        "once each"
    );
    assert_eq!(config.tools.builtin.env["Z"], "costs ${A}");
    assert_eq!(
        filled
            .at(&KeyPath::of("tools.builtin.env.X"))
            .unwrap()
            .filled,
        "2",
        "a filled text is kept to match a list item back to its fill"
    );
    assert_eq!(
        variables("tools.builtin.env.Z"),
        Vec::<(String, bool)>::new(),
        "an escape names no variable"
    );
    assert_eq!(
        variables("telemetry.otlp.headers.H"),
        [("UNSET".to_owned(), false)]
    );
    assert_eq!(filled.at(&KeyPath::of("tools.builtin.env.Y")), None);
    assert_eq!(
        filled
            .within(&KeyPath::of("tools.builtin.env"))
            .map(|(key, _)| key.to_string())
            .collect::<Vec<_>>(),
        ["tools.builtin.env.X", "tools.builtin.env.Z"]
    );
    assert_eq!(
        filled.iter().count(),
        5,
        "{:?}",
        filled
            .iter()
            .map(|(key, _)| key.to_string())
            .collect::<Vec<_>>()
    );
    let key = filled.at(&KeyPath::of("model.api_key")).unwrap();
    assert!(key.is_one_variable());
    assert_eq!(key.default(), None);
    assert_eq!(
        key.filled, "",
        "a credential's value isn't copied into the record"
    );
    assert_eq!(
        format!("{key:?}"),
        "Fill { written: \"${A}\", variables: [Variable { name: \"A\", set: true }], .. }",
        "the text the fill gave is in no Debug form"
    );
    assert!(
        !filled
            .at(&KeyPath::of("model.name"))
            .unwrap()
            .is_one_variable()
    );
    assert_eq!(
        filled
            .at(&KeyPath::of("telemetry.otlp.headers.H"))
            .unwrap()
            .default(),
        Some("x")
    );
}

/// Every text but the prompts' is filled, in every section and at any
/// depth, and always as text.
#[test]
fn every_text_but_a_prompt_is_filled_and_a_prompt_is_never_scanned() {
    let held = env(&[("V", "value"), ("PORT", "8080")]);
    let text = "
run:
  transcript_path: ${V}/t.json
  completion_schema: { type: object, description: '${V}', items: ['${V}'] }
model:
  provider: openai
  name: ${V}
  api_key: ${V}
  base_url: http://${V}
prompt:
  system: ${V} and ${UNSET} and $${V}
  skills: ['${V}/SKILL.md']
tools:
  builtin: { root: '${V}', env: { KEPT: '${V}', PORT: '${PORT}' }, credentials: { C: '${V}' } }
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
        Some(json!({ "type": "object", "description": "value", "items": ["value"] }))
    );
    assert_eq!(real.model.name, "value");
    assert_eq!(real.model.api_key, Some(Secret::new("value")));
    assert_eq!(real.model.base_url.as_deref(), Some("http://value"));
    assert_eq!(
        real.prompt.system.as_deref(),
        Some("${V} and ${UNSET} and $${V}"),
        "a prompt is text, and an unset variable in it refuses nothing"
    );
    assert_eq!(real.prompt.skills, [PathBuf::from("value/SKILL.md")]);
    assert_eq!(real.tools.builtin.root, Some("value".into()));
    assert_eq!(real.tools.builtin.env["KEPT"], "value");
    assert_eq!(real.tools.builtin.env["PORT"], "8080", "a number is text");
    assert_eq!(real.tools.builtin.credentials["C"], Secret::new("value"));
    assert_eq!(
        real.tools.mcp[0],
        McpServer::Stdio {
            name: "value".to_owned(),
            command: "value".to_owned(),
            args: vec!["value".to_owned()],
            env: BTreeMap::from([("E".to_owned(), "value".to_owned())]),
            startup_timeout: std::time::Duration::from_secs(30),
            call_timeout: std::time::Duration::from_secs(300),
            names: crate::McpNames::Prefixed,
            instructions: true,
        }
    );
    assert!(matches!(
        &real.tools.mcp[1],
        McpServer::Http { url, headers, .. }
            if url == "http://value" && headers["H"] == Secret::new("value")
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
        Some(&Secret::new("value"))
    );
    assert_eq!(real.telemetry.file.path, Some("value.jsonl".into()));
    assert_eq!(real.telemetry.resource["team"], "value");
    assert_eq!(
        real.filled().at(&KeyPath::of("prompt.system")),
        None,
        "nothing of a prompt is recorded"
    );
    assert_eq!(
        config("prompt: { system_file: '${V}.md' }", &held)
            .unwrap()
            .prompt
            .system_file,
        Some("${V}.md".into())
    );
}

/// A number setting takes a filled text as it takes any text: refused.
#[test]
fn a_setting_that_holds_no_text_refuses_a_filled_variable_as_it_refuses_text() {
    let error = config("run: { max_turns: '${N}' }", &env(&[("N", "8")])).unwrap_err();

    assert_eq!(
        error.to_string(),
        "run.max_turns (line 1): \"${N}\" is refused: expected a nonzero u32",
        "the refusal shows the setting as written, not what the variable gave"
    );
}

#[test]
fn a_setting_whose_variable_is_not_set_is_refused_by_its_key_and_shown_as_written() {
    for (text, key) in [
        ("model: { name: '${UNSET}' }", "model.name"),
        ("prompt: { skills: [a, '${UNSET}'] }", "prompt.skills[1]"),
        (
            "tools: { builtin: { env: { A: '${UNSET}' } } }",
            "tools.builtin.env.A",
        ),
        (
            "tools: { mcp: [{ name: a, transport: stdio, command: a, args: [x, '${UNSET}'] }] }",
            "tools.mcp[0].args[1]",
        ),
        (
            "run: { completion_schema: { a: [{ b: '${UNSET}' }] } }",
            "run.completion_schema.a[0].b",
        ),
    ] {
        let error = config(text, &nothing).unwrap_err();

        assert_eq!(
            error,
            ConfigError::Invalid {
                key: key.to_owned(),
                place: Some(Place::Line(1)),
                value: Some("\"${UNSET}\"".to_owned()),
                reason: "the variable `UNSET` isn't set".to_owned(),
            },
            "{text}"
        );
    }
    let credential = config(
        "tools: { builtin: { credentials: { T: '${UNSET}' } } }",
        &nothing,
    )
    .unwrap_err();
    assert_eq!(
        credential.to_string(),
        "tools.builtin.credentials.T (line 1): its value is refused: the variable `UNSET` isn't \
         set",
        "a credential is never shown, written or not"
    );
}

/// The key is the one setting an unset variable leaves unset: whether a key
/// is needed is the provider's to say.
#[test]
fn an_unset_variable_at_the_key_leaves_it_unset_and_recorded() {
    let real = config("model: { api_key: '${KEY}' }", &nothing).unwrap();

    assert_eq!(real.model.api_key, None);
    let fill = real.filled().at(&KeyPath::of("model.api_key")).unwrap();
    assert_eq!(fill.written, "${KEY}");
    assert_eq!(
        fill.variables,
        [Variable {
            name: "KEY".to_owned(),
            set: false
        }]
    );

    let empty = config("model: { api_key: '${KEY}' }", &env(&[("KEY", "")])).unwrap();
    assert_eq!(empty.model.api_key, Some(Secret::new("")));
    assert!(
        empty
            .filled()
            .at(&KeyPath::of("model.api_key"))
            .unwrap()
            .variables[0]
            .set
    );
}

/// After the fill every `${` is text, so an escape survives the fill and
/// what it writes reads back as the same text.
#[test]
fn an_escape_survives_the_fill_as_the_text_it_writes() {
    let real = config("model: { name: 'costs $${PRICE}' }", &nothing).unwrap();

    assert_eq!(real.model.name, "costs ${PRICE}");
    assert_eq!(
        real.filled()
            .at(&KeyPath::of("model.name"))
            .unwrap()
            .written,
        "costs $${PRICE}"
    );
}

/// A text that wants a fill is refused when none ran, by its key, except a
/// prompt, which no fill reads.
#[test]
fn a_config_read_without_a_fill_refuses_a_text_that_wants_one() {
    for (text, key) in [
        ("model: { name: '${MODEL}' }", "model.name"),
        (
            "tools: { builtin: { root: 'x$${y}' } }",
            "tools.builtin.root",
        ),
        (
            "tools: { builtin: { env: { A: '${A}' } } }",
            "tools.builtin.env.A",
        ),
    ] {
        let error = Config::from_str(text, Format::Yaml).unwrap_err();

        assert!(
            matches!(&error, ConfigError::Invalid { key: refused, .. } if refused == key),
            "{text}: {error:?}"
        );
        assert!(
            error.to_string().ends_with(
                "is refused: it was never filled; call .substituted(lookup) on the RawConfig \
                 before .config()"
            ),
            "{error}"
        );
    }
    assert_eq!(
        Config::from_str("model: { name: '${MODEL}' }", Format::Yaml)
            .unwrap_err()
            .to_string(),
        "model.name (line 1): \"${MODEL}\" is refused: it was never filled; call \
         .substituted(lookup) on the RawConfig before .config()"
    );
    let prompt = Config::from_str(
        "prompt: { system: 'Fix ${X}.', system_file: null }",
        Format::Yaml,
    )
    .unwrap();
    assert_eq!(prompt.prompt.system.as_deref(), Some("Fix ${X}."));
    assert!(!prompt.filled().ran());
    let credential = Config::from_str("model: { api_key: '${KEY}' }", Format::Yaml).unwrap_err();
    assert_eq!(
        credential.to_string(),
        "model.api_key (line 1): its value is refused: it was never filled; call \
         .substituted(lookup) on the RawConfig before .config()"
    );
}

/// A fill that finds nothing still counts as having run, so a config that
/// writes `$${` and nothing else passes through it.
#[test]
fn a_fill_that_finds_nothing_ran_all_the_same() {
    let real = config("model: { name: plain }", &nothing).unwrap();

    assert!(real.filled().ran());
    assert!(real.filled().is_empty());
    assert!(
        !Config::from_str("model: { name: plain }", Format::Yaml)
            .unwrap()
            .filled()
            .ran()
    );
}

/// An override stated after the fill is as it's written, and nothing is
/// recorded for the key it set.
#[test]
fn an_override_after_the_fill_takes_the_record_of_what_it_replaced_away() {
    let mut raw = raw("model: { name: '${A}' }", &env(&[("A", "1")])).unwrap();

    raw.set("model.name=plain").unwrap();
    let config = raw.config().unwrap();

    assert_eq!(config.model.name, "plain");
    assert_eq!(config.filled().at(&KeyPath::of("model.name")), None);
}
