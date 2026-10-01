//! The network exporter settled against an environment the test states,
//! since a test can't set a variable of its own process: what turns it on
//! and off, which transport, and whose headers go with it.

use std::ffi::OsString;

use super::*;
use crate::config::Format;

/// A collector the environment names, which no message may show.
const FROM_THE_ENVIRONMENT: &str = "http://collector.internal:4317";

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

/// An environment that holds what `held` says, and nothing else.
fn env(held: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    move |asked| {
        held.iter()
            .find(|(name, _)| name == asked)
            .map(|(_, value)| value.into())
    }
}

fn nothing(_: &str) -> Option<OsString> {
    None
}

/// What the config `text` settles to in `held`, with `written` and `real`
/// one config, since nothing here substitutes a variable.
fn settled(
    text: &str,
    held: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Option<OtlpSettings>, Refusal> {
    let config = config(text);
    settings(&config, &config, held)
}

fn on(text: &str, held: &dyn Fn(&str) -> Option<OsString>) -> OtlpSettings {
    settled(text, held).unwrap().unwrap()
}

/// Whether the config `text` has no network exporter in `held`.
fn off(text: &str, held: &dyn Fn(&str) -> Option<OsString>) -> bool {
    matches!(settled(text, held), Ok(None))
}

#[test]
fn an_endpoint_the_config_states_turns_the_exporter_on_whatever_the_environment_says() {
    let on = on(
        "telemetry: { otlp: { endpoint: 'http://localhost:4317', headers: { a: b } } }",
        &env(&[("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT)]),
    );

    assert_eq!(on.endpoint.as_deref(), Some("http://localhost:4317"));
    assert!(
        on.strip_environment_headers,
        "the environment's headers aren't sent to the config's endpoint"
    );
    assert_eq!(on.headers, [("a".to_owned(), "b".to_owned())]);
    assert_eq!(on.transport, Transport::Grpc);
}

#[test]
fn an_endpoint_variable_turns_the_exporter_on_with_the_endpoint_left_to_the_exporter() {
    for variable in ENDPOINT_VARIABLES {
        let on = on(
            "telemetry: { otlp: { headers: { a: b } } }",
            &env(&[(variable, FROM_THE_ENVIRONMENT)]),
        );

        assert_eq!(on.endpoint, None, "{variable}");
        assert!(
            !on.strip_environment_headers,
            "{variable}: the environment's headers go to the environment's endpoint"
        );
        assert_eq!(on.headers, [("a".to_owned(), "b".to_owned())]);
    }
}

#[test]
fn without_an_endpoint_from_either_the_exporter_is_off() {
    assert!(off("", &nothing));
    assert!(
        off("", &env(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "")])),
        "a variable set to nothing is unset, as the exporter reads it"
    );
    assert!(
        off(
            "",
            &env(&[
                ("OTEL_EXPORTER_OTLP_HEADERS", "a=b"),
                ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json"),
            ])
        ),
        "with the exporter off, nothing else of the environment is read"
    );
}

#[test]
fn enabled_false_turns_the_exporter_off_whatever_the_environment_says() {
    assert!(off(
        "telemetry: { otlp: { enabled: false } }",
        &env(&[("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT)])
    ));
}

#[test]
fn enabled_false_beside_an_endpoint_is_refused_as_a_setting_without_effect() {
    let refused = settled(
        "telemetry: { otlp: { enabled: false, endpoint: '${COLLECTOR}' } }",
        &env(&[("COLLECTOR", FROM_THE_ENVIRONMENT)]),
    )
    .unwrap_err();

    assert_eq!(
        refused,
        Refusal::invalid(
            "telemetry.otlp.enabled",
            "it turns the network exporter off, and `telemetry.otlp.endpoint: ${COLLECTOR}` \
             turns it on; a config that wants it off states no endpoint"
        )
    );
}

#[test]
fn otel_traces_exporter_none_turns_the_exporter_off_and_any_other_value_leaves_it_on() {
    let with_endpoint = "telemetry: { otlp: { endpoint: 'http://localhost:4317' } }";
    for none in ["none", "NONE", " None "] {
        assert!(
            off(with_endpoint, &env(&[("OTEL_TRACES_EXPORTER", none)])),
            "{none:?}: off even beside the config's endpoint"
        );
        assert!(
            off(
                "",
                &env(&[
                    ("OTEL_TRACES_EXPORTER", none),
                    ("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT),
                ])
            ),
            "{none:?}"
        );
    }
    for other in ["otlp", "console", "", "none,otlp"] {
        let on = on(
            "",
            &env(&[
                ("OTEL_TRACES_EXPORTER", other),
                ("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT),
            ]),
        );
        assert_eq!(on.endpoint, None, "{other:?}");
    }
}

#[test]
fn the_protocol_the_config_states_wins_and_the_environments_is_read_only_when_it_states_none() {
    let with = |protocol: &str| {
        format!("telemetry: {{ otlp: {{ endpoint: 'http://h:1', protocol: {protocol} }} }}")
    };
    let http = env(&[("OTEL_EXPORTER_OTLP_PROTOCOL", "http/protobuf")]);
    let json = env(&[("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json")]);

    assert_eq!(on(&with("grpc"), &http).transport, Transport::Grpc);
    assert_eq!(
        on(&with("http"), &nothing).transport,
        Transport::HttpProtobuf
    );
    assert_eq!(
        on(&with("grpc"), &json).transport,
        Transport::Grpc,
        "a stated protocol leaves the variable unread"
    );

    assert_eq!(on(&with("null"), &nothing).transport, Transport::Grpc);
    assert_eq!(on(&with("null"), &http).transport, Transport::HttpProtobuf);
    for (value, transport) in [
        ("grpc", Transport::Grpc),
        ("GRPC", Transport::Grpc),
        (" HTTP/Protobuf ", Transport::HttpProtobuf),
        ("", Transport::Grpc),
        ("  ", Transport::Grpc),
    ] {
        let held = env(&[("OTEL_EXPORTER_OTLP_PROTOCOL", value)]);
        assert_eq!(on(&with("null"), &held).transport, transport, "{value:?}");
    }
}

#[test]
fn a_protocol_from_the_environment_that_lablet_cannot_send_is_refused_naming_the_variable() {
    for value in ["http/json", "websocket", "http"] {
        let refused = settled(
            "",
            &env(&[
                ("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT),
                ("OTEL_EXPORTER_OTLP_PROTOCOL", value),
            ]),
        )
        .unwrap_err();

        assert_eq!(
            refused,
            Refusal::invalid(
                "telemetry.otlp.protocol",
                format!(
                    "`OTEL_EXPORTER_OTLP_PROTOCOL` holds {value:?}, which lablet can't send; it \
                     sends `grpc` and `http/protobuf`"
                )
            ),
            "{value}"
        );
        assert!(
            !format!("{refused:?}").contains("collector.internal"),
            "the endpoint variable's value is in no message: {refused:?}"
        );
    }
}

/// The refusal as a config shows it, which is how `check` prints it: the
/// key, with no place when the config states nothing there, and `null`.
#[test]
fn the_refused_protocol_is_shown_under_its_key_as_the_config_writes_it() {
    let written = config("telemetry: { otlp: { headers: { a: b } } }");
    let held = env(&[
        ("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT),
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json"),
    ]);

    let refused = settings(&written, &written, &held).unwrap_err();

    assert_eq!(
        written.refused(refused).to_string(),
        "telemetry.otlp.protocol: null is refused: `OTEL_EXPORTER_OTLP_PROTOCOL` holds \
         \"http/json\", which lablet can't send; it sends `grpc` and `http/protobuf`"
    );
}

/// An endpoint of nothing states none: it turns the exporter on no more
/// than a null one, and with the environment's endpoint beside it the
/// environment's headers go with that endpoint, as they do when the config
/// states none.
#[test]
fn an_endpoint_of_nothing_states_none_whether_written_so_or_given_by_a_variable() {
    let written = config("telemetry: { otlp: { endpoint: '' } }");
    assert!(off("telemetry: { otlp: { endpoint: '' } }", &nothing));
    let on = settings(
        &written,
        &written,
        &env(&[("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT)]),
    )
    .unwrap()
    .unwrap();
    assert_eq!(on.endpoint, None);
    assert!(
        !on.strip_environment_headers,
        "the environment's headers go to the environment's endpoint"
    );

    let written = config("telemetry: { otlp: { endpoint: '${COLLECTOR}' } }");
    let held = env(&[("COLLECTOR", "")]);
    let real = written.substituted(&held).unwrap();
    assert!(matches!(settings(&written, &real, &held), Ok(None)));
    let held = env(&[
        ("COLLECTOR", ""),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT),
    ]);
    let real = written.substituted(&held).unwrap();
    let on = settings(&written, &real, &held).unwrap().unwrap();
    assert_eq!(on.endpoint, None);
    assert!(!on.strip_environment_headers);
}

#[test]
fn enabled_false_beside_an_endpoint_of_nothing_is_a_config_that_states_no_endpoint() {
    assert!(off(
        "telemetry: { otlp: { enabled: false, endpoint: '' } }",
        &env(&[("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT)])
    ));
}

/// The variable a refusal of the environment's endpoint names: the
/// signal's own when it's set to something, else the generic one, which is
/// the order the exporter reads them in.
#[test]
fn the_variable_at_fault_is_the_signals_own_when_set_to_something_else_the_generic_one() {
    let both = env(&[
        ("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT),
        ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", "http://[not a host"),
    ]);
    assert_eq!(
        endpoint_variable(Signal::Traces, &both),
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"
    );
    assert_eq!(
        endpoint_variable(Signal::Logs, &both),
        "OTEL_EXPORTER_OTLP_ENDPOINT"
    );

    let empty_own = env(&[("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", "")]);
    assert_eq!(
        endpoint_variable(Signal::Logs, &empty_own),
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "a variable set to nothing is unset, as the exporter reads it"
    );
    assert_eq!(
        endpoint_variable(Signal::Traces, &nothing),
        "OTEL_EXPORTER_OTLP_ENDPOINT"
    );
}
