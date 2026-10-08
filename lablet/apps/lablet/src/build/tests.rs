//! What lablet reads of where it runs, against an environment the test
//! states, since a test can't set a variable of its own process: the check
//! of the key's variable, the names of lablet's secrets, and `${VAR}`.

use lablet_run::telemetry::generated::GenAiClientInferenceOperationDetails;
use lablet_run::telemetry::generated::key;

use super::*;

/// What stands for a key that was written where the variable's name
/// belongs. It reads as a name, so only the environment refuses it.
const PASTED: &str = "sk_live_0123456789abcdef";

/// A key as long as a key is, and so one that's cut.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

fn model(text: &str) -> Config {
    config(text)
}

/// Whether the key's variable is set, for the config as it runs and as
/// it's written, which here are one.
fn key_set(config: &Config, held: impl Fn(&str) -> Option<OsString>) -> Result<(), BuildError> {
    key_is_set(&config.model, config, &held)
}

/// An environment that holds `value` under `name` and nothing else.
fn holding(name: &'static str, value: &'static str) -> impl Fn(&str) -> Option<OsString> {
    move |asked| (asked == name).then(|| value.into())
}

fn nothing(_: &str) -> Option<OsString> {
    None
}

fn refusal(model: &Config, held: impl Fn(&str) -> Option<OsString>) -> String {
    let error = key_set(model, held).unwrap_err();
    assert!(matches!(error, BuildError::KeyVariable { .. }), "{error:?}");
    assert!(!format!("{error:?}").contains(PASTED), "{error:?}");
    error.to_string()
}

#[test]
fn a_variable_that_holds_a_key_is_taken() {
    let named = model(&format!("model: {{ api_key_env: {PASTED} }}"));

    assert_eq!(key_set(&named, holding(PASTED, "a key")), Ok(()));
    assert_eq!(
        key_set(&model(""), holding("ANTHROPIC_API_KEY", "a key")),
        Ok(())
    );
}

#[test]
fn a_variable_that_is_not_set_is_refused_and_a_name_not_written_in_capitals_is_not_shown() {
    for named in [PASTED, "work_key", "Work_Key", "ANTHROPIC_API_KEy"] {
        let config = model(&format!("model: {{ api_key_env: {named} }}"));

        let shown = refusal(&config, holding("ANTHROPIC_API_KEY", "a key"));

        assert_eq!(
            shown,
            "model.api_key_env (line 1) is refused: the variable it names isn't set, and the provider \
             `anthropic` needs a key",
            "{named}"
        );
        assert!(!shown.contains(named), "{named}: {shown}");
    }
}

#[test]
fn a_variable_that_is_empty_is_refused_and_a_name_not_written_in_capitals_is_not_shown() {
    let named = model(&format!("model: {{ api_key_env: {PASTED} }}"));

    assert_eq!(
        refusal(&named, holding(PASTED, "")),
        "model.api_key_env (line 1) is refused: the variable it names is empty, and the provider \
         `anthropic` needs a key"
    );
}

#[test]
fn a_variable_the_config_names_in_capitals_is_named_when_it_is_not_set_or_empty() {
    for named in ["ANTHROPIC_API_KEY", "WORK_KEY_2", "_K"] {
        let config = model(&format!("model: {{ api_key_env: {named} }}"));

        assert_eq!(
            refusal(&config, nothing),
            format!(
                "model.api_key_env (line 1) is refused: `{named}`, the variable it names, isn't set, \
                 and the provider `anthropic` needs a key"
            )
        );
    }
    let empty = model("model: { api_key_env: WORK_KEY }");
    assert_eq!(
        refusal(&empty, holding("WORK_KEY", "")),
        "model.api_key_env (line 1) is refused: `WORK_KEY`, the variable it names, is empty, and the \
         provider `anthropic` needs a key"
    );
}

#[test]
fn the_variable_that_is_read_when_the_config_names_none_is_shown() {
    assert_eq!(
        refusal(&model(""), nothing),
        "model.api_key_env is refused: `ANTHROPIC_API_KEY`, the variable that's read when \
         the config names none, isn't set, and the provider `anthropic` needs a key"
    );
    assert_eq!(
        refusal(&model(""), holding("ANTHROPIC_API_KEY", "")),
        "model.api_key_env is refused: `ANTHROPIC_API_KEY`, the variable that's read when \
         the config names none, is empty, and the provider `anthropic` needs a key"
    );
}

#[test]
fn a_provider_that_needs_no_key_has_no_variable_read() {
    let never = |_: &str| -> Option<OsString> { panic!("the environment was read") };

    for provider in ["openai", "fake"] {
        let named = model(&format!(
            "model: {{ provider: {provider}, api_key_env: {PASTED} }}"
        ));

        assert_eq!(key_set(&named, never), Ok(()), "{provider}");
    }
}

/// T16: the check names the variables a run withholds and the names whose
/// values it cuts, with a note on a value that isn't, and never a value.
#[tokio::test]
async fn the_check_names_what_a_run_withholds_and_cuts_and_never_a_value() {
    let scratch = lablet_test_support::Scratch::new("check-names");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "model: {{ provider: fake, script: '{}', api_key_env: WORK_KEY }}\n\
         prompt: {{ system: Hi. }}\n\
         telemetry: {{ otlp: {{ headers: {{ X-Short: '${{SHORT}}' }} }} }}",
        script.display()
    );
    let held = |name: &str| match name {
        "WORK_KEY" => Some(KEY.into()),
        "SHORT" => Some("short".into()),
        _ => None,
    };

    let checked = check_in(&config(&text), &held).await.unwrap();

    assert_eq!(
        checked.withheld(),
        &["SHORT".to_owned(), "WORK_KEY".to_owned()].into()
    );
    assert_eq!(
        checked.cut(),
        ["SHORT (under 16 bytes, not cut)", "WORK_KEY"]
    );
    assert!(!format!("{checked:?}").contains(KEY), "{checked:?}");
}

/// The `OTEL_*` variables and the context variables are the one
/// exception: read for every config, since lablet inherits them and they
/// configure its telemetry whatever the config names.
#[tokio::test]
async fn a_config_that_names_no_secret_reads_only_the_opentelemetry_variables() {
    let never = |name: &str| -> Option<OsString> {
        assert!(
            name.starts_with("OTEL_") || ["TRACEPARENT", "TRACESTATE", "BAGGAGE"].contains(&name),
            "the environment was read: {name}"
        );
        None
    };
    let scratch = lablet_test_support::Scratch::new("check-no-secret");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}",
        script.display()
    );

    let checked = check_in(&config(&text), &never).await.unwrap();

    assert!(checked.withheld().is_empty());
    assert_eq!(checked.cut(), Vec::<String>::new());
}

/// C19 at the library: a header variable the exporter reads is named among
/// what's cut and not among what's withheld, since every command inherits
/// it, and its value is nowhere.
#[tokio::test]
async fn the_check_names_an_otlp_header_variable_among_the_cut_and_withholds_it_from_no_command() {
    const TOKEN: &str = "otlp-0123456789abcdef";
    let scratch = lablet_test_support::Scratch::new("check-otlp-headers");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}",
        script.display()
    );
    let held = |name: &str| match name {
        "OTEL_EXPORTER_OTLP_TRACES_HEADERS" => {
            Some(format!("authorization=Bearer%20{TOKEN}").into())
        }
        _ => None,
    };

    let checked = check_in(&config(&text), &held).await.unwrap();

    assert!(checked.withheld().is_empty());
    assert_eq!(checked.cut(), ["OTEL_EXPORTER_OTLP_TRACES_HEADERS"]);
    assert!(!format!("{checked:?}").contains(TOKEN), "{checked:?}");
}

/// C19 at the library: an endpoint variable the exporter reads, holding
/// credentials, is named among what's cut and not among what's withheld,
/// and its password is nowhere.
#[tokio::test]
async fn the_check_names_an_otlp_endpoint_variable_with_credentials_among_the_cut_and_withholds_it_from_no_command()
 {
    const PASSWORD: &str = "otlp-pw-0123456789abcdef";
    let scratch = lablet_test_support::Scratch::new("check-otlp-endpoint");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}",
        script.display()
    );
    let held = |name: &str| match name {
        "OTEL_EXPORTER_OTLP_ENDPOINT" => {
            Some(format!("https://collector:{PASSWORD}@collector.internal:4317").into())
        }
        _ => None,
    };

    let checked = check_in(&config(&text), &held).await.unwrap();

    assert!(checked.withheld().is_empty());
    assert_eq!(checked.cut(), ["OTEL_EXPORTER_OTLP_ENDPOINT"]);
    assert!(!format!("{checked:?}").contains(PASSWORD), "{checked:?}");
}

/// Where the telemetry file goes: no file for a null path, standard error
/// for `-`, and no file at all when `OTEL_SDK_DISABLED` turns telemetry
/// off, whatever the path.
#[test]
fn a_null_path_writes_no_file_a_dash_is_standard_error_and_a_disabled_sdk_writes_none() {
    let target = |path: &str, disabled: bool| {
        file_target(
            &config(&format!("telemetry: {{ file: {{ path: {path} }} }}")),
            disabled,
        )
    };

    assert_eq!(target("null", false), None);
    assert_eq!(target(r#""-""#, false), Some(FileTarget::Stderr));
    assert_eq!(
        target("out.jsonl", false),
        Some(FileTarget::Path("out.jsonl".into()))
    );
    for path in ["null", r#""-""#, "out.jsonl"] {
        assert_eq!(target(path, true), None, "{path}");
    }
}

/// A config of the fake provider ending at once, with `telemetry` stated
/// beside it, in `scratch`.
fn ending(scratch: &lablet_test_support::Scratch, telemetry: &str) -> Config {
    let script = scratch.write("script.yaml", ENDS);
    config(&format!(
        "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {telemetry}",
        script.display()
    ))
}

/// O29 before the wire: `OTEL_SDK_DISABLED=true` wins over a config that
/// names a file and an endpoint, so a build makes neither.
#[tokio::test]
async fn otel_sdk_disabled_wins_over_a_named_file_and_endpoint() {
    let scratch = lablet_test_support::Scratch::new("sdk-disabled");
    let config = ending(
        &scratch,
        "{ file: { path: runs.otlp.jsonl }, otlp: { endpoint: 'http://127.0.0.1:1' } }",
    );

    let disabled = prepare(&config, &holding("OTEL_SDK_DISABLED", "true"))
        .await
        .unwrap();
    let enabled = prepare(&config, &nothing).await.unwrap();

    assert!(disabled.disabled);
    assert_eq!(disabled.target, None);
    assert!(disabled.otlp.is_none());
    assert!(!enabled.disabled);
    assert!(enabled.target.is_some());
    assert!(enabled.otlp.is_some());
}

/// The GenAI instrumentation's switch for capturing content.
const CAPTURE: &str = "OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT";

#[tokio::test]
async fn the_genai_capture_variable_decides_when_the_config_states_nothing() {
    let scratch = lablet_test_support::Scratch::new("capture-variable");
    let config = ending(&scratch, "{ otlp: { enabled: false } }");

    for (value, captured) in [("TRUE", true), ("false", false), ("1", false)] {
        let held = |name: &str| (name == CAPTURE).then(|| value.into());
        let prepared = prepare(&config, &held).await.unwrap();
        assert_eq!(prepared.capture_content, captured, "{value}");
    }
    assert!(!prepare(&config, &nothing).await.unwrap().capture_content);
}

#[tokio::test]
async fn a_stated_capture_content_wins() {
    let scratch = lablet_test_support::Scratch::new("capture-stated");

    for (stated, value, captured) in [("false", "true", false), ("true", "false", true)] {
        let config = ending(
            &scratch,
            &format!("{{ capture_content: {stated}, otlp: {{ enabled: false }} }}"),
        );
        let held = |name: &str| (name == CAPTURE).then(|| value.into());
        let prepared = prepare(&config, &held).await.unwrap();
        assert_eq!(prepared.capture_content, captured, "{stated}");
    }
}

const ENDS: &str = "
- response:
    content:
      - text: Done.
    finish: end_turn
";

/// C5: the digest a run carries is of the config as it's written, so two
/// runs whose variables hold different values share it, while each runs
/// with what its variable held.
#[tokio::test]
async fn two_runs_whose_variables_differ_share_a_config_digest_and_run_with_their_values() {
    let scratch = lablet_test_support::Scratch::new("digest-before-substitution");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "
model: {{ provider: fake, script: '{}', name: '${{MODEL}}' }}
prompt: {{ system: 'You fix ${{LANGUAGE}} tests.' }}
telemetry: {{ capture_content: true, file: {{ path: '{}' }}, otlp: {{ enabled: false }} }}
",
        script.display(),
        scratch.at("telemetry.otlp.jsonl").display()
    );
    let mut runs = Vec::new();

    for (model, language) in [("scripted-1", "Rust"), ("scripted-2", "Go")] {
        let held = |name: &str| match name {
            "MODEL" => Some(model.into()),
            "LANGUAGE" => Some(language.into()),
            _ => None,
        };
        let mut lablet = build_in(config(&text), &held).await.unwrap();
        let finished = lablet.run(crate::RunRequest::new("Fix it.").unwrap()).await;
        lablet.shutdown().await;
        runs.push(finished.summary.outcome.run_id);
    }

    // What each run was built with, as its own telemetry says: the digest
    // and the model on its root span, and the system prompt in the content
    // record of its first provider call.
    let exported =
        lablet_conformance::otlp::Exported::read(&scratch.at("telemetry.otlp.jsonl")).unwrap();
    let written = config(&text).digest().to_string();
    let seen: Vec<(String, String, bool)> = runs
        .iter()
        .zip(["You fix Rust tests.", "You fix Go tests."])
        .map(|(run_id, system)| {
            let of_run = |attributes: &lablet_conformance::otlp::Attributes| {
                attributes.get(key::GEN_AI_CONVERSATION_ID)
                    == Some(&serde_json::Value::String(run_id.to_string()))
            };
            let root = exported
                .spans_of(
                    lablet_run::telemetry::generated::LabletInvokeAgent::GEN_AI_OPERATION_NAME,
                )
                .into_iter()
                .find(|span| of_run(&span.attributes))
                .unwrap();
            let content = exported
                .records_of(GenAiClientInferenceOperationDetails::NAME)
                .into_iter()
                .find(|record| {
                    of_run(&record.attributes)
                        && record
                            .attributes
                            .contains_key(key::GEN_AI_SYSTEM_INSTRUCTIONS)
                })
                .unwrap();
            (
                root.attributes[key::LABLET_CONFIG_DIGEST]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                root.attributes[key::GEN_AI_REQUEST_MODEL]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                content.attributes[key::GEN_AI_SYSTEM_INSTRUCTIONS]
                    .as_str()
                    .unwrap()
                    .contains(system),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (written.clone(), "scripted-1".to_owned(), true),
            (written, "scripted-2".to_owned(), true),
        ]
    );
}

/// The SDK's settings a build reads from its environment reach the
/// providers it makes: with the sampler `always_off`, a run exports no
/// span, and its wide event still.
#[tokio::test]
async fn the_sampler_the_environment_names_reaches_the_providers_a_build_makes() {
    let scratch = lablet_test_support::Scratch::new("build-sampler");
    let script = scratch.write("script.yaml", ENDS);
    let path = scratch.at("telemetry.otlp.jsonl");
    let text = format!(
        "
model: {{ provider: fake, script: '{}', name: scripted-1 }}
prompt: {{ system: 'You fix tests.' }}
telemetry: {{ file: {{ path: '{}' }}, otlp: {{ enabled: false }} }}
",
        script.display(),
        path.display()
    );
    let held = holding("OTEL_TRACES_SAMPLER", "always_off");

    let mut lablet = build_in(config(&text), &held).await.unwrap();
    lablet.run(crate::RunRequest::new("Fix it.").unwrap()).await;
    lablet.shutdown().await;

    let exported = lablet_conformance::otlp::Exported::read(&path).unwrap();
    assert!(exported.spans.is_empty(), "{:?}", exported.spans);
    assert_eq!(
        exported
            .records_of(lablet_run::telemetry::generated::LabletRun::NAME)
            .len(),
        1
    );
}

/// C16: a value a variable gave is refused as the config writes it, and
/// nothing of what the variable holds is in the message.
#[tokio::test]
async fn a_value_a_variable_gave_is_refused_as_it_is_written() {
    const HELD: &str = "internal-gateway-7f3a";
    let text = "model: { provider: openai, base_url: '${GATEWAY}' }\nprompt: { system: Hi. }";
    let held = holding("GATEWAY", HELD);

    let error = check_in(&config(text), &held).await.unwrap_err();

    assert_eq!(
        error,
        BuildError::Config(ConfigError::Invalid {
            key: "model.base_url".to_owned(),
            place: Some(Place::Line(1)),
            value: Some("\"${GATEWAY}\"".to_owned()),
            reason: "a URL begins `http://` or `https://` and names a host, as \
                     `http://localhost:11434/v1` does"
                .to_owned(),
        })
    );
    for shown in [error.to_string(), format!("{error:?}")] {
        assert!(!shown.contains(HELD), "{shown}");
    }
    assert_eq!(error.class(), ErrorClass::Config);
    assert_eq!(
        check_in(
            &config(text),
            &holding("GATEWAY", "http://localhost:4000/v1")
        )
        .await
        .map(drop),
        Ok(()),
        "a URL passes, and the provider that has no adapter yet isn't selected"
    );
}

/// C10: a variable a config names that isn't set is a config error.
#[tokio::test]
async fn a_variable_that_is_not_set_is_refused_by_the_key_it_is_in() {
    let error = check_in(
        &config("model: { provider: openai, name: '${MODEL}' }\nprompt: { system: Hi. }"),
        &nothing,
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "model.name (line 1): \"${MODEL}\" is refused: the variable `MODEL` isn't set"
    );
}

/// C13: a setting the provider can't apply is refused by the check, and
/// the config without it passes, with the resolved config leaving out what
/// the provider can't apply.
#[tokio::test]
async fn the_check_refuses_a_setting_the_provider_cannot_apply_and_resolves_the_rest() {
    let key = holding("ANTHROPIC_API_KEY", "a key");
    for (text, key_name, reached) in [
        (
            "model: { seed: 7 }\nprompt: { system: Hi. }",
            "model.seed",
            "the provider `anthropic`",
        ),
        (
            "model: { provider: openai, base_url: 'http://localhost:11434/v1', thinking: adaptive }\n\
             prompt: { system: Hi. }",
            "model.thinking",
            "the provider `openai` over its API `chat_completions`",
        ),
    ] {
        let error = check_in(&config(text), &key).await.unwrap_err();
        assert!(
            matches!(
                &error,
                BuildError::Config(ConfigError::NotApplied { key, reached: said, .. })
                    if *key == key_name && said == reached
            ),
            "{error:?}"
        );
        assert!(error.to_string().contains(key_name), "{error}");
        assert!(error.to_string().contains(reached), "{error}");
    }

    for (text, left_out) in [
        ("prompt: { system: Hi. }", "seed"),
        (
            "model: { provider: openai, base_url: 'http://localhost:11434/v1' }\n\
             prompt: { system: Hi. }",
            "thinking",
        ),
    ] {
        let checked = check_in(&config(text), &key).await.unwrap();
        let resolved = serde_json::to_value(checked.resolved()).unwrap();

        assert!(resolved["model"].get(left_out).is_none(), "{resolved}");
        assert_eq!(checked.resolved(), &config(text).resolved());
    }
}

/// C17: a config that states only what it must is checked into the
/// resolved config of the spec's own block, which states every default.
#[tokio::test]
async fn a_config_that_states_only_what_it_must_resolves_to_every_default_of_the_spec() {
    let mut spec: serde_json::Value =
        serde_saphyr::from_str(include_str!("../config/tests/spec.yaml")).unwrap();
    // The spec's block lists two servers as examples, which no default has.
    spec["tools"]["mcp"] = serde_json::json!([]);
    let spec = Config::from_str(&spec.to_string(), Format::Json).unwrap();

    let checked = check_in(
        &config("prompt: { system: You are ... }"),
        &holding("ANTHROPIC_API_KEY", "a key"),
    )
    .await
    .unwrap();

    assert_eq!(checked.resolved(), &spec.resolved());
    assert_eq!(checked.tools(), []);
}

/// C15: whether the telemetry is on standard error is read from the path
/// as a run writes to it, once `${VAR}` is substituted, and only a path
/// that's `-` whole is standard error.
#[test]
fn the_telemetry_is_on_standard_error_when_its_path_is_a_dash_once_variables_are_substituted() {
    let on_stderr = |path: &str, env: Env<'_>| {
        let config = config(&format!("telemetry: {{ file: {{ path: {path} }} }}"));
        telemetry_on_stderr_in(&config, env)
    };

    assert!(
        !on_stderr(r#""-""#, &holding("OTEL_SDK_DISABLED", "true")),
        "no telemetry is written at all"
    );

    assert!(on_stderr(r#""-""#, &nothing));
    assert!(on_stderr("'${T}'", &holding("T", "-")));

    assert!(!on_stderr("'${T}'", &holding("T", "telemetry.jsonl")));
    assert!(!on_stderr("'${T}'", &nothing), "a build refuses it");
    assert!(!on_stderr(r#""-/""#, &nothing), "it names a file");
    assert!(!on_stderr("'${T}/'", &holding("T", "-")), "it names a file");
    assert!(!on_stderr(r#""-.jsonl""#, &nothing));
    assert!(!on_stderr("null", &nothing));
}

/// An endpoint the exporter refuses is shown as the config writes it, or
/// with no value when that holds an `@`: user information is a secret, and
/// where it ends can't be told once it may hold an unencoded `/`. When the
/// config states no endpoint, the refusal names the variable the exporter
/// read in its place, the signal's own before the generic one, and shows
/// nothing of either.
#[test]
fn a_refused_endpoint_is_shown_as_written_unless_it_holds_an_at_or_names_the_variable_read() {
    use crate::export::Signal;
    const PASSWORD: &str = "hunter2-0123456789abcdef";
    let refused = |signal| OtelBuildError::Endpoint { signal };

    let written = config("telemetry: { otlp: { endpoint: 'http://[not a host' } }");
    let shown = otlp_refused(&written, [None, None], refused(Signal::Traces));
    assert_eq!(
        shown.to_string(),
        "telemetry.otlp.endpoint (line 1): \"http://[not a host\" is refused: it isn't a URL the \
         exporter accepts"
    );

    for endpoint in [
        format!("http://user:{PASSWORD}@[not a host"),
        format!("user:{PASSWORD}@[not a host"),
        format!("https://user:ab/{PASSWORD}@[not a host"),
    ] {
        let written = config(&format!(
            "telemetry: {{ otlp: {{ endpoint: '{endpoint}' }} }}"
        ));
        let shown = otlp_refused(&written, [None, None], refused(Signal::Traces));
        assert_eq!(
            shown.to_string(),
            "telemetry.otlp.endpoint (line 1): its value is refused: it isn't a URL the exporter \
             accepts",
            "{endpoint}"
        );
        assert!(!format!("{shown:?}").contains(PASSWORD), "{shown:?}");
    }

    let written = config("telemetry: { otlp: { headers: { a: b } } }");
    let read = [
        Some("OTEL_EXPORTER_OTLP_ENDPOINT"),
        Some("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"),
    ];
    let shown = otlp_refused(&written, read, refused(Signal::Traces));
    assert_eq!(
        shown,
        BuildError::Config(ConfigError::Invalid {
            key: "telemetry.otlp.endpoint".to_owned(),
            place: None,
            value: None,
            reason: "`OTEL_EXPORTER_OTLP_ENDPOINT`, which is read since the config states no \
                     endpoint, holds what isn't a URL the exporter accepts"
                .to_owned(),
        })
    );
    let shown = otlp_refused(&written, read, refused(Signal::Logs));
    assert_eq!(
        shown.to_string(),
        "telemetry.otlp.endpoint: its value is refused: `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, which \
         is read since the config states no endpoint, holds what isn't a URL the exporter accepts"
    );
    assert!(!format!("{shown:?}").contains("not a host"), "{shown:?}");

    let shown = otlp_refused(
        &written,
        [None, None],
        OtelBuildError::HttpClient {
            signal: Signal::Traces,
            reason: "it failed".to_owned(),
        },
    );
    assert_eq!(
        shown.to_string(),
        "telemetry.otlp.endpoint: its value is refused: the HTTP client couldn't be made: it \
         failed",
        "the default endpoint, which the config doesn't state, isn't shown as `null`"
    );
}

/// TLS to the collector that can't be set up is refused with what stood in
/// the way and no guess at why, since roots that can't be loaded and an
/// address TLS can't name are refused alike: by the endpoint as the config
/// writes it, or by the variable the exporter read it from, showing
/// nothing of what that holds.
#[test]
fn tls_that_cannot_be_set_up_is_refused_by_the_endpoint_as_written_or_the_variable_read() {
    use crate::export::Signal;
    let tls = |signal| OtelBuildError::Tls {
        signal,
        reason: "transport error: no native certs found".to_owned(),
    };

    let written = config("telemetry: { otlp: { endpoint: 'https://collector.internal:4317' } }");
    let shown = otlp_refused(&written, [None, None], tls(Signal::Traces));
    assert_eq!(
        shown.to_string(),
        "telemetry.otlp.endpoint (line 1): \"https://collector.internal:4317\" is refused: TLS to \
         the collector couldn't be set up: transport error: no native certs found"
    );

    let written = config("telemetry: { otlp: { headers: { a: b } } }");
    let shown = otlp_refused(
        &written,
        [None, Some("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT")],
        tls(Signal::Logs),
    );
    assert_eq!(
        shown,
        BuildError::Config(ConfigError::Invalid {
            key: "telemetry.otlp.endpoint".to_owned(),
            place: None,
            value: None,
            reason: "`OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, which is read since the config states no \
                     endpoint, names the collector, and TLS to it couldn't be set up: transport \
                     error: no native certs found"
                .to_owned(),
        })
    );
    assert!(
        !format!("{shown:?}").contains("collector.internal"),
        "{shown:?}"
    );
}
