use serde_json::json;

use super::*;
use crate::{Config, Format, RawConfig, Refusal};

/// A key as long as a key is.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

/// The config `text` holds, with each `${VAR}` filled from `held`.
fn filled(text: &str, held: &[(&str, &str)]) -> Config {
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RawConfig::from_str(text, Format::Yaml)
        .unwrap()
        .substituted(move |asked: &str| -> Option<std::ffi::OsString> {
            held.iter()
                .find(|(name, _)| name == asked)
                .map(|(_, value)| value.into())
        })
        .unwrap()
        .config()
        .unwrap()
}

/// The mask over a config with no fill.
fn plain(key: &str, value: &Value) -> Option<String> {
    shown(&KeyPath::of(key), Some(value), &Filled::default())
}

/// Every key a credential sits under is one the mask never shows a value
/// of, whatever it holds, but for the marker a secret set in code is
/// written as.
#[test]
fn a_credential_s_value_is_never_shown_but_the_marker_is() {
    for key in [
        "model.api_key",
        "tools.builtin.credentials.GITHUB_TOKEN",
        "telemetry.otlp.headers.Authorization",
        "tools.mcp[0].headers.X-Token",
        "tools.mcp[2].headers.H",
    ] {
        assert!(is_credential(&KeyPath::of(key)), "{key}");
        for value in [
            json!(KEY),
            json!("ANTHROPIC_API_KEY"),
            json!(""),
            json!(7),
            json!([KEY]),
        ] {
            assert_eq!(plain(key, &value), None, "{key}: {value}");
        }
        assert_eq!(
            plain(key, &json!(Secret::MARKER)),
            Some("\"[secret]\"".to_owned()),
            "{key}"
        );
    }
    for key in [
        "model.name",
        "tools.builtin.env.TOKEN",
        "tools.mcp[0].url",
        "tools.builtin.credential",
        "telemetry.otlp.endpoint",
    ] {
        assert!(!is_credential(&KeyPath::of(key)), "{key}");
    }

    // A filled credential is shown as the file wrote it when that's one
    // `${VAR}` alone, or a header, whose `Bearer ${T}` form is the usual
    // one; the key or a tool's credential written beside its `${VAR}`
    // shows nothing, since the text beside it may be the value.
    let config = filled(
        "
model: { provider: fake, script: run.yaml, api_key: '${K}' }
tools: { builtin: { credentials: { A: '${G}', B: 'ghp-${G}' } } }
telemetry: { otlp: { headers: { Authorization: 'Bearer ${T}' } } }
",
        &[
            ("K", KEY),
            ("G", "ghp-0123456789abcdef"),
            ("T", "tok-0123456789abcdef"),
        ],
    );
    let tree = serde_json::to_value(&config).unwrap();
    let at = |key: &str| {
        let key = KeyPath::of(key);
        shown(&key, key.find(&tree), config.filled())
    };
    assert_eq!(at("model.api_key"), Some("\"${K}\"".to_owned()));
    assert_eq!(
        at("tools.builtin.credentials.A"),
        Some("\"${G}\"".to_owned())
    );
    assert_eq!(at("tools.builtin.credentials.B"), None);
    assert_eq!(
        at("telemetry.otlp.headers.Authorization"),
        Some("\"Bearer ${T}\"".to_owned())
    );
    let mixed = filled(
        "model: { provider: fake, script: run.yaml, api_key: 'sk-${K}' }",
        &[("K", KEY)],
    );
    let tree = serde_json::to_value(&mixed).unwrap();
    let key = KeyPath::of("model.api_key");
    assert_eq!(shown(&key, key.find(&tree), mixed.filled()), None);
    assert_eq!(mixed.written_text("model.api_key"), None);
}

/// A value that may hold a credential is never shown: user information
/// before an `@`, and a URL's query.
#[test]
fn a_value_holding_an_at_or_a_url_holding_a_query_is_never_shown() {
    for hidden in [
        "https://user:pw-0123456789abcdef@gw.example/v1",
        "user:pw@h",
        "mail me @ home",
        "https://gw.example/v1?token=0123456789abcdef",
    ] {
        assert_eq!(plain("model.base_url", &json!(hidden)), None, "{hidden}");
        assert_eq!(plain("prompt.system", &json!(hidden)), None, "{hidden}");
    }
    assert_eq!(
        plain("model.base_url", &json!("https://gw.example/v1")),
        Some("\"https://gw.example/v1\"".to_owned())
    );
    assert_eq!(
        plain("prompt.system", &json!("what? no URL")),
        Some("\"what? no URL\"".to_owned()),
        "a query is a URL's"
    );
}

/// A filled setting is shown as the file wrote it, whatever the variable
/// gave, and a written text that may hold a credential isn't shown either.
#[test]
fn a_filled_setting_is_shown_as_written_and_never_as_filled() {
    let config = filled(
        "
model: { name: '${MODEL}', base_url: 'https://${U}:${P}@gw.example/v1' }
tools: { builtin: { root: '${REPO}/work', env: { T: '${TOK}', PORT: '${PORT}' } } }
telemetry: { file: { path: 'https://u:literal-0123456789@h/${X}' } }
",
        &[
            ("MODEL", "model-0123456789"),
            ("U", "user"),
            ("P", "pw-0123456789abcdef"),
            ("REPO", "/home/lab/repo"),
            ("TOK", "tok-0123456789abcdef"),
            ("PORT", "8080"),
            ("X", "x"),
        ],
    );
    let tree = serde_json::to_value(&config).unwrap();
    let at = |key: &str| {
        let key = KeyPath::of(key);
        shown(&key, key.find(&tree), config.filled())
    };

    assert_eq!(at("model.name"), Some("\"${MODEL}\"".to_owned()));
    assert_eq!(
        at("tools.builtin.root"),
        Some("\"${REPO}/work\"".to_owned())
    );
    assert_eq!(at("tools.builtin.env.T"), Some("\"${TOK}\"".to_owned()));
    assert_eq!(at("tools.builtin.env.PORT"), Some("\"${PORT}\"".to_owned()));
    assert_eq!(
        at("model.base_url"),
        None,
        "the written URL holds an `@`, so not even its written form is shown"
    );
    assert_eq!(at("telemetry.file.path"), None);
    assert_eq!(
        at("prompt.skills"),
        Some("[]".to_owned()),
        "a setting the fill didn't change is shown as it is"
    );
    assert_eq!(
        config.written_text("tools.builtin.root").as_deref(),
        Some("${REPO}/work")
    );
    assert_eq!(config.written_text("model.base_url"), None);
}

/// The serialisation of a resolved config writes back every key the fill
/// changed, so `check --resolved` prints `${VAR}` and no value a variable
/// gave, while the digest writes back the settings taken as written and no
/// other.
#[test]
fn the_mask_writes_back_every_filled_key_for_the_resolved_form_and_the_paths_for_the_digest() {
    let config = filled(
        "
model: { provider: fake, script: '${REPO}/run.yaml', name: '${MODEL}', api_key: '${KEY}' }
prompt: { system: 'Fix ${LANG}.' }
run: { timeout: '${T}' }
tools:
  builtin: { root: '${REPO}', enabled: [bash], env: { TOK: '${TOK}' }, credentials: { C: '${KEY}' } }
  mcp:
    - { name: s, transport: stdio, command: '${BIN}/server', args: ['${ARG}'], env: { E: '${E}' } }
telemetry: { otlp: { headers: { Authorization: 'Bearer ${KEY}' } } }
",
        &[
            ("REPO", "/home/lab/repo"),
            ("MODEL", "scripted-7"),
            ("KEY", KEY),
            ("T", "7m"),
            ("TOK", "tok-0123456789abcdef"),
            ("BIN", "/opt/bin"),
            ("ARG", "--fast"),
            ("E", "e-value"),
        ],
    );
    let resolved = config.resolved();

    let written = serde_json::to_value(&resolved).unwrap();
    assert_eq!(written["model"]["script"], json!("${REPO}/run.yaml"));
    assert_eq!(written["model"]["name"], json!("${MODEL}"));
    assert_eq!(written["model"]["api_key"], json!("${KEY}"));
    assert_eq!(written["run"]["timeout"], json!("${T}"));
    assert_eq!(written["tools"]["builtin"]["root"], json!("${REPO}"));
    assert_eq!(written["tools"]["builtin"]["env"]["TOK"], json!("${TOK}"));
    assert_eq!(
        written["tools"]["builtin"]["credentials"]["C"],
        json!("${KEY}")
    );
    assert_eq!(
        written["tools"]["mcp"][0]["command"],
        json!("${BIN}/server")
    );
    assert_eq!(written["tools"]["mcp"][0]["args"][0], json!("${ARG}"));
    assert_eq!(written["tools"]["mcp"][0]["env"]["E"], json!("${E}"));
    assert_eq!(
        written["telemetry"]["otlp"]["headers"]["Authorization"],
        json!("Bearer ${KEY}")
    );
    assert_eq!(written["prompt"]["system"], json!("Fix ${LANG}."));
    let shown = written.to_string();
    for value in [
        KEY,
        "scripted-7",
        "tok-0123456789abcdef",
        "/home/lab",
        "/opt/bin",
        "--fast",
        "e-value",
        "7m",
    ] {
        assert!(!shown.contains(value), "{value}: {shown}");
    }

    let digested = serde_json::to_value(Masked::new(
        &resolved.sections(),
        &resolved.filled,
        WrittenBack::AsWritten,
    ))
    .unwrap();
    assert_eq!(digested["model"]["script"], json!("${REPO}/run.yaml"));
    assert_eq!(digested["model"]["name"], json!("scripted-7"), "filled");
    assert_eq!(digested["run"]["timeout"], json!("7m"), "filled");
    assert_eq!(digested["tools"]["builtin"]["root"], json!("${REPO}"));
    assert_eq!(digested["tools"]["builtin"]["env"]["TOK"], json!("${TOK}"));
    assert_eq!(
        digested["tools"]["mcp"][0]["command"],
        json!("${BIN}/server")
    );
    assert_eq!(digested["tools"]["mcp"][0]["args"][0], json!("${ARG}"));
    assert_eq!(digested["tools"]["mcp"][0]["env"]["E"], json!("${E}"));
    assert_eq!(
        digested["model"]["api_key"],
        json!("[secret]"),
        "typed, so the marker"
    );
    assert_eq!(
        digested["tools"]["builtin"]["credentials"]["C"],
        json!("[secret]")
    );

    let debug = format!("{resolved:?}");
    for value in [KEY, "scripted-7", "tok-0123456789abcdef"] {
        assert!(!debug.contains(value), "{value}: {debug}");
    }
    assert!(debug.contains("${MODEL}"), "{debug}");
}

/// A secret set in code, which no fill recorded, is written as the marker
/// wherever the config is written out.
#[test]
fn a_secret_set_in_code_is_written_as_the_marker() {
    let mut config = Config::default();
    config.model.api_key = Some(Secret::new(KEY));
    config
        .tools
        .builtin
        .credentials
        .insert("T".to_owned(), Secret::new("tok-0123456789abcdef"));

    let written = serde_json::to_value(config.resolved()).unwrap();

    assert_eq!(written["model"]["api_key"], json!("[secret]"));
    assert!(!written.to_string().contains("0123456789"), "{written}");
    assert_eq!(
        serde_json::to_value(&config).unwrap()["model"]["api_key"],
        json!("[secret]")
    );
}

/// The keys the digest takes as written are the paths, a command's
/// variables and what starts a server, and no other.
#[test]
fn the_digest_takes_the_paths_and_a_command_s_variables_as_written() {
    for key in [
        "model.script",
        "run.transcript_path",
        "prompt.system_file",
        "prompt.skills[0]",
        "tools.builtin.root",
        "tools.builtin.env.PATH",
        "tools.mcp[0].command",
        "tools.mcp[3].args[1]",
        "tools.mcp[0].env.E",
    ] {
        assert!(digested_as_written(&KeyPath::of(key)), "{key}");
    }
    for key in [
        "model.name",
        "model.base_url",
        "run.timeout",
        "tools.builtin.enabled[0]",
        "tools.builtin.credentials.T",
        "tools.mcp[0].name",
        "tools.mcp[0].url",
        "tools.mcp[x].command",
        "telemetry.file.path",
    ] {
        assert!(!digested_as_written(&KeyPath::of(key)), "{key}");
    }
}

/// An item of a list the config keeps as a set, `tools.allow`, `tools.deny`
/// or `tools.builtin.enabled`, is written back by the text the fill gave it,
/// since the set holds its items in its own order and not the file's: the
/// resolved form, a `Debug` form and a message show `${A}` for the item a
/// variable gave and the literal item as it is, and the resolved form reads
/// back to the same digest, since the digest takes the lists filled.
#[test]
fn an_item_of_a_list_kept_as_a_set_is_written_back_by_the_text_the_fill_gave_it() {
    let held = [("A", "read_file"), ("T", "read_file"), ("D", "write_file")];
    let config = filled(
        "
tools:
  allow: ['${A}', bash]
  deny: [bash, '${D}']
  builtin: { root: /work, enabled: ['${T}', bash] }
",
        &held,
    );
    let resolved = config.resolved();

    let written = serde_json::to_value(&resolved).unwrap();
    assert_eq!(written["tools"]["allow"], json!(["bash", "${A}"]));
    assert_eq!(written["tools"]["deny"], json!(["bash", "${D}"]));
    assert_eq!(
        written["tools"]["builtin"]["enabled"],
        json!(["bash", "${T}"])
    );
    for form in [format!("{config:?}"), format!("{resolved:?}")] {
        assert!(
            !form.contains("read_file") && !form.contains("write_file"),
            "{form}"
        );
    }
    let tree = serde_json::to_value(&config).unwrap();
    let key = KeyPath::of("tools.builtin.enabled");
    assert_eq!(
        shown(&key, key.find(&tree), config.filled()),
        Some("[\"bash\",\"${T}\"]".to_owned())
    );
    assert_eq!(
        config.written_text_at(&KeyPath::of("tools.allow").index(0)),
        Some("${A}".to_owned())
    );
    assert_eq!(
        config.written_text_at(&KeyPath::of("tools.allow").index(1)),
        Some("bash".to_owned())
    );
    let digested = serde_json::to_value(Masked::new(
        &resolved.sections(),
        &resolved.filled,
        WrittenBack::AsWritten,
    ))
    .unwrap();
    assert_eq!(
        digested["tools"]["builtin"]["enabled"],
        json!(["bash", "read_file"]),
        "filled"
    );
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let back = RawConfig::from_str(&written.to_string(), Format::Json)
        .unwrap()
        .substituted(move |asked: &str| -> Option<std::ffi::OsString> {
            held.iter()
                .find(|(name, _)| name == asked)
                .map(|(_, value)| value.into())
        })
        .unwrap()
        .config()
        .unwrap();
    assert_eq!(back.digest(), config.digest());
}

/// A section is shown setting by setting, each as the file wrote it and
/// under its own rule, and not at all when one of them is never shown: a
/// refusal at a section's key shows no filled value and no credential, and
/// nothing at all at a `credentials` map that holds one.
#[test]
fn a_section_is_shown_setting_by_setting_as_written_and_not_at_all_when_one_is_never_shown() {
    let held = [
        ("MODEL", "scripted-7"),
        ("K", KEY),
        ("PORT", "8080"),
        ("PW", "pw-0123456789abcdef"),
        ("TOK", "tok-0123456789abcdef"),
    ];
    let config = filled(
        "
model: { provider: fake, script: run.yaml, name: '${MODEL}', api_key: '${K}' }
tools:
  builtin:
    root: /work
    enabled: [bash]
    env: { PORT: '${PORT}' }
",
        &held,
    );
    let tree = serde_json::to_value(&config).unwrap();
    let at = |key: &str| {
        let key = KeyPath::of(key);
        shown(&key, key.find(&tree), config.filled())
    };
    // The section whole, before the cut a message makes.
    let whole = |key: &str| {
        let key = KeyPath::of(key);
        shown_whole(&key, key.find(&tree).unwrap(), config.filled()).map(|value| value.to_string())
    };

    assert_eq!(
        at("tools.builtin.env"),
        Some("{\"PORT\":\"${PORT}\"}".to_owned())
    );
    let builtin = whole("tools.builtin").unwrap();
    assert!(
        builtin.contains("\"credentials\":{}")
            && builtin.contains("\"env\":{\"PORT\":\"${PORT}\"}"),
        "{builtin}"
    );
    assert!(!builtin.contains("8080"), "{builtin}");
    let model = whole("model").unwrap();
    assert!(
        model.contains("\"name\":\"${MODEL}\"") && model.contains("\"api_key\":\"${K}\""),
        "{model}"
    );
    assert!(
        !model.contains("scripted-7") && !model.contains(KEY),
        "{model}"
    );
    let refused = config
        .refused(Refusal::invalid("tools.builtin.env", "probe"))
        .to_string();
    assert_eq!(
        refused,
        "tools.builtin.env (line 7): {\"PORT\":\"${PORT}\"} is refused: probe"
    );

    let hidden = filled(
        "
model: { api_key: null }
tools: { builtin: { env: { PORT: '${PORT}', HOST: 'https://u:${PW}@h/x' }, credentials: { T: '${TOK}' } } }
",
        &held,
    );
    let tree = serde_json::to_value(&hidden).unwrap();
    let at = |key: &str| {
        let key = KeyPath::of(key);
        shown(&key, key.find(&tree), hidden.filled())
    };
    assert_eq!(
        at("tools.builtin.env"),
        None,
        "a value may hold a credential"
    );
    assert_eq!(
        at("tools.builtin.credentials"),
        None,
        "nothing at a credentials map that holds one is shown"
    );
    assert_eq!(at("tools"), None);
    let model = KeyPath::of("model");
    assert!(
        shown_whole(&model, model.find(&tree).unwrap(), hidden.filled())
            .unwrap()
            .to_string()
            .contains("\"api_key\":null")
    );
    let refused = hidden
        .refused(Refusal::invalid("tools.builtin.env", "probe"))
        .to_string();
    assert_eq!(
        refused,
        "tools.builtin.env (line 3): its value is refused: probe"
    );
    assert_eq!(hidden.written_text("tools.builtin.env"), None);
}
