use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;
use std::time::Duration;

use lablet_test_support::Scratch;
use serde_json::json;

use super::*;

/// The config of spec §7, comments and all. This file is the source of its
/// defaults, and the spec's block quotes it: a test never reads
/// `product/spec.md`, which can drift.
const SPEC: &str = include_str!("tests/spec.yaml");

fn yaml(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
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
            api_key_env: None,
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
            capture_content: false,
            otlp: Otlp {
                endpoint: None,
                protocol: OtlpProtocol::Grpc,
                headers: BTreeMap::new(),
            },
            file: TelemetryFile { path: None },
            resource: BTreeMap::new(),
        },
        source: None,
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
/// servers and a system prompt as examples.
#[test]
fn the_config_the_spec_writes_is_read_whole_and_its_values_are_the_defaults() {
    let config = yaml(SPEC);

    let mut expected = defaults();
    expected.model.api_key_env = Some("ANTHROPIC_API_KEY".to_owned());
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
    assert_eq!(config, expected);
    assert_eq!(config.tools.mcp[0].name(), "docs");
    assert_eq!(config.tools.mcp[1].name(), "search");
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

#[test]
fn a_key_the_config_does_not_know_is_refused_by_name_in_every_section() {
    for (text, key) in [
        ("runs: {}", "runs"),
        ("run: { max_turn: 3 }", "max_turn"),
        (
            "run: { context: { mask: { trigger_tokens: 1, keep_last: 1, keep: 2 } } }",
            "keep",
        ),
        ("model: { nme: scripted-1 }", "nme"),
        (
            "model: { pricing: { input: 1, output: 1, cache_read: 1, cache_write: 1, cached: 1 } }",
            "cached",
        ),
        ("prompt: { sytem: Hi. }", "sytem"),
        ("tools: { alow: [bash] }", "alow"),
        ("tools: { builtin: { roots: work } }", "roots"),
        (
            "tools: { mcp: [{ name: docs, transport: stdio, command: npx, colour: red }] }",
            "colour",
        ),
        (
            "tools: { mcp: [{ name: docs, transport: http, url: 'http://localhost/mcp', command: npx }] }",
            "command",
        ),
        ("telemetry: { capture: true }", "capture"),
        ("telemetry: { otlp: { endpoints: [] } }", "endpoints"),
        ("telemetry: { file: { paths: [] } }", "paths"),
    ] {
        let reason = refusal(text, Format::Yaml);

        assert!(
            reason.contains(&format!("unknown field `{key}`")),
            "{text}: {reason}"
        );
    }
    let reason = refusal(r#"{"run": {"max_turn": 3}}"#, Format::Json);
    assert!(reason.contains("unknown field `max_turn`"), "{reason}");
}

#[test]
fn a_value_no_setting_takes_is_refused_with_the_values_it_takes() {
    for (text, said) in [
        (
            "run: { completion: sometimes }",
            "unknown variant `sometimes`, expected one of natural, explicit",
        ),
        (
            "model: { provider: gemini }",
            "unknown variant `gemini`, expected one of anthropic, openai, fake",
        ),
        (
            "model: { effort: extreme }",
            "unknown variant `extreme`, expected one of low, medium, high, xhigh, max",
        ),
        (
            "tools: { builtin: { enabled: [task_complete] } }",
            "unknown variant `task_complete`, expected one of bash, read_file, write_file",
        ),
        (
            "tools: { output_cut: tail }",
            "unknown variant `tail`, expected one of preview, head, head_tail",
        ),
        (
            "tools: { mcp: [{ name: docs, transport: pipe }] }",
            "unknown variant `pipe`, expected one of stdio, http",
        ),
        ("run: { max_turns: 0 }", "expected a nonzero u32"),
        (
            "tools: { max_concurrent_calls: 0 }",
            "expected a nonzero u32",
        ),
        (
            "model: { thinking: { budget: 0 } }",
            "expected a nonzero u32",
        ),
        ("run: { max_retries: -1 }", "invalid u32"),
    ] {
        let reason = refusal(text, Format::Yaml);

        assert!(reason.contains(said), "{text}: {reason}");
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

    for written in ["10", "ten minutes", "-5s"] {
        let reason = refusal(&format!("run: {{ timeout: {written} }}"), Format::Yaml);

        assert!(
            reason.contains(&format!("{written:?} isn't a duration")),
            "{reason}"
        );
    }
    let reason = refusal(r#"{"run": {"timeout": 10}}"#, Format::Json);
    assert!(reason.contains("expected a string"), "{reason}");
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
        r#"{"run": {"timeout": "1m", "timeout": "2m"}}"#,
        Format::Json,
    );
    assert!(reason.contains("duplicate field `timeout`"), "{reason}");
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
        Err(ConfigError::Syntax {
            format: Format::Yaml,
            ..
        })
    ));
}
