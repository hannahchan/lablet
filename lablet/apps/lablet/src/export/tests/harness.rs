//! A `Telemetry` built from a test's settings, and a run emitted through its
//! tracer and its logger as the loop and the composition root emit one, by
//! `testing.rs`, then flushed and read back.

use std::path::PathBuf;
use std::time::Duration;

use lablet_conformance::otlp::Exported;

pub(super) use crate::export::testing::{
    CONTENT, CONTENT_PER_RUN, OTHER_RUN, RUN, RUN_KEY, Records, SPANS_PER_RUN, Scratch, VERSION,
    WIDE, after, emit_run, scope,
};
use crate::export::{FileTarget, FlushError, OtlpSettings, Telemetry};

/// The file of the run `run` in `scratch`, when each run has its own.
pub(super) fn file_of(scratch: &Scratch, run: &str) -> PathBuf {
    scratch.at(&format!("lablet-{run}.otlp.jsonl"))
}

/// What a test says of the telemetry it builds.
pub(super) struct Settings {
    /// The file the telemetry exports to, when it exports to one.
    pub(super) target: Option<FileTarget>,
    /// The collector the telemetry exports to, when it exports to one.
    pub(super) otlp: Option<OtlpSettings>,
    /// The composer's resource attributes.
    pub(super) resource: Vec<(String, String)>,
    /// How long a flush waits for a destination, in place of the builder's
    /// five seconds.
    pub(super) flush_timeout: Option<Duration>,
    /// How long a shutdown waits, in place of the builder's five seconds.
    pub(super) shutdown_timeout: Option<Duration>,
}

impl Settings {
    /// A file of its own for each run in `scratch`, no collector, and a
    /// resource the composer adds `team: evals` and
    /// `deployment.environment.name: ci` to.
    pub(super) fn in_scratch(scratch: &Scratch) -> Self {
        Self {
            target: Some(FileTarget::EachRun {
                directory: scratch.path().to_owned(),
            }),
            otlp: None,
            resource: vec![
                ("team".to_owned(), "evals".to_owned()),
                ("deployment.environment.name".to_owned(), "ci".to_owned()),
            ],
            flush_timeout: None,
            shutdown_timeout: None,
        }
    }
}

/// The telemetry `settings` describe. With a collector it must be called
/// inside a tokio runtime.
pub(super) fn built(settings: Settings) -> Telemetry {
    let Settings {
        target,
        otlp,
        resource,
        flush_timeout,
        shutdown_timeout,
    } = settings;
    let mut builder = Telemetry::builder(VERSION, scope()).resource(resource);
    if let Some(target) = target {
        builder = builder.file(target);
    }
    if let Some(settings) = otlp {
        builder = builder.otlp(settings);
    }
    if let Some(timeout) = flush_timeout {
        builder = builder.flush_timeout(timeout);
    }
    if let Some(timeout) = shutdown_timeout {
        builder = builder.shutdown_timeout(timeout);
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

/// The queues each failure of `error` names.
pub(super) fn queues(error: &FlushError) -> Vec<&str> {
    error
        .failures()
        .iter()
        .map(|failure| failure.split(": ").next().unwrap())
        .collect()
}
