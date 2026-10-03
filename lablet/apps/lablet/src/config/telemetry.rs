//! The `telemetry` section: what's captured, and where it's exported.

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::written::path;

/// The `telemetry` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Telemetry {
    /// Whether prompts, responses and tool content reach telemetry.
    pub capture_content: bool,
    /// The OTLP network exporter.
    pub otlp: Otlp,
    /// The OTLP/JSON file exporter.
    pub file: TelemetryFile,
    /// The composer's own resource attributes, which every export carries.
    /// They're stated over those `OTEL_RESOURCE_ATTRIBUTES` names, winning a
    /// key both name, and `service.name` and `service.version` stay
    /// lablet's whichever names them.
    pub resource: BTreeMap<String, String>,
}

/// The `telemetry.otlp` section. A field it leaves out is the
/// environment's: lablet inherits the `OTEL_*` variables the exporter
/// reads, and a field the config states wins.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Otlp {
    /// Whether a run may export over the network at all. `true` leaves it
    /// to the endpoint, the config's or the environment's. `false` turns
    /// the exporter off whatever the environment says, and is refused
    /// beside a stated endpoint, which it would make a setting without
    /// effect.
    pub enabled: bool,
    /// Where the collector listens. An endpoint turns the network exporter
    /// on, and the headers the environment names aren't sent to it. `None`
    /// leaves it to `OTEL_EXPORTER_OTLP_ENDPOINT` and the signal variables,
    /// any of which turns the exporter on, with the environment's headers;
    /// without one the exporter is off. On `http` it's a base URL, to which
    /// `/v1/traces` and `/v1/logs` are appended. Its user information is a
    /// secret, cut everywhere and out of the digest; set to nothing it states
    /// none.
    pub endpoint: Option<String>,
    /// The protocol the collector is sent. `None` leaves it to
    /// `OTEL_EXPORTER_OTLP_PROTOCOL`, and is gRPC when that isn't set; a
    /// value lablet can't send, `http/json` among them, is refused.
    pub protocol: Option<OtlpProtocol>,
    /// Headers sent with every export, over any header the environment
    /// names. Every value is a secret, written or substituted: a variable
    /// substituted into one is withheld from every command, the value is
    /// cut out of every tool result, and it's left out of the config
    /// digest.
    pub headers: BTreeMap<String, String>,
}

impl Default for Otlp {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: None,
            protocol: None,
            headers: BTreeMap::new(),
        }
    }
}

/// The protocol an OTLP collector is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OtlpProtocol {
    /// OTLP over gRPC.
    Grpc,
    /// OTLP over HTTP, as protobuf.
    Http,
}

/// The `telemetry.file` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetryFile {
    /// The file every run's OTLP/JSON lines are appended to, or `-` for
    /// standard error. `None` gives each run a file of its own in the
    /// working directory, named `lablet-<run_id>.otlp.jsonl`, and no file
    /// at all when the network exporter is on, by the config's endpoint or
    /// the environment's.
    #[serde(serialize_with = "path::optional")]
    pub path: Option<PathBuf>,
}
