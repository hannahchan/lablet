//! What the command line reads of the telemetry, against an environment
//! the test states, since a test can't set a variable of its own process:
//! where the file goes, what turns it all off, content capture, and the
//! refusals of what the network exporter would refuse.

use std::ffi::OsString;

use lablet_config::Format;
use lablet_conformance::receiver::{Mode, Receiver};
use lablet_prepare::prepare;

use super::*;

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

/// An environment that holds `value` under `name` and nothing else.
fn holding(name: &'static str, value: &'static str) -> impl Fn(&str) -> Option<OsString> {
    move |asked| (asked == name).then(|| value.into())
}

fn nothing(_: &str) -> Option<OsString> {
    None
}

/// The check of `config` in `env`, gone on through [`settle`].
fn settled(config: &Config, env: Env<'_>) -> (Prepared, Exports, Inbound) {
    settle(config, prepare(config, env).unwrap(), env).unwrap()
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

    let (_, disabled, _) = settled(&config, &holding("OTEL_SDK_DISABLED", "true"));
    let (_, enabled, _) = settled(&config, &nothing);

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
        let (prepared, _, _) = settled(&config, &held);
        assert_eq!(prepared.capture_content(), captured, "{value}");
    }
    assert!(!settled(&config, &nothing).0.capture_content());
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
        let (prepared, _, _) = settled(&config, &held);
        assert_eq!(prepared.capture_content(), captured, "{stated}");
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

/// A client key is read into the TLS identity when its signal's exporter
/// speaks TLS, and what its file holds is cut from what a tool returns,
/// each line too, as a value with several lines is. Its variable holds a
/// path, which is no secret: it's named among what's cut and withheld from
/// no command.
#[tokio::test]
async fn a_client_keys_contents_are_cut_from_a_tools_result_and_its_path_is_not() {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = lablet_test_support::Scratch::new("secrets-client-key");
    let key_file = scratch.write("client.key", receiver.client_key());
    let certificate_file = scratch.write("client.pem", receiver.client_certificate());
    let (key_path, certificate_path) = (
        key_file.display().to_string(),
        certificate_file.display().to_string(),
    );
    let held = |name: &str| match name {
        "OTEL_EXPORTER_OTLP_CLIENT_KEY" => Some(OsString::from(&key_path)),
        "OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE" => Some(OsString::from(&certificate_path)),
        _ => None,
    };
    scratch.create_dir("work");
    let config = ending(
        &scratch,
        &format!(
            "{{ otlp: {{ endpoint: 'https://collector.internal:4318' }} }}\ntools: {{ builtin: \
             {{ root: '{}', enabled: [bash] }} }}",
            scratch.at("work").display()
        ),
    );

    let (prepared, _, _) = settled(&config, &held);
    let derived = prepared.into_secrets();

    let cut: Vec<String> = derived.cut.iter().map(ToString::to_string).collect();
    assert_eq!(cut, ["OTEL_EXPORTER_OTLP_CLIENT_KEY"]);
    assert!(derived.withheld.is_empty());
    let key = receiver.client_key();
    let line = key.lines().nth(1).unwrap();
    let result = format!("cat said:\n{key}\nand then: {line}\nfrom {key_path}");
    let cut = derived.values.redacted(&result);
    assert!(!cut.contains(line), "{cut}");
    assert!(cut.contains(&key_path), "the path is no secret: {cut}");
    assert!(!format!("{derived:?}").contains(line));
}
