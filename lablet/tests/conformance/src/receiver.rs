//! An OTLP receiver in the test's own process, with a listener for each way
//! a collector is spoken to: gRPC and HTTP, each plain, over TLS with a
//! certificate from a CA of the receiver's own, and over TLS that asks the
//! client for a certificate that CA signed. Each is on a port of its own on
//! the loopback interface, so a network scenario runs wherever the tests
//! do, with no collector to start. Every listener takes gzip, and the HTTP
//! ones take protobuf and JSON bodies.
//!
//! Each request a listener accepts is kept as one OTLP/JSON line, the form
//! the file exporter writes, so the reader of the file reads the network's
//! exports back the same way. The headers of each request are kept beside
//! it, metadata on gRPC, so a test can say what a header was, or that it
//! wasn't sent, and how the body was encoded.

use std::io::Read as _;
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
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;
use tonic::codec::CompressionEncoding;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};

use crate::must;
use crate::otlp::{Exported, ReadError};

/// The content type of an OTLP/HTTP protobuf request and response.
const PROTOBUF: &str = "application/x-protobuf";

/// The content type of an OTLP/HTTP JSON request and response.
const JSON: &str = "application/json";

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
    /// The gRPC listener that speaks TLS and takes only a client that shows
    /// a certificate the receiver's CA signed.
    GrpcClientTls,
    /// The HTTP listener.
    Http,
    /// The HTTP listener that speaks TLS.
    Https,
    /// The HTTP listener that speaks TLS and takes only a client that shows
    /// a certificate the receiver's CA signed.
    HttpsClientTls,
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

/// What a test is told of each request as it arrives, before it's
/// answered.
type Watch = Arc<dyn Fn(&Received) + Send + Sync>;

/// What the listeners share.
#[derive(Clone)]
struct Shared {
    mode: Mode,
    kept: Arc<Mutex<Kept>>,
    watch: Option<Watch>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("mode", &self.mode)
            .field("kept", &self.kept)
            .finish_non_exhaustive()
    }
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
        let received = Received {
            transport,
            signal,
            headers,
            line,
        };
        if let Some(watch) = &self.watch {
            watch(&received);
        }
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .push(received);
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
        .add_service(
            TraceServiceServer::new(services.clone()).accept_compressed(CompressionEncoding::Gzip),
        )
        .add_service(LogsServiceServer::new(services).accept_compressed(CompressionEncoding::Gzip));
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

/// What a receiver's CA signed for it: the CA's certificate, the TLS
/// listeners' certificate, for the loopback address by number and by name,
/// and a client's certificate. Every key is made here and kept nowhere
/// else.
struct Certificates {
    ca: String,
    ca_der: CertificateDer<'static>,
    listener: String,
    listener_der: CertificateDer<'static>,
    listener_key: KeyPair,
    client: String,
    client_key: String,
}

fn certificates() -> Certificates {
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
    let listener_key = must(KeyPair::generate(), "making the listener's key");
    let listener = must(
        listener.signed_by(&listener_key, &ca),
        "signing the listener's certificate",
    );
    let mut client = must(
        CertificateParams::new(Vec::new()),
        "stating the client's certificate",
    );
    client.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    client
        .distinguished_name
        .push(DnType::CommonName, "lablet test client");
    let client_key = must(KeyPair::generate(), "making the client's key");
    let client = must(
        client.signed_by(&client_key, &ca),
        "signing the client's certificate",
    );
    Certificates {
        ca: ca.pem(),
        ca_der: ca.der().clone(),
        listener: listener.pem(),
        listener_der: listener.der().clone(),
        listener_key,
        client: client.pem(),
        client_key: client_key.serialize_pem(),
    }
}

impl Certificates {
    /// The identity of a gRPC listener that speaks TLS.
    fn identity(&self) -> Identity {
        Identity::from_pem(&self.listener, self.listener_key.serialize_pem())
    }

    /// The acceptor of an HTTP listener that speaks TLS, which asks the
    /// client for a certificate the CA signed when `client_tls` says so.
    fn acceptor(&self, client_tls: bool) -> TlsAcceptor {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = must(
            rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
                .with_safe_default_protocol_versions(),
            "choosing the TLS versions",
        );
        let config = if client_tls {
            let mut roots = RootCertStore::empty();
            must(roots.add(self.ca_der.clone()), "trusting the CA");
            config.with_client_cert_verifier(must(
                WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider).build(),
                "making the client's verifier",
            ))
        } else {
            config.with_no_client_auth()
        };
        let mut config = must(
            config.with_single_cert(
                vec![self.listener_der.clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.listener_key.serialize_der())),
            ),
            "setting up the HTTPS listener",
        );
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        TlsAcceptor::from(Arc::new(config))
    }
}

/// A listener that speaks TLS, for the HTTP listeners: a connection whose
/// handshake fails is dropped, and the next one waited for.
struct TlsListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let Ok((stream, address)) = self.listener.accept().await else {
                continue;
            };
            if let Ok(stream) = self.acceptor.accept(stream).await {
                return (stream, address);
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

/// How an OTLP/HTTP request's body is written, and its response is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Protobuf,
    Json,
}

/// The request `body` holds, written as `headers` say: gzip, when its
/// content encoding is, and JSON, when its content type is, else protobuf.
fn decoded<T: prost::Message + Default + DeserializeOwned>(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(T, Encoding), String> {
    let gzip = headers
        .get(header::CONTENT_ENCODING)
        .is_some_and(|encoding| encoding.as_bytes().eq_ignore_ascii_case(b"gzip"));
    let body = if gzip {
        let mut plain = Vec::new();
        flate2::read::GzDecoder::new(body)
            .read_to_end(&mut plain)
            .map_err(|error| error.to_string())?;
        plain
    } else {
        body.to_vec()
    };
    let json = headers
        .get(header::CONTENT_TYPE)
        .is_some_and(|kind| kind.as_bytes().starts_with(JSON.as_bytes()));
    if json {
        serde_json::from_slice(&body)
            .map(|request| (request, Encoding::Json))
            .map_err(|error| error.to_string())
    } else {
        T::decode(body.as_slice())
            .map(|request| (request, Encoding::Protobuf))
            .map_err(|error| error.to_string())
    }
}

/// Answers an OTLP/HTTP request as a collector does: an empty response of
/// the signal's kind in the request's encoding.
fn answered<T: prost::Message + Serialize + Default>(
    encoding: Encoding,
) -> axum::response::Response {
    match encoding {
        Encoding::Protobuf => (
            [(header::CONTENT_TYPE, PROTOBUF)],
            T::default().encode_to_vec(),
        )
            .into_response(),
        Encoding::Json => (
            [(header::CONTENT_TYPE, JSON)],
            must(
                serde_json::to_vec(&T::default()),
                "writing a response as JSON",
            ),
        )
            .into_response(),
    }
}

/// What an HTTP listener shares with its handlers: what every listener
/// shares, and which of them it is.
#[derive(Debug, Clone)]
struct OverHttp {
    shared: Shared,
    transport: Transport,
}

async fn traces_over_http(
    State(over): State<OverHttp>,
    headers_sent: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let (request, encoding) = match decoded::<ExportTraceServiceRequest>(&headers_sent, &body) {
        Ok(decoded) => decoded,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    over.shared
        .took(
            over.transport,
            Signal::Traces,
            headers(&headers_sent),
            &request,
        )
        .await;
    answered::<ExportTraceServiceResponse>(encoding)
}

async fn logs_over_http(
    State(over): State<OverHttp>,
    headers_sent: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let (request, encoding) = match decoded::<ExportLogsServiceRequest>(&headers_sent, &body) {
        Ok(decoded) => decoded,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    over.shared
        .took(
            over.transport,
            Signal::Logs,
            headers(&headers_sent),
            &request,
        )
        .await;
    answered::<ExportLogsServiceResponse>(encoding)
}

/// The routes of an HTTP listener that is `transport`.
fn routes(shared: &Shared, transport: Transport) -> axum::Router {
    axum::Router::new()
        .route("/v1/traces", post(traces_over_http))
        .route("/v1/logs", post(logs_over_http))
        .with_state(OverHttp {
            shared: shared.clone(),
            transport,
        })
}

/// Binds a free port of the loopback interface, for a listener.
async fn bound(what: &str) -> TcpListener {
    must(TcpListener::bind("127.0.0.1:0").await, what)
}

/// Serves `app` on `listener` until the sender it returns is used or
/// dropped.
fn serve_http<L>(listener: L, app: axum::Router) -> oneshot::Sender<()>
where
    L: axum::serve::Listener,
    L::Addr: std::fmt::Debug,
{
    let (stop, stopped) = oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await;
    });
    stop
}

/// The receiver: six listeners, each stopped when the receiver is dropped.
#[derive(Debug)]
pub struct Receiver {
    grpc: SocketAddr,
    grpc_tls: SocketAddr,
    grpc_client_tls: SocketAddr,
    http: SocketAddr,
    https: SocketAddr,
    https_client_tls: SocketAddr,
    ca: String,
    client: String,
    client_key: String,
    kept: Arc<Mutex<Kept>>,
    stop: Vec<oneshot::Sender<()>>,
}

impl Receiver {
    /// Binds each listener to a free port on the loopback interface and
    /// serves them on the runtime this is called on, in `mode`, with a CA
    /// made for this receiver alone.
    ///
    /// # Panics
    ///
    /// Panics when a port can't be bound or the certificates can't be made,
    /// which is a fault of where the test runs.
    pub async fn start(mode: Mode) -> Self {
        Self::serving(mode, None).await
    }

    /// As [`Self::start`], and `watch` is called with each request as it
    /// arrives, before it's kept and answered: what holds at the moment a
    /// signal reaches a collector.
    ///
    /// # Panics
    ///
    /// As [`Self::start`].
    pub async fn watching(mode: Mode, watch: impl Fn(&Received) + Send + Sync + 'static) -> Self {
        Self::serving(mode, Some(Arc::new(watch))).await
    }

    async fn serving(mode: Mode, watch: Option<Watch>) -> Self {
        let shared = Shared {
            mode,
            kept: Arc::default(),
            watch,
        };
        let kept = Arc::clone(&shared.kept);
        let services = |transport| Services {
            shared: shared.clone(),
            transport,
        };
        let certificates = certificates();

        let (grpc, stop_grpc) = serve(Server::builder(), services(Transport::Grpc)).await;
        let tls = must(
            Server::builder().tls_config(ServerTlsConfig::new().identity(certificates.identity())),
            "setting up the TLS listener",
        );
        let (grpc_tls, stop_grpc_tls) = serve(tls, services(Transport::GrpcTls)).await;
        let client_tls = must(
            Server::builder().tls_config(
                ServerTlsConfig::new()
                    .identity(certificates.identity())
                    .client_ca_root(Certificate::from_pem(&certificates.ca)),
            ),
            "setting up the TLS listener that asks for a client's certificate",
        );
        let (grpc_client_tls, stop_grpc_client_tls) =
            serve(client_tls, services(Transport::GrpcClientTls)).await;

        let plain = bound("binding the HTTP listener").await;
        let http = must(plain.local_addr(), "reading the HTTP port");
        let stop_http = serve_http(plain, routes(&shared, Transport::Http));
        let secure = bound("binding the HTTPS listener").await;
        let https = must(secure.local_addr(), "reading the HTTPS port");
        let stop_https = serve_http(
            TlsListener {
                listener: secure,
                acceptor: certificates.acceptor(false),
            },
            routes(&shared, Transport::Https),
        );
        let client_listener =
            bound("binding the HTTPS listener for clients with a certificate").await;
        let https_client_tls = must(client_listener.local_addr(), "reading the HTTPS port");
        let stop_https_client_tls = serve_http(
            TlsListener {
                listener: client_listener,
                acceptor: certificates.acceptor(true),
            },
            routes(&shared, Transport::HttpsClientTls),
        );

        Self {
            grpc,
            grpc_tls,
            grpc_client_tls,
            http,
            https,
            https_client_tls,
            ca: certificates.ca,
            client: certificates.client,
            client_key: certificates.client_key,
            kept,
            stop: vec![
                stop_grpc,
                stop_grpc_tls,
                stop_grpc_client_tls,
                stop_http,
                stop_https,
                stop_https_client_tls,
            ],
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

    /// The endpoint of the gRPC listener that speaks TLS and takes only a
    /// client that shows [`Self::client_certificate`], as a config states
    /// it.
    #[must_use]
    pub fn grpc_client_tls_endpoint(&self) -> String {
        format!("https://{}", self.grpc_client_tls)
    }

    /// The certificate of the CA that signed the TLS listeners', as PEM: a
    /// file of it is what `SSL_CERT_FILE` or the exporter's certificate
    /// variable names for a client that trusts it.
    #[must_use]
    pub fn ca_certificate(&self) -> &str {
        &self.ca
    }

    /// A client's certificate the receiver's CA signed, as PEM, which the
    /// listeners that ask for one take.
    #[must_use]
    pub fn client_certificate(&self) -> &str {
        &self.client
    }

    /// The private key of [`Self::client_certificate`], as PEM. It's test
    /// material, made for this receiver alone.
    #[must_use]
    pub fn client_key(&self) -> &str {
        &self.client_key
    }

    /// The HTTP listener's endpoint, as a config states it: the base URL,
    /// to which the exporter appends each signal's path.
    #[must_use]
    pub fn http_endpoint(&self) -> String {
        format!("http://{}", self.http)
    }

    /// The HTTP listener that speaks TLS, as a config states it: the base
    /// URL. Its certificate is the gRPC TLS listener's.
    #[must_use]
    pub fn https_endpoint(&self) -> String {
        format!("https://{}", self.https)
    }

    /// The HTTP listener that speaks TLS and takes only a client that shows
    /// [`Self::client_certificate`], as a config states it: the base URL.
    #[must_use]
    pub fn https_client_tls_endpoint(&self) -> String {
        format!("https://{}", self.https_client_tls)
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
