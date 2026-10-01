use std::fmt::Write as _;
use std::time::Duration;

use opentelemetry_proto::tonic::collector::trace::v1::trace_service_client::TraceServiceClient;
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tonic::metadata::MetadataValue;

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
/// headers of the answer.
async fn posted(endpoint: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> String {
    let address = endpoint.trim_start_matches("http://");
    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: {PROTOBUF}\r\n\
         Content-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        write!(request, "{name}: {value}\r\n").unwrap();
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    String::from_utf8_lossy(&answer).into_owned()
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
    let (grpc, http) = (receiver.grpc_endpoint(), receiver.http_endpoint());
    drop(receiver);

    // The listeners end on their own tasks, which the drop only signals.
    let refused = |endpoint: String| async move {
        for _ in 0..100 {
            if TcpStream::connect(endpoint.trim_start_matches("http://"))
                .await
                .is_err()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    };
    assert!(refused(grpc).await, "the gRPC listener still listens");
    assert!(refused(http).await, "the HTTP listener still listens");
}
