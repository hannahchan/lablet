//! The OTLP/JSON file exporter: each export is one line, the JSON form of
//! the request a collector would have been sent.
//!
//! The format is the one the OpenTelemetry Collector's file exporter writes
//! and its OTLP JSON file receiver reads, so a file written here can be
//! replayed into a collector. Nothing in a line is lablet's own.

use std::fs::{File, OpenOptions};
use std::future::{self, Future};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use lablet_model::RunId;
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::logs::tonic::group_logs_by_resource_and_scope;
use opentelemetry_proto::transform::trace::tonic::group_spans_by_resource_and_scope;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::logs::{LogBatch, LogExporter};
use opentelemetry_sdk::trace::{SpanData, SpanExporter};

/// Where the OTLP/JSON lines of an observer's runs go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTarget {
    /// Each run has a file of its own in this directory, named
    /// `lablet-<run_id>.otlp.jsonl`.
    EachRun {
        /// The directory, which isn't made when it's missing.
        directory: PathBuf,
    },
    /// One file, which every run is appended to.
    Path(PathBuf),
    /// Standard error.
    Stderr,
}

/// Where the lines of the run in progress go.
#[derive(Debug)]
enum Destination {
    /// No run has started, so there's no file to name yet.
    Unnamed,
    File(PathBuf),
    /// The run has no file, and why.
    Refused(String),
    Stderr,
}

#[derive(Debug)]
struct Open {
    destination: Destination,
    /// The file that was written last, kept open while the lines keep going
    /// to it.
    file: Option<(PathBuf, File)>,
}

/// What the file exporters of one observer write to. The spans, the log
/// records and the wide event each have an exporter of their own, and one
/// line is written at a time whichever of them writes it.
#[derive(Debug, Clone)]
pub(crate) struct Sink {
    target: FileTarget,
    open: Arc<Mutex<Open>>,
}

/// The name of a run's own file.
///
/// # Errors
///
/// Returns why the run id can't be part of a file's name: a name that held
/// a separator would be a path, and would put the file somewhere other than
/// the directory the target names.
fn name_of(run_id: &RunId) -> Result<String, String> {
    let id = run_id.as_str();
    let reason = if id.contains('/') {
        "it holds a `/`"
    } else if id.contains('\0') {
        "it holds a NUL"
    } else {
        return Ok(format!("lablet-{id}.otlp.jsonl"));
    };
    Err(format!(
        "the run id {id:?} can't be part of the name of the run's telemetry file: {reason}"
    ))
}

impl Sink {
    pub(crate) fn new(target: FileTarget) -> Self {
        let destination = match &target {
            FileTarget::EachRun { .. } => Destination::Unnamed,
            FileTarget::Path(path) => Destination::File(path.clone()),
            FileTarget::Stderr => Destination::Stderr,
        };
        Self {
            target,
            open: Arc::new(Mutex::new(Open {
                destination,
                file: None,
            })),
        }
    }

    /// Names the file of the run that's starting, when each run has its
    /// own. Nothing is opened here: the loop is waiting.
    pub(crate) fn start(&self, run_id: &RunId) {
        let FileTarget::EachRun { directory } = &self.target else {
            return;
        };
        let destination = match name_of(run_id) {
            Ok(name) => Destination::File(directory.join(name)),
            Err(reason) => Destination::Refused(reason),
        };
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .destination = destination;
    }

    /// Writes one line, whole, and sees it through to the file.
    fn write(&self, line: &str) -> Result<(), String> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let Open { destination, file } = &mut *open;
        match destination {
            Destination::Unnamed => {
                Err("no run has started, so the telemetry has no file to go to".to_owned())
            }
            Destination::Refused(reason) => Err(reason.clone()),
            Destination::Stderr => {
                let mut stderr = io::stderr().lock();
                stderr
                    .write_all(line.as_bytes())
                    .and_then(|()| stderr.flush())
                    .map_err(|error| format!("standard error couldn't be written: {error}"))
            }
            Destination::File(path) => append(file, path, line).map_err(|error| {
                // A file that failed is opened again for the next line, in
                // case what was wrong with it has been put right.
                *file = None;
                format!("{} couldn't be written: {error}", path.display())
            }),
        }
    }
}

/// Appends `line` to the file at `path`, which is opened, and made when
/// it's missing, unless `file` is that file already.
fn append(file: &mut Option<(PathBuf, File)>, path: &Path, line: &str) -> io::Result<()> {
    let file = match file {
        Some((open, file)) if open == path => file,
        _ => {
            let opened = OpenOptions::new().create(true).append(true).open(path)?;
            &mut file.insert((path.to_owned(), opened)).1
        }
    };
    file.write_all(line.as_bytes())?;
    file.flush()
}

/// One request as one line: its compact JSON and a newline.
fn line(json: serde_json::Result<String>) -> Result<String, String> {
    let mut line =
        json.map_err(|error| format!("an export couldn't be written as JSON: {error}"))?;
    line.push('\n');
    Ok(line)
}

/// Writes each batch of spans as one `ExportTraceServiceRequest`.
#[derive(Debug)]
pub(crate) struct FileSpanExporter {
    sink: Sink,
    resource: ResourceAttributesWithSchema,
}

impl FileSpanExporter {
    pub(crate) fn new(sink: Sink) -> Self {
        Self {
            sink,
            resource: ResourceAttributesWithSchema::default(),
        }
    }
}

impl SpanExporter for FileSpanExporter {
    /// The line is written before this returns, so the future has nothing
    /// left to wait for.
    fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        let request = ExportTraceServiceRequest {
            resource_spans: group_spans_by_resource_and_scope(batch, &self.resource),
        };
        future::ready(
            line(serde_json::to_string(&request))
                .and_then(|line| self.sink.write(&line))
                .map_err(OTelSdkError::InternalFailure),
        )
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.resource = resource.into();
    }
}

/// Writes each batch of log records as one `ExportLogsServiceRequest`.
#[derive(Debug)]
pub(crate) struct FileLogExporter {
    sink: Sink,
    resource: ResourceAttributesWithSchema,
}

impl FileLogExporter {
    pub(crate) fn new(sink: Sink) -> Self {
        Self {
            sink,
            resource: ResourceAttributesWithSchema::default(),
        }
    }
}

impl LogExporter for FileLogExporter {
    /// The line is written before this returns, so the future has nothing
    /// left to wait for.
    fn export(&self, batch: LogBatch<'_>) -> impl Future<Output = OTelSdkResult> + Send {
        let request = ExportLogsServiceRequest {
            resource_logs: group_logs_by_resource_and_scope(&batch, &self.resource),
        };
        future::ready(
            line(serde_json::to_string(&request))
                .and_then(|line| self.sink.write(&line))
                .map_err(OTelSdkError::InternalFailure),
        )
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.resource = resource.into();
    }
}

#[cfg(test)]
mod tests;
