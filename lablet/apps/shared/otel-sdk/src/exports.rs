//! What a checked config's telemetry is built from, and its building, so
//! that `lablet-prepare` stops at checked values and each root builds the
//! SDK itself.

use lablet_run::telemetry::SCOPE;
use lablet_run::telemetry::generated::SCHEMA_URL;
use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::Resource;

use crate::export::{FileTarget, OtelBuildError, OtlpSettings, Telemetry};
use crate::otel_env::Sdk;

/// The version of lablet, the workspace's, which the instrumentation scope
/// names.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What a checked config's telemetry is built from.
pub struct Exports {
    /// Where the file exporter writes, when the run has one.
    pub target: Option<FileTarget>,
    /// What the network exporter is built from, when the run has one.
    pub otlp: Option<OtlpSettings>,
    /// Whether `OTEL_SDK_DISABLED` turns all telemetry off.
    pub disabled: bool,
    /// The sampler, the span limits and the batch processors' settings.
    pub sdk: Sdk,
    /// What every export describes itself with.
    pub resource: Resource,
}

impl Exports {
    /// The variable each signal's endpoint was read from, traces then logs,
    /// for a refusal of what only making the network exporter finds.
    #[must_use]
    pub fn endpoint_variables(&self) -> [Option<&'static str>; 2] {
        self.otlp
            .as_ref()
            .map_or([None; 2], OtlpSettings::endpoint_variables)
    }

    /// The telemetry of every run, under lablet's instrumentation scope.
    ///
    /// # Errors
    ///
    /// Returns what only making the network exporter finds: trust roots
    /// that can't be loaded, or other TLS to the collector that can't be
    /// set up, and an HTTP client that can't be made.
    pub fn telemetry(self) -> Result<Telemetry, OtelBuildError> {
        let Self {
            target,
            otlp,
            disabled,
            sdk,
            resource,
        } = self;
        let scope = InstrumentationScope::builder(SCOPE)
            .with_version(VERSION)
            .with_schema_url(SCHEMA_URL)
            .build();
        let mut telemetry = Telemetry::builder(scope).resource(resource).sdk(sdk);
        if disabled {
            telemetry = telemetry.disabled();
        }
        if let Some(target) = target {
            telemetry = telemetry.file(target);
        }
        if let Some(settings) = otlp {
            telemetry = telemetry.otlp(settings);
        }
        telemetry.build()
    }
}
