//! The OTLP/JSON file exporter: each export is one line, the JSON form of
//! the request a collector would have been sent.
//!
//! The format is the one the OpenTelemetry Collector's file exporter writes
//! and its OTLP JSON file receiver reads, so a file written here can be
//! replayed into a collector. Nothing in a line is lablet's own.

use std::fs::{File, OpenOptions};
use std::future::{self, Future};
use std::io;
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
    Stderr {
        /// Whether a write that failed left part of a line on standard
        /// error. It's one stream whatever run writes to it, so a run's
        /// start leaves this as it is.
        torn: bool,
    },
}

#[derive(Debug)]
struct Open {
    destination: Destination,
    /// The file that was written last.
    held: Option<Held<File>>,
}

/// A file that lines were written to, which is a `W`.
#[derive(Debug)]
struct Held<W> {
    path: PathBuf,
    /// The file, kept open while the lines keep going to it; `None` until a
    /// line opens it again.
    file: Option<W>,
    /// Whether a write that failed left part of a line at the end of the
    /// file.
    torn: bool,
}

impl<W: io::Write> Held<W> {
    /// Appends `line` to the file at `path`, which `open` opens unless
    /// `held` is that file and has it open.
    ///
    /// A file that failed is let go of, so the next line opens it again, in
    /// case what was wrong with it has been put right.
    fn append(
        held: &mut Option<Self>,
        path: &Path,
        line: &str,
        open: impl FnOnce(&Path) -> io::Result<W>,
    ) -> io::Result<()> {
        let held = match held {
            Some(held) if held.path == path => held,
            _ => held.insert(Self {
                path: path.to_owned(),
                file: None,
                torn: false,
            }),
        };
        let written = held.write(line, open);
        if written.is_err() {
            held.let_go();
        }
        written
    }

    fn write(&mut self, line: &str, open: impl FnOnce(&Path) -> io::Result<W>) -> io::Result<()> {
        let file = match &mut self.file {
            Some(file) => file,
            None => self.file.insert(open(&self.path)?),
        };
        put(file, line, &mut self.torn)
    }

    /// Closes the file, which the next line opens again.
    fn let_go(&mut self) {
        self.file = None;
    }
}

/// Writes `line` whole to `to` and sees it through, after the newline that
/// ends the part of a line `torn` says a failed write left there.
fn put(to: &mut impl io::Write, line: &str, torn: &mut bool) -> io::Result<()> {
    // A line written straight after part of another would be one line with
    // it, and an export that was written whole would be lost to one that
    // wasn't.
    if *torn {
        whole(to, "\n", torn)?;
    }
    whole(to, line, torn)?;
    to.flush()
}

/// Writes all of `text` to `file`, and says in `torn` whether `file` then
/// ends with a part of `text` that isn't all of it.
fn whole(file: &mut impl io::Write, text: &str, torn: &mut bool) -> io::Result<()> {
    let mut left = text.as_bytes();
    while !left.is_empty() {
        match file.write(left) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(wrote) => {
                left = &left[wrote..];
                *torn = !left.is_empty();
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// What the file exporters of one observer write to. The spans, the log
/// records and the wide event each have an exporter of their own, and one
/// line is written at a time whichever of them writes it.
#[derive(Debug, Clone)]
pub(crate) struct Sink {
    target: FileTarget,
    open: Arc<Mutex<Open>>,
}

/// The most bytes a run id may hold in the name of its file: what lablet
/// holds a run id to before its run, which a file's name has room for
/// beside `lablet-` and `.otlp.jsonl`.
const RUN_ID_MAX_BYTES: usize = 128;

/// The name of a run's own file.
///
/// # Errors
///
/// Returns why the run id can't be part of a file's name: a name that held
/// a separator would be a path, and would put the file somewhere other than
/// the directory the target names, and one too long would name no file.
fn name_of(run_id: &RunId) -> Result<String, String> {
    let id = run_id.as_str();
    let reason = if id.contains('/') {
        "it holds a `/`"
    } else if id.contains('\0') {
        "it holds a NUL"
    } else if id.len() > RUN_ID_MAX_BYTES {
        "it's longer than 128 bytes"
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
            FileTarget::Stderr => Destination::Stderr { torn: false },
        };
        Self {
            target,
            open: Arc::new(Mutex::new(Open {
                destination,
                held: None,
            })),
        }
    }

    /// Names the file of the run that's starting, when each run has its
    /// own, and lets go of the file that's open, so that the run's first
    /// line opens its path: a file that was moved since it was opened would
    /// take the lines its path is to hold. Nothing is opened here: the loop
    /// is waiting.
    pub(crate) fn start(&self, run_id: &RunId) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(held) = &mut open.held {
            held.let_go();
        }
        if let FileTarget::EachRun { directory } = &self.target {
            open.destination = match name_of(run_id) {
                Ok(name) => Destination::File(directory.join(name)),
                Err(reason) => Destination::Refused(reason),
            };
        }
    }

    /// Writes one line, whole, and sees it through to the file.
    ///
    /// An error names no path: the caller may have taken the path from a
    /// variable, and what a variable holds is in no message.
    fn write(&self, line: &str) -> Result<(), String> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let Open { destination, held } = &mut *open;
        match destination {
            Destination::Unnamed => {
                Err("no run has started, so the telemetry has no file to go to".to_owned())
            }
            Destination::Refused(reason) => Err(reason.clone()),
            Destination::Stderr { torn } => put(&mut io::stderr().lock(), line, torn)
                .map_err(|error| format!("standard error couldn't be written: {error}")),
            Destination::File(path) => Held::append(held, path, line, to_append)
                .map_err(|error| format!("the telemetry file couldn't be written: {error}")),
        }
    }
}

/// Opens the file at `path` to append to, and makes it when it's missing.
fn to_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
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
