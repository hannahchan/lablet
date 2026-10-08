//! The OTLP exporter's share of the seam: `OTEL_SDK_DISABLED`, the two
//! exporter selectors, every `OTEL_EXPORTER_OTLP_*` variable, generic and
//! for each signal, and the GenAI instrumentation's capture variable.
//!
//! Each setting of a signal is its own variable's when that gives one
//! lablet can use, else the generic variable's, but for the headers: a
//! signal's header variable that's set decides even when it leaves no
//! header. Each generic variable is read once for both signals, so a value
//! lablet can't use is warned about once.

use std::path::PathBuf;
use std::str::FromStr as _;
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};

use super::{Choice, Named, Parse, Timeout, Variables};
use crate::export::{Signal, Transport, decode_headers};
use crate::otlp::{Hidden, Inherited, InheritedEndpoint, Variable};
use lablet_config::Env;

const SDK_DISABLED: &str = "OTEL_SDK_DISABLED";

const CAPTURE_CONTENT: &str = "OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT";

/// The specification's default of how long one export may take.
const TIMEOUT: Duration = Duration::from_secs(10);

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
    /// Whether the GenAI capture variable turns content capture on, or
    /// `None` when it says nothing.
    pub capture_content: Option<bool>,
    /// What the environment gives the spans' exporter.
    pub(crate) traces: Inherited,
    /// What the environment gives the log records' exporter.
    pub(crate) logs: Inherited,
}

/// What an environment that sets nothing gives.
impl Default for Exporter {
    fn default() -> Self {
        Self::read(&Variables(&|_| None))
    }
}

impl Exporter {
    /// The variables whose values are headers, generic and for each signal,
    /// which are secrets whenever they're set.
    pub const HEADER_VARIABLES: [&'static str; 3] = [GENERIC.headers, TRACES.headers, LOGS.headers];

    /// The variables whose values are endpoints, generic and for each
    /// signal, whose user information is a secret.
    pub const ENDPOINT_VARIABLES: [&'static str; 3] =
        [GENERIC.endpoint, TRACES.endpoint, LOGS.endpoint];

    pub(super) fn read(variables: &Variables<'_>) -> Self {
        let generic = Family::read(variables, &GENERIC);
        Self {
            sdk_disabled: sdk_disabled(variables),
            capture_content: variables.get(CAPTURE_CONTENT),
            traces: inherited(variables, Signal::Traces, &generic),
            logs: inherited(variables, Signal::Logs, &generic),
        }
    }

    /// Whether `OTEL_SDK_DISABLED` in `env` turns all telemetry off, for a
    /// caller that has to know before anything else is read.
    pub fn sdk_disabled_in(env: Env<'_>) -> bool {
        sdk_disabled(&Variables(env))
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
