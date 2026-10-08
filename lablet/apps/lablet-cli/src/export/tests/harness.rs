//! A `Telemetry` built from a test's settings, and a run emitted through its
//! tracer and its logger as the loop and lablet-run's runner emit one, by
//! `testing.rs`, then flushed and read back.

use std::path::PathBuf;

use lablet_conformance::otlp::Exported;

use crate::export::telemetry::FlushError;
pub(super) use crate::export::testing::{
    CONTENT, CONTENT_PER_RUN, OTHER_RUN, RUN, RUN_KEY, Records, SPANS_PER_RUN, Scratch, VERSION,
    WIDE, after, emit_run, otlp_of, scope, sdk_of,
};
use crate::export::{FileTarget, OtlpSettings, Telemetry, resource};
use crate::otel_env::{self, Sdk};

/// The file every run in `scratch` is appended to, unless a test says
/// otherwise.
pub(super) fn file_of(scratch: &Scratch) -> PathBuf {
    scratch.at("telemetry.otlp.jsonl")
}

/// What a test says of the telemetry it builds.
pub(super) struct Settings {
    /// The file the telemetry exports to, when it exports to one.
    pub(super) target: Option<FileTarget>,
    /// The collector the telemetry exports to, when it exports to one.
    pub(super) otlp: Option<OtlpSettings>,
    /// The composer's resource attributes.
    pub(super) resource: Vec<(String, String)>,
    /// The sampler, the span limits and the batch processors' settings.
    pub(super) sdk: Sdk,
}

impl Settings {
    /// The file of [`file_of`] in `scratch`, no collector, a resource the
    /// composer adds `team: evals` and `deployment.environment.name: ci`
    /// to, and the specification's defaults for the SDK.
    pub(super) fn in_scratch(scratch: &Scratch) -> Self {
        Self {
            target: Some(FileTarget::Path(file_of(scratch))),
            otlp: None,
            resource: vec![
                ("team".to_owned(), "evals".to_owned()),
                ("deployment.environment.name".to_owned(), "ci".to_owned()),
            ],
            sdk: Sdk::default(),
        }
    }
}

/// The telemetry `settings` describe. With a collector it must be called
/// inside a tokio runtime.
pub(super) fn built(settings: Settings) -> Telemetry {
    let Settings {
        target,
        otlp,
        resource: attributes,
        sdk,
    } = settings;
    let mut builder = Telemetry::builder(scope())
        .resource(resource(VERSION, attributes, &otel_env::Context::default()))
        .sdk(sdk);
    if let Some(target) = target {
        builder = builder.file(target);
    }
    if let Some(settings) = otlp {
        builder = builder.otlp(settings);
    }
    builder.build().unwrap()
}

/// One run under the id `run`, flushed with its wide event, as
/// `Lablet::run` does.
pub(super) async fn run(
    telemetry: &Telemetry,
    run: &str,
    records: Records,
) -> Result<(), FlushError> {
    let wide = emit_run(telemetry, run, records);
    telemetry.flush(wide).await
}

/// The runs whose wide events `exported` holds, in order.
pub(super) fn runs_of(exported: &Exported) -> Vec<&str> {
    exported
        .records_of(WIDE)
        .into_iter()
        .map(|wide| wide.attributes[RUN_KEY].as_str().unwrap())
        .collect()
}

/// The providers each failure of `error` names.
pub(super) fn providers(error: &FlushError) -> Vec<&str> {
    error
        .failures()
        .iter()
        .map(|failure| failure.split(": ").next().unwrap())
        .collect()
}
