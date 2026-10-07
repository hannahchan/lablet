//! Where and how each signal is exported over the network, resolved from
//! the config and what the seam read of the `OTEL_*` environment: a field
//! the config states wins, and a field it leaves `null` is the
//! environment's, the signal's own variable before the generic one, and
//! then the specification's default. Every value is resolved here, the
//! endpoint always with its scheme, so the exporter's builders are told
//! each one and their own reading of the process environment never
//! decides, but for no compression, which a builder has no setting for.

use std::fmt;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use crate::config::{Config, ConfigError, Otlp, OtlpProtocol, Refusal};
use crate::export::{Signal, Transport};
use crate::otel_env;

/// The endpoint of an OTLP/HTTP collector when nothing names one, to which
/// each signal's path is appended.
const HTTP_DEFAULT: &str = "http://localhost:4318";

/// The endpoint of an OTLP/gRPC collector when nothing names one.
const GRPC_DEFAULT: &str = "http://localhost:4317";

/// The key a refusal of what the exporter is sent to is made under.
const ENDPOINT_KEY: &str = "telemetry.otlp.endpoint";

/// A value that may be a secret, which no `Debug` form shows.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Hidden(pub(crate) String);

impl fmt::Debug for Hidden {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("..")
    }
}

/// A value, and the variable it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Variable<T> {
    /// The variable.
    pub(crate) name: &'static str,
    /// What it holds, as the seam read it.
    pub(crate) value: T,
}

/// The endpoint the environment names for a signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InheritedEndpoint {
    /// The variable it was read from.
    pub(crate) variable: &'static str,
    /// Whether that's the generic variable, which takes the signal's path
    /// on HTTP, where the signal's own is taken as it's written.
    pub(crate) generic: bool,
    /// What it holds, as text: a value that isn't UTF-8 is read lossily,
    /// and then parses as no URL.
    pub(crate) text: Hidden,
}

/// What the environment gives one signal's exporter, as the seam reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Inherited {
    /// Whether the signal's selector turns its OTLP exporter on, or
    /// `None` when it says nothing lablet serves.
    pub(crate) selected: Option<bool>,
    /// The protocol, when a variable names one lablet serves.
    pub(crate) protocol: Option<Transport>,
    /// The endpoint, when a variable names one.
    pub(crate) endpoint: Option<InheritedEndpoint>,
    /// The headers: the signal's variable's when it's set, even to pairs
    /// that are all dropped, else the generic variable's, when it's set.
    pub(crate) headers: Option<Vec<(String, Hidden)>>,
    /// Every header name either variable sets, which comes off every
    /// request whichever of them the exporter merged in.
    pub(crate) header_names: Vec<String>,
    /// How long one export may take.
    pub(crate) timeout: Duration,
    /// Whether exports are compressed with gzip.
    pub(crate) gzip: bool,
    /// Whether a gRPC endpoint without a scheme is spoken to without TLS.
    pub(crate) insecure: bool,
    /// The file of the certificates the exporter trusts, in place of the
    /// platform's roots.
    pub(crate) certificate: Option<Variable<PathBuf>>,
    /// The file of the client's private key.
    pub(crate) client_key: Option<Variable<PathBuf>>,
    /// The file of the client's certificate.
    pub(crate) client_certificate: Option<Variable<PathBuf>>,
}

/// What the network exporters are built from: each signal's destination,
/// when its signal is exported over the network.
pub(crate) struct OtlpSettings {
    /// Where spans go.
    pub(crate) traces: Option<Destination>,
    /// Where log records go.
    pub(crate) logs: Option<Destination>,
}

impl OtlpSettings {
    /// Each destination, with its signal.
    pub(crate) fn destinations(&self) -> impl Iterator<Item = (Signal, &Destination)> {
        [(Signal::Traces, &self.traces), (Signal::Logs, &self.logs)]
            .into_iter()
            .filter_map(|(signal, destination)| Some((signal, destination.as_ref()?)))
    }

    /// The variable each destination's endpoint was read from: `None` for
    /// one the config states, or the protocol's default.
    pub(crate) fn endpoint_variables(&self) -> [Option<&'static str>; 2] {
        [&self.traces, &self.logs]
            .map(|destination| destination.as_ref().and_then(|of| of.endpoint_variable))
    }

    /// The client keys the exporters read, each with the variable that
    /// names its file, once each.
    pub(crate) fn client_keys(&self) -> Vec<(&'static str, &SecretString)> {
        let mut keys: Vec<(&'static str, &SecretString)> = Vec::new();
        for (_, destination) in self.destinations() {
            if let Some(identity) = &destination.tls.identity
                && !keys
                    .iter()
                    .any(|(variable, _)| *variable == identity.key_variable)
            {
                keys.push((identity.key_variable, &identity.key));
            }
        }
        keys
    }
}

impl fmt::Debug for OtlpSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OtlpSettings")
            .field("traces", &self.traces)
            .field("logs", &self.logs)
            .finish()
    }
}

/// Where one signal's exporter sends, and how, every value resolved.
pub(crate) struct Destination {
    /// The protocol.
    pub(crate) transport: Transport,
    /// The endpoint, with its scheme, and with the signal's path on HTTP.
    /// It may carry credentials.
    pub(crate) endpoint: SecretString,
    /// The variable the endpoint was read from, or `None` when the config
    /// states it or it's the protocol's default.
    pub(crate) endpoint_variable: Option<&'static str>,
    /// The headers every export carries, and the only ones the environment
    /// may have: the config's, when it states any, else the
    /// environment's.
    pub(crate) headers: Vec<(String, SecretString)>,
    /// Every header name the environment's variables set, which comes off
    /// every request before `headers` go on.
    pub(crate) taken_off: Vec<String>,
    /// How long one export may take, its retries included.
    pub(crate) timeout: Duration,
    /// Whether exports are compressed with gzip.
    pub(crate) gzip: bool,
    /// What the exporter trusts and shows of itself over TLS.
    pub(crate) tls: Tls,
}

impl fmt::Debug for Destination {
    /// Shows neither a header's value nor the endpoint, which may carry
    /// credentials.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Destination")
            .field("transport", &self.transport)
            .field("endpoint_variable", &self.endpoint_variable)
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("taken_off", &self.taken_off)
            .field("timeout", &self.timeout)
            .field("gzip", &self.gzip)
            .field("tls", &self.tls)
            .finish_non_exhaustive()
    }
}

/// The TLS material the environment names, loaded.
#[derive(Debug, Default)]
pub(crate) struct Tls {
    /// The certificates trusted in place of the platform's roots, as PEM.
    pub(crate) roots: Option<Vec<u8>>,
    /// The client's identity, for a collector that asks for one.
    pub(crate) identity: Option<ClientIdentity>,
}

/// A client certificate and its private key, as PEM.
pub(crate) struct ClientIdentity {
    /// The certificate.
    pub(crate) certificate: Vec<u8>,
    /// The private key, which is a secret.
    pub(crate) key: SecretString,
    /// The variable that names the key's file.
    pub(crate) key_variable: &'static str,
}

impl fmt::Debug for ClientIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientIdentity")
            .field("key_variable", &self.key_variable)
            .finish_non_exhaustive()
    }
}

impl From<OtlpProtocol> for Transport {
    fn from(protocol: OtlpProtocol) -> Self {
        match protocol {
            OtlpProtocol::Grpc => Self::Grpc,
            OtlpProtocol::HttpProtobuf => Self::HttpProtobuf,
            OtlpProtocol::HttpJson => Self::HttpJson,
        }
    }
}

/// What the network exporters are built from, for `real`, a config with
/// `${VAR}` substituted, and `exporter`, what the seam read of the
/// environment, or `None` when no signal is exported over the network:
/// `OTEL_SDK_DISABLED` is `true`, `telemetry.otlp.enabled` is `false`, or
/// it's `null` and each signal's selector is `none`.
///
/// The TLS material the environment names is read and loaded here for
/// each signal that's exported over the network, whether or not its
/// endpoint speaks TLS, so a check reads what a build reads, a file that
/// can't be loaded is refused wherever the exporter is on, and a client
/// key's contents are cut whenever lablet holds them.
///
/// # Errors
///
/// Returns a [`ConfigError`] shown from `written`, the same config before
/// substitution: `telemetry.otlp.enabled: false` beside an endpoint, which
/// it would make a setting without effect; and a certificate, client key
/// or client certificate the environment names that can't be loaded, or a
/// client key or certificate named without the other, naming the variable
/// and never what the file holds.
pub(crate) fn settings(
    written: &Config,
    real: &Config,
    exporter: &otel_env::Exporter,
) -> Result<Option<OtlpSettings>, ConfigError> {
    let otlp = &real.telemetry.otlp;
    let endpoint = otlp
        .endpoint
        .as_deref()
        .filter(|endpoint| !endpoint.is_empty());
    if otlp.enabled == Some(false) && endpoint.is_some() {
        // An endpoint that holds an `@` is shown by its key alone, as the
        // refusal of an endpoint shows it, since user information is a
        // secret and where it ends can't be told.
        let shown = match written.written_text(ENDPOINT_KEY) {
            Some(endpoint) if !endpoint.contains('@') => {
                format!("`telemetry.otlp.endpoint: {endpoint}`")
            }
            _ => "`telemetry.otlp.endpoint`".to_owned(),
        };
        return Err(written.refused(Refusal::invalid(
            "telemetry.otlp.enabled",
            format!(
                "it turns the network exporter off, and {shown} names where it sends; a config \
                 that wants it off states no endpoint"
            ),
        )));
    }
    if exporter.sdk_disabled {
        return Ok(None);
    }
    let traces = destination(written, otlp, endpoint, Signal::Traces, &exporter.traces)?;
    let logs = destination(written, otlp, endpoint, Signal::Logs, &exporter.logs)?;
    if traces.is_none() && logs.is_none() {
        return Ok(None);
    }
    Ok(Some(OtlpSettings { traces, logs }))
}

/// Where `signal` goes, or `None` when it isn't exported over the network.
fn destination(
    written: &Config,
    otlp: &Otlp,
    stated: Option<&str>,
    signal: Signal,
    inherited: &Inherited,
) -> Result<Option<Destination>, ConfigError> {
    if !otlp.enabled.or(inherited.selected).unwrap_or(true) {
        return Ok(None);
    }
    let transport = otlp
        .protocol
        .map(Transport::from)
        .or(inherited.protocol)
        .unwrap_or(Transport::HttpProtobuf);
    let (endpoint, endpoint_variable) = match (stated, &inherited.endpoint) {
        (Some(stated), _) => (at_signal(stated, transport, signal), None),
        (None, Some(named)) if named.generic => (
            at_signal(&named.text.0, transport, signal),
            Some(named.variable),
        ),
        (None, Some(named)) => (named.text.0.clone(), Some(named.variable)),
        (None, None) => match transport {
            Transport::Grpc => (GRPC_DEFAULT.to_owned(), None),
            Transport::HttpProtobuf | Transport::HttpJson => {
                (at_signal(HTTP_DEFAULT, transport, signal), None)
            }
        },
    };
    let endpoint = with_scheme(endpoint, transport, inherited.insecure);
    let headers = match &otlp.headers {
        Some(stated) => stated
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
            .collect(),
        None => inherited
            .headers
            .iter()
            .flatten()
            .map(|(name, value)| (name.clone(), SecretString::from(value.0.clone())))
            .collect(),
    };
    let tls = tls(written, signal, stated.is_some(), inherited)?;
    if !speaks_tls(&endpoint) {
        for variable in [
            &inherited.certificate,
            &inherited.client_key,
            &inherited.client_certificate,
        ]
        .into_iter()
        .flatten()
        {
            tracing::warn!(
                "`{}` names a file for TLS, and the endpoint for {signal} doesn't speak TLS, so \
                 the file isn't used, though it's still loaded and refused when it can't be",
                variable.name
            );
        }
    }
    Ok(Some(Destination {
        transport,
        endpoint: SecretString::from(endpoint),
        endpoint_variable,
        headers,
        taken_off: inherited.header_names.clone(),
        timeout: inherited.timeout,
        gzip: inherited.gzip,
        tls,
    }))
}

/// `endpoint` with `signal`'s path appended on HTTP, as the exporter
/// appends it to `OTEL_EXPORTER_OTLP_ENDPOINT`; a gRPC endpoint as it is.
fn at_signal(endpoint: &str, transport: Transport, signal: Signal) -> String {
    if !transport.is_http() {
        return endpoint.to_owned();
    }
    let path = match (endpoint.ends_with('/'), signal.path().strip_prefix('/')) {
        (true, Some(rest)) => rest,
        _ => signal.path(),
    };
    format!("{endpoint}{path}")
}

/// Whether `endpoint` has a scheme: a `://` before any `/`, `?` or `#`.
fn has_scheme(endpoint: &str) -> bool {
    endpoint
        .split_once("://")
        .is_some_and(|(scheme, _)| !scheme.contains(['/', '?', '#']))
}

/// `endpoint` with a scheme: a gRPC endpoint written without one is given
/// `http://` when the environment says it's insecure and `https://`
/// otherwise, as the specification gives it one. An HTTP endpoint is a URL
/// with its scheme, or it's refused as one the exporter doesn't accept.
fn with_scheme(endpoint: String, transport: Transport, insecure: bool) -> String {
    if transport != Transport::Grpc || has_scheme(&endpoint) {
        endpoint
    } else if insecure {
        format!("http://{endpoint}")
    } else {
        format!("https://{endpoint}")
    }
}

/// Whether the exporter speaks TLS to `endpoint`, which has its scheme.
fn speaks_tls(endpoint: &str) -> bool {
    endpoint
        .split_once("://")
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
}

/// The TLS material the environment names for `signal`, loaded as the
/// exporters load it. A refusal is under `telemetry.otlp.endpoint`, and
/// says when the config states no endpoint, which `stated` tells, since
/// the config then holds nothing at fault.
fn tls(
    written: &Config,
    signal: Signal,
    stated: bool,
    inherited: &Inherited,
) -> Result<Tls, ConfigError> {
    let refused = |reason: String| ConfigError::Invalid {
        key: ENDPOINT_KEY.to_owned(),
        place: written.place_of(ENDPOINT_KEY),
        value: None,
        reason: if stated {
            format!("TLS to the collector for {signal} can't be set up: {reason}")
        } else {
            format!(
                "the config states no endpoint, and TLS to the collector for {signal}, which \
                 the environment's variables set, can't be set up: {reason}"
            )
        },
    };
    let read = |variable: &Variable<PathBuf>| {
        std::fs::read(&variable.value).map_err(|error| {
            refused(format!(
                "`{}` names a file that can't be read: {error}",
                variable.name
            ))
        })
    };
    let roots = match &inherited.certificate {
        Some(certificate) => {
            let pem = read(certificate)?;
            let held =
                reqwest::Certificate::from_pem_bundle(&pem).is_ok_and(|held| !held.is_empty());
            if held && tls_loads(Some(&pem), None) {
                Some(pem)
            } else {
                return Err(refused(format!(
                    "`{}` names a file that doesn't hold PEM certificates that can be loaded",
                    certificate.name
                )));
            }
        }
        None => None,
    };
    let identity = match (&inherited.client_key, &inherited.client_certificate) {
        (None, None) => None,
        (Some(key), Some(certificate)) => {
            let certificate_pem = read(certificate)?;
            let key_pem = SecretString::from(String::from_utf8(read(key)?).map_err(|_| {
                refused(format!(
                    "`{}` names a file that isn't a PEM private key",
                    key.name
                ))
            })?);
            let identity = ClientIdentity {
                certificate: certificate_pem,
                key: key_pem,
                key_variable: key.name,
            };
            // The reason the client's library gives is left out: it reads
            // the key, and what it says of one isn't held to say nothing
            // of what it holds.
            if !tls_loads(None, Some(&identity)) {
                return Err(refused(format!(
                    "`{}` and `{}` name files that don't hold a PEM client certificate and its \
                     private key that can be loaded",
                    certificate.name, key.name
                )));
            }
            Some(identity)
        }
        (Some(key), None) => {
            return Err(refused(format!(
                "`{}` names a client key, and no client certificate is named beside it",
                key.name
            )));
        }
        (None, Some(certificate)) => {
            return Err(refused(format!(
                "`{}` names a client certificate, and no client key is named beside it",
                certificate.name
            )));
        }
    };
    Ok(Tls { roots, identity })
}

/// `builder` trusting `roots` alone, when they're given, and showing
/// `identity`, when it's given.
pub(crate) fn with_tls(
    mut builder: reqwest::blocking::ClientBuilder,
    roots: Option<&[u8]>,
    identity: Option<&ClientIdentity>,
) -> Result<reqwest::blocking::ClientBuilder, reqwest::Error> {
    if let Some(roots) = roots {
        builder = builder.tls_certs_only(reqwest::Certificate::from_pem_bundle(roots)?);
    }
    if let Some(identity) = identity {
        let mut pem = identity.certificate.clone();
        pem.push(b'\n');
        pem.extend_from_slice(identity.key.expose_secret().as_bytes());
        builder = builder.identity(reqwest::Identity::from_pem(&pem)?);
    }
    Ok(builder)
}

/// The client `builder` makes, made on a thread of its own: it starts a
/// runtime of its own, which can't be done from inside another's. The
/// error is why the thread failed, and the inner one the client's.
pub(crate) fn made(
    builder: reqwest::blocking::ClientBuilder,
) -> Result<Result<reqwest::blocking::Client, reqwest::Error>, String> {
    thread::Builder::new()
        .name("lablet-otlp-http-client".to_owned())
        .spawn(move || builder.build())
        .map_err(|error| format!("its thread couldn't start: {error}"))?
        .join()
        .map_err(|_| "its thread didn't run to its end".to_owned())
}

/// Whether `roots` and `identity` load as the HTTP exporter's client loads
/// them, which reading their PEM doesn't show: the certificates and the key
/// are parsed only when a client is made with them. A client is made with
/// them alone, trusting no other roots, so nothing but what they hold
/// decides. The gRPC exporter's channel skips a root it can't parse, where
/// this refuses it, so a file means the same to both.
fn tls_loads(roots: Option<&[u8]>, identity: Option<&ClientIdentity>) -> bool {
    let builder = reqwest::blocking::Client::builder().tls_certs_only([]);
    with_tls(builder, roots, identity)
        .is_ok_and(|builder| made(builder).is_ok_and(|client| client.is_ok()))
}

#[cfg(test)]
mod tests;
