//! Where and how each signal is exported over the network, resolved from
//! the config's `telemetry.otlp` section alone: a named endpoint sends both
//! signals, over the protocol it states or HTTP/protobuf, with the headers
//! and the TLS it states, and no endpoint sends nothing. Every value is
//! resolved here, the endpoint always with its scheme, and stated on the
//! exporter's builders, so their own reading of the process environment
//! never decides, but for compression, which a builder has no setting for
//! and the exporter reads for itself.

use std::fmt;
use std::path::Path;
use std::thread;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use crate::export::{Signal, Transport};
use lablet_config::{Config, ConfigError, KeyPath, OtlpProtocol, Refusal};

/// The section the TLS files are named in.
const TLS_KEY: &str = "telemetry.otlp.tls";

/// How long one export may take, its retries included: the specification's
/// default, stated on every builder since the exporter would otherwise read
/// `OTEL_EXPORTER_OTLP_TIMEOUT` for it. A field for it waits for a
/// deployment that asks.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(10);

/// The target of this module's warnings, the module they were written in
/// before the kernels were split out, so a line of the diagnostic log, and
/// a `RUST_LOG` directive that names it, are as they were.
const TARGET: &str = "lablet::otlp";

/// What the network exporters are built from: each signal's destination,
/// when its signal is exported over the network.
pub struct OtlpSettings {
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

    /// What the file `telemetry.otlp.tls.client_key` names holds, when the
    /// config names one and an endpoint: the private key the exporters show
    /// a collector. The cut takes it from [`client_key`], which reads it
    /// whether or not an endpoint is named.
    #[must_use]
    pub fn client_key(&self) -> Option<&SecretString> {
        self.destinations()
            .find_map(|(_, destination)| destination.tls.identity.as_ref())
            .map(|identity| &identity.key)
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
    /// The headers every export carries, and the only ones: the config's.
    pub(crate) headers: Vec<(String, SecretString)>,
    /// How long one export may take, its retries included.
    pub(crate) timeout: Duration,
    /// What the exporter trusts and shows of itself over TLS.
    pub(crate) tls: Tls,
}

impl fmt::Debug for Destination {
    /// Shows neither a header's value nor the endpoint, which may carry
    /// credentials.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Destination")
            .field("transport", &self.transport)
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("timeout", &self.timeout)
            .field("tls", &self.tls)
            .finish_non_exhaustive()
    }
}

/// The TLS material the config names, loaded.
#[derive(Debug, Default, Clone)]
pub(crate) struct Tls {
    /// The certificates trusted in place of the platform's roots, as PEM.
    pub(crate) roots: Option<Vec<u8>>,
    /// The client's identity, for a collector that asks for one.
    pub(crate) identity: Option<ClientIdentity>,
}

/// A client certificate and its private key, as PEM.
#[derive(Clone)]
pub(crate) struct ClientIdentity {
    /// The certificate.
    pub(crate) certificate: Vec<u8>,
    /// The private key, which is a secret.
    pub(crate) key: SecretString,
}

impl fmt::Debug for ClientIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientIdentity").finish_non_exhaustive()
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

/// What the file `telemetry.otlp.tls.client_key` names holds, whenever the
/// config states it, endpoint or none, so its contents are cut from what a
/// tool returns whatever else the section states.
///
/// # Errors
///
/// Returns a [`ConfigError`] shown as the file wrote it, by the key: a file
/// that can't be read, or that holds what isn't text, naming a filled
/// path's variable and never what the file holds.
pub fn client_key(config: &Config) -> Result<Option<SecretString>, ConfigError> {
    let Some(path) = &config.telemetry.otlp.tls.client_key else {
        return Ok(None);
    };
    let named = named(config, "client_key");
    let refused = |reason: String| {
        config.refused(Refusal::Invalid {
            key: tls_key("client_key"),
            reason,
        })
    };
    let bytes = std::fs::read(path)
        .map_err(|error| refused(format!("{named} names a file that can't be read: {error}")))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| refused(format!("{named} names a file that isn't a PEM private key")))?;
    Ok(Some(SecretString::from(text)))
}

/// What the network exporters are built from, for `config`, or `None` when
/// it names no endpoint: nothing goes over the network then, whatever else
/// the section states. `key` is what [`client_key`] read of the client
/// key's file, so the key the exporter shows and the contents cut are one
/// read.
///
/// The TLS files the config names are read and loaded here whenever an
/// endpoint is named, whether or not it speaks TLS, so a check reads what
/// a build reads, and a file that can't be loaded is refused wherever the
/// exporter is on.
///
/// # Errors
///
/// Returns a [`ConfigError`] shown as the file wrote it: a `ca_file` or
/// `client_cert` that can't be read, and TLS files that can't be loaded,
/// naming a filled path's variable and never what the file holds, and a
/// client key or certificate named without the other.
pub fn settings(
    config: &Config,
    key: Option<&SecretString>,
) -> Result<Option<OtlpSettings>, ConfigError> {
    let otlp = &config.telemetry.otlp;
    let Some(endpoint) = otlp
        .endpoint
        .as_deref()
        .filter(|endpoint| !endpoint.is_empty())
    else {
        return Ok(None);
    };
    let transport = otlp
        .protocol
        .map_or(Transport::HttpProtobuf, Transport::from);
    let headers: Vec<(String, SecretString)> = otlp
        .headers
        .iter()
        .flat_map(lablet_config::Headers::iter)
        .map(|(name, value)| {
            (
                name.clone(),
                SecretString::from(value.expose_secret().to_owned()),
            )
        })
        .collect();
    let tls = tls(config, key)?;
    // Both signals go to the one endpoint, whose scheme a signal's path
    // doesn't change, so a file is warned about once.
    if !speaks_tls(&with_scheme(endpoint.to_owned(), transport)) {
        for (name, path) in [
            ("ca_file", &otlp.tls.ca_file),
            ("client_cert", &otlp.tls.client_cert),
            ("client_key", &otlp.tls.client_key),
        ] {
            if path.is_some() {
                tracing::warn!(
                    target: TARGET,
                    "`{TLS_KEY}.{name}` names a file for TLS, and the endpoint doesn't speak TLS, \
                     so the file isn't used, though it's still loaded and refused when it can't be"
                );
            }
        }
    }
    let destination = |signal| {
        let endpoint = with_scheme(at_signal(endpoint, transport, signal), transport);
        Destination {
            transport,
            endpoint: SecretString::from(endpoint),
            headers: headers.clone(),
            timeout: TIMEOUT,
            tls: tls.clone(),
        }
    };
    let traces = destination(Signal::Traces);
    let logs = destination(Signal::Logs);
    Ok(Some(OtlpSettings {
        traces: Some(traces),
        logs: Some(logs),
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
/// `https://`, as the specification gives it one where nothing says the
/// connection is insecure. An HTTP endpoint is a URL with its scheme, or
/// it's refused as one the exporter doesn't accept.
fn with_scheme(endpoint: String, transport: Transport) -> String {
    if transport != Transport::Grpc || has_scheme(&endpoint) {
        endpoint
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

/// The key of the TLS setting `name`.
fn tls_key(name: &str) -> KeyPath {
    KeyPath::of(TLS_KEY).key(name)
}

/// How the file the TLS setting `name` names is named in a reason: by the
/// variable that filled the path, when one did, since the path is then the
/// variable's to say, and as "the setting" otherwise, since the path is
/// shown beside it.
fn named(config: &Config, name: &str) -> String {
    config
        .filled()
        .at(&tls_key(name))
        .and_then(|fill| fill.names().next())
        .map_or_else(
            || "the setting".to_owned(),
            |variable| format!("`{variable}`"),
        )
}

/// The TLS material `telemetry.otlp.tls` names, loaded as the exporters
/// load it, with `client_key` what the client key's file holds. A refusal
/// is under the key of the file at fault, and names the variable that
/// filled the path where one did, as the mask shows it.
fn tls(config: &Config, client_key: Option<&SecretString>) -> Result<Tls, ConfigError> {
    let tls = &config.telemetry.otlp.tls;
    let refused = |name: &str, reason: String| {
        config.refused(Refusal::Invalid {
            key: tls_key(name),
            reason,
        })
    };
    let named = |name: &str| named(config, name);
    let read = |name: &str, path: &Path| {
        std::fs::read(path).map_err(|error| {
            refused(
                name,
                format!("{} names a file that can't be read: {error}", named(name)),
            )
        })
    };
    let roots = match &tls.ca_file {
        Some(path) => {
            let pem = read("ca_file", path)?;
            let held =
                reqwest::Certificate::from_pem_bundle(&pem).is_ok_and(|held| !held.is_empty());
            if held && tls_loads(Some(&pem), None) {
                Some(pem)
            } else {
                return Err(refused(
                    "ca_file",
                    format!(
                        "{} names a file that doesn't hold PEM certificates that can be loaded",
                        named("ca_file")
                    ),
                ));
            }
        }
        None => None,
    };
    let identity = match (client_key, &tls.client_cert) {
        (None, None) => None,
        (Some(key_pem), Some(certificate_path)) => {
            let certificate_pem = read("client_cert", certificate_path)?;
            let identity = ClientIdentity {
                certificate: certificate_pem,
                key: key_pem.clone(),
            };
            // The reason the client's library gives is left out: it reads
            // the key, and what it says of one isn't held to say nothing
            // of what it holds.
            if !tls_loads(None, Some(&identity)) {
                return Err(refused(
                    "client_cert",
                    format!(
                        "{} and {} name files that don't hold a PEM client certificate and its \
                         private key that can be loaded",
                        named("client_cert"),
                        named("client_key")
                            .replace("the setting", "`telemetry.otlp.tls.client_key`")
                    ),
                ));
            }
            Some(identity)
        }
        (Some(_), None) => {
            return Err(refused(
                "client_key",
                "a client certificate and its key land together, and `telemetry.otlp.tls.client_cert` \
                 isn't stated; state it beside"
                    .to_owned(),
            ));
        }
        (None, Some(_)) => {
            return Err(refused(
                "client_cert",
                "a client certificate and its key land together, and `telemetry.otlp.tls.client_key` \
                 isn't stated; state it beside"
                    .to_owned(),
            ));
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
