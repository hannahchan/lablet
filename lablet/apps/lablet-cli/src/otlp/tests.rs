//! Each signal's destination resolved from the config alone: whether
//! anything is exported, over which protocol, to which endpoint, with
//! whose headers, and the TLS material it's given.

use std::ffi::OsString;

use lablet_conformance::receiver::{Mode, Receiver};
use lablet_test_support::Scratch;

use super::*;
use lablet_config::{Format, RawConfig};

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

/// The config `text` holds, with each `${VAR}` filled from `held`.
fn filled(text: &str, held: &[(&str, &str)]) -> Config {
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RawConfig::from_str(text, Format::Yaml)
        .unwrap()
        .substituted(move |asked: &str| -> Option<OsString> {
            held.iter()
                .find(|(name, _)| name == asked)
                .map(|(_, value)| value.into())
        })
        .unwrap()
        .config()
        .unwrap()
}

/// What the config `text` resolves to.
fn resolved(text: &str) -> Result<Option<OtlpSettings>, ConfigError> {
    settings(&config(text))
}

fn on(text: &str) -> OtlpSettings {
    resolved(text).unwrap().unwrap()
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

/// With no endpoint named, nothing goes over the network, whatever else the
/// section states: a file, a protocol, headers or TLS files name no place.
#[test]
fn with_no_endpoint_named_nothing_goes_over_the_network_whatever_else_is_named() {
    for text in [
        "",
        "telemetry: { file: { path: runs.otlp.jsonl } }",
        "telemetry: { otlp: { protocol: grpc, headers: { a: b } } }",
        "telemetry: { otlp: { tls: { ca_file: /no/such/ca.pem } } }",
        "telemetry: { otlp: { endpoint: '' } }",
    ] {
        assert!(matches!(resolved(text), Ok(None)), "{text}");
    }
}

/// A named endpoint sends both signals, over HTTP/protobuf unless the
/// protocol says otherwise, with the specification's timeout and no header.
#[test]
fn a_stated_endpoint_goes_to_both_signals_over_the_stated_protocol() {
    let settings = on("telemetry: { otlp: { endpoint: 'http://collector:4318' } }");

    assert_eq!(
        where_to(&settings),
        [
            (
                Signal::Traces,
                Transport::HttpProtobuf,
                "http://collector:4318/v1/traces".to_owned()
            ),
            (
                Signal::Logs,
                Transport::HttpProtobuf,
                "http://collector:4318/v1/logs".to_owned()
            ),
        ]
    );
    for (_, destination) in settings.destinations() {
        assert_eq!(destination.timeout, Duration::from_secs(10));
        assert!(destination.headers.is_empty());
    }

    let grpc = on("telemetry: { otlp: { protocol: grpc, endpoint: 'http://collector:4317' } }");
    assert_eq!(
        where_to(&grpc),
        [
            (
                Signal::Traces,
                Transport::Grpc,
                "http://collector:4317".to_owned()
            ),
            (
                Signal::Logs,
                Transport::Grpc,
                "http://collector:4317".to_owned()
            ),
        ]
    );
    let json = on("telemetry: { otlp: { protocol: http/json, endpoint: 'http://c:4318/' } }");
    assert_eq!(
        where_to(&json)
            .into_iter()
            .map(|(_, transport, endpoint)| (transport, endpoint))
            .collect::<Vec<_>>(),
        [
            (Transport::HttpJson, "http://c:4318/v1/traces".to_owned()),
            (Transport::HttpJson, "http://c:4318/v1/logs".to_owned()),
        ],
        "a base URL that ends in `/` takes the path without a second"
    );
}

#[test]
fn a_grpc_endpoint_is_given_a_scheme_and_a_url_keeps_its_own() {
    let endpoint = |text: &str| where_to(&on(text))[0].2.clone();

    assert_eq!(
        endpoint("telemetry: { otlp: { protocol: grpc, endpoint: 'collector:4317' } }"),
        "https://collector:4317",
        "the specification's scheme for one written without, since nothing says it's insecure"
    );
    assert_eq!(
        endpoint("telemetry: { otlp: { protocol: grpc, endpoint: 'collector:4317/p://q' } }"),
        "https://collector:4317/p://q",
        "a `://` after a `/` ends no scheme"
    );
    assert_eq!(
        endpoint("telemetry: { otlp: { protocol: grpc, endpoint: 'http://collector:4317' } }"),
        "http://collector:4317"
    );
}

/// An endpoint that is one `${VAR}` and comes out empty states none, as one
/// written empty does.
#[test]
fn an_endpoint_filled_from_an_empty_variable_is_unstated() {
    let unset = filled(
        "telemetry: { otlp: { endpoint: '${COLLECTOR:-}', protocol: '${P:-}' } }",
        &[],
    );
    let empty = filled(
        "telemetry: { otlp: { endpoint: '${COLLECTOR}' } }",
        &[("COLLECTOR", "")],
    );
    let named = filled(
        "telemetry: { otlp: { endpoint: '${COLLECTOR}' } }",
        &[("COLLECTOR", "http://c:4318")],
    );

    assert!(settings(&unset).unwrap().is_none());
    assert!(settings(&empty).unwrap().is_none());
    assert_eq!(
        where_to(&settings(&named).unwrap().unwrap())[0].2,
        "http://c:4318/v1/traces"
    );
}

/// The headers are the config's, in the map form and in OpenTelemetry's
/// `k=v` text, each value decoded, and `{}` names none.
#[test]
fn the_configs_headers_are_all_that_is_sent_in_either_form() {
    let map = on("telemetry: { otlp: { endpoint: 'http://c:4318', headers: { x-config: c } } }");
    let text = on(
        "telemetry: { otlp: { endpoint: 'http://c:4318', headers: 'authorization=Bearer%20t, x-env=1' } }",
    );
    let empty = on("telemetry: { otlp: { endpoint: 'http://c:4318', headers: {} } }");

    for (_, destination) in map.destinations() {
        assert_eq!(header_pairs(destination), [("x-config", "c")]);
    }
    for (_, destination) in text.destinations() {
        assert_eq!(
            header_pairs(destination),
            [("authorization", "Bearer t"), ("x-env", "1")]
        );
    }
    for (_, destination) in empty.destinations() {
        assert!(destination.headers.is_empty());
    }
}

// The TLS material the config names

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

/// What a config that sends to `endpoint` with the `tls` section `tls`
/// resolves to.
fn with_tls_to(endpoint: &str, tls: &str) -> Result<Option<OtlpSettings>, ConfigError> {
    resolved(&format!(
        "telemetry: {{ otlp: {{ endpoint: '{endpoint}', tls: {{ {tls} }} }} }}"
    ))
}

/// What a config that sends over TLS with the `tls` section `tls` resolves
/// to.
fn over_tls(tls: &str) -> Result<Option<OtlpSettings>, ConfigError> {
    with_tls_to("https://collector.internal:4318", tls)
}

#[tokio::test]
async fn the_ca_and_the_client_identity_are_read_for_an_endpoint_that_speaks_tls() {
    let material = Material::new("otlp-tls-read").await;
    let (ca, certificate, key) = (
        material.at("ca.pem"),
        material.at("client.pem"),
        material.at("client.key"),
    );

    let settings = over_tls(&format!(
        "ca_file: '{ca}', client_cert: '{certificate}', client_key: '{key}'"
    ))
    .unwrap()
    .unwrap();

    for (_, destination) in settings.destinations() {
        assert!(destination.tls.roots.is_some());
        let identity = destination.tls.identity.as_ref().unwrap();
        assert_eq!(identity.key.expose_secret(), material.key);
    }
    assert_eq!(
        settings
            .client_key()
            .map(secrecy::ExposeSecret::expose_secret),
        Some(material.key.as_str())
    );
    assert!(
        !format!("{settings:?}").contains("PRIVATE KEY"),
        "the key is shown by no `Debug`"
    );
    let plain = on("telemetry: { otlp: { endpoint: 'http://c:4318' } }");
    assert!(plain.client_key().is_none());
}

/// A file that can't be read or holds no PEM certificates is refused by its
/// key, as the config writes the path, and by the variable that filled it,
/// never by what the file holds.
#[tokio::test]
async fn a_ca_file_that_cannot_be_read_or_holds_none_is_refused_by_its_key_naming_a_variable_that_filled_it()
 {
    let material = Material::new("otlp-tls-refused").await;
    let missing = material.at("missing.pem");
    let nothing = material.at("nothing.pem");

    let unread = over_tls(&format!("ca_file: '{missing}'"))
        .unwrap_err()
        .to_string();
    assert!(
        unread.starts_with(&format!(
            "telemetry.otlp.tls.ca_file (line 1): \"{missing}\" is refused: the file names a file \
             that can't be read: "
        )),
        "{unread}"
    );
    let empty = over_tls(&format!("ca_file: '{nothing}'"))
        .unwrap_err()
        .to_string();
    assert_eq!(
        empty,
        format!(
            "telemetry.otlp.tls.ca_file (line 1): \"{nothing}\" is refused: the file names a file \
             that doesn't hold PEM certificates that can be loaded"
        )
    );

    let from_variable = filled(
        "telemetry: { otlp: { endpoint: 'https://c:4318', tls: { ca_file: '${CA:-}' } } }",
        &[("CA", &missing)],
    );
    let refused = settings(&from_variable).unwrap_err().to_string();
    assert!(
        refused.starts_with(
            "telemetry.otlp.tls.ca_file (line 1): \"${CA:-}\" is refused: `CA` names a file that \
             can't be read: "
        ),
        "{refused}"
    );
    assert!(!refused.contains("missing.pem"), "{refused}");
}

/// PEM whose body isn't DER is split by the client's library without a
/// word, and refused only when a client is made with it, so it's loaded
/// here as a client would load it: a certificate, a key, and a certificate
/// and key that don't belong together are each refused by their keys, and
/// nothing of what the files hold or what the library said.
#[tokio::test]
async fn tls_material_whose_pem_is_whole_and_whose_body_does_not_load_is_refused() {
    let material = Material::new("otlp-tls-not-der").await;
    let (bad_certificate, bad_key) = (material.at("bad.pem"), material.at("bad.key"));
    let (certificate, key) = (material.at("client.pem"), material.at("client.key"));
    let other = Receiver::start(Mode::Answers).await;
    let other_key = material.scratch.write("other.key", other.client_key());
    let other_key = other_key.display().to_string();

    let roots = over_tls(&format!("ca_file: '{bad_certificate}'"))
        .unwrap_err()
        .to_string();
    let identities = [
        (&certificate, &bad_key),
        (&bad_certificate, &key),
        (&certificate, &other_key),
    ]
    .map(|(certificate, key)| {
        over_tls(&format!(
            "client_cert: '{certificate}', client_key: '{key}'"
        ))
        .unwrap_err()
        .to_string()
    });

    assert!(
        roots.ends_with(
            "is refused: the file names a file that doesn't hold PEM certificates that can be \
             loaded"
        ),
        "{roots}"
    );
    for refused in identities {
        assert!(
            refused.starts_with("telemetry.otlp.tls.client_cert (line 1): ")
                && refused.ends_with(
                    "is refused: the file and `telemetry.otlp.tls.client_key` name files that \
                     don't hold a PEM client certificate and its private key that can be loaded"
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
async fn a_client_key_without_its_certificate_is_refused_and_the_reverse() {
    let material = Material::new("otlp-tls-alone").await;
    let (certificate, key) = (material.at("client.pem"), material.at("client.key"));

    let key_alone = over_tls(&format!("client_key: '{key}'"))
        .unwrap_err()
        .to_string();
    let certificate_alone = over_tls(&format!("client_cert: '{certificate}'"))
        .unwrap_err()
        .to_string();
    let mismatched = over_tls(&format!(
        "client_cert: '{certificate}', client_key: '{}'",
        material.at("nothing.pem")
    ))
    .unwrap_err()
    .to_string();

    assert_eq!(
        key_alone,
        format!(
            "telemetry.otlp.tls.client_key (line 1): \"{key}\" is refused: a client certificate \
             and its key land together, and `telemetry.otlp.tls.client_cert` isn't stated; state \
             it beside"
        )
    );
    assert_eq!(
        certificate_alone,
        format!(
            "telemetry.otlp.tls.client_cert (line 1): \"{certificate}\" is refused: a client \
             certificate and its key land together, and `telemetry.otlp.tls.client_key` isn't \
             stated; state it beside"
        )
    );
    assert!(
        mismatched.ends_with(
            "name files that don't hold a PEM client certificate and its private key that can be \
             loaded"
        ),
        "{mismatched}"
    );
    assert!(!mismatched.contains("no PEM here"));
}

/// TLS files are read and loaded whenever an endpoint is named, even one
/// that doesn't speak TLS, where they change nothing: one that can't be
/// loaded is refused, and a client key's contents are held to be cut.
#[tokio::test]
async fn tls_files_for_an_endpoint_that_does_not_speak_tls_are_loaded_and_the_key_held() {
    let material = Material::new("otlp-tls-plain").await;
    let (certificate, key) = (material.at("client.pem"), material.at("client.key"));
    let plain = "http://collector.internal:4318";

    let missing = with_tls_to(
        plain,
        &format!("client_cert: '{certificate}', client_key: /no/such/client.key"),
    )
    .unwrap_err()
    .to_string();
    let settings = with_tls_to(
        plain,
        &format!("client_cert: '{certificate}', client_key: '{key}'"),
    )
    .unwrap()
    .unwrap();

    assert!(
        missing.contains("telemetry.otlp.tls.client_key (line 1)")
            && missing.contains("the file names a file that can't be read"),
        "{missing}"
    );
    assert_eq!(
        settings
            .client_key()
            .map(secrecy::ExposeSecret::expose_secret),
        Some(material.key.as_str())
    );
}
