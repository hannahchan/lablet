//! An OTLP receiver in the test's own process: a gRPC listener and an HTTP
//! one, each on a port of its own on the loopback interface, so a network
//! scenario runs wherever the tests do, with no collector to start.
//!
//! Each request a listener accepts is kept as one OTLP/JSON line, the form
//! the file exporter writes, so the reader of the file reads the network's
//! exports back the same way. The headers of each request are kept beside
//! it, metadata on gRPC, so a test can say what a header was, or that it
//! wasn't sent.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::post;
use opentelemetry_proto::tonic::collector::logs::v1::logs_service_server::{
    LogsService, LogsServiceServer,
};
use opentelemetry_proto::tonic::collector::logs::v1::{
    ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
    TraceService, TraceServiceServer,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use prost::Message as _;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;

use crate::must;
use crate::otlp::{Exported, ReadError};

/// The content type of an OTLP/HTTP protobuf request and response.
const PROTOBUF: &str = "application/x-protobuf";

/// How a receiver answers what it's sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Keeps each request and answers it as a collector does.
    Answers,
    /// Accepts each connection, reads each request, and never answers: a
    /// collector that hangs.
    NeverAnswers,
}

/// Which signal a request carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Spans.
    Traces,
    /// Log records.
    Logs,
}

/// Which listener a request came in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// The gRPC listener.
    Grpc,
    /// The HTTP listener.
    Http,
}

/// One request the receiver kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    /// The listener it came in on.
    pub transport: Transport,
    /// The signal it carried.
    pub signal: Signal,
    /// Its headers, in the order they arrived; metadata on gRPC, with the
    /// names in lower case as HTTP/2 carries them.
    pub headers: Vec<(String, String)>,
    /// The request as one OTLP/JSON line, newline and all.
    pub line: String,
}

impl Received {
    /// What the request exported, read back.
    ///
    /// # Errors
    ///
    /// Returns a [`ReadError`] when the line can't be read, which is a
    /// fault of the receiver's own.
    pub fn exported(&self) -> Result<Exported, ReadError> {
        Exported::parse(&self.line)
    }
}

#[derive(Debug, Default)]
struct Kept {
    requests: Vec<Received>,
}

/// What the two listeners share.
#[derive(Debug, Clone)]
struct Shared {
    mode: Mode,
    kept: Arc<Mutex<Kept>>,
}

impl Shared {
    /// Keeps `request` as one line, and then answers or doesn't, as the
    /// mode says. The line is the request serialised with the same derives
    /// the file exporter uses, so both sides read back alike.
    async fn took<T: Serialize>(
        &self,
        transport: Transport,
        signal: Signal,
        headers: Vec<(String, String)>,
        request: &T,
    ) {
        let mut line = must(serde_json::to_string(request), "writing a request as JSON");
        line.push('\n');
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .push(Received {
                transport,
                signal,
                headers,
                line,
            });
        if self.mode == Mode::NeverAnswers {
            std::future::pending::<()>().await;
        }
    }
}

/// The metadata of a gRPC request as headers, which is what the exporter
/// set them as.
fn metadata(request: &tonic::Request<impl Sized>) -> Vec<(String, String)> {
    request
        .metadata()
        .clone()
        .into_headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}

fn headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}

#[tonic::async_trait]
impl TraceService for Shared {
    async fn export(
        &self,
        request: tonic::Request<ExportTraceServiceRequest>,
    ) -> Result<tonic::Response<ExportTraceServiceResponse>, tonic::Status> {
        let headers = metadata(&request);
        self.took(Transport::Grpc, Signal::Traces, headers, request.get_ref())
            .await;
        Ok(tonic::Response::new(ExportTraceServiceResponse::default()))
    }
}

#[tonic::async_trait]
impl LogsService for Shared {
    async fn export(
        &self,
        request: tonic::Request<ExportLogsServiceRequest>,
    ) -> Result<tonic::Response<ExportLogsServiceResponse>, tonic::Status> {
        let headers = metadata(&request);
        self.took(Transport::Grpc, Signal::Logs, headers, request.get_ref())
            .await;
        Ok(tonic::Response::new(ExportLogsServiceResponse::default()))
    }
}

/// Answers an OTLP/HTTP request as a collector does: a protobuf response
/// of the signal's kind, or `400` for a body that isn't the request.
fn answered(response: Result<Vec<u8>, prost::DecodeError>) -> axum::response::Response {
    match response {
        Ok(body) => ([(header::CONTENT_TYPE, PROTOBUF)], body).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

async fn traces_over_http(
    State(shared): State<Shared>,
    headers_sent: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let request = match ExportTraceServiceRequest::decode(body) {
        Ok(request) => request,
        Err(error) => return answered(Err(error)),
    };
    shared
        .took(
            Transport::Http,
            Signal::Traces,
            headers(&headers_sent),
            &request,
        )
        .await;
    answered(Ok(ExportTraceServiceResponse::default().encode_to_vec()))
}

async fn logs_over_http(
    State(shared): State<Shared>,
    headers_sent: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let request = match ExportLogsServiceRequest::decode(body) {
        Ok(request) => request,
        Err(error) => return answered(Err(error)),
    };
    shared
        .took(
            Transport::Http,
            Signal::Logs,
            headers(&headers_sent),
            &request,
        )
        .await;
    answered(Ok(ExportLogsServiceResponse::default().encode_to_vec()))
}

/// The receiver: two listeners, each stopped when the receiver is dropped.
#[derive(Debug)]
pub struct Receiver {
    grpc: SocketAddr,
    http: SocketAddr,
    kept: Arc<Mutex<Kept>>,
    stop: Option<(oneshot::Sender<()>, oneshot::Sender<()>)>,
}

impl Receiver {
    /// Binds both listeners to a free port each on the loopback interface
    /// and serves them on the runtime this is called on, in `mode`.
    ///
    /// # Panics
    ///
    /// Panics when a port can't be bound, which is a fault of where the
    /// test runs.
    pub async fn start(mode: Mode) -> Self {
        let shared = Shared {
            mode,
            kept: Arc::default(),
        };
        let kept = Arc::clone(&shared.kept);

        let grpc_listener = must(
            TcpListener::bind("127.0.0.1:0").await,
            "binding the gRPC listener",
        );
        let grpc = must(grpc_listener.local_addr(), "reading the gRPC port");
        let (stop_grpc, stopped) = oneshot::channel();
        let router = Server::builder()
            .add_service(TraceServiceServer::new(shared.clone()))
            .add_service(LogsServiceServer::new(shared.clone()));
        tokio::spawn(async move {
            // The listener ends when the receiver is dropped, and nothing
            // reads what it says of that.
            let _ = router
                .serve_with_incoming_shutdown(TcpIncoming::from(grpc_listener), async {
                    let _ = stopped.await;
                })
                .await;
        });

        let http_listener = must(
            TcpListener::bind("127.0.0.1:0").await,
            "binding the HTTP listener",
        );
        let http = must(http_listener.local_addr(), "reading the HTTP port");
        let (stop_http, stopped) = oneshot::channel();
        let app = axum::Router::new()
            .route("/v1/traces", post(traces_over_http))
            .route("/v1/logs", post(logs_over_http))
            .with_state(shared);
        tokio::spawn(async move {
            let _ = axum::serve(http_listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await;
        });

        Self {
            grpc,
            http,
            kept,
            stop: Some((stop_grpc, stop_http)),
        }
    }

    /// A port on the loopback interface that nothing listens on: it was
    /// bound and let go of, so a connection to it is refused.
    ///
    /// # Panics
    ///
    /// Panics when a port can't be bound, which is a fault of where the
    /// test runs.
    #[must_use]
    pub fn closed() -> SocketAddr {
        let listener = must(
            std::net::TcpListener::bind("127.0.0.1:0"),
            "binding a port to close",
        );
        must(listener.local_addr(), "reading the port to close")
    }

    /// The gRPC listener's endpoint, as a config states it.
    #[must_use]
    pub fn grpc_endpoint(&self) -> String {
        format!("http://{}", self.grpc)
    }

    /// The HTTP listener's endpoint, as a config states it: the base URL,
    /// to which the exporter appends each signal's path.
    #[must_use]
    pub fn http_endpoint(&self) -> String {
        format!("http://{}", self.http)
    }

    /// Every request kept so far, in the order they arrived.
    #[must_use]
    pub fn requests(&self) -> Vec<Received> {
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .clone()
    }

    /// Everything the requests so far exported, read back as the file
    /// exporter's lines are: each request is one line, in the order they
    /// arrived.
    ///
    /// # Errors
    ///
    /// Returns a [`ReadError`] when a request can't be read back, which is
    /// a fault of the receiver's own.
    pub fn exported(&self) -> Result<Exported, ReadError> {
        let lines: String = self
            .requests()
            .into_iter()
            .map(|request| request.line)
            .collect();
        Exported::parse(&lines)
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if let Some((grpc, http)) = self.stop.take() {
            // A listener that has already ended has no one to tell.
            let _ = grpc.send(());
            let _ = http.send(());
        }
    }
}

#[cfg(test)]
mod tests;
