//! The OTLP network exporters: `opentelemetry-otlp`'s, one for each signal,
//! each made by the builder of its own protocol, gRPC, HTTP/protobuf or
//! HTTP/JSON, with every setting lablet resolved stated on it.
//!
//! The exporter's builders read the process environment into their
//! defaults. Lablet states every value they read, and gives the gRPC
//! exporter a channel of its own and the HTTP exporter a client of its own,
//! each with lablet's TLS, so their reading never decides. Headers are
//! merged the other way, the process environment's over the code's, so on
//! each request, after that merge, a gRPC interceptor and a client that
//! wraps the HTTP one replace them: the exporter's own headers are kept,
//! but for a name the environment's header variables set, and the resolved
//! set is put on. Compression is the one setting the exporter's reading
//! still decides, when lablet resolves none, since its builder has no
//! setting for none; lablet reads the variables as it reads them.

use std::fmt;
use std::str::FromStr as _;
use std::sync::Arc;

use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::{
    Compression, ExporterBuildError, LogExporter, Protocol, SpanExporter, WithExportConfig as _,
    WithHttpConfig as _, WithTonicConfig as _,
};
use reqwest::header::{
    CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT,
};
use secrecy::{ExposeSecret as _, SecretString};
use tonic::metadata::MetadataMap;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity, Uri};

pub(crate) use crate::otlp::OtlpSettings;
use crate::otlp::{Destination, Tls, made, with_tls};

/// How an OTLP collector is spoken to, as the specification's protocols
/// name it. One is always chosen, since the exporter would otherwise pick
/// its own, and with JSON compiled in its own is JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Transport {
    /// OTLP over gRPC.
    Grpc,
    /// OTLP over HTTP, as protobuf.
    HttpProtobuf,
    /// OTLP over HTTP, as JSON.
    HttpJson,
}

impl Transport {
    /// Whether this is one of the HTTP protocols, whose endpoints take the
    /// signal's path.
    pub(crate) const fn is_http(self) -> bool {
        matches!(self, Self::HttpProtobuf | Self::HttpJson)
    }
}

/// Which signal an exporter is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Signal {
    /// Spans.
    Traces,
    /// Log records.
    Logs,
}

impl Signal {
    /// The path an OTLP/HTTP collector takes this signal at.
    pub(crate) const fn path(self) -> &'static str {
        match self {
            Self::Traces => "/v1/traces",
            Self::Logs => "/v1/logs",
        }
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Traces => "traces",
            Self::Logs => "logs",
        })
    }
}

/// Why the network exporters couldn't be made. No message holds the
/// endpoint or a header's value: either may carry credentials.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OtelBuildError {
    /// The endpoint isn't one the exporter accepts.
    #[error("the OTLP endpoint for {signal} is refused: it isn't a URL the exporter accepts")]
    Endpoint {
        /// The signal whose exporter refused it.
        signal: Signal,
    },
    /// A header's name or value isn't one a header may have.
    #[error("the OTLP header `{name}` is refused: {reason}")]
    Header {
        /// The header's name, as the config writes it.
        name: String,
        /// What's wrong with it.
        reason: &'static str,
    },
    /// The exporter failed to make itself, for a reason of its own.
    #[error("the OTLP exporter for {signal} couldn't be made: {reason}")]
    Exporter {
        /// The signal the exporter is of.
        signal: Signal,
        /// What the exporter said.
        reason: String,
    },
    /// TLS to the collector couldn't be set up, which is found when the
    /// exporter is made: the platform's trust roots couldn't be loaded,
    /// say.
    #[error("TLS for the OTLP exporter for {signal} couldn't be set up: {reason}")]
    Tls {
        /// The signal the exporter is of.
        signal: Signal,
        /// What stood in the way.
        reason: String,
    },
    /// The HTTP client the exports go through couldn't be made.
    #[error("the OTLP HTTP client for {signal} couldn't be made: {reason}")]
    HttpClient {
        /// The signal the exporter is of.
        signal: Signal,
        /// What stood in the way.
        reason: String,
    },
}

/// The headers `value` names, as the exporter reads them from
/// `OTEL_EXPORTER_OTLP_HEADERS` and its per-signal forms: `name=value`
/// pairs between commas, each trimmed, with the value percent-decoded. A
/// pair whose name or value is empty is left out, and a value whose
/// percent-escapes don't decode is kept as it's written.
#[must_use]
pub(crate) fn decode_headers(value: &str) -> Vec<(String, String)> {
    value
        .split_terminator(',')
        .map(str::trim)
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            let decoded = percent_decoded(value.trim()).unwrap_or_else(|| value.to_owned());
            (!name.trim().is_empty() && !decoded.is_empty())
                .then(|| (name.trim().to_owned(), decoded))
        })
        .collect()
}

/// `value` with each `%xx` replaced by its byte, or nothing when an escape
/// is cut short, isn't hex, or the bytes aren't UTF-8.
fn percent_decoded(value: &str) -> Option<String> {
    let mut decoded = String::with_capacity(value.len());
    let mut bytes = Vec::new();
    let mut chars = value.chars();
    loop {
        let next = chars.next();
        if next == Some('%') {
            let escape = [chars.next()?, chars.next()?];
            bytes.push(u8::from_str_radix(&escape.iter().collect::<String>(), 16).ok()?);
            continue;
        }
        if !bytes.is_empty() {
            decoded.push_str(std::str::from_utf8(&bytes).ok()?);
            bytes.clear();
        }
        match next {
            Some(char) => decoded.push(char),
            None => return Some(decoded),
        }
    }
}

/// The headers of one signal's exporter: those the exporter sets of its
/// own on its transport, which are kept, the names the environment's
/// header variables set, which never are, and the resolved set, which goes
/// on.
#[derive(Debug)]
struct Headers {
    set: Vec<(HeaderName, SecretString)>,
    own: &'static [HeaderName],
    taken_off: Vec<HeaderName>,
}

/// What the gRPC exporter puts in a request's metadata of its own, before
/// the environment's headers.
static GRPC_OWN: [HeaderName; 1] = [USER_AGENT];

/// What the HTTP exporter puts on a request of its own: the body's type and
/// encoding, and its user agent.
static HTTP_OWN: [HeaderName; 3] = [CONTENT_TYPE, CONTENT_ENCODING, USER_AGENT];

impl Headers {
    /// Replaces `headers`, which the exporter has given what it read of the
    /// process environment, with the exporter's own and the resolved set,
    /// so a header the environment sets reaches a request only through
    /// what lablet resolved, whatever the exporter read.
    fn apply(&self, headers: &mut HeaderMap) {
        let mut kept = HeaderMap::new();
        for name in self.own {
            if self.taken_off.contains(name) {
                continue;
            }
            if let Some(value) = headers.remove(name) {
                kept.insert(name, value);
            }
        }
        *headers = kept;
        for (name, value) in &self.set {
            // Every value was checked when the exporter was made.
            let Ok(mut value) = HeaderValue::from_str(value.expose_secret()) else {
                continue;
            };
            value.set_sensitive(true);
            headers.insert(name, value);
        }
    }
}

/// The interceptor of a gRPC exporter, which runs after the exporter has
/// appended the process environment's metadata.
fn interceptor(
    headers: Arc<Headers>,
) -> impl FnMut(tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> + Clone {
    move |mut request| {
        let mut map = std::mem::replace(request.metadata_mut(), MetadataMap::new()).into_headers();
        headers.apply(&mut map);
        *request.metadata_mut() = MetadataMap::from_headers(map);
        Ok(request)
    }
}

/// The HTTP client of an HTTP exporter: a blocking client, as the exporter
/// would make for itself, since the SDK's batch threads have no reactor to
/// poll an async one on, behind the resolved headers.
#[derive(Debug)]
struct Blocking {
    client: reqwest::blocking::Client,
    headers: Arc<Headers>,
}

#[async_trait::async_trait]
impl HttpClient for Blocking {
    async fn send_bytes(&self, mut request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        self.headers.apply(request.headers_mut());
        self.client.send_bytes(request).await
    }
}

/// The blocking client of `destination`, with its timeout and its TLS. A
/// certificate the environment names replaces the platform's roots, as
/// other SDKs use it.
fn blocking_client(
    signal: Signal,
    destination: &Destination,
) -> Result<reqwest::blocking::Client, OtelBuildError> {
    let fault = |reason: String| OtelBuildError::HttpClient { signal, reason };
    let builder = with_tls(
        reqwest::blocking::Client::builder().timeout(destination.timeout),
        destination.tls.roots.as_deref(),
        destination.tls.identity.as_ref(),
    )
    .map_err(|error| OtelBuildError::Tls {
        signal,
        reason: chain(&error),
    })?;
    made(builder)
        .map_err(fault)?
        .map_err(|error| fault(chain(&error)))
}

/// `error` and each of its causes, in turn.
fn chain(error: &dyn std::error::Error) -> String {
    let mut reason = error.to_string();
    let mut cause = error.source();
    while let Some(error) = cause {
        reason = format!("{reason}: {error}");
        cause = error.source();
    }
    reason
}

/// The TLS a gRPC exporter speaks to an `https` endpoint: the certificates
/// the environment names in place of the platform's roots, or else the
/// roots `with_enabled_roots` turns on, those the `tls-roots` feature of
/// `opentelemetry-otlp` builds tonic with: the platform's, as
/// `rustls-native-certs` loads them, or in their place the certificates
/// `SSL_CERT_FILE` and `SSL_CERT_DIR` name. And the client's identity,
/// when the environment names one.
fn client_tls(tls: &Tls) -> ClientTlsConfig {
    let config = match &tls.roots {
        Some(roots) => ClientTlsConfig::new().ca_certificate(Certificate::from_pem(roots)),
        None => ClientTlsConfig::new().with_enabled_roots(),
    };
    match &tls.identity {
        Some(identity) => config.identity(Identity::from_pem(
            &identity.certificate,
            identity.key.expose_secret(),
        )),
        None => config,
    }
}

/// The channel of a gRPC exporter to `destination`, as the exporter would
/// make it, but with lablet's TLS: lazy, so nothing connects until the
/// first export. It needs a tokio runtime to be made in, for the channel's
/// worker.
fn channel(signal: Signal, destination: &Destination) -> Result<Channel, OtelBuildError> {
    Ok(endpoint(signal, destination)?.connect_lazy())
}

/// The endpoint a gRPC exporter's channel to `destination` is made from,
/// each stage of an export bounded by the destination's timeout. tonic's
/// timeout bounds the request alone, and a lazy channel connects, and
/// shakes hands over TLS, before the request is sent, so the connection and
/// the handshake are bounded too, or a collector that holds the socket and
/// never answers holds the export as long as it likes.
pub(super) fn endpoint(
    signal: Signal,
    destination: &Destination,
) -> Result<Endpoint, OtelBuildError> {
    let endpoint = Endpoint::from_shared(destination.endpoint.expose_secret().to_owned())
        .map_err(|_| OtelBuildError::Endpoint { signal })?
        .timeout(destination.timeout)
        .connect_timeout(destination.timeout);
    if speaks_tls(&endpoint) {
        endpoint
            .tls_config(client_tls(&destination.tls).timeout(destination.timeout))
            .map_err(|error| OtelBuildError::Tls {
                signal,
                reason: chain(&error),
            })
    } else {
        Ok(endpoint)
    }
}

/// Whether the exporter speaks TLS to `endpoint`, which it does for
/// `https`.
fn speaks_tls(endpoint: &Endpoint) -> bool {
    endpoint.uri().scheme_str() == Some("https")
}

/// Whether `endpoint`, which always has its scheme, is a URL
/// `transport`'s exporter accepts, parsed as it parses it.
#[must_use]
pub(crate) fn accepted(transport: Transport, endpoint: &str) -> bool {
    match transport {
        Transport::Grpc => Endpoint::from_shared(endpoint.to_owned()).is_ok(),
        Transport::HttpProtobuf | Transport::HttpJson => endpoint.parse::<Uri>().is_ok_and(|uri| {
            matches!(uri.scheme_str(), Some("http" | "https")) && uri.authority().is_some()
        }),
    }
}

/// Holds `settings` to what the exporters take, making none: each
/// endpoint, parsed as its protocol's exporter parses it, and each header's
/// name and value. What can only be made is left to the build: the HTTP
/// client, and the trust roots of a gRPC exporter that speaks TLS.
///
/// # Errors
///
/// Returns what [`TelemetryBuilder::build`](super::telemetry::TelemetryBuilder::build)
/// returns for the same settings, so a check refuses what a build refuses,
/// but for what only making the exporters finds: [`OtelBuildError::Tls`],
/// [`OtelBuildError::HttpClient`] and [`OtelBuildError::Exporter`].
pub(crate) fn validate(settings: &OtlpSettings) -> Result<(), OtelBuildError> {
    for (signal, destination) in settings.destinations() {
        if !accepted(destination.transport, destination.endpoint.expose_secret()) {
            return Err(OtelBuildError::Endpoint { signal });
        }
        headers_of(destination)?;
    }
    Ok(())
}

/// The headers of `destination`, each one checked.
fn headers_of(destination: &Destination) -> Result<Headers, OtelBuildError> {
    let own: &'static [HeaderName] = if destination.transport == Transport::Grpc {
        &GRPC_OWN
    } else {
        &HTTP_OWN
    };
    let set = destination
        .headers
        .iter()
        .map(|(name, value)| {
            let refused = |reason| OtelBuildError::Header {
                name: name.clone(),
                reason,
            };
            let header = HeaderName::from_str(name)
                .map_err(|_| refused("its name isn't one a header may have"))?;
            HeaderValue::from_str(value.expose_secret())
                .map_err(|_| refused("its value isn't one a header may have"))?;
            Ok((header, value.clone()))
        })
        .collect::<Result<_, _>>()?;
    let taken_off = destination
        .taken_off
        .iter()
        .filter_map(|name| HeaderName::from_str(name).ok())
        .collect();
    Ok(Headers {
        set,
        own,
        taken_off,
    })
}

/// The exporters `settings` describe, of spans and of log records, each
/// when its signal is exported. They need a tokio runtime to be made in,
/// for a gRPC channel's worker.
///
/// # Errors
///
/// Returns an [`OtelBuildError`] when an endpoint or a header is refused,
/// TLS to a collector can't be set up, or an exporter or its client can't
/// be made.
pub(crate) fn exporters(
    settings: &OtlpSettings,
) -> Result<(Option<SpanExporter>, Option<LogExporter>), OtelBuildError> {
    validate(settings)?;
    let spans = settings.traces.as_ref().map(span_exporter).transpose()?;
    let records = settings.logs.as_ref().map(log_exporter).transpose()?;
    Ok((spans, records))
}

/// The protocol the HTTP exporter is told, which is always told: with
/// `http-json` compiled in, the exporter's own default is JSON.
const fn http_protocol(transport: Transport) -> Protocol {
    match transport {
        Transport::HttpJson => Protocol::HttpJson,
        Transport::Grpc | Transport::HttpProtobuf => Protocol::HttpBinary,
    }
}

/// What the exporter's own refusal comes to, without its message, which
/// may hold the endpoint whole.
fn refused(signal: Signal, error: ExporterBuildError) -> OtelBuildError {
    match error {
        ExporterBuildError::InvalidConfiguration(_) => OtelBuildError::Endpoint { signal },
        ExporterBuildError::InternalFailure(reason) => OtelBuildError::Exporter { signal, reason },
    }
}

/// Compression is stated only when it's gzip: the exporter's builder has
/// no setting for none, and when none is resolved the exporter's own
/// reading of the same variables, which the seam reads as it does, comes
/// to none too.
macro_rules! exporter {
    ($name:ident, $exporter:ident, $signal:expr) => {
        fn $name(destination: &Destination) -> Result<$exporter, OtelBuildError> {
            let signal = $signal;
            let headers = Arc::new(headers_of(destination)?);
            if destination.transport == Transport::Grpc {
                let mut builder = $exporter::builder()
                    .with_tonic()
                    .with_channel(channel(signal, destination)?)
                    .with_interceptor(interceptor(headers))
                    .with_timeout(destination.timeout);
                if destination.gzip {
                    builder = builder.with_compression(Compression::Gzip);
                }
                builder.build().map_err(|error| refused(signal, error))
            } else {
                let mut builder = $exporter::builder()
                    .with_http()
                    .with_protocol(http_protocol(destination.transport))
                    .with_endpoint(destination.endpoint.expose_secret())
                    .with_timeout(destination.timeout)
                    .with_http_client(Blocking {
                        client: blocking_client(signal, destination)?,
                        headers,
                    });
                if destination.gzip {
                    builder = builder.with_compression(Compression::Gzip);
                }
                builder.build().map_err(|error| refused(signal, error))
            }
        }
    };
}

exporter!(span_exporter, SpanExporter, Signal::Traces);
exporter!(log_exporter, LogExporter, Signal::Logs);

#[cfg(test)]
mod tests;
