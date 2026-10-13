use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::num::NonZeroU32;
use std::time::Duration;

use lablet_test_support::Scratch;
use serde_json::json;

use super::*;

/// The config of spec §7, comments and all. This file is the source of its
/// defaults, and the spec's block quotes it: a test never reads
/// `product/spec.md`, which can drift.
const SPEC: &str = include_str!("tests/spec.yaml");

/// What the spec's `${ANTHROPIC_API_KEY}` is filled with here.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

fn yaml(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

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

fn refusal(text: &str, format: Format) -> String {
    match Config::from_str(text, format) {
        Err(ConfigError::Syntax {
            format: read_as,
            reason,
        }) => {
            assert_eq!(read_as, format);
            reason
        }
        other => panic!("{text:?} was read as {other:?}"),
    }
}

/// Every default, written out, so that a default that changes is a test
/// that fails.
fn defaults() -> Config {
    Config {
        run: Run {
            completion: Completion::Natural,
            max_turns: None,
            timeout: Duration::from_secs(600),
            max_total_tokens: None,
            max_retries: 10,
            retry_backoff_base: Duration::from_millis(500),
            retry_backoff_max: Duration::from_secs(32),
            retry_jitter: 0.25,
            retry_hint_max: Duration::from_secs(60),
            max_consecutive_invalid_turns: NonZeroU32::new(3),
            provider_timeout: Duration::from_secs(600),
            context: Context::Full,
            transcript_path: None,
            transcript_format: TranscriptFormat::Json,
            completion_schema: None,
        },
        model: Model {
            provider: Provider::Anthropic,
            api: None,
            script: None,
            name: "claude-sonnet-5".to_owned(),
            api_key: None,
            base_url: None,
            max_tokens: 32_000,
            temperature: None,
            thinking: None,
            effort: None,
            seed: None,
            reasoning_replay: None,
            cache: None,
            cache_scope: CacheScope::Shared,
            pricing: None,
        },
        prompt: Prompt {
            system: None,
            system_file: None,
            skills: Vec::new(),
            skills_mode: SkillsMode::Tool,
        },
        tools: Tools {
            builtin: Builtin {
                root: None,
                enabled: BTreeSet::new(),
                timeout: Duration::from_secs(120),
                env: BTreeMap::new(),
                credentials: BTreeMap::new(),
            },
            mcp: Vec::new(),
            mcp_lifetime: McpLifetime::Run,
            mcp_result: McpResult::Structured,
            allow: None,
            deny: Vec::new(),
            max_concurrent_calls: NonZeroU32::new(10).unwrap(),
            max_output_bytes: Some(50_000),
            output_cut: OutputCut::Preview,
            output_preview_bytes: 2_000,
            max_description_chars: Some(2_048),
        },
        telemetry: Telemetry {
            capture_content: None,
            propagators: None,
            otlp: Otlp {
                endpoint: None,
                protocol: None,
                headers: None,
                tls: Tls {
                    ca_file: None,
                    client_cert: None,
                    client_key: None,
                },
            },
            file: TelemetryFile { path: None },
            resource: Resource::default(),
        },
        source: None,
        places: Places::default(),
        prompt_file: None,
        filled: Filled::default(),
        based: false,
    }
}

#[test]
fn a_config_that_states_nothing_has_every_default_the_spec_gives() {
    assert_eq!(yaml(""), defaults());
    assert_eq!(Config::from_str("{}", Format::Json).unwrap(), defaults());
    assert_eq!(Config::default(), defaults());
}

#[test]
fn a_section_that_states_one_setting_keeps_the_defaults_of_the_rest() {
    let config = yaml("run:\n  max_turns: 5\ntools:\n  builtin:\n    timeout: 2s\n");

    let mut expected = defaults();
    expected.run.max_turns = NonZeroU32::new(5);
    expected.tools.builtin.timeout = Duration::from_secs(2);
    assert_eq!(config, expected);
}

/// The spec states its defaults, the provider's own among them, and two MCP
/// servers, a system prompt and the key's `${VAR}` as examples.
#[test]
fn the_config_the_spec_writes_is_read_whole_and_its_values_are_the_defaults() {
    let config = filled(SPEC, &[("ANTHROPIC_API_KEY", KEY)]);

    let mut expected = defaults();
    expected.model.api_key = Some(Secret::new(KEY));
    expected.model.thinking = Some(Thinking::ProviderDefault);
    expected.model.reasoning_replay = Some(false);
    expected.model.cache = Some(true);
    expected.prompt.system = Some("You are ...".to_owned());
    expected.tools.mcp = vec![
        McpServer::Stdio {
            name: "docs".to_owned(),
            command: "npx".to_owned(),
            args: vec!["-y".to_owned(), "@example/docs-mcp".to_owned()],
            env: BTreeMap::new(),
            startup_timeout: Duration::from_secs(30),
            call_timeout: Duration::from_secs(300),
            names: McpNames::Prefixed,
            instructions: true,
        },
        McpServer::Http {
            name: "search".to_owned(),
            url: "http://localhost:8080/mcp".to_owned(),
            headers: BTreeMap::new(),
            startup_timeout: Duration::from_secs(30),
            call_timeout: Duration::from_secs(300),
            names: McpNames::Prefixed,
            instructions: true,
        },
    ];
    assert_eq!(
        config
            .filled()
            .at(&KeyPath::of("model.api_key"))
            .unwrap()
            .written,
        "${ANTHROPIC_API_KEY}"
    );
    let config = Config {
        filled: Filled::default(),
        ..config
    };
    assert_eq!(config, expected);
    assert_eq!(config.tools.mcp[0].name(), "docs");
    assert_eq!(config.tools.mcp[1].name(), "search");
    assert!(
        Config::from_str(SPEC, Format::Yaml).is_err(),
        "the spec's config wants a fill"
    );
}

#[test]
fn json_and_yaml_read_to_the_same_config() {
    let from_yaml = yaml(
        "
run:
  completion: explicit
  timeout: 90s
  retry_jitter: 0
  context: { mask: { trigger_tokens: 150000, keep_last: 5 } }
  transcript_path: out/{run_id}.json
model:
  provider: openai
  api: chat_completions
  effort: xhigh
  thinking: { budget: 2048 }
  pricing: { input: 3, output: 15, cache_read: 0.3, cache_write: 3.75 }
tools:
  builtin: { root: work, enabled: [write_file, bash], env: { CI: '1' } }
  allow: [bash]
  max_output_bytes: null
  output_cut: head_tail
telemetry:
  file: { path: '-' }
  resource: { team: evals }
",
    );
    let from_json = Config::from_str(
        &json!({
            "run": {
                "completion": "explicit",
                "timeout": "1m 30s",
                "retry_jitter": 0.0,
                "context": { "mask": { "trigger_tokens": 150_000, "keep_last": 5 } },
                "transcript_path": "out/{run_id}.json",
            },
            "model": {
                "provider": "openai",
                "api": "chat_completions",
                "effort": "xhigh",
                "thinking": { "budget": 2048 },
                "pricing": { "input": 3, "output": 15, "cache_read": 0.3, "cache_write": 3.75 },
            },
            "tools": {
                "builtin": { "root": "work", "enabled": ["bash", "write_file"], "env": { "CI": "1" } },
                "allow": ["bash"],
                "max_output_bytes": null,
                "output_cut": "head_tail",
            },
            "telemetry": { "file": { "path": "-" }, "resource": { "team": "evals" } },
        })
        .to_string(),
        Format::Json,
    )
    .unwrap();

    assert_eq!(from_yaml, from_json);
    assert_eq!(from_yaml.run.completion, Completion::Explicit);
    assert_eq!(from_yaml.run.timeout, Duration::from_secs(90));
    assert_eq!(
        from_yaml.run.context,
        Context::Mask {
            trigger_tokens: 150_000,
            keep_last: 5
        }
    );
    assert_eq!(from_yaml.model.effort, Some(Effort::XHigh));
    assert_eq!(
        from_yaml.model.thinking,
        Some(Thinking::Budget(NonZeroU32::new(2_048).unwrap()))
    );
    assert_eq!(
        from_yaml.model.pricing,
        Some(Pricing {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75
        })
    );
    assert_eq!(
        from_yaml.tools.builtin.enabled,
        BTreeSet::from([BuiltinTool::Bash, BuiltinTool::WriteFile])
    );
    assert_eq!(from_yaml.tools.allow, Some(vec!["bash".to_owned()]));
    assert_eq!(from_yaml.tools.max_output_bytes, None);
    assert_eq!(from_yaml.tools.output_cut, OutputCut::HeadTail);
    assert_eq!(from_yaml.telemetry.file.path, Some("-".into()));
}

/// The refusal of `text`, which is YAML.
fn refused(text: &str) -> ConfigError {
    Config::from_str(text, Format::Yaml).unwrap_err()
}

fn invalid(key: &str, line: u32, value: &str, reason: &str) -> ConfigError {
    ConfigError::Invalid {
        key: key.to_owned(),
        place: Some(Place::Line(line)),
        value: Some(value.to_owned()),
        reason: reason.to_owned(),
    }
}

#[test]
fn a_key_the_config_does_not_know_is_refused_by_its_path_in_every_section() {
    for (text, key) in [
        ("runs: {}", "runs"),
        ("run: { max_turn: 3 }", "run.max_turn"),
        (
            "run: { context: { mask: { trigger_tokens: 1, keep_last: 1, keep: 2 } } }",
            "run.context.mask.keep",
        ),
        ("model: { nme: scripted-1 }", "model.nme"),
        (
            "model: { api_key_env: ANTHROPIC_API_KEY }",
            "model.api_key_env",
        ),
        (
            "model: { pricing: { input: 1, output: 1, cache_read: 1, cache_write: 1, cached: 1 } }",
            "model.pricing.cached",
        ),
        ("prompt: { sytem: Hi. }", "prompt.sytem"),
        ("tools: { alow: [bash] }", "tools.alow"),
        ("tools: { builtin: { roots: work } }", "tools.builtin.roots"),
        (
            "tools: { mcp: [{ name: docs, transport: stdio, command: npx, colour: red }] }",
            "tools.mcp[0].colour",
        ),
        (
            "tools: { mcp: [{ name: docs, transport: http, url: 'http://localhost/mcp', command: npx }] }",
            "tools.mcp[0].command",
        ),
        ("telemetry: { capture: true }", "telemetry.capture"),
        (
            "telemetry: { otlp: { endpoints: [] } }",
            "telemetry.otlp.endpoints",
        ),
        ("telemetry: { file: { paths: [] } }", "telemetry.file.paths"),
    ] {
        match refused(text) {
            ConfigError::UnknownKey {
                key: refused,
                place,
                known,
            } => {
                assert_eq!(refused, key, "{text}");
                assert_eq!(place, Some(Place::Line(1)), "{text}");
                assert!(!known.is_empty(), "{text}");
            }
            other => panic!("{text}: {other:?}"),
        }
    }
    let refusal = Config::from_str("{\n  \"run\": {\n    \"max_turn\": 3\n  }\n}", Format::Json);
    assert_eq!(
        refusal.unwrap_err().to_string(),
        "run.max_turn (line 3) is refused: no setting has the key; the keys beside it are \
         `completion`, `max_turns`, `timeout`, `max_total_tokens`, `max_retries`, \
         `retry_backoff_base`, `retry_backoff_max`, `retry_jitter`, `retry_hint_max`, \
         `max_consecutive_invalid_turns`, `provider_timeout`, `context`, `transcript_path`, \
         `transcript_format`, `completion_schema`"
    );
}

/// The key a config named the key's variable under before `api_key` is
/// refused as any unknown key is, and the refusal names `api_key` beside it.
#[test]
fn a_config_that_writes_api_key_env_is_refused_as_an_unknown_key_naming_api_key() {
    let refusal = refused("model:\n  api_key_env: ANTHROPIC_API_KEY\n").to_string();

    assert!(
        refusal.starts_with(
            "model.api_key_env (line 2) is refused: no setting has the key; the keys beside it are"
        ),
        "{refusal}"
    );
    assert!(refusal.contains("`api_key`"), "{refusal}");
    assert!(!refusal.contains("`api_key_env`"), "{refusal}");
}

#[test]
fn a_refusal_names_the_line_its_key_is_written_on() {
    let text = "
run:
  max_turns: 5
  completion: sometimes
tools:
  allow:
    - bash
    - 7
";
    assert_eq!(
        refused(text),
        invalid(
            "run.completion",
            4,
            "\"sometimes\"",
            "the accepted values are `natural`, `explicit`"
        )
    );
    assert_eq!(
        refused(&text.replace("sometimes", "natural")),
        invalid("tools.allow[1]", 8, "7", "expected a string")
    );
    assert_eq!(
        refused(&text.replace("sometimes", "natural")).to_string(),
        "tools.allow[1] (line 8): 7 is refused: expected a string"
    );
}

#[test]
fn a_value_no_setting_takes_is_refused_with_the_values_it_takes() {
    for (text, key, value, said) in [
        (
            "run: { completion: sometimes }",
            "run.completion",
            "\"sometimes\"",
            "the accepted values are `natural`, `explicit`",
        ),
        (
            "model: { provider: gemini }",
            "model.provider",
            "\"gemini\"",
            "the accepted values are `anthropic`, `openai`, `fake`",
        ),
        (
            "model: { effort: extreme }",
            "model.effort",
            "\"extreme\"",
            "the accepted values are `low`, `medium`, `high`, `xhigh`, `max`",
        ),
        (
            "tools: { builtin: { enabled: [task_complete] } }",
            "tools.builtin.enabled[0]",
            "\"task_complete\"",
            "the accepted values are `bash`, `read_file`, `write_file`",
        ),
        (
            "tools: { output_cut: tail }",
            "tools.output_cut",
            "\"tail\"",
            "the accepted values are `preview`, `head`, `head_tail`",
        ),
        (
            "tools: { mcp: [{ name: docs, transport: pipe }] }",
            "tools.mcp[0].transport",
            "\"pipe\"",
            "the accepted values are `stdio`, `http`",
        ),
        (
            "run: { context: { masked: { trigger_tokens: 1, keep_last: 1 } } }",
            "run.context",
            r#"{"masked":{"keep_last":1,"trigger_tokens":1}}"#,
            "the accepted values are `full`, `mask`",
        ),
        (
            "run: { max_turns: 0 }",
            "run.max_turns",
            "0",
            "expected a nonzero u32",
        ),
        (
            "tools: { max_concurrent_calls: 0 }",
            "tools.max_concurrent_calls",
            "0",
            "expected a nonzero u32",
        ),
        (
            "model: { thinking: { budget: 0 } }",
            "model.thinking.budget",
            "0",
            "expected a nonzero u32",
        ),
        (
            "run: { max_retries: -1 }",
            "run.max_retries",
            "-1",
            "expected u32",
        ),
        (
            "run: { max_turns: five }",
            "run.max_turns",
            "\"five\"",
            "expected a nonzero u32",
        ),
    ] {
        assert_eq!(refused(text), invalid(key, 1, value, said), "{text}");
    }
}

#[test]
fn a_duration_is_read_in_humantime_syntax_and_anything_else_is_refused_as_written() {
    let config = yaml(
        "
run:
  timeout: 1h 30m
  retry_backoff_base: 250ms
  retry_backoff_max: 8s
  retry_hint_max: 2m
  provider_timeout: 45s
tools:
  builtin: { timeout: 1s 500ms }
",
    );

    assert_eq!(config.run.timeout, Duration::from_mins(90));
    assert_eq!(config.run.retry_backoff_base, Duration::from_millis(250));
    assert_eq!(config.run.retry_backoff_max, Duration::from_secs(8));
    assert_eq!(config.run.retry_hint_max, Duration::from_secs(120));
    assert_eq!(config.run.provider_timeout, Duration::from_secs(45));
    assert_eq!(config.tools.builtin.timeout, Duration::from_millis(1_500));

    for written in ["ten minutes", "-5s", "5"] {
        assert_eq!(
            refused(&format!("run: {{ timeout: '{written}' }}")),
            invalid(
                "run.timeout",
                1,
                &format!("{written:?}"),
                "a duration is a number and a unit, as `500ms`, `10m` and `1h 30m` are"
            )
        );
    }
    assert_eq!(
        refused("run: { timeout: 10 }"),
        invalid("run.timeout", 1, "10", "expected a string")
    );
}

#[test]
fn null_turns_a_cap_off_and_leaving_it_out_keeps_the_default() {
    let off = yaml(
        "
run: { max_consecutive_invalid_turns: null }
tools: { max_output_bytes: null, max_description_chars: null }
",
    );

    assert_eq!(off.run.max_consecutive_invalid_turns, None);
    assert_eq!(off.tools.max_output_bytes, None);
    assert_eq!(off.tools.max_description_chars, None);

    let kept = yaml("run: { max_retries: 2 }\ntools: { deny: [] }\n");
    assert_eq!(kept.run.max_consecutive_invalid_turns, NonZeroU32::new(3));
    assert_eq!(kept.tools.max_output_bytes, Some(50_000));
    assert_eq!(kept.tools.max_description_chars, Some(2_048));
}

#[test]
fn yaml_reads_yes_as_text_and_a_key_written_twice_as_an_error() {
    assert_eq!(
        yaml("prompt: { system: yes }").prompt.system.as_deref(),
        Some("yes")
    );

    let reason = refusal("run:\n  timeout: 1m\n  timeout: 2m\n", Format::Yaml);
    assert!(
        reason.contains("duplicate mapping key: timeout"),
        "{reason}"
    );
    let reason = refusal(
        "{\"run\": {\"timeout\": \"1m\",\n \"timeout\": \"2m\"}}",
        Format::Json,
    );
    assert_eq!(reason, "the key `timeout` is written twice, on line 2");
}

#[test]
fn text_that_is_not_in_the_format_is_refused_and_the_error_names_the_format() {
    let error = Config::from_str("run: [", Format::Yaml).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("the config can't be read as YAML: "),
        "{error}"
    );

    let error = Config::from_str("run: {}", Format::Json).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("the config can't be read as JSON: "),
        "{error}"
    );
}

#[test]
fn a_file_is_read_in_the_format_its_name_says_and_the_config_keeps_where_it_was_read_from() {
    let scratch = Scratch::new("formats");
    let text_of = |format| match format {
        Format::Yaml => "run: { max_turns: 4 }",
        Format::Json => r#"{"run": {"max_turns": 4}}"#,
    };

    for (name, format) in [
        ("lablet.yaml", Format::Yaml),
        ("lablet.yml", Format::Yaml),
        ("lablet.json", Format::Json),
    ] {
        let path = scratch.write(name, text_of(format));
        assert_eq!(Format::of(&path), Some(format));

        let config = Config::from_path(&path).unwrap();

        assert_eq!(config.run.max_turns, NonZeroU32::new(4));
        assert_eq!(config.source(), Some(path.as_path()));
    }
    assert_eq!(yaml("").source(), None);
}

#[test]
fn a_file_that_cannot_be_read_as_a_config_is_refused_and_the_error_names_it() {
    let scratch = Scratch::new("refused");

    let toml = scratch.write("lablet.toml", "[run]");
    let bare = scratch.write("lablet", "run: {}");
    for path in [toml, bare] {
        assert_eq!(Format::of(&path), None);
        let error = Config::from_path(&path).unwrap_err();
        assert_eq!(
            error,
            ConfigError::UnknownFormat {
                path: path.display().to_string()
            }
        );
        assert!(error.to_string().contains(&path.display().to_string()));
    }

    let missing = scratch.at("missing.yaml");
    match Config::from_path(&missing).unwrap_err() {
        ConfigError::Unreadable { path, reason } => {
            assert_eq!(path, missing.display().to_string());
            assert!(!reason.is_empty());
        }
        other => panic!("{other:?}"),
    }

    let misspelt = scratch.write("misspelt.yaml", "run: { max_turn: 3 }");
    assert!(matches!(
        Config::from_path(&misspelt),
        Err(ConfigError::UnknownKey { key, .. }) if key == "run.max_turn"
    ));
    let text = scratch.write("list.json", "[1, 2]");
    assert_eq!(
        Config::from_path(&text),
        Err(ConfigError::Syntax {
            format: Format::Json,
            reason: "a config is a mapping of sections, such as `run:` and `model:`".to_owned(),
        })
    );
}

#[test]
fn a_message_shows_a_value_as_json_writes_it_and_cuts_a_long_one() {
    let key = KeyPath::of("prompt.system");
    let none = Filled::default();

    assert_eq!(
        shown(&key, Some(&json!("Fix it.")), &none),
        Some("\"Fix it.\"".to_owned())
    );
    assert_eq!(shown(&key, Some(&json!(7)), &none), Some("7".to_owned()));
    assert_eq!(shown(&key, None, &none), None);
    let long = "\u{e9}".repeat(200);
    let cut = shown(&key, Some(&json!(long)), &none).unwrap();
    assert_eq!(cut.chars().count(), shown::SHOWN_CHARS + 1);
    assert!(
        cut.starts_with("\"\u{e9}") && cut.ends_with("\u{e9}…"),
        "{cut}"
    );
    let fits = "a".repeat(shown::SHOWN_CHARS - 2);
    assert_eq!(
        shown(&key, Some(&json!(fits)), &none),
        Some(format!("\"{fits}\""))
    );
}

/// The key is a secret: a value of the wrong kind there is refused without
/// being shown, as every value under the key is but the marker.
#[test]
fn a_key_of_the_wrong_kind_is_refused_without_its_value() {
    let error = refused("model: { api_key: [sk-ant-api03-secret] }");
    assert_eq!(
        error,
        ConfigError::Invalid {
            key: "model.api_key".to_owned(),
            place: Some(Place::Line(1)),
            value: None,
            reason: "expected a string".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "model.api_key (line 1): its value is refused: expected a string"
    );
}

/// R55: the marker a secret set in code is written as doesn't read back.
#[test]
fn the_marker_of_a_secret_set_in_code_is_refused_when_read_back() {
    for (text, key) in [
        ("model: { api_key: '[secret]' }", "model.api_key"),
        (
            "tools: { builtin: { credentials: { T: '[secret]' } } }",
            "tools.builtin.credentials.T",
        ),
        (
            "telemetry: { otlp: { headers: { Authorization: '[secret]' } } }",
            "telemetry.otlp.headers.Authorization",
        ),
    ] {
        let error = refused(text);

        assert_eq!(
            error.to_string(),
            format!(
                "{key} (line 1): \"[secret]\" is refused: it's how lablet writes a secret set in \
                 code; set it with Secret::new"
            ),
            "{text}"
        );
    }
}

/// A config's `Debug` form is the masked tree: a secret is the marker, a
/// filled setting is as the file wrote it, and a value that may hold a
/// credential is hidden.
#[test]
fn a_config_s_debug_form_shows_no_secret_and_no_filled_value() {
    let config = filled(
        "
model: { api_key: '${KEY}', name: '${MODEL}', base_url: 'https://u:${P}@gw.example/v1' }
tools: { builtin: { root: work, enabled: [bash], env: { T: '${TOK}' }, credentials: { C: '${KEY}' } } }
telemetry: { otlp: { headers: { Authorization: 'Bearer ${KEY}' } } }
",
        &[
            ("KEY", KEY),
            ("MODEL", "model-0123456789"),
            ("P", "pw-0123456789abcdef"),
            ("TOK", "tok-0123456789abcdef"),
        ],
    );

    let shown = format!("{config:?}");

    for hidden in [
        KEY,
        "model-0123456789",
        "pw-0123456789abcdef",
        "tok-0123456789abcdef",
    ] {
        assert!(!shown.contains(hidden), "{hidden}: {shown}");
    }
    assert!(shown.contains("${MODEL}"), "{shown}");
    assert!(shown.contains("${TOK}"), "{shown}");
    assert!(shown.contains("${KEY}"), "{shown}");
    let code = Config {
        model: Model {
            api_key: Some(Secret::new(KEY)),
            base_url: Some("https://u:pw-0123456789abcdef@gw.example/v1".to_owned()),
            ..Model::default()
        },
        ..Config::default()
    };
    let shown = format!("{code:?}");
    assert!(
        !shown.contains(KEY) && !shown.contains("pw-0123"),
        "{shown}"
    );
    assert!(shown.contains("[secret]"), "{shown}");
    assert!(code.made_in_code());
    assert!(!config.made_in_code());
}

/// C21: the config takes the specification's spellings alone.
#[test]
fn protocol_http_is_refused_naming_the_three_values() {
    let refused = Config::from_str("telemetry: { otlp: { protocol: http } }", Format::Yaml);

    assert_eq!(
        refused.unwrap_err().to_string(),
        "telemetry.otlp.protocol (line 1): \"http\" is refused: the accepted values are `grpc`, \
         `http/protobuf`, `http/json`"
    );
    for protocol in ["grpc", "http/protobuf", "http/json"] {
        yaml(&format!(
            "telemetry: {{ otlp: {{ protocol: {protocol} }} }}"
        ));
    }
}

/// A `RawConfig`'s `Debug` form is masked as a config's is: the fill
/// wrote what each variable holds into its tree, and the form shows each
/// filled setting as the file wrote it. A credential's value, the key, a
/// `credentials` value and a header text among them, is the marker before
/// the fill and after it, since the raw tree types none as a secret, and a
/// text may write one out.
#[test]
fn a_filled_raw_config_s_debug_form_shows_no_filled_value() {
    let text = "model: { api_key: '${KEY}', name: 'm-${MODEL}', base_url: 'https://${U}:${P}@gw.example/v1' }\ntelemetry: { resource: 'tok=${TOK}' }\n";
    let held = [
        ("KEY", KEY),
        ("MODEL", "model-0123456789"),
        ("U", "user-0123456789ab"),
        ("P", "pw-0123456789abcdef"),
        ("TOK", "tok-0123456789abcdef"),
    ];
    let lookup = |asked: &str| {
        held.iter()
            .find(|(name, _)| *name == asked)
            .map(|(_, value)| std::ffi::OsString::from(value))
    };
    let raw = RawConfig::from_str(text, Format::Yaml)
        .unwrap()
        .substituted(lookup)
        .unwrap();

    let shown = format!("{raw:?}");

    for (_, hidden) in held {
        assert!(!shown.contains(hidden), "{hidden}: {shown}");
    }
    assert!(
        shown.contains("m-${MODEL}") && shown.contains(Secret::MARKER),
        "{shown}"
    );

    let literals = [
        "sk-ant-literal0123456789abcdef",
        "ghp_literal0123456789abcdef",
        "literalheadertoken0123456789",
    ];
    let written = format!(
        "model: {{ api_key: '{}' }}\ntools: {{ builtin: {{ credentials: {{ GH: '{}' }} }} }}\ntelemetry: {{ otlp: {{ headers: 'authorization=Bearer%20{},x=${{TOK}}' }} }}\n",
        literals[0], literals[1], literals[2]
    );
    let raw = RawConfig::from_str(&written, Format::Yaml).unwrap();
    let before = format!("{raw:?}");
    let after = format!("{:?}", raw.substituted(lookup).unwrap());
    for shown in [before, after] {
        for literal in literals {
            assert!(!shown.contains(literal), "{literal}: {shown}");
        }
        assert!(shown.contains(Secret::MARKER), "{shown}");
    }
}

/// A URL set in code with user information serialises, as `--resolved`
/// and a host's own serialisation write it, with that information as the
/// marker a secret set in code is written as, as the key is; a URL the
/// fill gave is written back as the file wrote it.
#[test]
fn a_url_s_user_information_set_in_code_serialises_as_the_marker() {
    let mut config = Config::default();
    config.model.base_url = Some("https://u:pw-0123456789abcdef@gw.example/v1".to_owned());
    config.telemetry.otlp.endpoint = Some("http://u:pw-0123456789abcdef@127.0.0.1:4318".to_owned());

    let written = serde_json::to_string(&config.resolved()).unwrap();

    assert!(!written.contains("pw-0123"), "{written}");
    assert!(
        written.contains("https://[secret]@gw.example/v1")
            && written.contains("http://[secret]@127.0.0.1:4318"),
        "{written}"
    );
    let mut plain = config.clone();
    plain.model.base_url = Some("https://gw.example/v1".to_owned());
    plain.telemetry.otlp.endpoint = None;
    config.telemetry.otlp.endpoint = None;
    assert_eq!(config.digest(), plain.digest());
}

/// A value written as one `${VAR}` whole that comes out empty isn't
/// stated, whatever the setting takes: a number, a duration and a list's
/// item are left out as an optional setting is, and the setting keeps its
/// default; the resolved form writes it back as the file wrote it.
#[test]
fn a_whole_reference_that_comes_out_empty_is_unstated_in_every_setting() {
    let config = filled(
        "model: { max_tokens: '${MT:-}' }\nrun: { max_retries: '${R:-}', timeout: '${T:-}', max_turns: '${M:-}' }\ntools: { allow: [read_file, '${E:-}'] }\n",
        &[],
    );
    let default = Config::default();

    assert_eq!(config.model.max_tokens, default.model.max_tokens);
    assert_eq!(config.run.max_retries, default.run.max_retries);
    assert_eq!(config.run.timeout, default.run.timeout);
    assert_eq!(config.run.max_turns, None);
    assert_eq!(config.tools.allow, Some(vec!["read_file".to_owned()]));
    let resolved = serde_json::to_value(config.resolved()).unwrap();
    assert_eq!(resolved["model"]["max_tokens"], json!("${MT:-}"));
    assert_eq!(resolved["run"]["timeout"], json!("${T:-}"));
}

/// R55 for a variable that holds the marker names the variable, whose fix
/// is to hold the secret; written in the text, the fix is `Secret::new`.
#[test]
fn the_marker_a_variable_holds_is_refused_naming_the_variable() {
    let refusal = RawConfig::from_str("model: { api_key: '${K}' }", Format::Yaml)
        .unwrap()
        .substituted(|_: &str| Some(OsString::from(Secret::MARKER)))
        .unwrap()
        .config()
        .unwrap_err()
        .to_string();
    assert_eq!(
        refusal,
        "model.api_key (line 1): \"${K}\" is refused: `K` holds the marker lablet writes a secret \
         set in code as; set it to the secret itself"
    );
    let written = Config::from_str("model: { api_key: '[secret]' }", Format::Yaml)
        .unwrap_err()
        .to_string();
    assert!(written.ends_with("set it with Secret::new"), "{written}");
}

/// A list or a text of no propagator is refused with the fix, `none`, which
/// names none and says so.
#[test]
fn propagators_that_name_none_are_refused_with_the_fix() {
    for written in ["[]", "''", "' , '"] {
        let refusal = Config::from_str(
            &format!("telemetry: {{ propagators: {written} }}"),
            Format::Yaml,
        )
        .unwrap_err()
        .to_string();
        assert!(
            refusal.ends_with("is refused: it names no propagator; write `none` to name none"),
            "{written}: {refusal}"
        );
    }
    let none = Config::from_str("telemetry: { propagators: none }", Format::Yaml).unwrap();
    assert_eq!(none.telemetry.propagators, Some(Propagators::none()));
}
