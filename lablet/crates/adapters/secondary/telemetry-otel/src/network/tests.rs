use reqwest::header::HeaderMap;
use tonic::metadata::MetadataValue;

use super::*;

fn settings(transport: Transport, headers: &[(&str, &str)]) -> OtlpSettings {
    OtlpSettings {
        transport,
        endpoint: Some("http://127.0.0.1:1".to_owned()),
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        strip_environment_headers: false,
    }
}

fn headers(set: &[(&str, &str)], strip: &[&str]) -> Headers {
    Headers {
        set: set
            .iter()
            .map(|(name, value)| {
                (
                    HeaderName::from_str(name).unwrap(),
                    SecretString::from(*value),
                )
            })
            .collect(),
        strip: strip
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

// Decoding the environment's headers as the exporter does

#[test]
fn the_pairs_of_a_header_variable_are_split_on_commas_and_trimmed() {
    assert_eq!(
        decode_headers(" a=1 , b = two,c=3,"),
        [("a", "1"), ("b", "two"), ("c", "3")]
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
    );
}

#[test]
fn a_value_is_percent_decoded_and_one_that_does_not_decode_is_kept_as_written() {
    assert_eq!(
        decode_headers("a=Bearer%20t%C3%B6ken,b=100%,c=x%zz"),
        [("a", "Bearer töken"), ("b", "100%"), ("c", "x%zz")]
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
    );
}

#[test]
fn a_value_that_doesnt_decode_keeps_its_leading_space_as_the_exporter_does() {
    // The exporter trims the pair, then the value only on the way into the
    // decoder, so a value that doesn't decode keeps the space after `=`.
    assert_eq!(
        decode_headers("a= x%zz "),
        [("a".to_owned(), " x%zz".to_owned())]
    );
}

#[test]
fn a_pair_without_a_name_or_a_value_or_an_equals_sign_is_left_out() {
    assert_eq!(
        decode_headers("=1,a=,b,c=3,,"),
        [("c".to_owned(), "3".to_owned())]
    );
    assert!(decode_headers("").is_empty());
}

#[test]
fn an_escape_that_is_cut_short_or_not_utf8_leaves_the_value_as_written() {
    assert_eq!(
        decode_headers("a=x%4,b=%ff"),
        [("a", "x%4"), ("b", "%ff")].map(|(name, value)| (name.to_owned(), value.to_owned()))
    );
}

// Setting the config's headers after the exporter's merge

#[test]
fn the_configs_headers_replace_every_value_of_their_name_and_the_rest_stay() {
    let headers = headers(&[("authorization", "Bearer config")], &[]);
    let mut map = HeaderMap::new();
    map.append("authorization", "Bearer env".parse().unwrap());
    map.append("authorization", "Bearer code".parse().unwrap());
    map.append("x-other", "kept".parse().unwrap());

    headers.apply(&mut map);

    assert_eq!(values(&map, "authorization"), ["Bearer config"]);
    assert_eq!(values(&map, "x-other"), ["kept"]);
    assert!(
        map.get("authorization").unwrap().is_sensitive(),
        "the value is marked sensitive, so a `Debug` of the map hides it"
    );
}

#[test]
fn the_environments_headers_are_taken_off_and_the_configs_put_on() {
    let headers = headers(&[("x-config", "yes")], &["authorization", "x-env"]);
    let mut map = HeaderMap::new();
    map.insert("authorization", "Bearer env".parse().unwrap());
    map.insert("x-env", "env".parse().unwrap());
    map.insert("x-other", "kept".parse().unwrap());

    headers.apply(&mut map);

    let mut names: Vec<_> = map.keys().map(HeaderName::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["x-config", "x-other"]);
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
    let mut interceptor = interceptor(Arc::new(headers(
        &[("authorization", "Bearer config")],
        &["x-env"],
    )));
    let mut request = tonic::Request::new(());
    request
        .metadata_mut()
        .append("authorization", MetadataValue::from_static("Bearer env"));
    request
        .metadata_mut()
        .append("x-env", MetadataValue::from_static("env"));
    request
        .metadata_mut()
        .append("x-other", MetadataValue::from_static("kept"));

    let request = interceptor(request).unwrap();

    let metadata = request.metadata();
    let authorization: Vec<_> = metadata
        .get_all("authorization")
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    assert_eq!(authorization, ["Bearer config"]);
    assert_eq!(metadata.get("x-env"), None);
    assert_eq!(metadata.get("x-other").unwrap(), "kept");
}

// The HTTP endpoint's path

#[test]
fn a_signals_path_is_appended_to_the_http_endpoint_once_whatever_its_trailing_slash() {
    assert_eq!(
        at_path("http://collector:4318", "/v1/traces"),
        "http://collector:4318/v1/traces"
    );
    assert_eq!(
        at_path("http://collector:4318/", "/v1/logs"),
        "http://collector:4318/v1/logs"
    );
    assert_eq!(
        at_path("http://collector:4318/otlp", "/v1/traces"),
        "http://collector:4318/otlp/v1/traces"
    );
}

#[test]
fn the_endpoint_of_each_signal_is_the_base_url_with_the_signals_path_on_http_and_as_is_on_grpc() {
    let http = Network::new(&settings(Transport::HttpProtobuf, &[])).unwrap();
    let grpc = Network::new(&settings(Transport::Grpc, &[])).unwrap();

    assert_eq!(
        http.of(Signal::Traces).endpoint.as_deref(),
        Some("http://127.0.0.1:1/v1/traces")
    );
    assert_eq!(
        http.of(Signal::Logs).endpoint.as_deref(),
        Some("http://127.0.0.1:1/v1/logs")
    );
    assert_eq!(
        grpc.of(Signal::Traces).endpoint.as_deref(),
        Some("http://127.0.0.1:1")
    );
    assert_eq!(
        grpc.of(Signal::Logs).endpoint.as_deref(),
        Some("http://127.0.0.1:1")
    );
}

#[test]
fn an_endpoint_the_config_leaves_out_is_set_on_no_exporter() {
    let network = Network::new(&OtlpSettings {
        endpoint: None,
        ..settings(Transport::HttpProtobuf, &[])
    })
    .unwrap();

    assert_eq!(network.of(Signal::Traces).endpoint, None);
}

// What's refused, and what a refusal says

#[test]
fn a_header_whose_name_or_value_is_not_a_headers_is_refused_without_its_value() {
    let bad_name = Network::new(&settings(Transport::Grpc, &[("not a name", "v")]));
    let bad_value = Network::new(&settings(Transport::Grpc, &[("x-token", "has\nnewline")]));

    let bad_name = bad_name.err().unwrap();
    assert_eq!(
        bad_name,
        OtelBuildError::Header {
            name: "not a name".to_owned(),
            reason: "its name isn't one a header may have",
        }
    );
    let bad_value = bad_value.err().unwrap();
    assert_eq!(
        bad_value.to_string(),
        "the OTLP header `x-token` is refused: its value isn't one a header may have"
    );
    assert!(!bad_value.to_string().contains("newline"));
}

#[tokio::test]
async fn an_endpoint_the_exporter_refuses_is_refused_without_the_endpoint_in_the_message() {
    for transport in [Transport::Grpc, Transport::HttpProtobuf] {
        let network = Network::new(&OtlpSettings {
            endpoint: Some("http://secret:token@[not a host".to_owned()),
            ..settings(transport, &[])
        })
        .unwrap();

        let refused = network.exporters().err().unwrap();

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
async fn the_exporters_of_either_transport_are_made_from_an_endpoint_nothing_listens_on() {
    for transport in [Transport::Grpc, Transport::HttpProtobuf] {
        let network = Network::new(&settings(transport, &[("x-token", "a value")])).unwrap();

        let made = network.exporters();

        assert!(made.is_ok(), "{transport:?}");
    }
}

#[test]
fn the_settings_print_neither_the_endpoint_nor_a_headers_value() {
    let settings = OtlpSettings {
        endpoint: Some("http://user:hunter2hunter2@collector:4317".to_owned()),
        ..settings(
            Transport::Grpc,
            &[("authorization", "Bearer hunter2hunter2")],
        )
    };

    let shown = format!("{settings:?}");

    assert_eq!(
        shown,
        "OtlpSettings { transport: Grpc, endpoint: Some(\"..\"), headers: [\"authorization\"], \
         strip_environment_headers: false }"
    );
}

#[test]
fn the_default_timeout_is_the_exporters_ten_seconds() {
    // The variables aren't set where the tests run: the gate strips none, so
    // a developer who exports one sees this fail and knows why.
    assert_eq!(timeout_of(Signal::Traces), Duration::from_secs(10));
    assert_eq!(timeout_of(Signal::Logs), Duration::from_secs(10));
}

// Holding the settings to what the exporters take, making none

/// Endpoints the exporters take, ones they refuse, and ones one transport
/// takes and the other refuses: the gRPC exporter gives an endpoint without
/// a scheme one, and the HTTP exporter parses what it's given.
const ENDPOINTS: [&str; 8] = [
    "http://127.0.0.1:1",
    "https://collector.internal:4317/",
    "collector.internal:4317",
    "collector.internal:4317/otlp",
    "http://user:hunter2hunter2@collector.internal:4317",
    "http://[not a host",
    "::::not-a-url",
    "",
];

fn validated(transport: Transport, endpoint: &str) -> Result<(), OtelBuildError> {
    validate(&OtlpSettings {
        endpoint: Some(endpoint.to_owned()),
        ..settings(transport, &[])
    })
}

#[tokio::test]
async fn the_check_of_an_endpoint_answers_as_the_exporters_do_on_either_transport() {
    for transport in [Transport::Grpc, Transport::HttpProtobuf] {
        for endpoint in ENDPOINTS {
            let settings = OtlpSettings {
                endpoint: Some(endpoint.to_owned()),
                ..settings(transport, &[])
            };

            let checked = validate(&settings);
            let made = Network::new(&settings).unwrap().exporters().map(|_| ());

            assert_eq!(checked, made, "{transport:?} {endpoint:?}");
        }
    }
}

#[test]
fn the_check_takes_what_each_transports_exporter_takes_and_refuses_the_rest() {
    let refused = Err(OtelBuildError::Endpoint {
        signal: Signal::Traces,
    });
    for transport in [Transport::Grpc, Transport::HttpProtobuf] {
        assert_eq!(validated(transport, "http://127.0.0.1:1"), Ok(()));
        assert_eq!(validated(transport, "http://[not a host"), refused);
        assert_eq!(
            validated(transport, ""),
            Ok(()),
            "{transport:?}: an empty endpoint is left out, and the exporter's own default stands"
        );
    }
    assert_eq!(
        validated(Transport::Grpc, "collector.internal:4317/otlp"),
        Ok(()),
        "the gRPC exporter gives it a scheme"
    );
    assert_eq!(
        validated(Transport::HttpProtobuf, "collector.internal:4317/otlp"),
        refused,
        "the HTTP exporter parses it as given, with the signal's path"
    );
}

#[test]
fn the_check_refuses_a_header_as_the_exporters_are_refused_one() {
    assert_eq!(
        validate(&settings(Transport::Grpc, &[("not a name", "v")])),
        Err(OtelBuildError::Header {
            name: "not a name".to_owned(),
            reason: "its name isn't one a header may have",
        })
    );
    assert_eq!(
        validate(&settings(
            Transport::HttpProtobuf,
            &[("x-token", "has\nnewline")]
        )),
        Err(OtelBuildError::Header {
            name: "x-token".to_owned(),
            reason: "its value isn't one a header may have",
        })
    );
}

#[test]
fn the_check_takes_an_endpoint_the_settings_leave_out() {
    // The endpoint variables aren't set where the tests run, as the timeout
    // ones aren't, so the exporter's own default stands, which it takes.
    for transport in [Transport::Grpc, Transport::HttpProtobuf] {
        assert_eq!(
            validate(&OtlpSettings {
                endpoint: None,
                ..settings(transport, &[])
            }),
            Ok(()),
            "{transport:?}"
        );
    }
}
