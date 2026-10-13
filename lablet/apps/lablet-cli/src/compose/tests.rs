//! What the command line's check reports beside what the kernels check:
//! where a run's telemetry goes, and which retired variables are set, in an
//! environment the test states, since a test can't set a variable of its
//! own process.

use lablet_config::{Format, RawConfig};

use super::*;

const ENDS: &str = "
- response:
    content:
      - text: Done.
    finish: end_turn
";

/// A config of the fake provider ending at once, with `telemetry` stated
/// beside it, in `scratch`.
fn ending(scratch: &lablet_test_support::Scratch, telemetry: &str) -> Config {
    ending_filled(scratch, telemetry, &[])
}

/// [`ending`], with each `${VAR}` filled from `held`.
fn ending_filled(
    scratch: &lablet_test_support::Scratch,
    telemetry: &str,
    held: &[(&str, &str)],
) -> Config {
    let script = scratch.write("script.yaml", ENDS);
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RawConfig::from_str(
        &format!(
            "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {telemetry}",
            script.display()
        ),
        Format::Yaml,
    )
    .unwrap()
    .substituted(move |asked: &str| {
        held.iter()
            .find(|(name, _)| name == asked)
            .map(|(_, value)| std::ffi::OsString::from(value))
    })
    .unwrap()
    .config()
    .unwrap()
}

/// The report of `config`, in an environment where `set` names are set.
async fn reported(config: &Config, set: &[&str]) -> Report {
    let names: Vec<String> = set.iter().map(|name| (*name).to_owned()).collect();
    let set = move |name: &str| names.iter().any(|set| set == name);
    check_in(config, Vec::new(), &set).await.unwrap()
}

#[tokio::test]
async fn the_report_says_where_a_run_sends_and_writes_and_that_nothing_is_named() {
    let scratch = lablet_test_support::Scratch::new("report-telemetry");

    let nothing = reported(&ending(&scratch, "{}"), &[]).await;
    assert_eq!(nothing.telemetry(), Vec::<String>::new());

    let path = scratch.at("runs.otlp.jsonl").display().to_string();
    let both = reported(
        &ending_filled(
            &scratch,
            &format!(
                "{{ file: {{ path: '{path}' }}, otlp: {{ endpoint: 'http://${{U}}:${{P}}@127.0.0.1:1/base?token=x', protocol: grpc }} }}"
            ),
            &[("U", "user"), ("P", "pw-0123456789abcdef")],
        ),
        &["OTEL_EXPORTER_OTLP_COMPRESSION"],
    )
    .await;
    assert_eq!(
        both.telemetry(),
        [
            "file (line 3)",
            "OTLP grpc to http://127.0.0.1:1/base (line 3)",
            "OTEL_EXPORTER_OTLP_COMPRESSION is set, and the exporter reads it itself",
        ],
        "the endpoint without its user information and its query"
    );

    let file_alone = reported(
        &ending(&scratch, &format!("{{ file: {{ path: '{path}' }} }}")),
        &["OTEL_EXPORTER_OTLP_COMPRESSION"],
    )
    .await;
    assert_eq!(
        file_alone.telemetry(),
        ["file (line 3)"],
        "compression is read only where an endpoint is named"
    );
}

#[tokio::test]
async fn the_report_names_the_set_variables_of_the_roster_that_the_file_maps_nothing_onto() {
    let scratch = lablet_test_support::Scratch::new("report-unread");
    let config = ending(&scratch, "{}");

    let none = reported(&config, &["HOME", "PATH", "TRACEPARENT", "RUST_LOG"]).await;
    assert_eq!(none.unread(), Vec::<Retired>::new());

    let some = reported(
        &config,
        &[
            "ANTHROPIC_API_KEY",
            "OTEL_TRACES_SAMPLER",
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_EXPORTER_OTLP_COMPRESSION",
            "TRACEPARENT",
        ],
    )
    .await;
    let named: Vec<(&str, Option<&str>)> = some
        .unread()
        .iter()
        .map(|retired| (retired.name, retired.field))
        .collect();
    assert_eq!(
        named,
        [
            ("ANTHROPIC_API_KEY", Some("model.api_key")),
            (
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                Some("telemetry.otlp.endpoint")
            ),
            ("OTEL_TRACES_SAMPLER", None),
        ],
        "in the roster's order; the context and compression variables are read, so they're not on it"
    );

    // A variable the file writes `${NAME}` for is read there.
    let script = scratch.write("script.yaml", ENDS);
    let mapped = lablet_config::RawConfig::from_str(
        &format!(
            "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {{ otlp: {{ endpoint: '${{OTEL_EXPORTER_OTLP_ENDPOINT:-}}' }} }}",
            script.display()
        ),
        Format::Yaml,
    )
    .unwrap()
    .substituted(|_| None)
    .unwrap()
    .config()
    .unwrap();
    let mapped = reported(
        &mapped,
        &["OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_SDK_DISABLED"],
    )
    .await;
    assert_eq!(
        mapped
            .unread()
            .iter()
            .map(|retired| retired.name)
            .collect::<Vec<_>>(),
        ["OTEL_SDK_DISABLED"]
    );
}
