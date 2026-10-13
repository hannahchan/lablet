//! What the command line reads of the telemetry, from the config alone:
//! where the file goes, content capture, the refusals of what the network
//! exporter would refuse, and what a client key's file holds.

use std::ffi::OsString;

use lablet_config::{Format, RawConfig};
use lablet_conformance::receiver::{Mode, Receiver};
use lablet_prepare::prepare;

use super::*;
use crate::export::Signal;

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

/// The config `text` holds, with each `${VAR}` filled from `held`, or why it
/// was refused.
fn filled(text: &str, held: &[(&str, &str)]) -> Result<Config, lablet_config::ConfigError> {
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RawConfig::from_str(text, Format::Yaml)?
        .substituted(move |asked: &str| -> Option<OsString> {
            held.iter()
                .find(|(name, _)| name == asked)
                .map(|(_, value)| value.into())
        })?
        .config()
}

/// The check of `config` with an empty base handed in, gone on through
/// [`settle`].
fn settled(config: &Config) -> (Prepared, Exports) {
    settle(prepare(config, Some(Vec::new())).unwrap()).unwrap()
}

/// Where the telemetry file goes: no file for a null path, standard error
/// for `-`, and a file for anything else.
#[test]
fn a_null_path_writes_no_file_and_a_dash_is_standard_error() {
    let target = |path: &str| {
        file_target(&config(&format!(
            "telemetry: {{ file: {{ path: {path} }} }}"
        )))
    };

    assert_eq!(target("null"), None);
    assert_eq!(target(r#""-""#), Some(FileTarget::Stderr));
    assert_eq!(
        target("out.jsonl"),
        Some(FileTarget::Path("out.jsonl".into()))
    );
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

/// With nothing named, a build makes neither a file nor a network exporter,
/// and a named file and endpoint make both.
#[test]
fn nothing_named_builds_no_destination_and_a_named_file_and_endpoint_build_both() {
    let scratch = lablet_test_support::Scratch::new("settle-destinations");

    let (_, nothing) = settled(&ending(&scratch, "{}"));
    assert_eq!(nothing.target, None);
    assert!(nothing.otlp.is_none());

    let (_, both) = settled(&ending(
        &scratch,
        "{ file: { path: /tmp/runs.otlp.jsonl }, otlp: { endpoint: 'http://127.0.0.1:1' } }",
    ));
    assert!(both.target.is_some());
    assert!(both.otlp.is_some());
}

/// `telemetry.capture_content` decides alone, and a filled text is typed.
#[test]
fn capture_content_is_the_configs_alone_and_a_filled_text_is_typed() {
    let scratch = lablet_test_support::Scratch::new("capture-content");
    let script = scratch.write("script.yaml", ENDS).display().to_string();
    let with = |value: &str, held: &[(&str, &str)]| {
        let config = filled(
            &format!(
                "model: {{ provider: fake, script: '{script}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {{ capture_content: {value} }}"
            ),
            held,
        )
        .unwrap();
        settled(&config).0.capture_content()
    };

    assert!(!with("null", &[]));
    assert!(!with("false", &[]));
    assert!(with("true", &[]));
    assert!(with("'${C}'", &[("C", "true")]));
    assert!(!with("'${C}'", &[("C", "false")]));
    assert!(!with("'${C:-}'", &[]), "unset, so not stated");
}

const ENDS: &str = "
- response:
    content:
      - text: Done.
    finish: end_turn
";

/// C15: whether the telemetry is on standard error is read from the path
/// as a run writes to it, once `${VAR}` is filled, and only a path that's
/// `-` whole is standard error.
#[test]
fn the_telemetry_is_on_standard_error_when_its_path_is_a_dash_once_variables_are_filled() {
    let on_stderr = |path: &str, held: &[(&str, &str)]| {
        filled(&format!("telemetry: {{ file: {{ path: {path} }} }}"), held)
            .is_ok_and(|config| telemetry_on_stderr(&config))
    };

    assert!(on_stderr(r#""-""#, &[]));
    assert!(on_stderr("'${T}'", &[("T", "-")]));

    assert!(!on_stderr("'${T}'", &[("T", "telemetry.jsonl")]));
    assert!(!on_stderr("'${T}'", &[]), "a build refuses it");
    assert!(!on_stderr(r#""-/""#, &[]), "it names a file");
    assert!(!on_stderr("'${T}/'", &[("T", "-")]), "it names a file");
    assert!(!on_stderr(r#""-.jsonl""#, &[]));
    assert!(!on_stderr("null", &[]));
}

/// An endpoint the exporter refuses is shown as the config writes it, or
/// with no value when that holds an `@`: user information is a secret, and
/// where it ends can't be told once it may hold an unencoded `/`. An
/// endpoint a variable filled is refused naming the variable, and shows
/// nothing of what it holds. User information written out reaches no
/// exporter, since the check refuses it first, but for one with a `/` in
/// its password, which no URL reader finds.
#[test]
fn a_refused_endpoint_is_shown_as_written_unless_it_holds_an_at_and_a_filled_one_names_its_variable()
 {
    const PASSWORD: &str = "hunter2-0123456789abcdef";
    let refused = |signal| OtelBuildError::Endpoint { signal };

    let written = config("telemetry: { otlp: { endpoint: 'http://[not a host' } }");
    let shown = otlp_refused(&written, refused(Signal::Traces));
    assert_eq!(
        shown.to_string(),
        "telemetry.otlp.endpoint (line 1): \"http://[not a host\" is refused: it isn't a URL the \
         exporter accepts"
    );

    for (endpoint, reason) in [
        (
            "http://${U}:${P}@[not a host".to_owned(),
            "filled from `U` and `P`, it isn't a URL the exporter accepts",
        ),
        (
            "${U}:${P}@[not a host".to_owned(),
            "filled from `U` and `P`, it isn't a URL the exporter accepts",
        ),
        (
            format!("https://user:ab/{PASSWORD}@[not a host"),
            "it isn't a URL the exporter accepts",
        ),
    ] {
        let written = filled(
            &format!("telemetry: {{ otlp: {{ endpoint: '{endpoint}' }} }}"),
            &[("U", "user"), ("P", PASSWORD)],
        )
        .unwrap();
        let shown = otlp_refused(&written, refused(Signal::Traces));
        assert_eq!(
            shown.to_string(),
            format!("telemetry.otlp.endpoint (line 1): its value is refused: {reason}"),
            "{endpoint}"
        );
        assert!(!format!("{shown:?}").contains(PASSWORD), "{shown:?}");
    }

    let from_variable = filled(
        "telemetry: { otlp: { endpoint: '${COLLECTOR}' } }",
        &[("COLLECTOR", "http://[not a host")],
    )
    .unwrap();
    let shown = otlp_refused(&from_variable, refused(Signal::Logs));
    assert_eq!(
        shown,
        BuildError::Config(ConfigError::Invalid {
            key: "telemetry.otlp.endpoint".to_owned(),
            place: Some(Place::Line(1)),
            value: Some("\"${COLLECTOR}\"".to_owned()),
            reason: "`COLLECTOR` holds what isn't a URL the exporter accepts".to_owned(),
        })
    );
    assert!(!format!("{shown:?}").contains("not a host"), "{shown:?}");

    let shown = otlp_refused(
        &written,
        OtelBuildError::HttpClient {
            signal: Signal::Traces,
            reason: "it failed".to_owned(),
        },
    );
    assert_eq!(
        shown.to_string(),
        "telemetry.otlp.endpoint (line 1): \"http://[not a host\" is refused: the HTTP client \
         couldn't be made: it failed"
    );
}

/// TLS to the collector that can't be set up is refused with what stood in
/// the way and no guess at why, since roots that can't be loaded and an
/// address TLS can't name are refused alike: by the endpoint as the config
/// writes it, or by the variable that filled it, showing nothing of what
/// that holds.
#[test]
fn tls_that_cannot_be_set_up_is_refused_by_the_endpoint_as_written_or_the_variable_that_filled_it()
{
    let tls = |signal| OtelBuildError::Tls {
        signal,
        reason: "transport error: no native certs found".to_owned(),
    };

    let written = config("telemetry: { otlp: { endpoint: 'https://collector.internal:4317' } }");
    let shown = otlp_refused(&written, tls(Signal::Traces));
    assert_eq!(
        shown.to_string(),
        "telemetry.otlp.endpoint (line 1): \"https://collector.internal:4317\" is refused: TLS to \
         the collector couldn't be set up: transport error: no native certs found"
    );

    let from_variable = filled(
        "telemetry: { otlp: { endpoint: '${COLLECTOR}' } }",
        &[("COLLECTOR", "https://collector.internal:4317")],
    )
    .unwrap();
    let shown = otlp_refused(&from_variable, tls(Signal::Logs));
    assert_eq!(
        shown,
        BuildError::Config(ConfigError::Invalid {
            key: "telemetry.otlp.endpoint".to_owned(),
            place: Some(Place::Line(1)),
            value: Some("\"${COLLECTOR}\"".to_owned()),
            reason: "`COLLECTOR` names the collector, and TLS to it couldn't be set up: transport \
                     error: no native certs found"
                .to_owned(),
        })
    );
    assert!(
        !format!("{shown:?}").contains("collector.internal"),
        "{shown:?}"
    );
}

/// A client key is read into the TLS identity when the config names one,
/// and what its file holds is cut from what a tool returns, each line too,
/// as a value with several lines is, and named under `own:` as the file's
/// contents. Its path is no secret: nothing is dropped for it.
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
    scratch.create_dir("work");
    let script = scratch.write("script.yaml", ENDS).display().to_string();
    let config = filled(
        &format!(
            "model: {{ provider: fake, script: '{script}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {{ otlp: {{ endpoint: 'https://collector.internal:4318', tls: {{ client_cert: '{certificate_path}', client_key: '${{KEY_FILE}}' }} }} }}\ntools: {{ builtin: {{ root: '{}', enabled: [bash] }} }}",
            scratch.at("work").display()
        ),
        &[("KEY_FILE", &key_path)],
    )
    .unwrap();

    let (prepared, _) = settled(&config);
    let derived = prepared.into_secrets();

    let own: Vec<String> = derived
        .own
        .iter()
        .map(lablet_secrets::Credential::as_own)
        .collect();
    assert_eq!(
        own,
        ["telemetry.otlp.tls.client_key (line 3) -> its file's contents; cut"]
    );
    assert!(derived.dropped.is_empty());
    let key = receiver.client_key();
    let line = key.lines().nth(1).unwrap();
    let result = format!("cat said:\n{key}\nand then: {line}\nfrom {key_path}");
    let cut = derived.values.redacted(&result);
    assert!(!cut.contains(line), "{cut}");
    assert!(cut.contains(&key_path), "the path is no secret: {cut}");
    assert!(!format!("{derived:?}").contains(line));
}

/// A client key's contents are cut whenever the config states the key,
/// with no endpoint named and no certificate beside it, since the file is
/// a secret wherever the run exports; a stated file that can't be read is
/// refused by its key, endpoint or none, naming the variable that filled
/// its path.
#[tokio::test]
async fn a_client_keys_contents_are_cut_whenever_it_is_stated_and_an_unreadable_one_is_refused() {
    let receiver = Receiver::start(Mode::Answers).await;
    let scratch = lablet_test_support::Scratch::new("secrets-client-key-alone");
    let key_path = scratch
        .write("client.key", receiver.client_key())
        .display()
        .to_string();
    let missing = scratch.at("missing.key").display().to_string();
    let script = scratch.write("script.yaml", ENDS).display().to_string();
    scratch.create_dir("work");
    let work = scratch.at("work").display().to_string();
    let text = |otlp: &str| {
        format!(
            "model: {{ provider: fake, script: '{script}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {{ otlp: {{ tls: {{ client_key: '${{KEY_FILE}}' }}{otlp} }} }}\ntools: {{ builtin: {{ root: '{work}', enabled: [bash] }} }}"
        )
    };

    let config = filled(&text(""), &[("KEY_FILE", &key_path)]).unwrap();
    let (prepared, exports) = settled(&config);
    assert!(exports.otlp.is_none());
    let derived = prepared.into_secrets();
    let own: Vec<String> = derived
        .own
        .iter()
        .map(lablet_secrets::Credential::as_own)
        .collect();
    assert_eq!(
        own,
        ["telemetry.otlp.tls.client_key (line 3) -> its file's contents; cut"]
    );
    let line = receiver.client_key().lines().nth(1).unwrap();
    assert!(!derived.values.redacted(line).contains(line));

    for otlp in ["", ", endpoint: 'https://collector.internal:4318'"] {
        let config = filled(&text(otlp), &[("KEY_FILE", &missing)]).unwrap();
        let refused = settle(prepare(&config, Some(Vec::new())).unwrap())
            .err()
            .unwrap();
        let BuildError::Config(ConfigError::Invalid {
            key,
            place,
            value,
            reason,
        }) = refused
        else {
            panic!("{refused:?}");
        };
        assert_eq!(key, "telemetry.otlp.tls.client_key", "{otlp}");
        assert_eq!(place, Some(Place::Line(3)));
        assert_eq!(value.as_deref(), Some("\"${KEY_FILE}\""));
        assert!(
            reason.starts_with("`KEY_FILE` names a file that can't be read: "),
            "{reason}"
        );
    }
}

/// The endpoint a `telemetry:` line shows: its scheme, host, port and path,
/// and nothing from an `@` before them or a `?` after.
#[test]
fn the_endpoint_shown_keeps_the_scheme_host_port_and_path_alone() {
    assert_eq!(
        endpoint_shown("https://user:pw%40x@collector:4318/v1?token=t#frag"),
        "https://collector:4318/v1"
    );
    assert_eq!(
        endpoint_shown("http://localhost:4318"),
        "http://localhost:4318"
    );
    assert_eq!(endpoint_shown("collector:4317"), "collector:4317");
}
