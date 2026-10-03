//! An OTLP receiver in the test's own process: a gRPC listener, a second
//! that speaks TLS with a certificate from a CA of the receiver's own, and
//! an HTTP one, each on a port of its own on the loopback interface, so a
//! network scenario runs wherever the tests do, with no collector to start.
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
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Identity, Server, ServerTlsConfig};

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
    /// The gRPC listener that speaks TLS.
    GrpcTls,
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

/// What the listeners share.
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

/// The two OTLP services a gRPC listener serves: what the listeners share,
/// and which of them it is.
#[derive(Debug, Clone)]
struct Services {
    shared: Shared,
    transport: Transport,
}

#[tonic::async_trait]
impl TraceService for Services {
    async fn export(
        &self,
        request: tonic::Request<ExportTraceServiceRequest>,
    ) -> Result<tonic::Response<ExportTraceServiceResponse>, tonic::Status> {
        let headers = metadata(&request);
        self.shared
            .took(self.transport, Signal::Traces, headers, request.get_ref())
            .await;
        Ok(tonic::Response::new(ExportTraceServiceResponse::default()))
    }
}

#[tonic::async_trait]
impl LogsService for Services {
    async fn export(
        &self,
        request: tonic::Request<ExportLogsServiceRequest>,
    ) -> Result<tonic::Response<ExportLogsServiceResponse>, tonic::Status> {
        let headers = metadata(&request);
        self.shared
            .took(self.transport, Signal::Logs, headers, request.get_ref())
            .await;
        Ok(tonic::Response::new(ExportLogsServiceResponse::default()))
    }
}

/// Serves `services` through `server` on a free port of the loopback
/// interface until the sender it returns is used or dropped.
async fn serve(mut server: Server, services: Services) -> (SocketAddr, oneshot::Sender<()>) {
    let listener = must(
        TcpListener::bind("127.0.0.1:0").await,
        "binding a gRPC listener",
    );
    let address = must(listener.local_addr(), "reading a gRPC listener's port");
    let (stop, stopped) = oneshot::channel();
    let router = server
        .add_service(TraceServiceServer::new(services.clone()))
        .add_service(LogsServiceServer::new(services));
    tokio::spawn(async move {
        // The listener ends when the receiver is dropped, and nothing
        // reads what it says of that.
        let _ = router
            .serve_with_incoming_shutdown(TcpIncoming::from(listener), async {
                let _ = stopped.await;
            })
            .await;
    });
    (address, stop)
}

/// A CA of a receiver's own, as PEM, and the identity of its TLS listener,
/// whose certificate the CA signed for the loopback address, by number and
/// by name. Both keys are made here and kept nowhere else.
fn certificates() -> (String, Identity) {
    let mut ca = must(CertificateParams::new(Vec::new()), "stating the CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    ca.distinguished_name
        .push(DnType::CommonName, "lablet test CA");
    let ca = must(
        CertifiedIssuer::self_signed(ca, must(KeyPair::generate(), "making the CA's key")),
        "making the CA",
    );
    let mut listener = must(
        CertificateParams::new(["127.0.0.1".to_owned(), "localhost".to_owned()]),
        "stating the listener's certificate",
    );
    listener.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    listener
        .distinguished_name
        .push(DnType::CommonName, "lablet test receiver");
    let key = must(KeyPair::generate(), "making the listener's key");
    let certificate = must(
        listener.signed_by(&key, &ca),
        "signing the listener's certificate",
    );
    (
        ca.pem(),
        Identity::from_pem(certificate.pem(), key.serialize_pem()),
    )
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

/// The receiver: three listeners, each stopped when the receiver is
/// dropped.
#[derive(Debug)]
pub struct Receiver {
    grpc: SocketAddr,
    grpc_tls: SocketAddr,
    http: SocketAddr,
    ca: String,
    kept: Arc<Mutex<Kept>>,
    stop: Vec<oneshot::Sender<()>>,
}

impl Receiver {
    /// Binds the three listeners to a free port each on the loopback
    /// interface and serves them on the runtime this is called on, in
    /// `mode`, with a CA made for this receiver alone.
    ///
    /// # Panics
    ///
    /// Panics when a port can't be bound or the certificates can't be made,
    /// which is a fault of where the test runs.
    pub async fn start(mode: Mode) -> Self {
        let shared = Shared {
            mode,
            kept: Arc::default(),
        };
        let kept = Arc::clone(&shared.kept);
        let services = |transport| Services {
            shared: shared.clone(),
            transport,
        };

        let (grpc, stop_grpc) = serve(Server::builder(), services(Transport::Grpc)).await;
        let (ca, identity) = certificates();
        let tls = must(
            Server::builder().tls_config(ServerTlsConfig::new().identity(identity)),
            "setting up the TLS listener",
        );
        let (grpc_tls, stop_grpc_tls) = serve(tls, services(Transport::GrpcTls)).await;

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
            grpc_tls,
            http,
            ca,
            kept,
            stop: vec![stop_grpc, stop_grpc_tls, stop_http],
        }
    }

    /// A port on the loopback interface that nothing listens on, so a
    /// connection to it is refused. It's a port below the range the system
    /// hands out, one no service is assigned, which no listener a test binds
    /// at 0 can be given and no unprivileged process can bind: a port bound
    /// and let go of was taken by another test's receiver before the
    /// exporter connected.
    #[must_use]
    pub fn closed() -> SocketAddr {
        SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 4))
    }

    /// The gRPC listener's endpoint, as a config states it.
    #[must_use]
    pub fn grpc_endpoint(&self) -> String {
        format!("http://{}", self.grpc)
    }

    /// The endpoint of the gRPC listener that speaks TLS, as a config states
    /// it. Its certificate is for `127.0.0.1` and `localhost`, and a client
    /// that trusts [`Self::ca_certificate`] takes it.
    #[must_use]
    pub fn grpc_tls_endpoint(&self) -> String {
        format!("https://{}", self.grpc_tls)
    }

    /// The certificate of the CA that signed the TLS listener's, as PEM: a
    /// file of it is what `SSL_CERT_FILE` names for a client that trusts
    /// it.
    #[must_use]
    pub fn ca_certificate(&self) -> &str {
        &self.ca
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
        for stop in self.stop.drain(..) {
            // A listener that has already ended has no one to tell.
            let _ = stop.send(());
        }
    }
}

#[cfg(test)]
mod tests;
