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

use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::logs::tonic::group_logs_by_resource_and_scope;
use opentelemetry_proto::transform::trace::tonic::group_spans_by_resource_and_scope;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::logs::{LogBatch, LogExporter};
use opentelemetry_sdk::trace::{SpanData, SpanExporter};

/// Where the OTLP/JSON lines of a `Telemetry`'s runs go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileTarget {
    /// One file, which every run is appended to.
    Path(PathBuf),
    /// Standard error.
    Stderr,
}

/// Where the lines go.
#[derive(Debug)]
enum Destination {
    File(Held<File>),
    Stderr {
        /// Whether a write that failed left part of a line on standard
        /// error. It's one stream whatever run writes to it, so a run's
        /// start leaves this as it is.
        torn: bool,
    },
}

/// The file at one path that lines are written to, which is a `W`.
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
    /// The file at `path`, which the first line opens.
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            file: None,
            torn: false,
        }
    }

    /// Appends `line` to the file, which `open` opens unless it's open.
    ///
    /// A file that failed is let go of, so the next line opens it again, in
    /// case what was wrong with it has been put right.
    fn append(&mut self, line: &str, open: impl FnOnce(&Path) -> io::Result<W>) -> io::Result<()> {
        let written = self.write(line, open);
        if written.is_err() {
            self.let_go();
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

/// What the file exporters of one `Telemetry` write to. The spans and the
/// log records each have an exporter of their own, and one line is written
/// at a time whichever of them writes it.
#[derive(Debug, Clone)]
pub(crate) struct Sink {
    destination: Arc<Mutex<Destination>>,
}

impl Sink {
    pub(crate) fn new(target: FileTarget) -> Self {
        let destination = match target {
            FileTarget::Path(path) => Destination::File(Held::new(path)),
            FileTarget::Stderr => Destination::Stderr { torn: false },
        };
        Self {
            destination: Arc::new(Mutex::new(destination)),
        }
    }

    /// Lets go of the file that's open, as a run starts, so that the run's
    /// first line opens its path: a file that was moved since it was opened
    /// would take the lines its path is to hold. Nothing is opened here: the
    /// loop is waiting.
    pub(crate) fn start(&self) {
        let mut destination = self
            .destination
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Destination::File(held) = &mut *destination {
            held.let_go();
        }
    }

    /// Writes one line, whole, and sees it through to the file.
    ///
    /// An error names no path: the caller may have taken the path from a
    /// variable, and what a variable holds is in no message.
    fn write(&self, line: &str) -> Result<(), String> {
        let mut destination = self
            .destination
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match &mut *destination {
            Destination::Stderr { torn } => put(&mut io::stderr().lock(), line, torn)
                .map_err(|error| format!("standard error couldn't be written: {error}")),
            Destination::File(held) => held
                .append(line, to_append)
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
