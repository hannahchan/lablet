use std::num::NonZeroU32;

use serde_json::json;

use super::*;

fn yaml(text: &str) -> RawConfig {
    RawConfig::from_str(text, Format::Yaml).unwrap()
}

fn line(raw: &RawConfig, key: &KeyPath) -> Option<Place> {
    raw.places.get(key).copied()
}

fn override_refusal(raw: &mut RawConfig, key_value: &str) -> (Option<String>, String) {
    match raw.set(key_value) {
        Err(ConfigError::Override { key, reason }) => (key, reason),
        other => panic!("{key_value}: {other:?}"),
    }
}

const TEXT: &str = "
run:
  max_turns: 5
  timeout: 2m
tools:
  allow:
    - bash
    - read_file
  builtin: { root: work, env: { CI: 'true' } }
prompt:
  system: |
    You fix tests.
";

#[test]
fn a_yaml_tree_holds_the_line_of_every_key_and_item() {
    let raw = yaml(TEXT);

    assert_eq!(raw.tree()["run"]["max_turns"], json!(5));
    for (key, at) in [
        (KeyPath::of("run"), 2),
        (KeyPath::of("run.max_turns"), 3),
        (KeyPath::of("run.timeout"), 4),
        (KeyPath::of("tools.allow"), 6),
        (KeyPath::of("tools.allow").index(0), 7),
        (KeyPath::of("tools.allow").index(1), 8),
        (KeyPath::of("tools.builtin.env.CI"), 9),
        (KeyPath::of("prompt.system"), 11),
    ] {
        assert_eq!(line(&raw, &key), Some(Place::Line(at)), "{key}");
    }
}

#[test]
fn a_json_tree_holds_the_line_of_every_key_and_item_whatever_its_strings_hold() {
    let text = r#"{
  "run": { "max_turns": 5,
    "timeout": "2m" },
  "prompt": { "system": "a \"quoted\" {line} [of] text, \\ é" },
  "tools": { "allow": [
    "bash",
    "read_file"
  ], "deny": [], "builtin": { "enabled": [] } },
  "telemetry": { "resource": { "key": "v" } }
}"#;
    let raw = RawConfig::from_str(text, Format::Json).unwrap();

    assert_eq!(
        raw.tree()["prompt"]["system"],
        json!("a \"quoted\" {line} [of] text, \\ \u{e9}")
    );
    for (key, at) in [
        (KeyPath::of("run"), 2),
        (KeyPath::of("run.max_turns"), 2),
        (KeyPath::of("run.timeout"), 3),
        (KeyPath::of("prompt.system"), 4),
        (KeyPath::of("tools.allow").index(0), 6),
        (KeyPath::of("tools.allow").index(1), 7),
        (KeyPath::of("tools.deny"), 8),
        (KeyPath::of("tools.builtin.enabled"), 8),
        (KeyPath::of("telemetry.resource.key"), 9),
    ] {
        assert_eq!(line(&raw, &key), Some(Place::Line(at)), "{key}");
    }
}

#[test]
fn text_of_nothing_is_a_config_of_nothing_and_text_that_is_no_mapping_is_refused() {
    for text in ["", "# a comment\n", "~"] {
        assert_eq!(yaml(text).tree(), &json!({}), "{text:?}");
    }
    for (text, format) in [
        ("- run", Format::Yaml),
        ("5", Format::Yaml),
        ("[]", Format::Json),
        ("null", Format::Json),
        ("\"run\"", Format::Json),
    ] {
        assert_eq!(
            RawConfig::from_str(text, format),
            Err(ConfigError::Syntax {
                format,
                reason: "a config is a mapping of sections, such as `run:` and `model:`".to_owned(),
            }),
            "{text:?}"
        );
    }
    assert!(matches!(
        RawConfig::from_str("run: [", Format::Yaml),
        Err(ConfigError::Syntax {
            format: Format::Yaml,
            ..
        })
    ));
    assert!(matches!(
        RawConfig::from_str("{", Format::Json),
        Err(ConfigError::Syntax {
            format: Format::Json,
            ..
        })
    ));
}

#[test]
fn yaml_refuses_a_number_json_cannot_hold() {
    for number in [".nan", ".inf", "-.inf"] {
        assert!(
            matches!(
                RawConfig::from_str(&format!("model: {{ temperature: {number} }}"), Format::Yaml),
                Err(ConfigError::Syntax { .. })
            ),
            "{number}"
        );
    }
}

/// C3: an override is stated over what the text states, and a config read
/// from the tree holds it.
#[test]
fn an_override_states_a_setting_over_the_text_and_says_it_set_it() {
    let mut raw = yaml(TEXT);

    raw.set("run.max_turns=7").unwrap();

    assert_eq!(raw.tree()["run"]["max_turns"], json!(7));
    assert_eq!(
        line(&raw, &KeyPath::of("run.max_turns")),
        Some(Place::Override)
    );
    assert_eq!(raw.config().unwrap().run.max_turns, NonZeroU32::new(7));
    assert_eq!(
        line(&raw, &KeyPath::of("run.timeout")),
        Some(Place::Line(4)),
        "what the override didn't set keeps its line"
    );
}

#[test]
fn an_override_value_is_one_yaml_scalar() {
    let mut raw = yaml("");
    for (key_value, key, value) in [
        ("run.max_retries=3", "run.max_retries", json!(3)),
        ("run.retry_jitter=0.5", "run.retry_jitter", json!(0.5)),
        (
            "telemetry.capture_content=true",
            "telemetry.capture_content",
            json!(true),
        ),
        ("prompt.system=yes", "prompt.system", json!("yes")),
        (
            "prompt.system=Fix it = now",
            "prompt.system",
            json!("Fix it = now"),
        ),
        ("prompt.system='5'", "prompt.system", json!("5")),
        ("run.max_turns=null", "run.max_turns", json!(null)),
        ("run.max_turns=", "run.max_turns", json!(null)),
        ("run.max_total_tokens=-4", "run.max_total_tokens", json!(-4)),
        ("telemetry.file.path=-", "telemetry.file.path", json!("-")),
        ("tools.allow=[bash]", "tools.allow", json!("[bash]")),
        ("prompt.system=Fix: it", "prompt.system", json!("Fix: it")),
        (
            "prompt.system=\"unclosed",
            "prompt.system",
            json!("\"unclosed"),
        ),
        ("prompt.system=- a", "prompt.system", json!("- a")),
    ] {
        raw.set(key_value).unwrap();

        assert_eq!(
            KeyPath::of(key).find(raw.tree()),
            Some(&value),
            "{key_value}"
        );
    }
}

#[test]
fn an_override_adds_the_sections_on_the_way_to_its_key_and_replaces_what_was_under_it() {
    let mut raw = yaml(TEXT);

    raw.set("telemetry.file.path=-").unwrap();
    raw.set("tools.builtin=null").unwrap();
    raw.set("tools.allow.1=write_file").unwrap();

    assert_eq!(raw.tree()["telemetry"], json!({ "file": { "path": "-" } }));
    assert_eq!(raw.tree()["tools"]["builtin"], json!(null));
    assert_eq!(line(&raw, &KeyPath::of("tools.builtin.env.CI")), None);
    assert_eq!(
        line(&raw, &KeyPath::of("tools.builtin")),
        Some(Place::Override)
    );
    assert_eq!(raw.tree()["tools"]["allow"], json!(["bash", "write_file"]));
    assert_eq!(
        line(&raw, &KeyPath::of("tools.allow").index(1)),
        Some(Place::Override)
    );
    assert_eq!(
        line(&raw, &KeyPath::of("tools.allow").index(0)),
        Some(Place::Line(7))
    );

    raw.set("tools.builtin.root=elsewhere").unwrap();
    assert_eq!(
        raw.tree()["tools"]["builtin"],
        json!({ "root": "elsewhere" })
    );
}

#[test]
fn an_override_that_names_no_setting_or_holds_a_value_it_takes_not_is_refused_when_read() {
    let mut unknown = yaml(TEXT);
    unknown.set("run.max_turn=3").unwrap();

    assert!(matches!(
        unknown.config(),
        Err(ConfigError::UnknownKey { key, place: Some(Place::Override), .. })
            if key == "run.max_turn"
    ));

    let mut refused = yaml(TEXT);
    refused.set("run.max_turns=zero").unwrap();
    assert_eq!(
        refused.config().unwrap_err().to_string(),
        "run.max_turns (an override): \"zero\" is refused: expected a nonzero u32"
    );
}

#[test]
fn an_override_that_is_no_key_and_value_is_refused_without_its_value() {
    let mut raw = yaml(TEXT);

    let (key, reason) = override_refusal(&mut raw, "sk-ant-api03-secret");
    assert_eq!(key, None);
    assert_eq!(
        reason,
        "an override is written `key=value`, and this one holds no `=`"
    );

    for written in ["=5", "run..max_turns=5", ".run=5", "run.=5"] {
        let (key, reason) = override_refusal(&mut raw, written);
        assert_eq!(key.as_deref(), written.split_once('=').map(|(key, _)| key));
        assert!(
            reason.contains("none of them is empty"),
            "{written}: {reason}"
        );
    }

    let (key, reason) = override_refusal(&mut raw, "run.max_turns.cap=5");
    assert_eq!(key.as_deref(), Some("run.max_turns.cap"));
    assert_eq!(
        reason,
        "run.max_turns holds a value, and a value has no keys"
    );

    for place in ["2", "-1", "first"] {
        let (_, reason) = override_refusal(&mut raw, &format!("tools.allow.{place}=grep"));
        assert_eq!(
            reason,
            format!("tools.allow is a list of 2, and `{place}` is no place in it")
        );
    }
    assert_eq!(raw, yaml(TEXT), "a refused override changes nothing");
}

#[test]
fn a_config_read_from_a_file_keeps_where_it_was_read_from_through_its_tree() {
    let scratch = lablet_test_support::Scratch::new("raw-from-path");
    let path = scratch.write("lablet.yaml", "run: { max_turns: 4 }");

    let mut raw = RawConfig::from_path(&path).unwrap();
    raw.set("run.max_turns=6").unwrap();
    let config = raw.config().unwrap();

    assert_eq!(config.source(), Some(path.as_path()));
    assert_eq!(config.run.max_turns, NonZeroU32::new(6));
}
