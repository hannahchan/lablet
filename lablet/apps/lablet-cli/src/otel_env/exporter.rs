//! The OTLP exporter's share of the seam: `OTEL_SDK_DISABLED`, the two
//! exporter selectors, and every `OTEL_EXPORTER_OTLP_*` variable, generic
//! and for each signal. No production code reads them: the network exporter
//! is built from `telemetry.otlp` alone.
//!
//! Each setting of a signal is its own variable's when that gives one
//! lablet can use, else the generic variable's, but for the headers: a
//! signal's header variable that's set decides even when it leaves no
//! header. Each generic variable is read once for both signals, so a value
//! lablet can't use is warned about once.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};

use super::{Choice, Named, Parse, Timeout, Variables};
use crate::export::{Signal, Transport};
use lablet_otel_env::decode_headers;

const SDK_DISABLED: &str = "OTEL_SDK_DISABLED";

/// The specification's default of how long one export may take.
const TIMEOUT: Duration = Duration::from_secs(10);

/// A value that may be a secret, which no `Debug` form shows.
#[derive(Clone, PartialEq, Eq)]
pub struct Hidden(pub String);

impl fmt::Debug for Hidden {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("..")
    }
}

/// A value, and the variable it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable<T> {
    /// The variable.
    pub name: &'static str,
    /// What it holds, as the seam read it.
    pub value: T,
}

/// The endpoint the environment names for a signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InheritedEndpoint {
    /// The variable it was read from.
    pub variable: &'static str,
    /// Whether that's the generic variable, which takes the signal's path
    /// on HTTP, where the signal's own is taken as it's written.
    pub generic: bool,
    /// What it holds, as text: a value that isn't UTF-8 is read lossily,
    /// and then parses as no URL.
    pub text: Hidden,
}

/// What the environment gives one signal's exporter, as the seam reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inherited {
    /// Whether the signal's selector turns its OTLP exporter on, or
    /// `None` when it says nothing lablet serves.
    pub selected: Option<bool>,
    /// The protocol, when a variable names one lablet serves.
    pub protocol: Option<Transport>,
    /// The endpoint, when a variable names one.
    pub endpoint: Option<InheritedEndpoint>,
    /// The headers: the signal's variable's when it's set, even to pairs
    /// that are all dropped, else the generic variable's, when it's set.
    pub headers: Option<Vec<(String, Hidden)>>,
    /// Every header name either variable sets.
    pub header_names: Vec<String>,
    /// How long one export may take.
    pub timeout: Duration,
    /// Whether exports are compressed with gzip.
    pub gzip: bool,
    /// Whether a gRPC endpoint without a scheme is spoken to without TLS.
    pub insecure: bool,
    /// The file of the certificates the exporter trusts, in place of the
    /// platform's roots.
    pub certificate: Option<Variable<PathBuf>>,
    /// The file of the client's private key.
    pub client_key: Option<Variable<PathBuf>>,
    /// The file of the client's certificate.
    pub client_certificate: Option<Variable<PathBuf>>,
}

/// The names of one family of the exporter's variables: the generic one,
/// or one signal's.
struct Names {
    protocol: &'static str,
    endpoint: &'static str,
    headers: &'static str,
    timeout: &'static str,
    compression: &'static str,
    insecure: &'static str,
    certificate: &'static str,
    client_key: &'static str,
    client_certificate: &'static str,
}

const GENERIC: Names = Names {
    protocol: "OTEL_EXPORTER_OTLP_PROTOCOL",
    endpoint: "OTEL_EXPORTER_OTLP_ENDPOINT",
    headers: "OTEL_EXPORTER_OTLP_HEADERS",
    timeout: "OTEL_EXPORTER_OTLP_TIMEOUT",
    compression: "OTEL_EXPORTER_OTLP_COMPRESSION",
    insecure: "OTEL_EXPORTER_OTLP_INSECURE",
    certificate: "OTEL_EXPORTER_OTLP_CERTIFICATE",
    client_key: "OTEL_EXPORTER_OTLP_CLIENT_KEY",
    client_certificate: "OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE",
};

const TRACES: Names = Names {
    protocol: "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
    endpoint: "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    headers: "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    timeout: "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
    compression: "OTEL_EXPORTER_OTLP_TRACES_COMPRESSION",
    insecure: "OTEL_EXPORTER_OTLP_TRACES_INSECURE",
    certificate: "OTEL_EXPORTER_OTLP_TRACES_CERTIFICATE",
    client_key: "OTEL_EXPORTER_OTLP_TRACES_CLIENT_KEY",
    client_certificate: "OTEL_EXPORTER_OTLP_TRACES_CLIENT_CERTIFICATE",
};

const LOGS: Names = Names {
    protocol: "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL",
    endpoint: "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
    headers: "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
    timeout: "OTEL_EXPORTER_OTLP_LOGS_TIMEOUT",
    compression: "OTEL_EXPORTER_OTLP_LOGS_COMPRESSION",
    insecure: "OTEL_EXPORTER_OTLP_LOGS_INSECURE",
    certificate: "OTEL_EXPORTER_OTLP_LOGS_CERTIFICATE",
    client_key: "OTEL_EXPORTER_OTLP_LOGS_CLIENT_KEY",
    client_certificate: "OTEL_EXPORTER_OTLP_LOGS_CLIENT_CERTIFICATE",
};

impl Signal {
    /// The names of this signal's own variables.
    const fn names(self) -> &'static Names {
        match self {
            Self::Traces => &TRACES,
            Self::Logs => &LOGS,
        }
    }

    /// The variable that selects this signal's exporters.
    const fn selector(self) -> &'static str {
        match self {
            Self::Traces => "OTEL_TRACES_EXPORTER",
            Self::Logs => "OTEL_LOGS_EXPORTER",
        }
    }
}

/// The OTLP exporter's variables, as the seam reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exporter {
    /// Whether `OTEL_SDK_DISABLED` is `true`: no telemetry at all.
    pub sdk_disabled: bool,
    /// What the environment gives the spans' exporter.
    pub traces: Inherited,
    /// What the environment gives the log records' exporter.
    pub logs: Inherited,
}

/// What an environment that sets nothing gives.
impl Default for Exporter {
    fn default() -> Self {
        Self::read(&Variables(&|_| None))
    }
}

impl Exporter {
    /// Reads the exporter's variables from `variables`.
    #[must_use]
    pub fn read(variables: &Variables<'_>) -> Self {
        let generic = Family::read(variables, &GENERIC);
        Self {
            sdk_disabled: sdk_disabled(variables),
            traces: inherited(variables, Signal::Traces, &generic),
            logs: inherited(variables, Signal::Logs, &generic),
        }
    }
}

fn sdk_disabled(variables: &Variables<'_>) -> bool {
    variables.get(SDK_DISABLED).unwrap_or(false)
}

/// What one family of variables gives, before a signal's own variables
/// are put over the generic ones.
struct Family {
    protocol: Option<Transport>,
    endpoint: Option<InheritedEndpoint>,
    headers: Option<Headers>,
    timeout: Option<Timeout>,
    compression: Option<Compression>,
    insecure: Option<bool>,
    certificate: Option<Variable<PathBuf>>,
    client_key: Option<Variable<PathBuf>>,
    client_certificate: Option<Variable<PathBuf>>,
}

impl Family {
    fn read(variables: &Variables<'_>, names: &Names) -> Self {
        Self {
            protocol: variables
                .get::<Named<Transport>>(names.protocol)
                .map(|Named(transport)| transport),
            endpoint: raw(variables, names.endpoint).map(|text| InheritedEndpoint {
                variable: names.endpoint,
                generic: names.endpoint == GENERIC.endpoint,
                text: Hidden(text.to_string_lossy().into_owned()),
            }),
            // A header variable that's set is the signal's headers even
            // when every pair in it is dropped, as the exporter reads it, so
            // the generic variable's token never goes to the endpoint the
            // signal's variable was set for.
            headers: raw(variables, names.headers)
                .map(|_| variables.get(names.headers).unwrap_or_default()),
            timeout: variables.get(names.timeout),
            compression: variables.get(names.compression),
            // A Boolean that's set to what isn't one reads as `false`, as
            // the specification reads it, rather than leaving the signal's
            // setting to the generic variable.
            insecure: raw(variables, names.insecure)
                .map(|_| variables.get(names.insecure).unwrap_or(false)),
            certificate: path(variables, names.certificate),
            client_key: path(variables, names.client_key),
            client_certificate: path(variables, names.client_certificate),
        }
    }
}

/// What `name` holds as it's written, when it's set to something. An
/// endpoint or a path is taken whole: one that isn't UTF-8 is still what
/// the variable names.
fn raw(variables: &Variables<'_>, name: &str) -> Option<std::ffi::OsString> {
    (variables.0)(name).filter(|value| !value.is_empty())
}

fn path(variables: &Variables<'_>, name: &'static str) -> Option<Variable<PathBuf>> {
    raw(variables, name).map(|value| Variable {
        name,
        value: PathBuf::from(value),
    })
}

/// What the environment gives `signal`: its own variables over the
/// generic ones, setting by setting.
fn inherited(variables: &Variables<'_>, signal: Signal, generic: &Family) -> Inherited {
    let own = Family::read(variables, signal.names());
    let selected = variables
        .get::<Vec<Otlp>>(signal.selector())
        .map(|chosen| !chosen.is_empty());
    let header_names = [&own.headers, &generic.headers]
        .into_iter()
        .flatten()
        .flat_map(|headers| headers.names.iter().cloned())
        .fold(Vec::new(), |mut names: Vec<String>, name| {
            if !names.contains(&name) {
                names.push(name);
            }
            names
        });
    Inherited {
        selected,
        protocol: own.protocol.or(generic.protocol),
        endpoint: own.endpoint.or_else(|| generic.endpoint.clone()),
        headers: own
            .headers
            .or_else(|| generic.headers.clone())
            .map(|headers| headers.pairs),
        header_names,
        timeout: own
            .timeout
            .or(generic.timeout)
            .map_or(TIMEOUT, |Timeout(timeout)| timeout),
        gzip: own.compression.or(generic.compression) == Some(Compression::Gzip),
        insecure: own.insecure.or(generic.insecure).unwrap_or(false),
        certificate: own.certificate.or_else(|| generic.certificate.clone()),
        client_key: own.client_key.or_else(|| generic.client_key.clone()),
        client_certificate: own
            .client_certificate
            .or_else(|| generic.client_certificate.clone()),
    }
}

/// The one exporter a selector can name that lablet serves over OTLP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Otlp;

impl Choice for Otlp {
    fn named(name: &str) -> Option<Self> {
        (name == "otlp").then_some(Self)
    }
}

impl Choice for Transport {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "grpc" => Self::Grpc,
            "http/protobuf" => Self::HttpProtobuf,
            "http/json" => Self::HttpJson,
            _ => return None,
        })
    }
}

/// A compression a variable names, of the ones lablet sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    Gzip,
    None,
}

/// Matched in any case and untrimmed, as the exporter reads it: lablet
/// can't state none on the exporter's builder, so when it resolves none the
/// exporter's own reading decides, and the two have to agree on every
/// value.
impl Parse for Compression {
    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        if text.eq_ignore_ascii_case("gzip") {
            Some(Self::Gzip)
        } else if text.eq_ignore_ascii_case("none") {
            Some(Self::None)
        } else {
            ignored(text, "isn't one lablet serves, so it's ignored");
            None
        }
    }
}

/// The headers one variable names, each pair one a request may carry, and
/// the names it sets. A pair that isn't one is warned about, never with
/// its value, and dropped, and a value that leaves none gives nothing.
#[derive(Debug, Clone, Default)]
struct Headers {
    pairs: Vec<(String, Hidden)>,
    names: Vec<String>,
}

impl Parse for Headers {
    const SECRET: bool = true;

    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        let mut pairs = Vec::new();
        for pair in text.split_terminator(',').map(str::trim) {
            if pair.is_empty() {
                continue;
            }
            let Some((name, value)) = decode_headers(pair).pop() else {
                ignored(
                    pair,
                    "has a pair that isn't `name=value` with both, which is dropped",
                );
                continue;
            };
            let Ok(header) = HeaderName::from_str(&name) else {
                ignored(
                    pair,
                    "has a header whose name isn't one a header may have, which is dropped",
                );
                continue;
            };
            if HeaderValue::from_str(&value).is_err() {
                ignored(
                    pair,
                    &format!(
                        "has a value of the header `{header}` that isn't one a header may have, \
                         which is dropped"
                    ),
                );
                continue;
            }
            pairs.push((header.as_str().to_owned(), Hidden(value)));
        }
        if pairs.is_empty() {
            return None;
        }
        let names = pairs.iter().map(|(name, _)| name.clone()).collect();
        Some(Self { pairs, names })
    }
}

#[cfg(test)]
mod tests;
