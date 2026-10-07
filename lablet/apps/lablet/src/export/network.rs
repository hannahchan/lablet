//! The OTLP network exporters: `opentelemetry-otlp`'s, over gRPC or
//! HTTP/protobuf, with the config's headers set where the exporter can't
//! merge the environment's over them.
//!
//! The exporter reads the `OTEL_*` variables itself, and a field the config
//! states is set on its builder, so the config's value wins by field. Headers
//! are the one field the exporter merges the other way, the environment's
//! over the code's, so the config's go on each request after that merge: in
//! a gRPC interceptor, and in a client that wraps the HTTP one.

use std::fmt;
use std::str::FromStr as _;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::{
    ExporterBuildError, LogExporter, OTEL_EXPORTER_OTLP_ENDPOINT, OTEL_EXPORTER_OTLP_HEADERS,
    OTEL_EXPORTER_OTLP_INSECURE, OTEL_EXPORTER_OTLP_LOGS_ENDPOINT, OTEL_EXPORTER_OTLP_LOGS_HEADERS,
    OTEL_EXPORTER_OTLP_LOGS_INSECURE, OTEL_EXPORTER_OTLP_LOGS_TIMEOUT, OTEL_EXPORTER_OTLP_TIMEOUT,
    OTEL_EXPORTER_OTLP_TIMEOUT_DEFAULT, OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
    OTEL_EXPORTER_OTLP_TRACES_HEADERS, OTEL_EXPORTER_OTLP_TRACES_INSECURE,
    OTEL_EXPORTER_OTLP_TRACES_TIMEOUT, Protocol, SpanExporter, WithExportConfig,
    WithHttpConfig as _, WithTonicConfig as _,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use tonic::metadata::MetadataMap;
use tonic::transport::{ClientTlsConfig, Endpoint, Uri};

/// How an OTLP collector is spoken to. The exporter would pick one itself
/// where none is chosen, so one is always chosen here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Transport {
    /// OTLP over gRPC.
    Grpc,
    /// OTLP over HTTP, as protobuf.
    HttpProtobuf,
}

/// Which signal an exporter is of, as the environment's variables name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Signal {
    /// Spans.
    Traces,
    /// Log records.
    Logs,
}

impl Signal {
    /// The variable that names the endpoint of this signal alone, which the
    /// exporter reads before the generic one.
    #[must_use]
    pub(crate) const fn endpoint_variable(self) -> &'static str {
        match self {
            Self::Traces => OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
            Self::Logs => OTEL_EXPORTER_OTLP_LOGS_ENDPOINT,
        }
    }

    /// The variable that names the headers of this signal alone.
    const fn headers_variable(self) -> &'static str {
        match self {
            Self::Traces => OTEL_EXPORTER_OTLP_TRACES_HEADERS,
            Self::Logs => OTEL_EXPORTER_OTLP_LOGS_HEADERS,
        }
    }

    /// The variable that says whether this signal alone is sent without
    /// TLS to a gRPC endpoint that has no scheme.
    const fn insecure_variable(self) -> &'static str {
        match self {
            Self::Traces => OTEL_EXPORTER_OTLP_TRACES_INSECURE,
            Self::Logs => OTEL_EXPORTER_OTLP_LOGS_INSECURE,
        }
    }

    /// The variable that gives this signal alone its timeout.
    const fn timeout_variable(self) -> &'static str {
        match self {
            Self::Traces => OTEL_EXPORTER_OTLP_TRACES_TIMEOUT,
            Self::Logs => OTEL_EXPORTER_OTLP_LOGS_TIMEOUT,
        }
    }

    /// The path an OTLP/HTTP collector takes this signal at.
    const fn path(self) -> &'static str {
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

/// What the config states of the network exporter. A field it leaves out is
/// the environment's to decide, as the exporter reads it.
pub(crate) struct OtlpSettings {
    /// How the collector is spoken to.
    pub(crate) transport: Transport,
    /// Where the collector listens. On HTTP it's a base URL, to which the
    /// signal's path is appended, as the exporter appends it to
    /// `OTEL_EXPORTER_OTLP_ENDPOINT`. An empty one is left out, as the
    /// exporter leaves out an empty one it's given.
    pub(crate) endpoint: Option<String>,
    /// The headers every export carries, which win over the environment's
    /// of the same name. A value is a secret from here on.
    pub(crate) headers: Vec<(String, String)>,
    /// Whether the headers the environment names are kept off every
    /// export, as they are when the endpoint is the config's and not the
    /// environment's.
    pub(crate) strip_environment_headers: bool,
}

impl fmt::Debug for OtlpSettings {
    /// Shows neither a header's value nor the endpoint, which may carry
    /// credentials.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OtlpSettings")
            .field("transport", &self.transport)
            .field("endpoint", &self.endpoint.as_ref().map(|_| ".."))
            .field("headers", &names_of(&self.headers))
            .field("strip_environment_headers", &self.strip_environment_headers)
            .finish()
    }
}

fn names_of<V>(headers: &[(String, V)]) -> Vec<&str> {
    headers.iter().map(|(name, _)| name.as_str()).collect()
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
    /// TLS to the collector couldn't be set up, which the gRPC exporter
    /// does when it's made: the trust roots couldn't be loaded, say.
    #[error("TLS for the OTLP exporter for {signal} couldn't be set up: {reason}")]
    Tls {
        /// The signal the exporter is of.
        signal: Signal,
        /// What stood in the way.
        reason: String,
    },
    /// The HTTP client the exports go through couldn't be made.
    #[error("the OTLP HTTP client couldn't be made: {reason}")]
    HttpClient {
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

/// The headers of one signal's exporter: the config's, which every export
/// carries, and the names the environment set for the signal, which no
/// export carries when the endpoint is the config's.
#[derive(Debug)]
struct Headers {
    set: Vec<(HeaderName, SecretString)>,
    strip: Vec<HeaderName>,
}

impl Headers {
    /// Sets the config's headers on `headers`, which the exporter has
    /// already given the environment's, in place of any value of the same
    /// name, and takes the environment's off.
    fn apply(&self, headers: &mut HeaderMap) {
        for name in &self.strip {
            headers.remove(name);
        }
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
/// appended the environment's metadata.
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
/// poll an async one on, behind the config's headers.
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

/// The blocking client, made on a thread of its own: it starts a runtime of
/// its own, which can't be done from inside another's.
fn blocking_client(timeout: Duration) -> Result<reqwest::blocking::Client, OtelBuildError> {
    let fault = |reason: String| OtelBuildError::HttpClient { reason };
    thread::Builder::new()
        .name("lablet-otlp-http-client".to_owned())
        .spawn(move || {
            reqwest::blocking::Client::builder()
                .timeout(timeout)
                .build()
        })
        .map_err(|error| fault(format!("its thread couldn't start: {error}")))?
        .join()
        .map_err(|_| fault("its thread didn't run to its end".to_owned()))?
        .map_err(|error| fault(error.to_string()))
}

/// The timeout the exporter resolves for `signal` when the config states
/// none: the signal's variable, else the generic one, in milliseconds, else
/// ten seconds. The exporter applies it to the HTTP client it makes for
/// itself, and only as the deadline of its retries to a client it's handed,
/// so the client made here is given the same.
fn timeout_of(signal: Signal) -> Duration {
    let millis = |variable: &str| std::env::var(variable).ok()?.parse().ok();
    millis(signal.timeout_variable())
        .or_else(|| millis(OTEL_EXPORTER_OTLP_TIMEOUT))
        .map_or(OTEL_EXPORTER_OTLP_TIMEOUT_DEFAULT, Duration::from_millis)
}

/// The names of the headers the environment sets for `signal`, read as the
/// exporter reads them: the signal's variable when it's set, even to
/// nothing, else the generic one.
fn environment_header_names(signal: Signal) -> Vec<HeaderName> {
    std::env::var(signal.headers_variable())
        .or_else(|_| std::env::var(OTEL_EXPORTER_OTLP_HEADERS))
        .map(|value| {
            decode_headers(&value)
                .into_iter()
                .filter_map(|(name, _)| HeaderName::from_str(&name).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// `endpoint` with `path` appended, as the exporter appends a signal's path
/// to `OTEL_EXPORTER_OTLP_ENDPOINT`.
fn at_path(endpoint: &str, path: &str) -> String {
    let path = match (endpoint.ends_with('/'), path.strip_prefix('/')) {
        (true, Some(rest)) => rest,
        _ => path,
    };
    format!("{endpoint}{path}")
}

/// `builder` with `endpoint` set, when the config states one.
fn with_endpoint<B: WithExportConfig>(builder: B, endpoint: Option<&str>) -> B {
    match endpoint {
        Some(endpoint) => builder.with_endpoint(endpoint),
        None => builder,
    }
}

/// What the exporter's own refusal comes to, without its message, which
/// holds the endpoint whole. The exporter refuses TLS it can't set up as it
/// refuses an endpoint it can't parse, so for an endpoint it speaks TLS to,
/// TLS is set up again alone to tell the two apart.
fn refused(of: &OfSignal, error: ExporterBuildError) -> OtelBuildError {
    let signal = of.signal;
    match error {
        ExporterBuildError::InvalidConfiguration(_) => match of.tls.as_ref().and_then(tls_fault) {
            Some(reason) => OtelBuildError::Tls { signal, reason },
            None => OtelBuildError::Endpoint { signal },
        },
        ExporterBuildError::InternalFailure(reason) => OtelBuildError::Exporter { signal, reason },
    }
}

/// The TLS a gRPC exporter is given for an `https` endpoint. Without it the
/// exporter sets up TLS that trusts no root, and so refuses every
/// certificate. The roots `with_enabled_roots` turns on are those the
/// `tls-roots` feature of `opentelemetry-otlp` builds tonic with: the
/// platform's, as `rustls-native-certs` loads them, or in their place the
/// certificates `SSL_CERT_FILE` and `SSL_CERT_DIR` name when either is set.
/// They're loaded when the exporter is made.
fn tls() -> ClientTlsConfig {
    ClientTlsConfig::new().with_enabled_roots()
}

/// Why TLS to `endpoint` can't be set up as the exporter sets it up, or
/// nothing when it can. No reason holds a part of the endpoint.
fn tls_fault(endpoint: &Endpoint) -> Option<String> {
    let error = endpoint.clone().tls_config(tls()).err()?;
    let mut reason = error.to_string();
    let mut cause = std::error::Error::source(&error);
    while let Some(error) = cause {
        reason = format!("{reason}: {error}");
        cause = error.source();
    }
    Some(reason)
}

/// Whether the gRPC exporter takes `endpoint`'s scheme from it, which it
/// does when a `://` comes before any `/`, `?` or `#`, and gives it one
/// otherwise.
fn has_scheme(endpoint: &str) -> bool {
    endpoint
        .split_once("://")
        .is_some_and(|(scheme, _)| !scheme.contains(['/', '?', '#']))
}

/// Whether the environment says `signal` is sent without TLS to a gRPC
/// endpoint that has no scheme, as the exporter reads it: the signal's
/// variable, else the generic one, is `true` in any case.
fn insecure(signal: Signal) -> bool {
    std::env::var(signal.insecure_variable())
        .or_else(|_| std::env::var(OTEL_EXPORTER_OTLP_INSECURE))
        .is_ok_and(|value| value.eq_ignore_ascii_case("true"))
}

/// The endpoint the gRPC exporter sends `signal` to, parsed as it parses
/// it, when it speaks TLS to it: the stated one, else the environment's,
/// given `https://` when it has no scheme unless the environment says it's
/// insecure. The exporter's own default is `http://`, and an endpoint it
/// can't parse it refuses before it sets up TLS.
fn over_tls(stated: Option<&str>, signal: Signal) -> Option<Endpoint> {
    let endpoint = match stated {
        Some(stated) => stated.to_owned(),
        None => environment_endpoint(Transport::Grpc, signal).ok()??,
    };
    let endpoint = if has_scheme(&endpoint) {
        endpoint
    } else if insecure(signal) {
        return None;
    } else {
        format!("https://{endpoint}")
    };
    Endpoint::from_shared(endpoint)
        .ok()
        .filter(|endpoint| endpoint.uri().scheme_str() == Some("https"))
}

/// Holds `settings` to what the exporters take, making none: each header's
/// name and value, and the endpoint each signal is sent to, parsed as its
/// transport's exporter parses it. An endpoint the settings state is read as
/// it is, with the signal's path on HTTP; one they leave out is read from
/// the environment as the exporter reads it, and the exporter's own default
/// needs no parsing. What can only be made is left to the build: the HTTP
/// client, and the trust roots of a gRPC exporter that speaks TLS.
///
/// # Errors
///
/// Returns what [`TelemetryBuilder::build`](super::telemetry::TelemetryBuilder::build)
/// returns for the same settings, so a check refuses what a build refuses,
/// but for what only making the exporters finds: [`OtelBuildError::Tls`],
/// [`OtelBuildError::HttpClient`] and [`OtelBuildError::Exporter`].
pub(crate) fn validate(settings: &OtlpSettings) -> Result<(), OtelBuildError> {
    let network = Network::new(settings)?;
    for signal in [Signal::Traces, Signal::Logs] {
        let endpoint = match network.of(signal).endpoint {
            Some(stated) => stated,
            None => match environment_endpoint(network.transport, signal)? {
                Some(inherited) => inherited,
                None => continue,
            },
        };
        if !accepted(network.transport, &endpoint) {
            return Err(OtelBuildError::Endpoint { signal });
        }
    }
    Ok(())
}

/// The endpoint the exporter reads for `signal` from the environment, as it
/// reads it: the signal's variable as it is, else the generic one with the
/// signal's path on HTTP, and `None` for the exporter's own default. A
/// variable set to nothing is unset, and one that isn't UTF-8 is refused,
/// as the exporter refuses it.
fn environment_endpoint(
    transport: Transport,
    signal: Signal,
) -> Result<Option<String>, OtelBuildError> {
    let read = |variable: &str| match std::env::var(variable) {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(OtelBuildError::Endpoint { signal }),
    };
    if let Some(own) = read(signal.endpoint_variable())? {
        return Ok(Some(own));
    }
    Ok(
        read(OTEL_EXPORTER_OTLP_ENDPOINT)?.map(|generic| match transport {
            Transport::Grpc => generic,
            Transport::HttpProtobuf => at_path(&generic, signal.path()),
        }),
    )
}

/// Whether `endpoint` is one `transport`'s exporter accepts, parsed as it
/// parses it. The gRPC exporter gives an endpoint without a scheme one
/// before it parses, `https://` unless it's told the endpoint is insecure,
/// and either scheme parses alike, so `https://` stands for both.
fn accepted(transport: Transport, endpoint: &str) -> bool {
    match transport {
        Transport::Grpc => {
            let endpoint = if has_scheme(endpoint) {
                endpoint.to_owned()
            } else {
                format!("https://{endpoint}")
            };
            tonic::transport::Endpoint::from_shared(endpoint).is_ok()
        }
        Transport::HttpProtobuf => endpoint.parse::<Uri>().is_ok(),
    }
}

/// The settings, with each header's value a secret and each header checked.
pub(crate) struct Network {
    transport: Transport,
    endpoint: Option<String>,
    headers: Vec<(HeaderName, SecretString)>,
    strip_environment_headers: bool,
}

/// What one signal's exporters share.
struct OfSignal {
    signal: Signal,
    headers: Arc<Headers>,
    endpoint: Option<String>,
    /// The endpoint the gRPC exporter speaks TLS to, when it does.
    tls: Option<Endpoint>,
}

impl Network {
    /// Takes `settings`, refusing a header whose name or value isn't one a
    /// header may have.
    pub(crate) fn new(settings: &OtlpSettings) -> Result<Self, OtelBuildError> {
        let OtlpSettings {
            transport,
            endpoint,
            headers,
            strip_environment_headers,
        } = settings;
        let headers = headers
            .iter()
            .map(|(name, value)| {
                let refused = |reason| OtelBuildError::Header {
                    name: name.clone(),
                    reason,
                };
                let header = HeaderName::from_str(name)
                    .map_err(|_| refused("its name isn't one a header may have"))?;
                HeaderValue::from_str(value)
                    .map_err(|_| refused("its value isn't one a header may have"))?;
                Ok((header, SecretString::from(value.clone())))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            transport: *transport,
            endpoint: endpoint.clone().filter(|endpoint| !endpoint.is_empty()),
            headers,
            strip_environment_headers: *strip_environment_headers,
        })
    }

    fn of(&self, signal: Signal) -> OfSignal {
        let endpoint = self
            .endpoint
            .as_deref()
            .map(|endpoint| match self.transport {
                Transport::Grpc => endpoint.to_owned(),
                Transport::HttpProtobuf => at_path(endpoint, signal.path()),
            });
        let strip = if self.strip_environment_headers {
            environment_header_names(signal)
        } else {
            Vec::new()
        };
        let tls = match self.transport {
            Transport::Grpc => over_tls(endpoint.as_deref(), signal),
            Transport::HttpProtobuf => None,
        };
        OfSignal {
            signal,
            headers: Arc::new(Headers {
                set: self.headers.clone(),
                strip,
            }),
            endpoint,
            tls,
        }
    }

    /// The two exporters, of spans and of log records. They need a tokio
    /// runtime to be made in, for the gRPC channel's worker.
    ///
    /// # Errors
    ///
    /// Returns an [`OtelBuildError`] when the exporter refuses the endpoint,
    /// can't set up TLS to it, or can't make itself.
    pub(crate) fn exporters(&self) -> Result<(SpanExporter, LogExporter), OtelBuildError> {
        let traces = self.of(Signal::Traces);
        let logs = self.of(Signal::Logs);
        match self.transport {
            Transport::Grpc => Ok((grpc_spans(&traces)?, grpc_logs(&logs)?)),
            Transport::HttpProtobuf => Ok((
                http_spans(&traces, &blocking_client(timeout_of(Signal::Traces))?)?,
                http_logs(&logs, &blocking_client(timeout_of(Signal::Logs))?)?,
            )),
        }
    }
}

fn grpc_spans(of: &OfSignal) -> Result<SpanExporter, OtelBuildError> {
    let mut builder = SpanExporter::builder()
        .with_tonic()
        .with_interceptor(interceptor(Arc::clone(&of.headers)));
    if of.tls.is_some() {
        builder = builder.with_tls_config(tls());
    }
    with_endpoint(builder, of.endpoint.as_deref())
        .build()
        .map_err(|error| refused(of, error))
}

fn grpc_logs(of: &OfSignal) -> Result<LogExporter, OtelBuildError> {
    let mut builder = LogExporter::builder()
        .with_tonic()
        .with_interceptor(interceptor(Arc::clone(&of.headers)));
    if of.tls.is_some() {
        builder = builder.with_tls_config(tls());
    }
    with_endpoint(builder, of.endpoint.as_deref())
        .build()
        .map_err(|error| refused(of, error))
}

/// The protocol is set on the builder, as the transport is chosen here,
/// so `OTEL_EXPORTER_OTLP_PROTOCOL` can't make it JSON.
fn http_spans(
    of: &OfSignal,
    client: &reqwest::blocking::Client,
) -> Result<SpanExporter, OtelBuildError> {
    let builder = SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_http_client(Blocking {
            client: client.clone(),
            headers: Arc::clone(&of.headers),
        });
    with_endpoint(builder, of.endpoint.as_deref())
        .build()
        .map_err(|error| refused(of, error))
}

fn http_logs(
    of: &OfSignal,
    client: &reqwest::blocking::Client,
) -> Result<LogExporter, OtelBuildError> {
    let builder = LogExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_http_client(Blocking {
            client: client.clone(),
            headers: Arc::clone(&of.headers),
        });
    with_endpoint(builder, of.endpoint.as_deref())
        .build()
        .map_err(|error| refused(of, error))
}

#[cfg(test)]
mod tests;
