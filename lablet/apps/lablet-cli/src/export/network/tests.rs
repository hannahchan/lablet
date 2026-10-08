use std::time::Duration;

use reqwest::header::HeaderMap;
use tonic::metadata::MetadataValue;

use super::*;

/// What `transport` sends spans to `endpoint` with, `headers` among it,
/// and nothing else.
fn destination(transport: Transport, endpoint: &str, headers: &[(&str, &str)]) -> Destination {
    Destination {
        transport,
        endpoint: SecretString::from(endpoint),
        endpoint_variable: None,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
            .collect(),
        taken_off: Vec::new(),
        timeout: Duration::from_secs(10),
        gzip: false,
        tls: Tls::default(),
    }
}

/// Spans alone, sent to `destination`.
fn spans_to(destination: Destination) -> OtlpSettings {
    OtlpSettings {
        traces: Some(destination),
        logs: None,
    }
}

const TRANSPORTS: [Transport; 3] = [
    Transport::Grpc,
    Transport::HttpProtobuf,
    Transport::HttpJson,
];

/// The headers of an HTTP exporter that sets `set` and takes off
/// `taken_off`.
fn headers(set: &[(&str, &str)], taken_off: &[&str]) -> Headers {
    Headers {
        own: &HTTP_OWN,
        set: set
            .iter()
            .map(|(name, value)| {
                (
                    HeaderName::from_str(name).unwrap(),
                    SecretString::from(*value),
                )
            })
            .collect(),
        taken_off: taken_off
            .iter()
            .map(|name| HeaderName::from_str(name).unwrap())
            .collect(),
    }
}

fn values<'a>(map: &'a HeaderMap, name: &str) -> Vec<&'a str> {
    map.get_all(name)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect()
}

// Replacing the headers the exporter merged

/// What the exporter merged from the process environment is replaced, not
/// corrected, so a header lablet didn't resolve goes whatever its name.
#[test]
fn the_resolved_headers_replace_every_value_of_their_name_and_only_the_exporters_own_stay() {
    let headers = headers(&[("authorization", "Bearer config")], &[]);
    let mut map = HeaderMap::new();
    map.append("authorization", "Bearer env".parse().unwrap());
    map.append("authorization", "Bearer code".parse().unwrap());
    map.append("x-unread", "env".parse().unwrap());
    map.append("content-type", "application/x-protobuf".parse().unwrap());
    map.append("user-agent", "exporter".parse().unwrap());

    headers.apply(&mut map);

    assert_eq!(values(&map, "authorization"), ["Bearer config"]);
    assert!(map.get("x-unread").is_none());
    assert_eq!(values(&map, "content-type"), ["application/x-protobuf"]);
    assert_eq!(values(&map, "user-agent"), ["exporter"]);
    assert!(
        map.get("authorization").unwrap().is_sensitive(),
        "the value is marked sensitive, so a `Debug` of the map hides it"
    );
}

/// One of the exporter's own names the environment sets is taken off too,
/// since the exporter's merge left the environment's value under it.
#[test]
fn the_environments_headers_are_taken_off_and_the_configs_put_on() {
    let headers = headers(&[("x-config", "yes")], &["authorization", "user-agent"]);
    let mut map = HeaderMap::new();
    map.insert("authorization", "Bearer env".parse().unwrap());
    map.insert("user-agent", "env".parse().unwrap());
    map.insert("content-encoding", "gzip".parse().unwrap());

    headers.apply(&mut map);

    let mut names: Vec<_> = map.keys().map(HeaderName::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["content-encoding", "x-config"]);
}

#[test]
fn a_header_the_config_states_and_the_environment_names_is_the_configs() {
    let headers = headers(&[("authorization", "Bearer config")], &["authorization"]);
    let mut map = HeaderMap::new();
    map.insert("authorization", "Bearer env".parse().unwrap());

    headers.apply(&mut map);

    assert_eq!(values(&map, "authorization"), ["Bearer config"]);
}

#[test]
fn the_grpc_interceptor_sets_the_headers_on_the_requests_metadata() {
    let mut interceptor = interceptor(Arc::new(Headers {
        own: &GRPC_OWN,
        ..headers(&[("authorization", "Bearer config")], &["x-env"])
    }));
    let mut request = tonic::Request::new(());
    request
        .metadata_mut()
        .append("authorization", MetadataValue::from_static("Bearer env"));
    request
        .metadata_mut()
        .append("x-env", MetadataValue::from_static("env"));
    request
        .metadata_mut()
        .append("x-other", MetadataValue::from_static("env"));
    request
        .metadata_mut()
        .append("user-agent", MetadataValue::from_static("exporter"));
    request
        .metadata_mut()
        .append("content-type", MetadataValue::from_static("env"));

    let request = interceptor(request).unwrap();

    let metadata = request.metadata();
    let authorization: Vec<_> = metadata
        .get_all("authorization")
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    assert_eq!(authorization, ["Bearer config"]);
    assert_eq!(metadata.get("x-env"), None);
    assert_eq!(metadata.get("x-other"), None);
    assert_eq!(metadata.get("content-type"), None);
    assert_eq!(metadata.get("user-agent").unwrap(), "exporter");
}

// What's refused, and what a refusal says

#[test]
fn a_header_whose_name_or_value_is_not_a_headers_is_refused_without_its_value() {
    let bad_name = validate(&spans_to(destination(
        Transport::Grpc,
        "http://127.0.0.1:1",
        &[("not a name", "v")],
    )));
    let bad_value = validate(&spans_to(destination(
        Transport::HttpProtobuf,
        "http://127.0.0.1:1/v1/traces",
        &[("x-token", "has\nnewline")],
    )));

    assert_eq!(
        bad_name,
        Err(OtelBuildError::Header {
            name: "not a name".to_owned(),
            reason: "its name isn't one a header may have",
        })
    );
    let bad_value = bad_value.unwrap_err();
    assert_eq!(
        bad_value.to_string(),
        "the OTLP header `x-token` is refused: its value isn't one a header may have"
    );
    assert!(!bad_value.to_string().contains("newline"));
}

#[tokio::test]
async fn an_endpoint_the_exporter_refuses_is_refused_without_the_endpoint_in_the_message() {
    for transport in TRANSPORTS {
        let settings = spans_to(destination(
            transport,
            "http://secret:token@[not a host",
            &[],
        ));

        let refused = exporters(&settings).err().unwrap();

        assert_eq!(
            refused,
            OtelBuildError::Endpoint {
                signal: Signal::Traces
            },
            "{transport:?}"
        );
        let message = refused.to_string();
        assert!(!message.contains("secret"), "{message}");
        assert!(!message.contains("token"), "{message}");
        assert_eq!(
            message,
            "the OTLP endpoint for traces is refused: it isn't a URL the exporter accepts"
        );
    }
}

#[tokio::test]
async fn the_exporters_of_every_protocol_are_made_from_an_endpoint_nothing_listens_on() {
    for transport in TRANSPORTS {
        let settings = OtlpSettings {
            traces: Some(destination(
                transport,
                "http://127.0.0.1:1",
                &[("x-token", "a value")],
            )),
            logs: Some(Destination {
                gzip: true,
                ..destination(transport, "http://127.0.0.1:1", &[])
            }),
        };

        let (spans, records) = exporters(&settings).unwrap();

        assert!(spans.is_some() && records.is_some(), "{transport:?}");
    }
}

#[tokio::test]
async fn a_signal_with_no_destination_has_no_exporter() {
    let (spans, records) = exporters(&spans_to(destination(
        Transport::Grpc,
        "http://127.0.0.1:1",
        &[],
    )))
    .unwrap();

    assert!(spans.is_some());
    assert!(records.is_none());
}

#[tokio::test]
async fn a_grpc_endpoint_over_tls_trusts_the_certificates_it_is_given_without_the_platforms() {
    // A certificate that's no PEM a TLS library reads, which the platform's
    // roots would never be asked about: TLS that took it would fail here.
    let tls = Tls {
        roots: Some(
            b"-----BEGIN CERTIFICATE-----\nnot base64\n-----END CERTIFICATE-----\n".to_vec(),
        ),
        identity: None,
    };
    let settings = spans_to(Destination {
        tls,
        ..destination(Transport::Grpc, "https://collector.internal:4317", &[])
    });

    let refused = exporters(&settings).err().unwrap();

    assert!(
        matches!(
            refused,
            OtelBuildError::Tls {
                signal: Signal::Traces,
                ..
            }
        ),
        "{refused:?}"
    );
}

#[test]
fn the_settings_print_neither_an_endpoint_nor_a_headers_value() {
    let settings = spans_to(destination(
        Transport::Grpc,
        "http://user:hunter2hunter2@collector:4317",
        &[("authorization", "Bearer hunter2hunter2")],
    ));

    let shown = format!("{settings:?}");

    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(!shown.contains("collector"), "{shown}");
    assert!(shown.contains("\"authorization\""), "{shown}");
}

// Holding the settings to what the exporters take, making none

/// Endpoints the exporters take, ones they refuse, and ones one transport
/// takes and the other refuses. Each has its scheme, as every endpoint
/// lablet resolves does, but the ones a test writes without to show HTTP
/// refuses them.
const ENDPOINTS: [&str; 8] = [
    "http://127.0.0.1:1",
    "https://collector.internal:4317/",
    "collector.internal:4317",
    "http://collector.internal:4317/otlp",
    "http://user:hunter2hunter2@collector.internal:4317",
    "http://[not a host",
    "::::not-a-url",
    "unix:///run/otel.sock",
];

fn validated(transport: Transport, endpoint: &str) -> Result<(), OtelBuildError> {
    validate(&spans_to(destination(transport, endpoint, &[])))
}

#[tokio::test]
async fn the_check_of_an_endpoint_answers_as_the_exporters_do_on_every_protocol() {
    for transport in TRANSPORTS {
        for endpoint in ENDPOINTS {
            let settings = spans_to(destination(transport, endpoint, &[]));

            let checked = validate(&settings);
            let made = exporters(&settings).map(drop);

            assert_eq!(checked, made, "{transport:?} {endpoint:?}");
        }
    }
}

#[test]
fn the_check_takes_what_each_protocols_exporter_takes_and_refuses_the_rest() {
    let refused = Err(OtelBuildError::Endpoint {
        signal: Signal::Traces,
    });
    for transport in TRANSPORTS {
        assert_eq!(validated(transport, "http://127.0.0.1:1"), Ok(()));
        assert_eq!(validated(transport, "http://[not a host"), refused);
        assert_eq!(validated(transport, ""), refused, "{transport:?}");
    }
    for transport in [Transport::HttpProtobuf, Transport::HttpJson] {
        assert_eq!(
            validated(transport, "collector.internal:4318/v1/traces"),
            refused,
            "{transport:?}: an HTTP endpoint is a URL with its scheme"
        );
        assert_eq!(
            validated(transport, "unix:///run/otel.sock"),
            refused,
            "{transport:?}: an HTTP endpoint is spoken to over HTTP"
        );
    }
}
