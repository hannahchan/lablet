//! Each signal's destination resolved against an environment the test
//! states, since a test can't set a variable of its own process: whether
//! it's exported, over which protocol, to which endpoint, with whose
//! headers, and the TLS material it's given.

use std::ffi::OsString;

use lablet_conformance::receiver::{Mode, Receiver};
use lablet_test_support::Scratch;

use super::*;
use crate::config::Format;
use crate::otel_env::{Exporter, OtelEnv};

/// A collector the environment names, which no message may show.
const FROM_THE_ENVIRONMENT: &str = "http://collector.internal:4318";

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

/// What the config `text` resolves to in an environment that holds `held`,
/// with `written` and `real` one config, since nothing here substitutes a
/// variable.
fn resolved(text: &str, held: &[(&str, &str)]) -> Result<Option<OtlpSettings>, ConfigError> {
    let config = config(text);
    let exporter = OtelEnv::read(&env(held)).exporter;
    settings(&config, &config, &exporter)
}

fn on(text: &str, held: &[(&str, &str)]) -> OtlpSettings {
    resolved(text, held).unwrap().unwrap()
}

/// Which signals the config `text` exports over the network in `held`.
fn exported(text: &str, held: &[(&str, &str)]) -> (bool, bool) {
    resolved(text, held)
        .unwrap()
        .map_or((false, false), |settings| {
            (settings.traces.is_some(), settings.logs.is_some())
        })
}

/// Each signal's protocol and endpoint.
fn where_to(settings: &OtlpSettings) -> Vec<(Signal, Transport, String)> {
    settings
        .destinations()
        .map(|(signal, destination)| {
            (
                signal,
                destination.transport,
                destination.endpoint.expose_secret().to_owned(),
            )
        })
        .collect()
}

fn header_pairs(destination: &Destination) -> Vec<(&str, &str)> {
    destination
        .headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect()
}

#[test]
fn with_nothing_stated_or_set_both_signals_go_over_http_protobuf_to_localhost_4318() {
    let settings = on("", &[]);

    assert_eq!(
        where_to(&settings),
        [
            (
                Signal::Traces,
                Transport::HttpProtobuf,
                "http://localhost:4318/v1/traces".to_owned()
            ),
            (
                Signal::Logs,
                Transport::HttpProtobuf,
                "http://localhost:4318/v1/logs".to_owned()
            ),
        ]
    );
    for (_, destination) in settings.destinations() {
        assert_eq!(destination.timeout, Duration::from_secs(10));
        assert!(!destination.gzip);
        assert!(destination.headers.is_empty());
        assert_eq!(destination.endpoint_variable, None);
    }
}

#[test]
fn a_named_file_and_no_endpoint_still_sends_to_localhost_4318() {
    let settings = on("telemetry: { file: { path: runs.otlp.jsonl } }", &[]);

    assert_eq!(
        where_to(&settings)
            .into_iter()
            .map(|(_, _, endpoint)| endpoint)
            .collect::<Vec<_>>(),
        [
            "http://localhost:4318/v1/traces",
            "http://localhost:4318/v1/logs"
        ]
    );
}

#[test]
fn the_grpc_default_is_localhost_4317() {
    let settings = on("telemetry: { otlp: { protocol: grpc } }", &[]);

    assert_eq!(
        where_to(&settings)[0],
        (
            Signal::Traces,
            Transport::Grpc,
            "http://localhost:4317".to_owned()
        )
    );
}

#[test]
fn an_endpoint_no_longer_decides_whether_the_exporter_is_on() {
    let both_none = [
        ("OTEL_TRACES_EXPORTER", "none"),
        ("OTEL_LOGS_EXPORTER", "none"),
    ];
    assert_eq!(exported("", &[]), (true, true), "on without an endpoint");
    assert_eq!(
        exported(
            "telemetry: { otlp: { endpoint: 'http://localhost:4318' } }",
            &both_none
        ),
        (false, false),
        "and off beside one"
    );
    assert!(matches!(resolved("", &both_none), Ok(None)));
}

#[test]
fn each_signal_takes_its_own_exporter_selector() {
    assert_eq!(
        exported("", &[("OTEL_TRACES_EXPORTER", "none")]),
        (false, true)
    );
    assert_eq!(
        exported("", &[("OTEL_LOGS_EXPORTER", "none")]),
        (true, false)
    );
    assert_eq!(
        exported("", &[("OTEL_TRACES_EXPORTER", "console")]),
        (true, true),
        "a list naming nothing lablet serves is as if unset"
    );
}

#[test]
fn enabled_stated_wins_over_both_selectors() {
    let both_none = [
        ("OTEL_TRACES_EXPORTER", "none"),
        ("OTEL_LOGS_EXPORTER", "none"),
    ];
    let both_otlp = [
        ("OTEL_TRACES_EXPORTER", "otlp"),
        ("OTEL_LOGS_EXPORTER", "otlp"),
    ];

    assert_eq!(
        exported("telemetry: { otlp: { enabled: true } }", &both_none),
        (true, true)
    );
    assert_eq!(
        exported("telemetry: { otlp: { enabled: false } }", &both_otlp),
        (false, false)
    );
    assert_eq!(
        exported("telemetry: { otlp: { enabled: null } }", &both_none),
        (false, false)
    );
}

#[test]
fn otel_sdk_disabled_wins_over_a_config_that_turns_the_exporter_on() {
    assert!(matches!(
        resolved(
            "telemetry: { otlp: { enabled: true, endpoint: 'http://localhost:4318' } }",
            &[("OTEL_SDK_DISABLED", "true")]
        ),
        Ok(None)
    ));
}

#[test]
fn enabled_false_beside_an_endpoint_is_refused_as_a_setting_without_effect() {
    let written = config("telemetry: { otlp: { enabled: false, endpoint: '${COLLECTOR}' } }");
    let held = env(&[("COLLECTOR", FROM_THE_ENVIRONMENT)]);
    let real = written.substituted(&held).unwrap();

    let refused = settings(&written, &real, &Exporter::default()).unwrap_err();

    assert_eq!(
        refused.to_string(),
        "telemetry.otlp.enabled (line 1): false is refused: it turns the network exporter off, \
         and `telemetry.otlp.endpoint: ${COLLECTOR}` names where it sends; a config that wants \
         it off states no endpoint"
    );
    assert!(
        !refused.to_string().contains("collector.internal"),
        "{refused}"
    );
}

#[test]
fn enabled_false_beside_an_endpoint_of_nothing_is_a_config_that_states_no_endpoint() {
    assert_eq!(
        exported("telemetry: { otlp: { enabled: false, endpoint: '' } }", &[]),
        (false, false)
    );
}

#[test]
fn each_signal_takes_its_own_protocol_variable_before_the_generic_one() {
    let settings = on(
        "",
        &[
            ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"),
            ("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "http/json"),
        ],
    );

    assert_eq!(
        where_to(&settings),
        [
            (
                Signal::Traces,
                Transport::Grpc,
                "http://localhost:4317".to_owned()
            ),
            (
                Signal::Logs,
                Transport::HttpJson,
                "http://localhost:4318/v1/logs".to_owned()
            ),
        ]
    );
}

#[test]
fn the_protocol_the_config_states_wins_for_both_signals() {
    let settings = on(
        "telemetry: { otlp: { protocol: http/json } }",
        &[
            ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"),
            ("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "grpc"),
        ],
    );

    for (_, destination) in settings.destinations() {
        assert_eq!(destination.transport, Transport::HttpJson);
    }
}

#[test]
fn an_unknown_protocol_is_ignored_and_the_next_source_decides() {
    let settings = on(
        "",
        &[
            ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"),
            ("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "http/thrift"),
        ],
    );

    assert_eq!(where_to(&settings)[0].1, Transport::Grpc);
}

#[test]
fn every_endpoint_is_stated_with_its_scheme() {
    let endpoint = |text: &str, held: &[(&str, &str)]| where_to(&on(text, held))[0].2.clone();
    let grpc = "telemetry: { otlp: { protocol: grpc, endpoint: 'collector:4317' } }";

    assert_eq!(endpoint(grpc, &[]), "https://collector:4317");
    assert_eq!(
        endpoint(grpc, &[("OTEL_EXPORTER_OTLP_INSECURE", "true")]),
        "http://collector:4317"
    );
    assert_eq!(
        endpoint(
            "telemetry: { otlp: { protocol: grpc } }",
            &[("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", "collector:4317")]
        ),
        "https://collector:4317"
    );
    assert_eq!(
        endpoint(
            "telemetry: { otlp: { protocol: grpc, endpoint: 'collector:4317/p://q' } }",
            &[]
        ),
        "https://collector:4317/p://q",
        "a `://` after a `/` ends no scheme"
    );
    assert_eq!(
        endpoint(
            "telemetry: { otlp: { protocol: grpc, endpoint: 'http://collector:4317' } }",
            &[]
        ),
        "http://collector:4317"
    );
}

#[test]
fn an_empty_signal_insecure_variable_leaves_the_generic_one() {
    let settings = on(
        "telemetry: { otlp: { protocol: grpc, endpoint: 'collector:4317' } }",
        &[
            ("OTEL_EXPORTER_OTLP_INSECURE", "true"),
            ("OTEL_EXPORTER_OTLP_TRACES_INSECURE", ""),
            ("OTEL_EXPORTER_OTLP_LOGS_INSECURE", "false"),
        ],
    );

    assert_eq!(
        where_to(&settings)
            .into_iter()
            .map(|(_, _, endpoint)| endpoint)
            .collect::<Vec<_>>(),
        ["http://collector:4317", "https://collector:4317"]
    );
}

#[test]
fn the_configs_endpoint_is_a_base_url_on_http_and_the_generic_variable_one_too() {
    let config = on(
        "telemetry: { otlp: { endpoint: 'http://collector:4318/' } }",
        &[],
    );
    let variables = on(
        "",
        &[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://generic:4318/otlp"),
            (
                "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
                "http://logs:4318/custom",
            ),
        ],
    );

    assert_eq!(
        where_to(&config)
            .into_iter()
            .map(|(_, _, endpoint)| endpoint)
            .collect::<Vec<_>>(),
        [
            "http://collector:4318/v1/traces",
            "http://collector:4318/v1/logs"
        ]
    );
    assert_eq!(
        where_to(&variables)
            .into_iter()
            .map(|(_, _, endpoint)| endpoint)
            .collect::<Vec<_>>(),
        [
            "http://generic:4318/otlp/v1/traces",
            "http://logs:4318/custom"
        ],
        "a signal's own variable is taken as it's written"
    );
    assert_eq!(
        variables.endpoint_variables(),
        [
            Some("OTEL_EXPORTER_OTLP_ENDPOINT"),
            Some("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT")
        ]
    );
    assert_eq!(config.endpoint_variables(), [None, None]);
}

#[test]
fn the_configs_endpoint_wins_over_every_variable() {
    let settings = on(
        "telemetry: { otlp: { endpoint: 'http://config:4318' } }",
        &[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", FROM_THE_ENVIRONMENT),
            ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", FROM_THE_ENVIRONMENT),
        ],
    );

    assert_eq!(where_to(&settings)[0].2, "http://config:4318/v1/traces");
}

#[test]
fn an_endpoint_of_nothing_states_none_whether_written_so_or_given_by_a_variable() {
    let written = config("telemetry: { otlp: { endpoint: '${COLLECTOR}' } }");
    let held = env(&[("COLLECTOR", "")]);
    let real = written.substituted(&held).unwrap();

    let settings = settings(&written, &real, &Exporter::default())
        .unwrap()
        .unwrap();

    assert_eq!(where_to(&settings)[0].2, "http://localhost:4318/v1/traces");
}

#[test]
fn the_environments_headers_go_to_the_configs_endpoint_when_the_config_states_none() {
    let settings = on(
        "telemetry: { otlp: { endpoint: 'http://config:4318' } }",
        &[
            ("OTEL_EXPORTER_OTLP_HEADERS", "authorization=Bearer%20env"),
            ("OTEL_EXPORTER_OTLP_LOGS_HEADERS", "x-logs=own"),
        ],
    );

    let traces = settings.traces.as_ref().unwrap();
    let logs = settings.logs.as_ref().unwrap();
    assert_eq!(header_pairs(traces), [("authorization", "Bearer env")]);
    assert_eq!(header_pairs(logs), [("x-logs", "own")]);
    assert_eq!(
        logs.taken_off,
        ["x-logs", "authorization"],
        "the generic one's names come off before the signal's set goes on"
    );
}

#[test]
fn a_stated_header_map_is_all_that_is_sent() {
    let held = [
        ("OTEL_EXPORTER_OTLP_HEADERS", "authorization=env,x-env=e"),
        ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", "x-traces=t"),
    ];
    let stated = on("telemetry: { otlp: { headers: { x-config: c } } }", &held);
    let empty = on("telemetry: { otlp: { headers: {} } }", &held);

    let traces = stated.traces.as_ref().unwrap();
    assert_eq!(header_pairs(traces), [("x-config", "c")]);
    assert_eq!(traces.taken_off, ["x-traces", "authorization", "x-env"]);
    for (_, destination) in empty.destinations() {
        assert!(destination.headers.is_empty());
        assert!(!destination.taken_off.is_empty());
    }
}

#[test]
fn the_names_taken_off_are_every_name_either_header_variable_sets() {
    let settings = on(
        "telemetry: { otlp: { endpoint: 'http://config:4318', headers: { x-config: c } } }",
        &[
            ("OTEL_EXPORTER_OTLP_HEADERS", "authorization=Bearer%20token"),
            ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", ""),
        ],
    );

    for (signal, destination) in settings.destinations() {
        assert_eq!(destination.taken_off, ["authorization"], "{signal}");
        assert_eq!(header_pairs(destination), [("x-config", "c")], "{signal}");
    }
}

#[test]
fn the_timeout_and_gzip_are_the_environments_for_each_signal() {
    let settings = on(
        "",
        &[
            ("OTEL_EXPORTER_OTLP_TIMEOUT", "2500"),
            ("OTEL_EXPORTER_OTLP_LOGS_COMPRESSION", "gzip"),
        ],
    );

    let traces = settings.traces.as_ref().unwrap();
    let logs = settings.logs.as_ref().unwrap();
    assert_eq!(
        (traces.timeout, traces.gzip),
        (Duration::from_millis(2_500), false)
    );
    assert_eq!(
        (logs.timeout, logs.gzip),
        (Duration::from_millis(2_500), true)
    );
}

// The TLS material the environment names

/// A scratch directory with the receiver's CA, its client's certificate
/// and key, a file that holds no PEM, and a certificate and a key whose PEM
/// is whole but whose bodies are no DER, each as a file of its own.
struct Material {
    scratch: Scratch,
    key: String,
}

impl Material {
    async fn new(test: &str) -> Self {
        let receiver = Receiver::start(Mode::Answers).await;
        let scratch = Scratch::new(test);
        scratch.write("ca.pem", receiver.ca_certificate());
        scratch.write("client.pem", receiver.client_certificate());
        scratch.write("client.key", receiver.client_key());
        scratch.write("nothing.pem", "no PEM here\n");
        scratch.write("bad.pem", NOT_DER_CERTIFICATE);
        scratch.write("bad.key", NOT_DER_KEY);
        Self {
            scratch,
            key: receiver.client_key().to_owned(),
        }
    }

    fn at(&self, name: &str) -> String {
        self.scratch.at(name).display().to_string()
    }
}

/// A certificate whose PEM is whole and whose body isn't DER: a TLS library
/// that only splits PEM takes it, and one that loads it doesn't.
pub(crate) const NOT_DER_CERTIFICATE: &str =
    "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";

/// A private key whose PEM is whole and whose body isn't DER.
pub(crate) const NOT_DER_KEY: &str =
    "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n";

/// What a config that sends over TLS resolves to with `held`.
fn over_tls(held: &[(&str, &str)]) -> Result<Option<OtlpSettings>, ConfigError> {
    resolved(
        "telemetry: { otlp: { endpoint: 'https://collector.internal:4318' } }",
        held,
    )
}

#[tokio::test]
async fn the_certificate_and_the_client_identity_are_read_for_an_endpoint_that_speaks_tls() {
    let material = Material::new("otlp-tls-read").await;
    let (ca, certificate, key) = (
        material.at("ca.pem"),
        material.at("client.pem"),
        material.at("client.key"),
    );

    let settings = over_tls(&[
        ("OTEL_EXPORTER_OTLP_CERTIFICATE", &ca),
        ("OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE", &certificate),
        ("OTEL_EXPORTER_OTLP_LOGS_CLIENT_KEY", &key),
        ("OTEL_EXPORTER_OTLP_TRACES_CLIENT_KEY", &key),
    ])
    .unwrap()
    .unwrap();

    for (_, destination) in settings.destinations() {
        assert!(destination.tls.roots.is_some());
        let identity = destination.tls.identity.as_ref().unwrap();
        assert_eq!(identity.key.expose_secret(), material.key);
    }
    let keys: Vec<&str> = settings
        .client_keys()
        .into_iter()
        .map(|(variable, _)| variable)
        .collect();
    assert_eq!(
        keys,
        [
            "OTEL_EXPORTER_OTLP_TRACES_CLIENT_KEY",
            "OTEL_EXPORTER_OTLP_LOGS_CLIENT_KEY"
        ]
    );
    assert!(
        !format!("{settings:?}").contains("PRIVATE KEY"),
        "the key is shown by no `Debug`"
    );
}

#[tokio::test]
async fn a_certificate_file_that_cannot_be_read_or_holds_none_is_refused_naming_the_variable() {
    let material = Material::new("otlp-tls-refused").await;
    let missing = material.at("missing.pem");
    let nothing = material.at("nothing.pem");

    let unread = over_tls(&[("OTEL_EXPORTER_OTLP_CERTIFICATE", &missing)]).unwrap_err();
    let empty = over_tls(&[("OTEL_EXPORTER_OTLP_TRACES_CERTIFICATE", &nothing)]).unwrap_err();

    assert!(
        unread.to_string().starts_with(
            "telemetry.otlp.endpoint (line 1): its value is refused: TLS to the collector for traces can't be set up: `OTEL_EXPORTER_OTLP_CERTIFICATE` \
             names a file that can't be read: "
        ),
        "{unread}"
    );
    assert!(!unread.to_string().contains("missing.pem"), "{unread}");
    assert!(
        empty.to_string().ends_with(
            "`OTEL_EXPORTER_OTLP_TRACES_CERTIFICATE` names a file that doesn't hold PEM \
             certificates that can be loaded"
        ),
        "{empty}"
    );
}

/// PEM whose body isn't DER is split by the client's library without a
/// word, and refused only when a client is made with it, so it's loaded
/// here as a client would load it: a certificate, a key, and a certificate
/// and key that don't belong together are each refused naming the
/// variables, and nothing of what the files hold or what the library said.
#[tokio::test]
async fn tls_material_whose_pem_is_whole_and_whose_body_does_not_load_is_refused() {
    let material = Material::new("otlp-tls-not-der").await;
    let (bad_certificate, bad_key) = (material.at("bad.pem"), material.at("bad.key"));
    let (certificate, key) = (material.at("client.pem"), material.at("client.key"));
    let other = Receiver::start(Mode::Answers).await;
    let other_key = material.scratch.write("other.key", other.client_key());
    let other_key = other_key.display().to_string();

    let roots = over_tls(&[("OTEL_EXPORTER_OTLP_CERTIFICATE", &bad_certificate)]).unwrap_err();
    let identities = [
        (&certificate, &bad_key),
        (&bad_certificate, &key),
        (&certificate, &other_key),
    ]
    .map(|(certificate, key)| {
        over_tls(&[
            ("OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE", certificate),
            ("OTEL_EXPORTER_OTLP_CLIENT_KEY", key),
        ])
        .unwrap_err()
        .to_string()
    });

    assert!(
        roots.to_string().ends_with(
            "`OTEL_EXPORTER_OTLP_CERTIFICATE` names a file that doesn't hold PEM certificates \
             that can be loaded"
        ),
        "{roots}"
    );
    for refused in identities {
        assert!(
            refused.ends_with(
                "`OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE` and `OTEL_EXPORTER_OTLP_CLIENT_KEY` \
                 name files that don't hold a PEM client certificate and its private key that \
                 can be loaded"
            ),
            "{refused}"
        );
        assert!(
            !refused.contains("AAAA") && !refused.contains("parse"),
            "{refused}"
        );
    }
}

#[tokio::test]
async fn a_client_key_without_its_certificate_is_refused() {
    let material = Material::new("otlp-tls-alone").await;
    let (certificate, key) = (material.at("client.pem"), material.at("client.key"));

    let key_alone = over_tls(&[("OTEL_EXPORTER_OTLP_CLIENT_KEY", &key)]).unwrap_err();
    let certificate_alone =
        over_tls(&[("OTEL_EXPORTER_OTLP_LOGS_CLIENT_CERTIFICATE", &certificate)]).unwrap_err();
    let mismatched = over_tls(&[
        ("OTEL_EXPORTER_OTLP_CLIENT_KEY", &material.at("nothing.pem")),
        ("OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE", &certificate),
    ])
    .unwrap_err();

    assert!(
        key_alone.to_string().ends_with(
            "`OTEL_EXPORTER_OTLP_CLIENT_KEY` names a client key, and no client certificate is \
             named beside it"
        ),
        "{key_alone}"
    );
    assert!(
        certificate_alone.to_string().ends_with(
            "`OTEL_EXPORTER_OTLP_LOGS_CLIENT_CERTIFICATE` names a client certificate, and no \
             client key is named beside it"
        ),
        "{certificate_alone}"
    );
    assert!(
        mismatched.to_string().ends_with(
            "`OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE` and `OTEL_EXPORTER_OTLP_CLIENT_KEY` name \
             files that don't hold a PEM client certificate and its private key that can be \
             loaded"
        ),
        "{mismatched}"
    );
    assert!(!mismatched.to_string().contains("no PEM here"));
}

/// TLS files are read and loaded whenever a signal's exporter is on, even
/// for an endpoint that doesn't speak TLS, where they change nothing: one
/// that can't be loaded is refused, and a client key's contents are held
/// to be cut.
#[tokio::test]
async fn tls_files_for_an_endpoint_that_does_not_speak_tls_are_loaded_and_the_key_held() {
    let material = Material::new("otlp-tls-plain").await;
    let (certificate, key) = (material.at("client.pem"), material.at("client.key"));
    let plain = "telemetry: { otlp: { endpoint: 'http://collector.internal:4318' } }";

    let missing = resolved(
        plain,
        &[
            ("OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE", &certificate),
            ("OTEL_EXPORTER_OTLP_CLIENT_KEY", "/no/such/client.key"),
        ],
    )
    .unwrap_err();
    let settings = on(
        plain,
        &[
            ("OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE", &certificate),
            ("OTEL_EXPORTER_OTLP_CLIENT_KEY", &key),
        ],
    );

    assert!(
        missing
            .to_string()
            .contains("`OTEL_EXPORTER_OTLP_CLIENT_KEY` names a file that can't be read"),
        "{missing}"
    );
    let keys: Vec<&str> = settings
        .client_keys()
        .into_iter()
        .map(|(variable, _)| variable)
        .collect();
    assert_eq!(keys, ["OTEL_EXPORTER_OTLP_CLIENT_KEY"]);
}
