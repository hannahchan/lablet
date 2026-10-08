//! What lablet reads of where it runs, against an environment the test
//! states, since a test can't set a variable of its own process: the check
//! of the key's variable, the names of lablet's secrets, and `${VAR}`.

use std::ffi::OsString;

use super::*;

/// What stands for a key that was written where the variable's name
/// belongs. It reads as a name, so only the environment refuses it.
const PASTED: &str = "sk_live_0123456789abcdef";

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

    let disabled = prepare(&config, &holding("OTEL_SDK_DISABLED", "true")).unwrap();
    let enabled = prepare(&config, &nothing).unwrap();

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
        let prepared = prepare(&config, &held).unwrap();
        assert_eq!(prepared.capture_content, captured, "{value}");
    }
    assert!(!prepare(&config, &nothing).unwrap().capture_content);
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
        let prepared = prepare(&config, &held).unwrap();
        assert_eq!(prepared.capture_content, captured, "{stated}");
    }
}

const ENDS: &str = "
- response:
    content:
      - text: Done.
    finish: end_turn
";

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
    use lablet_otel_sdk::export::Signal;
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
    use lablet_otel_sdk::export::Signal;
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
