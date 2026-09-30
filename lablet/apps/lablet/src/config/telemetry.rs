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
    pub resource: BTreeMap<String, String>,
}

/// The `telemetry.otlp` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Otlp {
    /// Where the collector listens; `None` turns the network exporter off.
    pub endpoint: Option<String>,
    /// The protocol the collector is sent.
    pub protocol: OtlpProtocol,
    /// Headers sent with every export.
    pub headers: BTreeMap<String, String>,
}

/// The protocol an OTLP collector is sent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OtlpProtocol {
    /// OTLP over gRPC.
    #[default]
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
    /// working directory, named `lablet-<run_id>.otlp.jsonl`.
    #[serde(serialize_with = "path::optional")]
    pub path: Option<PathBuf>,
}
