use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use lablet_model::{CacheScope, Effort, Thinking};

use super::*;
use crate::config::Format;

const FAKE: &str = "
model: { provider: fake, script: scripts/run.yaml, name: scripted-1 }
prompt: { system: You fix tests. }
";

fn of(text: &str) -> Result<Settings, ConfigError> {
    Settings::of(&Config::from_str(text, Format::Yaml).unwrap())
}

/// The settings of the fake model's config with `more` stated too.
fn with(more: &str) -> Result<Settings, ConfigError> {
    of(&format!("{FAKE}{more}"))
}

fn refused(key: &str, value: &str, reason: &str) -> ConfigError {
    ConfigError::Invalid {
        key: key.to_owned(),
        value: value.to_owned(),
        reason: reason.to_owned(),
    }
}

fn name(tool: &str) -> ToolName {
    ToolName::new(tool).unwrap()
}

#[test]
fn a_config_that_states_what_it_must_comes_to_the_defaults() {
    let settings = with("").unwrap();

    assert_eq!(
        settings,
        Settings {
            stop: StopPolicy {
                max_turns: None,
                timeout: Duration::from_secs(600),
                max_total_tokens: None,
                max_consecutive_invalid_turns: NonZeroU32::new(3),
            },
            retry: RetryPolicy::new(RetrySettings {
                max_retries: 10,
                base: Duration::from_millis(500),
                max: Duration::from_secs(32),
                factor: 2.0,
                hint_max: Duration::from_secs(60),
                jitter: 0.25,
            })
            .unwrap(),
            provider: Selected::Fake {
                script: "scripts/run.yaml".into()
            },
            request: RequestParams {
                max_tokens: 32_000,
                temperature: None,
                thinking: Thinking::ProviderDefault,
                effort: None,
                seed: None,
                cache_scope: CacheScope::Shared,
            },
            pricing: None,
            calls: CallLimits {
                provider_timeout: Duration::from_secs(600),
                output_cap: Some(
                    OutputCap::new(50_000, OutputCut::Preview { bytes: 2_000 }).unwrap()
                ),
                max_concurrent_tool_calls: NonZeroU32::new(10).unwrap(),
            },
            completion: CompletionMode::Natural,
            filter: ToolFilter::default(),
            system: System::Text("You fix tests.".to_owned()),
            builtin: None,
        }
    );
}

#[test]
fn every_setting_the_loop_takes_is_the_one_the_config_states() {
    let settings = with(
        "
run:
  completion: explicit
  max_turns: 7
  timeout: 90s
  max_total_tokens: 100000
  max_retries: 2
  retry_backoff_base: 1s
  retry_backoff_max: 4s
  retry_jitter: 0.5
  retry_hint_max: 20s
  max_consecutive_invalid_turns: null
  provider_timeout: 30s
tools:
  builtin: { root: work, enabled: [write_file, bash], timeout: 5s, env: { CI: '1' } }
  allow: [bash, write_file]
  deny: [write_file]
  max_concurrent_calls: 3
  max_output_bytes: 1000
  output_cut: head_tail
",
    )
    .unwrap();

    assert_eq!(
        settings.stop,
        StopPolicy {
            max_turns: NonZeroU32::new(7),
            timeout: Duration::from_secs(90),
            max_total_tokens: Some(100_000),
            max_consecutive_invalid_turns: None,
        }
    );
    assert_eq!(
        settings.retry,
        RetryPolicy::new(RetrySettings {
            max_retries: 2,
            base: Duration::from_secs(1),
            max: Duration::from_secs(4),
            factor: 2.0,
            hint_max: Duration::from_secs(20),
            jitter: 0.5,
        })
        .unwrap()
    );
    assert_eq!(
        settings.calls,
        CallLimits {
            provider_timeout: Duration::from_secs(30),
            output_cap: Some(OutputCap::new(1_000, OutputCut::HeadTail).unwrap()),
            max_concurrent_tool_calls: NonZeroU32::new(3).unwrap(),
        }
    );
    assert_eq!(settings.completion, CompletionMode::Explicit);
    assert_eq!(
        settings.filter,
        ToolFilter {
            allow: vec![name("bash"), name("write_file")],
            deny: vec![name("write_file")],
        }
    );
    assert_eq!(
        settings.builtin,
        Some(lablet_tools_builtin::Settings {
            root: "work".into(),
            enabled: BTreeSet::from([Tool::Bash, Tool::WriteFile]),
            timeout: Duration::from_secs(5),
            env: BTreeMap::from([("CI".to_owned(), "1".to_owned())]),
        })
    );
}

#[test]
fn each_way_to_cut_an_output_is_the_loops_own_and_no_cap_is_none() {
    let cut = |more: &str| with(more).unwrap().calls.output_cap;

    assert_eq!(
        cut("tools: { output_cut: head, max_output_bytes: 10 }"),
        Some(OutputCap::new(10, OutputCut::Head).unwrap())
    );
    assert_eq!(
        cut("tools: { output_cut: preview, max_output_bytes: 10, output_preview_bytes: 4 }"),
        Some(OutputCap::new(10, OutputCut::Preview { bytes: 4 }).unwrap())
    );
    assert_eq!(
        cut("tools: { max_output_bytes: null, output_preview_bytes: 4 }"),
        None
    );
}

#[test]
fn the_request_parameters_are_the_resolved_models() {
    let anthropic = of("
model: { thinking: { budget: 2048 }, effort: max, temperature: 0.2, cache_scope: run, max_tokens: 8000 }
prompt: { system: Hi. }
")
    .unwrap();
    assert_eq!(anthropic.provider, Selected::Anthropic);
    assert_eq!(
        anthropic.request,
        RequestParams {
            max_tokens: 8_000,
            temperature: Some(0.2),
            thinking: Thinking::Budget(NonZeroU32::new(2_048).unwrap()),
            effort: Some(Effort::Max),
            seed: None,
            cache_scope: CacheScope::Run,
        }
    );

    let openai = of("
model: { provider: openai, seed: -7, effort: xhigh }
prompt: { system: Hi. }
")
    .unwrap();
    assert_eq!(openai.provider, Selected::Openai);
    assert_eq!(openai.request.seed, Some(-7));
    assert_eq!(openai.request.effort, Some(Effort::XHigh));
    assert_eq!(openai.request.thinking, Thinking::ProviderDefault);

    for (written, thinking) in [
        ("adaptive", Thinking::Adaptive),
        ("disabled", Thinking::Disabled),
        ("provider_default", Thinking::ProviderDefault),
    ] {
        let settings = of(&format!(
            "model: {{ thinking: {written} }}\nprompt: {{ system: Hi. }}"
        ))
        .unwrap();
        assert_eq!(settings.request.thinking, thinking);
    }
    for (written, effort) in [
        ("low", Effort::Low),
        ("medium", Effort::Medium),
        ("high", Effort::High),
    ] {
        let settings = of(&format!(
            "model: {{ effort: {written} }}\nprompt: {{ system: Hi. }}"
        ))
        .unwrap();
        assert_eq!(settings.request.effort, Some(effort));
    }
}

#[test]
fn pricing_is_the_rates_the_config_states() {
    let settings = of("
model:
  provider: fake
  script: run.yaml
  pricing: { input: 3, output: 15, cache_read: 0.3, cache_write: 3.75 }
prompt: { system: Hi. }
")
    .unwrap();

    assert_eq!(
        settings.pricing,
        Some(Pricing::new(Rates::new(3.0, 15.0, 0.3, 3.75).unwrap()))
    );
}

#[test]
fn a_rate_that_is_no_price_is_refused_by_its_key() {
    for (rates, key, value) in [
        (
            "input: -1, output: 15, cache_read: 0.3, cache_write: 3.75",
            "input",
            "-1",
        ),
        (
            "input: 3, output: .inf, cache_read: 0.3, cache_write: 3.75",
            "output",
            "inf",
        ),
        (
            "input: 3, output: 15, cache_read: .nan, cache_write: 3.75",
            "cache_read",
            "NaN",
        ),
        (
            "input: 3, output: 15, cache_read: 0.3, cache_write: -0.5",
            "cache_write",
            "-0.5",
        ),
    ] {
        assert_eq!(
            of(&format!(
                "model: {{ pricing: {{ {rates} }} }}\nprompt: {{ system: Hi. }}"
            )),
            Err(refused(
                &format!("model.pricing.{key}"),
                value,
                "a rate is a finite number of US dollars of at least 0"
            )),
            "{rates}"
        );
    }
}

#[test]
fn a_backoff_that_starts_above_its_cap_and_a_jitter_that_is_no_share_are_refused() {
    assert_eq!(
        with("run: { retry_backoff_base: 1m, retry_backoff_max: 32s }"),
        Err(refused(
            "run.retry_backoff_base",
            "1m",
            "a backoff starts no longer than `run.retry_backoff_max`, which is 32s"
        ))
    );
    assert!(with("run: { retry_backoff_base: 32s, retry_backoff_max: 32s }").is_ok());

    for (written, shown) in [("1.5", "1.5"), ("-0.1", "-0.1"), (".nan", "NaN")] {
        assert_eq!(
            with(&format!("run: {{ retry_jitter: {written} }}")),
            Err(refused(
                "run.retry_jitter",
                shown,
                "the jitter is a share of a wait, from 0 to 1"
            ))
        );
    }
    assert!(with("run: { retry_jitter: 1 }").is_ok());
}

fn not_applied(key: &'static str, value: &str, reached: &str) -> ConfigError {
    ConfigError::NotApplied {
        key,
        value: value.to_owned(),
        reached: reached.to_owned(),
    }
}

#[test]
fn a_setting_the_config_states_and_the_provider_cannot_apply_is_refused_by_name() {
    const ANTHROPIC: &str = "the provider `anthropic`";
    const FAKE: &str = "the provider `fake`";
    const RESPONSES: &str = "the provider `openai` over its API `responses`";
    const CHAT: &str = "the provider `openai` over its API `chat_completions`";

    for (model, key, value, reached) in [
        ("seed: 7", "model.seed", "7", ANTHROPIC),
        ("api: responses", "model.api", "responses", ANTHROPIC),
        ("script: run.yaml", "model.script", "run.yaml", ANTHROPIC),
        (
            "reasoning_replay: false",
            "model.reasoning_replay",
            "false",
            ANTHROPIC,
        ),
        (
            "provider: openai, thinking: adaptive",
            "model.thinking",
            "adaptive",
            RESPONSES,
        ),
        (
            "provider: openai, thinking: { budget: 2048 }",
            "model.thinking",
            "{ budget: 2048 }",
            RESPONSES,
        ),
        (
            "provider: openai, cache: true",
            "model.cache",
            "true",
            RESPONSES,
        ),
        (
            "provider: openai, script: run.yaml",
            "model.script",
            "run.yaml",
            RESPONSES,
        ),
        (
            "provider: openai, reasoning_replay: true",
            "model.reasoning_replay",
            "true",
            RESPONSES,
        ),
        (
            "provider: openai, api: chat_completions, effort: high",
            "model.effort",
            "high",
            CHAT,
        ),
        (
            "provider: openai, base_url: 'http://localhost:11434/v1', effort: low",
            "model.effort",
            "low",
            CHAT,
        ),
        (
            "provider: fake, script: run.yaml, cache: false",
            "model.cache",
            "false",
            FAKE,
        ),
        (
            "provider: fake, script: run.yaml, seed: 7",
            "model.seed",
            "7",
            FAKE,
        ),
        (
            "provider: fake, script: run.yaml, thinking: provider_default",
            "model.thinking",
            "provider_default",
            FAKE,
        ),
        (
            "provider: fake, script: run.yaml, effort: max",
            "model.effort",
            "max",
            FAKE,
        ),
        (
            "provider: fake, script: run.yaml, api: chat_completions",
            "model.api",
            "chat_completions",
            FAKE,
        ),
        (
            "provider: fake, script: run.yaml, base_url: 'http://localhost:4000'",
            "model.base_url",
            "http://localhost:4000",
            FAKE,
        ),
    ] {
        assert_eq!(
            of(&format!("model: {{ {model} }}\nprompt: {{ system: Hi. }}")),
            Err(not_applied(key, value, reached)),
            "{model}"
        );
    }
}

#[test]
fn a_setting_the_provider_applies_is_taken_stated_or_not() {
    for model in [
        "thinking: adaptive, effort: high, cache: false",
        "api_key_env: WORK_KEY, base_url: 'https://gateway.example', temperature: 0",
        "provider: openai, api: responses, seed: 7, effort: high",
        "provider: openai, api: chat_completions, seed: 7, reasoning_replay: true",
        "provider: fake, script: run.yaml, api_key_env: WORK_KEY, cache_scope: run, temperature: 1",
    ] {
        let settings = of(&format!("model: {{ {model} }}\nprompt: {{ system: Hi. }}"));

        assert!(settings.is_ok(), "{model}: {settings:?}");
    }
}

#[test]
fn a_key_written_where_its_variable_is_named_is_refused_and_never_shown() {
    for written in [
        "sk-ant-api03-0123456789abcdef",
        "'sk ant'",
        "'0123456789abcdef'",
        "'$ANTHROPIC_API_KEY'",
        "'${ANTHROPIC_API_KEY}'",
        "'ANTHROPIC_API_KEY=sk'",
        "cl\u{e9}",
        "''",
    ] {
        for provider in ["anthropic", "openai", "fake, script: run.yaml"] {
            let config = format!(
                "model: {{ provider: {provider}, api_key_env: {written} }}\nprompt: {{ system: Hi. }}"
            );

            let error = of(&config).unwrap_err();

            assert!(
                matches!(error, ConfigError::KeyVariable { .. }),
                "{config}: {error:?}"
            );
            let value = written.trim_matches('\'');
            for shown in [error.to_string(), format!("{error:?}")] {
                assert!(
                    value.is_empty() || !shown.contains(value),
                    "{config}: {shown}"
                );
            }
            assert_eq!(
                error.to_string(),
                "model.api_key_env is refused: it holds something other than the name of an \
                 environment variable, which is ASCII letters, digits and `_` and begins \
                 with no digit. What it holds isn't shown, since a key may have been \
                 written in its place"
            );
        }
    }
}

#[test]
fn the_name_of_a_variable_is_taken_as_the_key_s_variable() {
    for named in ["ANTHROPIC_API_KEY", "_key", "work_key_2", "K"] {
        let settings = of(&format!(
            "model: {{ api_key_env: {named} }}\nprompt: {{ system: Hi. }}"
        ));

        assert!(settings.is_ok(), "{named}: {settings:?}");
    }
}

#[test]
fn a_fake_model_needs_its_script() {
    assert_eq!(
        of("model: { provider: fake }\nprompt: { system: Hi. }"),
        Err(ConfigError::Missing {
            key: "model.script",
            reason: "the provider `fake` plays the script it names".to_owned(),
        })
    );
}

#[test]
fn a_temperature_is_a_number_and_a_thinking_budget_is_below_the_cap_on_output() {
    for (written, shown) in [(".nan", "NaN"), (".inf", "inf")] {
        assert_eq!(
            of(&format!(
                "model: {{ temperature: {written} }}\nprompt: {{ system: Hi. }}"
            )),
            Err(refused(
                "model.temperature",
                shown,
                "a temperature is a finite number"
            ))
        );
    }

    assert_eq!(
        of("model: { thinking: { budget: 4096 }, max_tokens: 4096 }\nprompt: { system: Hi. }"),
        Err(refused(
            "model.thinking",
            "{ budget: 4096 }",
            "a budget is fewer tokens than `model.max_tokens`, which is 4096"
        ))
    );
    assert!(
        of("model: { thinking: { budget: 4095 }, max_tokens: 4096 }\nprompt: { system: Hi. }")
            .is_ok()
    );
}

#[test]
fn a_config_states_one_system_prompt() {
    const MODEL: &str = "model: { provider: fake, script: run.yaml }\n";

    assert_eq!(
        of(&format!(
            "{MODEL}prompt: {{ system_file: prompts/system.md }}"
        ))
        .unwrap()
        .system,
        System::File("prompts/system.md".into())
    );
    assert_eq!(
        of(&format!("{MODEL}prompt: {{ system: '' }}"))
            .unwrap()
            .system,
        System::Text(String::new()),
        "a run may have no system prompt, which the config says by stating an empty one"
    );
    assert_eq!(
        of(&format!(
            "{MODEL}prompt: {{ system: Hi., system_file: prompts/system.md }}"
        )),
        Err(refused(
            "prompt.system_file",
            "prompts/system.md",
            "`prompt.system` is stated too, and a config states one of the two"
        ))
    );
    for nothing in ["", "prompt: { skills_mode: inline }"] {
        assert_eq!(
            of(&format!("{MODEL}{nothing}")),
            Err(ConfigError::Missing {
                key: "prompt.system",
                reason: "a config states `prompt.system` or `prompt.system_file`".to_owned(),
            })
        );
    }
}

#[test]
fn a_preview_longer_than_the_cap_is_refused_and_one_as_long_is_taken() {
    assert_eq!(
        with("tools: { max_output_bytes: 1000, output_preview_bytes: 1001 }"),
        Err(refused(
            "tools.output_preview_bytes",
            "1001",
            "a preview is no longer than `tools.max_output_bytes`, which is 1000"
        ))
    );
    assert_eq!(
        with("tools: { max_output_bytes: 1000 }"),
        Err(refused(
            "tools.output_preview_bytes",
            "2000",
            "a preview is no longer than `tools.max_output_bytes`, which is 1000"
        )),
        "the default preview is a preview like any other"
    );
    assert!(with("tools: { max_output_bytes: 1000, output_preview_bytes: 1000 }").is_ok());
    assert!(with("tools: { max_output_bytes: 1000, output_cut: head }").is_ok());
}

#[test]
fn a_list_of_tools_names_tools_and_an_allow_list_names_at_least_one() {
    for (list, written, shown, said) in [
        (
            "allow",
            "[bash, 'read file']",
            "\"read file\"",
            "has a character other than",
        ),
        ("deny", "['']", "\"\"", "tool name is empty"),
        (
            "deny",
            "[' bash']",
            "\" bash\"",
            "leading or trailing whitespace",
        ),
    ] {
        match with(&format!("tools: {{ {list}: {written} }}")) {
            Err(ConfigError::Invalid { key, value, reason }) => {
                assert_eq!(key, format!("tools.{list}"));
                assert_eq!(value, shown);
                assert!(reason.contains(said), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    assert_eq!(
        with("tools: { allow: [] }"),
        Err(refused(
            "tools.allow",
            "[]",
            "a list that names no tool would offer none; leave the list out to offer every tool, \
             and enable no tool to offer none"
        ))
    );
    assert_eq!(
        with("tools: { allow: null, deny: [] }").unwrap().filter,
        ToolFilter::default()
    );
}

#[test]
fn a_built_in_tool_needs_a_root_and_a_root_alone_serves_no_tool() {
    for (enabled, first) in [
        ("[bash]", "bash"),
        ("[write_file, read_file]", "read_file"),
        ("[write_file]", "write_file"),
    ] {
        assert_eq!(
            with(&format!("tools: {{ builtin: {{ enabled: {enabled} }} }}")),
            Err(ConfigError::Missing {
                key: "tools.builtin.root",
                reason: format!(
                    "`tools.builtin.enabled` holds `{first}`, and a built-in tool works under \
                     the root"
                ),
            })
        );
    }

    assert_eq!(
        with("tools: { builtin: { root: work } }").unwrap().builtin,
        None
    );
    let every = with("tools: { builtin: { root: work, enabled: [bash, read_file, write_file] } }")
        .unwrap()
        .builtin
        .unwrap();
    assert_eq!(every.enabled, BTreeSet::from(Tool::ALL));
}

#[test]
fn an_mcp_server_has_a_name_a_tool_name_can_carry() {
    let server = |name: &str| {
        with(&format!(
            "tools: {{ mcp: [{{ name: {name}, transport: stdio, command: npx }}] }}"
        ))
    };

    for (written, shown, said) in [
        ("''", "\"\"", "a server has a name"),
        (
            "'my docs'",
            "\"my docs\"",
            "a server's name holds ASCII letters, digits, `-` and `_`",
        ),
        (
            "docs.v2",
            "\"docs.v2\"",
            "a server's name holds ASCII letters, digits, `-` and `_`",
        ),
        (
            "docs__v2",
            "\"docs__v2\"",
            "a server's name holds no `__`, which is what parts a tool's name from its server's",
        ),
    ] {
        assert_eq!(
            server(written),
            Err(refused("tools.mcp.name", shown, said)),
            "{written}"
        );
    }
    for name in ["docs", "docs-v2", "docs_v2", "Docs2"] {
        assert!(server(name).is_ok(), "{name}");
    }
}

#[test]
fn the_first_setting_refused_is_the_first_in_the_order_of_the_sections() {
    let refusal = |text: &str| match of(text) {
        Err(ConfigError::Invalid { key, .. }) => key,
        Err(ConfigError::Missing { key, .. } | ConfigError::NotApplied { key, .. }) => {
            key.to_owned()
        }
        other => panic!("{other:?}"),
    };

    let everything_wrong = "
run: { retry_jitter: 2 }
model: { provider: fake, seed: 7 }
tools: { allow: [], builtin: { enabled: [bash] } }
";
    assert_eq!(refusal(everything_wrong), "run.retry_jitter");
    assert_eq!(
        refusal("model: { provider: fake, seed: 7 }\ntools: { allow: [] }"),
        "model.script"
    );
    assert_eq!(
        refusal("model: { provider: fake, script: run.yaml, seed: 7 }\ntools: { allow: [] }"),
        "model.seed"
    );
    assert_eq!(
        refusal("model: { provider: fake, script: run.yaml }\ntools: { allow: [] }"),
        "prompt.system"
    );
    assert_eq!(
        refusal(&format!(
            "{FAKE}tools: {{ allow: [], builtin: {{ enabled: [bash] }} }}"
        )),
        "tools.allow"
    );
}

#[test]
fn a_refusal_names_its_key_and_the_value_that_was_refused() {
    assert_eq!(
        refused(
            "run.retry_jitter",
            "2",
            "the jitter is a share of a wait, from 0 to 1"
        )
        .to_string(),
        "run.retry_jitter: 2 is refused: the jitter is a share of a wait, from 0 to 1"
    );
    assert_eq!(
        not_applied("model.seed", "7", "the provider `anthropic`").to_string(),
        "model.seed: 7 is refused: the provider `anthropic` can't apply it"
    );
    assert_eq!(
        ConfigError::Missing {
            key: "model.script",
            reason: "the provider `fake` plays the script it names".to_owned(),
        }
        .to_string(),
        "model.script isn't set: the provider `fake` plays the script it names"
    );
}
