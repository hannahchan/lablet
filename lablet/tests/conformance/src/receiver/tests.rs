use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use opentelemetry_proto::tonic::collector::trace::v1::trace_service_client::TraceServiceClient;
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
use prost::Message as _;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tonic::metadata::MetadataValue;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

use super::*;

/// One export of one span, named `name`.
fn one_span(name: &str) -> ExportTraceServiceRequest {
    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource {
                attributes: vec![KeyValue {
                    key: "service.name".to_owned(),
                    value: Some(AnyValue {
                        value: Some(any_value::Value::StringValue("probe".to_owned())),
                    }),
                    ..KeyValue::default()
                }],
                ..Resource::default()
            }),
            scope_spans: vec![ScopeSpans {
                scope: Some(InstrumentationScope {
                    name: "probe".to_owned(),
                    ..InstrumentationScope::default()
                }),
                spans: vec![Span {
                    trace_id: vec![1; 16],
                    span_id: vec![2; 8],
                    name: name.to_owned(),
                    ..Span::default()
                }],
                ..ScopeSpans::default()
            }],
            ..ResourceSpans::default()
        }],
    }
}

/// Sends `request` to the HTTP listener at `endpoint` with `headers`, as a
/// client that speaks HTTP/1.1 does, and returns the status line and the
/// headers of the answer. The body is protobuf unless `headers` say what it
/// is.
async fn posted(endpoint: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> String {
    let address = endpoint.trim_start_matches("http://");
    let stream = TcpStream::connect(address).await.unwrap();
    exchanged(stream, address, path, headers, body)
        .await
        .unwrap()
}

/// The exchange of [`posted`], over `stream`, or what failed it.
async fn exchanged(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    address: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<String, String> {
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        write!(request, "Content-Type: {PROTOBUF}\r\n").unwrap();
    }
    for (name, value) in headers {
        write!(request, "{name}: {value}\r\n").unwrap();
    }
    request.push_str("\r\n");
    let failed = |error: std::io::Error| error.to_string();
    stream.write_all(request.as_bytes()).await.map_err(failed)?;
    stream.write_all(body).await.map_err(failed)?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.map_err(failed)?;
    Ok(String::from_utf8_lossy(&answer).into_owned())
}

/// What [`posted`] gets over TLS from the listener at `endpoint`, as a
/// client that trusts the CA `ca` and shows `identity` when there is one,
/// or what failed the handshake or the exchange.
async fn posted_over_tls(
    endpoint: &str,
    ca: &str,
    identity: Option<(&str, &str)>,
    body: &[u8],
) -> Result<String, String> {
    let address = endpoint.trim_start_matches("https://");
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(ca.as_bytes()).unwrap())
        .unwrap();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots);
    let config = match identity {
        Some((certificate, key)) => config
            .with_client_auth_cert(
                vec![CertificateDer::from_pem_slice(certificate.as_bytes()).unwrap()],
                PrivateKeyDer::from_pem_slice(key.as_bytes()).unwrap(),
            )
            .unwrap(),
        None => config.with_no_client_auth(),
    };
    let stream = TcpStream::connect(address).await.unwrap();
    let stream = TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from("localhost").unwrap(), stream)
        .await
        .map_err(|error| error.to_string())?;
    let answer = exchanged(stream, address, "/v1/traces", &[], body).await?;
    if answer.is_empty() {
        Err("the listener closed the connection without an answer".to_owned())
    } else {
        Ok(answer)
    }
}

/// `body` compressed with gzip.
fn gzipped(body: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(body).unwrap();
    encoder.finish().unwrap()
}

#[tokio::test]
async fn a_grpc_export_is_answered_and_kept_with_its_metadata_as_one_line() {
    let receiver = Receiver::start(Mode::Answers).await;
    let mut client = TraceServiceClient::connect(receiver.grpc_endpoint())
        .await
        .unwrap();
    let mut request = tonic::Request::new(one_span("chat probe"));
    request
        .metadata_mut()
        .insert("x-token", MetadataValue::from_static("a made-up token"));

    let answered = client.export(request).await;

    answered.unwrap();
    let requests = receiver.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].transport, Transport::Grpc);
    assert_eq!(requests[0].signal, Signal::Traces);
    assert!(
        requests[0]
            .headers
            .contains(&("x-token".to_owned(), "a made-up token".to_owned())),
        "{:?}",
        requests[0].headers
    );
    assert!(requests[0].line.ends_with('\n'));
    let exported = receiver.exported().unwrap();
    assert_eq!(exported.lines, 1);
    assert_eq!(exported.spans.len(), 1);
    assert_eq!(exported.spans[0].name, "chat probe");
    assert_eq!(exported.spans[0].resource["service.name"], "probe");
}

/// A channel to the TLS listener of `receiver` that trusts `roots`.
async fn over_tls(receiver: &Receiver, roots: ClientTlsConfig) -> Result<Channel, String> {
    Endpoint::from_shared(receiver.grpc_tls_endpoint())
        .and_then(|endpoint| endpoint.tls_config(roots))
        .unwrap()
        .connect()
        .await
        .map_err(|error| format!("{:?}", std::error::Error::source(&error)))
}

#[tokio::test]
async fn a_grpc_export_over_tls_is_kept_from_a_client_that_trusts_the_receivers_ca() {
    let receiver = Receiver::start(Mode::Answers).await;
    let roots =
        ClientTlsConfig::new().ca_certificate(Certificate::from_pem(receiver.ca_certificate()));
    let mut client = TraceServiceClient::new(over_tls(&receiver, roots).await.unwrap());

    let answered = client
        .export(tonic::Request::new(one_span("chat probe")))
        .await;

    answered.unwrap();
    let requests = receiver.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        (requests[0].transport, requests[0].signal),
        (Transport::GrpcTls, Signal::Traces)
    );
    assert_eq!(receiver.exported().unwrap().spans[0].name, "chat probe");
}

#[tokio::test]
async fn a_client_that_trusts_no_root_or_another_receivers_ca_is_refused_at_the_handshake() {
    let receiver = Receiver::start(Mode::Answers).await;
    let other = Receiver::start(Mode::Answers).await;
    let another =
        ClientTlsConfig::new().ca_certificate(Certificate::from_pem(other.ca_certificate()));

    for roots in [ClientTlsConfig::new(), another] {
        let connected = over_tls(&receiver, roots).await;

        let refused = connected.unwrap_err();
        assert!(refused.contains("InvalidCertificate"), "{refused}");
    }
    assert!(receiver.requests().is_empty());
}

#[tokio::test]
async fn an_http_export_is_answered_as_protobuf_and_kept_with_its_headers() {
    let receiver = Receiver::start(Mode::Answers).await;
    let body = one_span("execute_tool probe").encode_to_vec();

    let answer = posted(
        &receiver.http_endpoint(),
        "/v1/traces",
        &[("x-token", "a made-up token")],
        &body,
    )
    .await;

    assert!(answer.starts_with("HTTP/1.1 200 "), "{answer}");
    assert!(
        answer
            .to_ascii_lowercase()
            .contains("content-type: application/x-protobuf"),
        "{answer}"
    );
    let requests = receiver.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        (requests[0].transport, requests[0].signal),
        (Transport::Http, Signal::Traces)
    );
    assert!(
        requests[0]
            .headers
            .contains(&("x-token".to_owned(), "a made-up token".to_owned())),
        "{:?}",
        requests[0].headers
    );
    assert_eq!(
        receiver.exported().unwrap().spans[0].name,
        "execute_tool probe"
    );
}

#[tokio::test]
async fn an_http_export_in_json_is_answered_in_json_and_kept() {
    let receiver = Receiver::start(Mode::Answers).await;
    let body = serde_json::to_vec(&one_span("chat probe")).unwrap();

    let answer = posted(
        &receiver.http_endpoint(),
        "/v1/traces",
        &[("Content-Type", JSON)],
        &body,
    )
    .await;

    assert!(answer.starts_with("HTTP/1.1 200 "), "{answer}");
    assert!(
        answer
            .to_ascii_lowercase()
            .contains("content-type: application/json"),
        "{answer}"
    );
    assert_eq!(receiver.exported().unwrap().spans[0].name, "chat probe");
}

#[tokio::test]
async fn a_gzip_http_export_is_taken_with_its_content_encoding_kept() {
    let receiver = Receiver::start(Mode::Answers).await;
    let body = gzipped(&one_span("chat probe").encode_to_vec());

    let answer = posted(
        &receiver.http_endpoint(),
        "/v1/traces",
        &[("Content-Encoding", "gzip")],
        &body,
    )
    .await;

    assert!(answer.starts_with("HTTP/1.1 200 "), "{answer}");
    let requests = receiver.requests();
    assert!(
        requests[0]
            .headers
            .contains(&("content-encoding".to_owned(), "gzip".to_owned())),
        "{:?}",
        requests[0].headers
    );
    assert_eq!(receiver.exported().unwrap().spans[0].name, "chat probe");
}

#[tokio::test]
async fn a_gzip_grpc_export_is_taken_with_its_encoding_kept() {
    let receiver = Receiver::start(Mode::Answers).await;
    let mut client = TraceServiceClient::connect(receiver.grpc_endpoint())
        .await
        .unwrap()
        .send_compressed(tonic::codec::CompressionEncoding::Gzip);

    client
        .export(tonic::Request::new(one_span("chat probe")))
        .await
        .unwrap();

    let requests = receiver.requests();
    assert!(
        requests[0]
            .headers
            .contains(&("grpc-encoding".to_owned(), "gzip".to_owned())),
        "{:?}",
        requests[0].headers
    );
    assert_eq!(receiver.exported().unwrap().spans[0].name, "chat probe");
}

#[tokio::test]
async fn an_https_export_is_kept_from_a_client_that_trusts_the_receivers_ca_and_refused_by_another()
{
    let receiver = Receiver::start(Mode::Answers).await;
    let other = Receiver::start(Mode::Answers).await;
    let body = one_span("chat probe").encode_to_vec();

    let answer = posted_over_tls(
        &receiver.https_endpoint(),
        receiver.ca_certificate(),
        None,
        &body,
    )
    .await
    .unwrap();
    let refused = posted_over_tls(
        &receiver.https_endpoint(),
        other.ca_certificate(),
        None,
        &body,
    )
    .await;

    assert!(answer.starts_with("HTTP/1.1 200 "), "{answer}");
    let refused = refused.unwrap_err();
    assert!(refused.contains("invalid peer certificate"), "{refused}");
    let requests = receiver.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].transport, Transport::Https);
}

#[tokio::test]
async fn an_https_listener_that_asks_for_a_client_certificate_takes_the_receivers_alone() {
    let receiver = Receiver::start(Mode::Answers).await;
    let other = Receiver::start(Mode::Answers).await;
    let body = one_span("chat probe").encode_to_vec();
    let endpoint = receiver.https_client_tls_endpoint();
    let ca = receiver.ca_certificate();

    let shown = posted_over_tls(
        &endpoint,
        ca,
        Some((receiver.client_certificate(), receiver.client_key())),
        &body,
    )
    .await;
    let none = posted_over_tls(&endpoint, ca, None, &body).await;
    let another = posted_over_tls(
        &endpoint,
        ca,
        Some((other.client_certificate(), other.client_key())),
        &body,
    )
    .await;

    assert!(shown.unwrap().starts_with("HTTP/1.1 200 "));
    none.unwrap_err();
    another.unwrap_err();
    let requests = receiver.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].transport, Transport::HttpsClientTls);
}

#[tokio::test]
async fn a_grpc_listener_that_asks_for_a_client_certificate_takes_the_receivers_alone() {
    let receiver = Receiver::start(Mode::Answers).await;
    let other = Receiver::start(Mode::Answers).await;
    let ca = Certificate::from_pem(receiver.ca_certificate());
    let export = |identity: Option<Identity>| {
        let mut tls = ClientTlsConfig::new().ca_certificate(ca.clone());
        if let Some(identity) = identity {
            tls = tls.identity(identity);
        }
        let endpoint = receiver.grpc_client_tls_endpoint();
        async move {
            let channel = Endpoint::from_shared(endpoint)
                .unwrap()
                .tls_config(tls)
                .unwrap()
                .connect()
                .await
                .map_err(|error| error.to_string())?;
            TraceServiceClient::new(channel)
                .export(tonic::Request::new(one_span("chat probe")))
                .await
                .map(drop)
                .map_err(|status| status.to_string())
        }
    };

    let shown = export(Some(Identity::from_pem(
        receiver.client_certificate(),
        receiver.client_key(),
    )))
    .await;
    let none = export(None).await;
    let another = export(Some(Identity::from_pem(
        other.client_certificate(),
        other.client_key(),
    )))
    .await;

    shown.unwrap();
    none.unwrap_err();
    another.unwrap_err();
    let requests = receiver.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].transport, Transport::GrpcClientTls);
}

#[tokio::test]
async fn a_body_that_is_not_the_request_is_refused_and_kept_by_neither_listener() {
    let receiver = Receiver::start(Mode::Answers).await;

    let answer = posted(&receiver.http_endpoint(), "/v1/logs", &[], b"not protobuf").await;

    assert!(answer.starts_with("HTTP/1.1 400 "), "{answer}");
    assert!(receiver.requests().is_empty());
}

#[tokio::test]
async fn a_receiver_that_never_answers_keeps_the_request_and_leaves_the_client_waiting() {
    let receiver = Receiver::start(Mode::NeverAnswers).await;
    let mut client = TraceServiceClient::connect(receiver.grpc_endpoint())
        .await
        .unwrap();

    let answered = tokio::time::timeout(
        Duration::from_millis(300),
        client.export(tonic::Request::new(one_span("chat probe"))),
    )
    .await;

    assert!(answered.is_err(), "the receiver answered: {answered:?}");
    assert_eq!(receiver.requests().len(), 1);
}

#[tokio::test]
async fn an_http_receiver_that_never_answers_keeps_the_request_and_leaves_the_client_waiting() {
    let receiver = Receiver::start(Mode::NeverAnswers).await;
    let body = one_span("chat probe").encode_to_vec();

    let answered = tokio::time::timeout(
        Duration::from_millis(300),
        posted(&receiver.http_endpoint(), "/v1/traces", &[], &body),
    )
    .await;

    assert!(answered.is_err(), "the receiver answered: {answered:?}");
    let requests = receiver.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        (requests[0].transport, requests[0].signal),
        (Transport::Http, Signal::Traces)
    );
}

#[tokio::test]
async fn a_closed_port_refuses_a_connection() {
    let closed = Receiver::closed();

    let connected = TcpStream::connect(closed).await;

    connected.unwrap_err();
}

#[tokio::test]
async fn a_receiver_that_was_dropped_listens_no_more() {
    let receiver = Receiver::start(Mode::Answers).await;
    let endpoints = [
        receiver.grpc_endpoint(),
        receiver.grpc_tls_endpoint(),
        receiver.grpc_client_tls_endpoint(),
        receiver.http_endpoint(),
        receiver.https_endpoint(),
        receiver.https_client_tls_endpoint(),
    ];
    drop(receiver);

    // The listeners end on their own tasks, which the drop only signals.
    let refused = |endpoint: String| async move {
        for _ in 0..100 {
            let address = endpoint.split_once("://").unwrap().1.to_owned();
            if TcpStream::connect(address).await.is_err() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    };
    for endpoint in endpoints {
        assert!(refused(endpoint.clone()).await, "{endpoint} still listens");
    }
}
