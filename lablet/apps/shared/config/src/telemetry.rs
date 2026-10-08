//! The `telemetry` section: what's captured, and where it's exported.

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::written::path;

/// The `telemetry` section. A field that's `null` is the environment's,
/// read as the OpenTelemetry specification reads it, and then the
/// specification's default; a field the config states wins.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Telemetry {
    /// Whether prompts, responses and tool content reach telemetry. `None`
    /// leaves it to `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT`,
    /// which turns it on only when it's `true` in any case.
    pub capture_content: Option<bool>,
    /// The OTLP network exporter.
    pub otlp: Otlp,
    /// The OTLP/JSON file exporter.
    pub file: TelemetryFile,
    /// The composer's own resource attributes, which every export carries.
    /// They're stated over `OTEL_SERVICE_NAME` and the attributes
    /// `OTEL_RESOURCE_ATTRIBUTES` names, winning a key both name, and
    /// `service.version` and the SDK's `telemetry.sdk.*` stay lablet's and
    /// the SDK's whichever names them.
    pub resource: BTreeMap<String, String>,
}

/// The `telemetry.otlp` section. A field that's `null` is the
/// environment's, read for each signal: the signal's own variable, else the
/// generic `OTEL_EXPORTER_OTLP_*` one. The timeout, the compression and TLS
/// are the environment's alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Otlp {
    /// Whether a run exports over the network. `None` leaves each signal to
    /// `OTEL_TRACES_EXPORTER` and `OTEL_LOGS_EXPORTER`, which turn it on
    /// unless they're `none`. `true` turns both on and `false` both off,
    /// whatever they say. `false` is refused beside a stated endpoint,
    /// which it would make a setting without effect.
    pub enabled: Option<bool>,
    /// Where the collector listens. `None` leaves it to the signal's
    /// variable and `OTEL_EXPORTER_OTLP_ENDPOINT`, and then to the
    /// protocol's default, `http://localhost:4318` or
    /// `http://localhost:4317`. On HTTP it's a base URL, to which
    /// `/v1/traces` and `/v1/logs` are appended. Its user information is a
    /// secret, cut everywhere and out of the digest; set to nothing it
    /// states none.
    pub endpoint: Option<String>,
    /// The protocol the collector is sent. `None` leaves it to the
    /// signal's variable and `OTEL_EXPORTER_OTLP_PROTOCOL`, and then
    /// HTTP/protobuf.
    pub protocol: Option<OtlpProtocol>,
    /// Headers sent with every export, and the only ones: none the
    /// environment names is sent beside them, `{}` included. `None` sends
    /// the environment's, wherever the endpoint came from. Every value is a
    /// secret, written or substituted: a variable substituted into one is
    /// withheld from every command, the value is cut out of every tool
    /// result, and it's left out of the config digest.
    pub headers: Option<BTreeMap<String, String>>,
}

/// The protocol an OTLP collector is sent, as the specification spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
pub enum OtlpProtocol {
    /// OTLP over gRPC.
    #[serde(rename = "grpc")]
    Grpc,
    /// OTLP over HTTP, as protobuf.
    #[serde(rename = "http/protobuf")]
    HttpProtobuf,
    /// OTLP over HTTP, as JSON.
    #[serde(rename = "http/json")]
    HttpJson,
}

/// The `telemetry.file` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetryFile {
    /// The file every run's OTLP/JSON lines are appended to, or `-` for
    /// standard error. `None` writes no file. No variable names one, and
    /// the file gets both signals whatever the network does.
    #[serde(serialize_with = "path::optional")]
    pub path: Option<PathBuf>,
}
