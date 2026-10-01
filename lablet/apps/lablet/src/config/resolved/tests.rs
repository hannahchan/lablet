use lablet_model::ConfigDigest;
use serde_json::{Value, json};

use super::*;
use crate::config::{Config, Format};

fn yaml(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

fn resolved_model(text: &str) -> Value {
    serde_json::to_value(yaml(text).resolved().model).unwrap()
}

#[test]
fn a_fake_model_resolves_to_what_the_fake_provider_applies() {
    assert_eq!(
        resolved_model("model: { provider: fake, script: scripts/run.yaml, name: scripted-1 }"),
        json!({
            "provider": "fake",
            "script": "scripts/run.yaml",
            "name": "scripted-1",
            "api_key_env": null,
            "max_tokens": 32_000,
            "temperature": null,
            "cache_scope": "shared",
            "pricing": null,
        })
    );
}

#[test]
fn an_anthropic_model_resolves_with_the_defaults_that_are_its_own() {
    assert_eq!(
        resolved_model(""),
        json!({
            "provider": "anthropic",
            "name": "claude-sonnet-5",
            "api_key_env": "ANTHROPIC_API_KEY",
            "base_url": null,
            "max_tokens": 32_000,
            "temperature": null,
            "thinking": "provider_default",
            "effort": null,
            "cache": true,
            "cache_scope": "shared",
            "pricing": null,
        })
    );
}

#[test]
fn an_openai_model_resolves_to_the_api_it_is_reached_through_and_what_that_api_applies() {
    assert_eq!(
        resolved_model("model: { provider: openai, name: gpt-5, seed: 7 }"),
        json!({
            "provider": "openai",
            "api": "responses",
            "name": "gpt-5",
            "api_key_env": null,
            "base_url": null,
            "max_tokens": 32_000,
            "temperature": null,
            "effort": null,
            "seed": 7,
            "cache_scope": "shared",
            "pricing": null,
        })
    );
    assert_eq!(
        resolved_model(
            "model: { provider: openai, name: qwen3, base_url: 'http://localhost:11434/v1' }"
        ),
        json!({
            "provider": "openai",
            "api": "chat_completions",
            "name": "qwen3",
            "api_key_env": null,
            "base_url": "http://localhost:11434/v1",
            "max_tokens": 32_000,
            "temperature": null,
            "seed": null,
            "reasoning_replay": false,
            "cache_scope": "shared",
            "pricing": null,
        })
    );
    assert_eq!(
        resolved_model(
            "model: { provider: openai, api: responses, base_url: 'http://localhost:4000' }"
        )["api"],
        json!("responses"),
        "the API the config states is the API, with a base URL or without"
    );
}

#[test]
fn a_resolved_config_holds_what_was_stated_in_place_of_a_default() {
    let model = resolved_model(
        "
model:
  api_key_env: WORK_KEY
  thinking: { budget: 2048 }
  effort: high
  cache: false
  cache_scope: run
  temperature: 0.7
  pricing: { input: 3, output: 15, cache_read: 0.3, cache_write: 3.75 }
",
    );

    assert_eq!(model["api_key_env"], json!("WORK_KEY"));
    assert_eq!(model["thinking"], json!({ "budget": 2048 }));
    assert_eq!(model["effort"], json!("high"));
    assert_eq!(model["cache"], json!(false));
    assert_eq!(model["cache_scope"], json!("run"));
    assert_eq!(model["temperature"], json!(0.7));
    assert_eq!(
        model["pricing"],
        json!({ "input": 3.0, "output": 15.0, "cache_read": 0.3, "cache_write": 3.75 })
    );
}

#[test]
fn a_setting_the_provider_cannot_apply_is_left_out_even_when_it_was_stated() {
    let model = resolved_model(
        "model: { provider: fake, script: run.yaml, thinking: adaptive, seed: 7, cache: true }",
    );

    for key in [
        "thinking",
        "seed",
        "cache",
        "effort",
        "api",
        "reasoning_replay",
    ] {
        assert_eq!(model.get(key), None, "{key}");
    }
}

#[test]
fn the_sections_every_provider_shares_resolve_to_what_they_hold_with_durations_written_one_way() {
    let resolved = serde_json::to_value(
        yaml(
            "
run: { timeout: 600s, transcript_path: 'out/{run_id}.json' }
prompt: { system_file: prompts/system.md }
tools: { builtin: { root: work, enabled: [write_file, bash] } }
telemetry: { resource: { team: evals } }
",
        )
        .resolved(),
    )
    .unwrap();

    assert_eq!(
        resolved["run"],
        json!({
            "completion": "natural",
            "max_turns": null,
            "timeout": "10m",
            "max_total_tokens": null,
            "max_retries": 10,
            "retry_backoff_base": "500ms",
            "retry_backoff_max": "32s",
            "retry_jitter": 0.25,
            "retry_hint_max": "1m",
            "max_consecutive_invalid_turns": 3,
            "provider_timeout": "10m",
            "context": "full",
            "transcript_path": "out/{run_id}.json",
            "transcript_format": "json",
            "completion_schema": null,
        })
    );
    assert_eq!(
        resolved["prompt"],
        json!({
            "system": null,
            "system_file": "prompts/system.md",
            "skills": [],
            "skills_mode": "tool",
        })
    );
    assert_eq!(
        resolved["tools"],
        json!({
            "builtin": {
                "root": "work",
                "enabled": ["bash", "write_file"],
                "timeout": "2m",
                "env": {},
            },
            "mcp": [],
            "allow": null,
            "deny": [],
            "max_concurrent_calls": 10,
            "max_output_bytes": 50_000,
            "output_cut": "preview",
            "output_preview_bytes": 2_000,
            "max_description_chars": 2_048,
        })
    );
    assert_eq!(
        resolved["telemetry"],
        json!({
            "capture_content": false,
            "otlp": { "endpoint": null, "protocol": "grpc", "headers": {} },
            "file": { "path": null },
            "resource": { "team": "evals" },
        })
    );
}

#[cfg(unix)]
#[test]
fn a_path_that_is_not_text_is_resolved_as_it_is_shown() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let mut config = yaml("model: { provider: fake }");
    config.model.script = Some(OsStr::from_bytes(b"scripts/\xff.yaml").into());
    config.run.transcript_path = Some(OsStr::from_bytes(b"out/\xff.json").into());
    config.prompt.skills = vec![OsStr::from_bytes(b"skills/\xff/SKILL.md").into()];

    let resolved = serde_json::to_value(config.resolved()).unwrap();

    assert_eq!(resolved["model"]["script"], json!("scripts/\u{fffd}.yaml"));
    assert_eq!(
        resolved["run"]["transcript_path"],
        json!("out/\u{fffd}.json")
    );
    assert_eq!(
        resolved["prompt"]["skills"],
        json!(["skills/\u{fffd}/SKILL.md"])
    );
    assert_eq!(config.digest().as_str().len(), 64);
}

const FAKE: &str = "
model: { provider: fake, script: scripts/run.yaml, name: scripted-1 }
prompt: { system: You fix tests. }
";

fn digest(more: &str) -> ConfigDigest {
    yaml(&format!("{FAKE}{more}")).digest()
}

/// What the digest of [`FAKE`] is taken of: the settings that say what a
/// run does, each default among them, and nothing that says where its
/// output goes. A default that changes, or a key that does, changes the
/// digest of every config, which a release has to say.
const FAKE_CANONICAL: &str = concat!(
    r#"{"model":{"api_key_env":null,"cache_scope":"shared","max_tokens":32000,"#,
    r#""name":"scripted-1","pricing":null,"provider":"fake","script":"scripts/run.yaml","#,
    r#""temperature":null},"#,
    r#""prompt":{"skills":[],"skills_mode":"tool","system":"You fix tests.","system_file":null},"#,
    r#""run":{"completion":"natural","completion_schema":null,"context":"full","#,
    r#""max_consecutive_invalid_turns":3,"max_retries":10,"max_total_tokens":null,"#,
    r#""max_turns":null,"provider_timeout":"10m","retry_backoff_base":"500ms","#,
    r#""retry_backoff_max":"32s","retry_hint_max":"1m","retry_jitter":0.25,"timeout":"10m"},"#,
    r#""tools":{"allow":null,"builtin":{"enabled":[]},"deny":[],"max_concurrent_calls":10,"#,
    r#""max_description_chars":2048,"max_output_bytes":50000,"mcp":[],"output_cut":"preview","#,
    r#""output_preview_bytes":2000}}"#,
);

#[test]
fn the_digest_is_of_the_settings_that_say_what_a_run_does_with_every_default_filled_in() {
    let expected = Sha256::digest(FAKE_CANONICAL)
        .iter()
        .fold(String::new(), |hex, byte| format!("{hex}{byte:02x}"));

    assert_eq!(digest("").as_str(), expected);
    assert_eq!(
        expected,
        "e6697aa9d6abe9e6e64979c8df1665a3064073b72d7ce5495685245563756eb2"
    );
}

#[test]
fn two_configs_that_differ_only_in_where_they_write_share_a_digest() {
    let plain = digest("");

    for output in [
        "run: { transcript_path: 'out/{run_id}.json' }",
        "run: { transcript_format: atif }",
        "telemetry: { capture_content: true }",
        "telemetry: { file: { path: out/telemetry.jsonl } }",
        "telemetry: { otlp: { endpoint: 'http://localhost:4317', protocol: http, headers: { a: b } } }",
        "telemetry: { resource: { team: evals } }",
    ] {
        assert_eq!(digest(output), plain, "{output}");
    }
}

#[test]
fn a_setting_that_says_what_a_run_does_changes_the_digest() {
    let plain = digest("");
    let mut seen = vec![plain];

    for behaviour in [
        "run: { max_turns: 5 }",
        "run: { max_turns: 6 }",
        "run: { completion: explicit }",
        "run: { timeout: 5m }",
        "run: { max_retries: 3 }",
        "run: { retry_jitter: 0 }",
        "run: { max_consecutive_invalid_turns: null }",
        "run: { completion_schema: { type: object } }",
        "run: { context: { mask: { trigger_tokens: 150000, keep_last: 5 } } }",
        "tools: { builtin: { root: work, enabled: [bash] } }",
        "tools: { builtin: { root: work, enabled: [bash], timeout: 5s } }",
        "tools: { builtin: { root: work, enabled: [bash], env: { CI: '1' } } }",
        "tools: { builtin: { root: elsewhere, enabled: [bash] } }",
        "tools: { deny: [bash] }",
        "tools: { allow: [bash] }",
        "tools: { max_output_bytes: null }",
        "tools: { output_cut: head }",
        "tools: { max_concurrent_calls: 1 }",
    ] {
        let digest = digest(behaviour);

        assert!(!seen.contains(&digest), "{behaviour}");
        seen.push(digest);
    }
    for model in [
        "model: { provider: fake, script: scripts/other.yaml, name: scripted-1 }",
        "model: { provider: fake, script: scripts/run.yaml, name: scripted-2 }",
        "model: { provider: fake, script: scripts/run.yaml, name: scripted-1, max_tokens: 4096 }",
        "model: { provider: fake, script: scripts/run.yaml, name: scripted-1, temperature: 0.2 }",
        "model: { provider: fake, script: scripts/run.yaml, name: scripted-1, cache_scope: run }",
        "model: { provider: anthropic, name: scripted-1 }",
        "model: { provider: anthropic, name: scripted-1, thinking: adaptive }",
        "model: { provider: anthropic, name: scripted-1, cache: false }",
        "model: { provider: openai, name: scripted-1 }",
        "model: { provider: openai, name: scripted-1, api: chat_completions }",
    ] {
        let digest = yaml(&format!("{model}\nprompt: {{ system: You fix tests. }}")).digest();

        assert!(!seen.contains(&digest), "{model}");
        seen.push(digest);
    }
    let prompted = yaml(
        "
model: { provider: fake, script: scripts/run.yaml, name: scripted-1 }
prompt: { system: 'You fix tests, tersely.' }
",
    )
    .digest();
    assert!(!seen.contains(&prompted));
}

#[test]
fn a_default_spelled_out_and_a_value_written_another_way_leave_the_digest_as_it_was() {
    let plain = digest("");

    for same in [
        "run: { max_turns: null, timeout: 10m, retry_jitter: 0.25 }",
        "run: { timeout: 600s }",
        "run: { timeout: 9m 60s }",
        "tools: { builtin: { enabled: [] }, allow: null, deny: [] }",
        "tools: { output_cut: preview, output_preview_bytes: 2000 }",
    ] {
        assert_eq!(digest(same), plain, "{same}");
    }
    assert_eq!(
        digest("tools: { builtin: { root: work, enabled: [read_file, bash, bash] } }"),
        digest("tools: { builtin: { root: work, enabled: [bash, read_file] } }"),
        "the tools are offered in one order, however the config lists them"
    );
    assert_eq!(
        yaml("model: { thinking: provider_default, cache: true }").digest(),
        yaml("").digest(),
        "a provider's own default is the same stated or not"
    );
}

fn resolved_tools(text: &str) -> Value {
    serde_json::to_value(yaml(text).resolved().tools).unwrap()
}

#[test]
fn lists_that_offer_the_same_tools_share_a_digest_however_they_name_them() {
    for list in ["allow", "deny"] {
        let digest = |names: &str| digest(&format!("tools: {{ {list}: {names} }}"));
        let both = digest("[bash, read_file]");

        assert_eq!(
            digest("[read_file, bash]"),
            both,
            "{list}, in another order"
        );
        assert_eq!(
            digest("[bash, read_file, bash]"),
            both,
            "{list}, a name twice"
        );
        assert_eq!(digest("[bash, bash]"), digest("[bash]"), "{list}");
        assert_ne!(digest("[bash]"), both, "{list}, of other tools");
        assert_eq!(
            resolved_tools(&format!(
                "tools: {{ {list}: [write_file, bash, write_file] }}"
            ))[list],
            json!(["bash", "write_file"])
        );
    }
}

#[test]
fn no_allow_list_is_held_apart_from_a_list() {
    assert_eq!(resolved_tools("")["allow"], json!(null));
    assert_eq!(
        resolved_tools("tools: { allow: null }")["allow"],
        json!(null)
    );
    assert_eq!(resolved_tools("tools: { allow: [] }")["allow"], json!([]));
    assert_eq!(
        yaml("tools: { allow: [bash] }").resolved().tools.allow,
        Some(["bash".to_owned()].into())
    );
    assert_ne!(digest("tools: { allow: [] }"), digest(""));
}

#[test]
fn the_length_of_a_preview_is_resolved_only_under_the_cut_that_makes_one() {
    for cut in ["head", "head_tail"] {
        let stated = format!("tools: {{ output_cut: {cut}, output_preview_bytes: 100 }}");
        let plain = format!("tools: {{ output_cut: {cut} }}");

        assert_eq!(digest(&stated), digest(&plain), "{cut}");
        assert_eq!(yaml(&stated).resolved(), yaml(&plain).resolved(), "{cut}");
        let resolved = resolved_tools(&stated);
        assert_eq!(resolved["output_cut"], json!(cut));
        assert_eq!(resolved.get("output_preview_bytes"), None, "{cut}");
    }
    assert_eq!(
        resolved_tools("tools: { output_preview_bytes: 100 }")["output_preview_bytes"],
        json!(100)
    );
    assert_ne!(digest("tools: { output_preview_bytes: 100 }"), digest(""));
}

/// Pairs of configs with the same effect: the second states a setting that
/// nothing of the config applies.
const SAME_EFFECT: [(&str, &str); 8] = [
    (
        "tools: { max_output_bytes: null }",
        "tools: { max_output_bytes: null, output_cut: head }",
    ),
    (
        "tools: { max_output_bytes: null }",
        "tools: { max_output_bytes: null, output_cut: head_tail }",
    ),
    (
        "tools: { max_output_bytes: null }",
        "tools: { max_output_bytes: null, output_preview_bytes: 100 }",
    ),
    ("", "tools: { mcp_lifetime: lablet }"),
    ("", "tools: { mcp_result: content }"),
    ("", "tools: { builtin: { root: work } }"),
    ("", "tools: { builtin: { timeout: 5s } }"),
    ("", "tools: { builtin: { env: { CI: '1' } } }"),
];

#[test]
fn a_setting_that_nothing_applies_is_left_out_and_leaves_the_digest_as_it_was() {
    for (plain, stated) in SAME_EFFECT {
        let resolved = |more: &str| yaml(&format!("{FAKE}{more}")).resolved();

        assert_eq!(digest(stated), digest(plain), "{stated}");
        assert_eq!(resolved(stated), resolved(plain), "{stated}");
    }
    let tools = resolved_tools("tools: { max_output_bytes: null, output_cut: head }");
    for key in [
        "output_cut",
        "output_preview_bytes",
        "mcp_lifetime",
        "mcp_result",
    ] {
        assert_eq!(tools.get(key), None, "{key}");
    }
    assert_eq!(tools["builtin"], json!({ "enabled": [] }));
}

const SERVER: &str = "{ transport: stdio, name: files, command: files-server }";

/// The same settings as [`SAME_EFFECT`]'s, in configs that apply them.
#[test]
fn a_setting_that_is_applied_changes_the_digest() {
    let served = format!("tools: {{ mcp: [{SERVER}] }}");
    let enabled = "tools: { builtin: { root: work, enabled: [bash] } }";
    for (plain, stated) in [
        ("", "tools: { output_cut: head }".to_owned()),
        ("", "tools: { output_cut: head_tail }".to_owned()),
        ("", "tools: { output_preview_bytes: 100 }".to_owned()),
        (
            &served,
            format!("tools: {{ mcp: [{SERVER}], mcp_lifetime: lablet }}"),
        ),
        (
            &served,
            format!("tools: {{ mcp: [{SERVER}], mcp_result: content }}"),
        ),
        (
            enabled,
            "tools: { builtin: { root: elsewhere, enabled: [bash] } }".to_owned(),
        ),
        (
            enabled,
            "tools: { builtin: { root: work, enabled: [bash], timeout: 5s } }".to_owned(),
        ),
        (
            enabled,
            "tools: { builtin: { root: work, enabled: [bash], env: { CI: '1' } } }".to_owned(),
        ),
    ] {
        assert_ne!(digest(&stated), digest(plain), "{stated}");
    }
    assert_eq!(
        resolved_tools(&served)["mcp_lifetime"],
        json!("run"),
        "a default that's applied is held as a stated value is"
    );
    assert_eq!(resolved_tools(&served)["mcp_result"], json!("structured"));
    assert_eq!(
        resolved_tools(enabled)["builtin"],
        json!({ "root": "work", "enabled": ["bash"], "timeout": "2m", "env": {} })
    );
}

/// A server at `url` with `headers`, as a config states it.
fn http_server(headers: &str, url: &str) -> String {
    format!("tools: {{ mcp: [{{ transport: http, name: s, url: '{url}', headers: {headers} }}] }}")
}

/// The digest of an `openai` model at the gateway `url`.
fn gateway(url: &str) -> ConfigDigest {
    yaml(&format!(
        "model: {{ provider: openai, base_url: '{url}' }}\nprompt: {{ system: You fix tests. }}"
    ))
    .digest()
}

/// Pairs of configs that differ only in a credential share a digest: a
/// rotated token isn't a change to what a run does.
#[test]
fn two_configs_that_differ_only_in_a_credential_share_a_digest() {
    let at = "http://h/mcp";

    assert_eq!(
        digest(&http_server(
            "{ Authorization: 'Bearer one-0123456789' }",
            at
        )),
        digest(&http_server(
            "{ Authorization: 'Bearer two-0123456789' }",
            at
        )),
        "a rotated literal header"
    );
    assert_eq!(
        digest(&http_server("{ Authorization: '${A}' }", at)),
        digest(&http_server("{ Authorization: '${B}' }", at)),
        "a header from another variable"
    );
    assert_eq!(
        digest(&http_server("{}", "https://u:p1@h/mcp")),
        digest(&http_server("{}", "https://u:p2@h/mcp")),
        "a URL password"
    );
    assert_eq!(
        digest(&http_server("{}", "https://u:p1@h/mcp")),
        digest(&http_server("{}", "https://h/mcp")),
        "user information at all"
    );
    assert_eq!(
        gateway("https://hannah:one-0123456789@gw.example/v1"),
        gateway("https://hannah:two-0123456789@gw.example/v1")
    );
    assert_eq!(
        gateway("https://hannah:one-0123456789@gw.example/v1"),
        gateway("https://gw.example/v1")
    );
}

/// What stays in the digest beside a credential: the header's key, the
/// URL's host, and the variables a command starts with.
#[test]
fn a_header_s_key_a_url_s_host_and_a_command_s_variables_change_the_digest() {
    let at = "http://h/mcp";

    assert_ne!(
        digest(&http_server("{ Authorization: x }", at)),
        digest(&http_server("{ Authorization: x, X-Tenant: y }", at)),
        "a header key added"
    );
    assert_ne!(
        digest(&http_server("{}", "https://u:p@h/mcp")),
        digest(&http_server("{}", "https://u:p@other/mcp")),
        "a URL host"
    );
    assert_ne!(
        gateway("https://u:p@gw.example/v1"),
        gateway("https://u:p@other.example/v1")
    );
    assert_ne!(
        digest("tools: { builtin: { root: work, enabled: [bash], env: { TOKEN: one } } }"),
        digest("tools: { builtin: { root: work, enabled: [bash], env: { TOKEN: two } } }"),
        "tools.builtin.env"
    );
}

/// The strip is the digest's alone: the resolved config holds every
/// credential as written, and reads back to the same digest.
#[test]
fn the_resolved_config_keeps_a_credential_as_written_and_reads_back_to_the_same_digest() {
    let config = yaml(&format!(
        "{FAKE}{}",
        http_server(
            "{ Authorization: 'Bearer token-0123456789' }",
            "https://u:p@h/mcp"
        )
    ));

    let printed = serde_json::to_value(config.resolved()).unwrap();

    assert_eq!(
        printed["tools"]["mcp"][0]["headers"]["Authorization"],
        json!("Bearer token-0123456789")
    );
    assert_eq!(
        printed["tools"]["mcp"][0]["url"],
        json!("https://u:p@h/mcp")
    );
    let read_back = Config::from_str(&printed.to_string(), Format::Json).unwrap();
    assert_eq!(read_back.digest(), config.digest());
    assert_ne!(config.digest(), digest(""), "the server is in the digest");
}

#[test]
fn the_digest_is_the_same_whatever_order_and_format_the_config_is_written_in() {
    let from_yaml = yaml(
        "
prompt: { system: You fix tests. }
run: { max_turns: 5, timeout: 5m }
model: { name: scripted-1, script: scripts/run.yaml, provider: fake }
tools: { builtin: { env: { B: '2', A: '1' }, enabled: [bash], root: work } }
",
    );
    let from_json = Config::from_str(
        &json!({
            "model": { "provider": "fake", "script": "scripts/run.yaml", "name": "scripted-1" },
            "tools": {
                "builtin": { "root": "work", "enabled": ["bash"], "env": { "A": "1", "B": "2" } },
            },
            "run": { "timeout": "300s", "max_turns": 5 },
            "prompt": { "system": "You fix tests." },
        })
        .to_string(),
        Format::Json,
    )
    .unwrap();

    assert_eq!(from_yaml.digest(), from_json.digest());
    assert_eq!(from_yaml.resolved().digest(), from_yaml.digest());
}

#[test]
fn where_a_config_was_read_from_is_no_part_of_its_digest() {
    let scratch = lablet_test_support::Scratch::new("digest-path");
    let path = scratch.at("lablet.yaml");
    std::fs::write(&path, FAKE).unwrap();

    let read = Config::from_path(&path).unwrap();

    assert_eq!(read.digest(), digest(""));
}

#[test]
fn canonical_json_is_compact_with_the_keys_of_every_object_in_order() {
    let mut written = String::new();

    write_canonical(
        &json!({
            "b": [{ "z": null, "a": 0.25 }, "two\n\"lines\"", true],
            "a": { "k\u{e9}y": 1, "K": -2 },
            "c": {},
            "d": [],
        }),
        &mut written,
    );

    assert_eq!(
        written,
        r#"{"a":{"K":-2,"kéy":1},"b":[{"a":0.25,"z":null},"two\n\"lines\"",true],"c":{},"d":[]}"#
    );
}

#[test]
fn a_setting_that_is_not_applied_has_no_value_and_one_that_is_has_its_own() {
    assert!(Applied::<bool>::No.is_no());
    assert!(!Applied::Yes(false).is_no());
    assert_eq!(Applied::<bool>::No.value(), None);
    assert_eq!(Applied::Yes(Some(7)).value(), Some(Some(7)));
    assert_eq!(serde_json::to_value(Applied::Yes(7)).unwrap(), json!(7));
    assert_eq!(
        serde_json::to_value(Applied::<u8>::No).unwrap(),
        json!(null)
    );
}
