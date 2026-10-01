//! Reads OTLP/JSON lines back into spans and log records, for a test to
//! assert on.
//!
//! The lines are the ones lablet's file exporter writes, which are the ones
//! the OpenTelemetry Collector's file exporter writes: each is the JSON
//! form of an `ExportTraceServiceRequest` or an `ExportLogsServiceRequest`.
//! Nothing in a line says which, so a reader tells by the line's one key,
//! `resourceSpans` or `resourceLogs`, as the Collector does.
//!
//! What's read is flattened: each span and each record holds the resource
//! and the scope of the export it came in, and the number of its line, so
//! a test can say what was exported with what and in what order.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{span, status};
use serde_json::Value;

/// The key of a line of spans.
const SPANS: &str = "resourceSpans";

/// The key of a line of log records.
const LOGS: &str = "resourceLogs";

/// Attributes by key. A value is the JSON it would be written as: an
/// `intValue` is an integer and a `doubleValue` a float, so the two differ
/// when they're compared.
pub type Attributes = BTreeMap<String, Value>;

/// Why lines couldn't be read back.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The file couldn't be read.
    #[error("{} couldn't be read: {reason}", path.display())]
    Unreadable {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        reason: String,
    },
    /// A line isn't an export.
    #[error("line {line}: {reason}")]
    Line {
        /// The line, counted from 1.
        line: usize,
        /// What's wrong with it.
        reason: String,
    },
}

/// The instrumentation scope an export names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// The scope's name.
    pub name: String,
    /// The scope's version.
    pub version: String,
    /// The schema the scope's signals follow.
    pub schema_url: String,
}

/// What kind of span one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpanKind {
    /// The export named no kind.
    Unspecified,
    /// An operation inside the application.
    Internal,
    /// The handling of a request from outside.
    Server,
    /// A request to a service outside.
    Client,
    /// A message sent to a broker.
    Producer,
    /// A message received from a broker.
    Consumer,
}

/// How a span ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Nothing was said.
    Unset,
    /// It ended well.
    Ok,
    /// It failed, with this to say about it.
    Error(String),
}

/// One event of a span.
#[derive(Debug, Clone, PartialEq)]
pub struct SpanEvent {
    /// The event's name.
    pub name: String,
    /// When it happened, in nanoseconds since the Unix epoch.
    pub time_unix_nano: u64,
    /// The event's attributes.
    pub attributes: Attributes,
}

/// One span, as it was exported.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    /// The line the span was exported in, counted from 1.
    pub line: usize,
    /// The resource of the span's export.
    pub resource: Attributes,
    /// The scope of the span's export.
    pub scope: Scope,
    /// The trace, in hex.
    pub trace_id: String,
    /// The span, in hex.
    pub span_id: String,
    /// The span's parent, in hex; `None` for a root span.
    pub parent_span_id: Option<String>,
    /// The flags, of which the lowest eight bits are the W3C trace flags.
    pub flags: u32,
    /// The span's name.
    pub name: String,
    /// The span's kind.
    pub kind: SpanKind,
    /// When the span started, in nanoseconds since the Unix epoch.
    pub start_unix_nano: u64,
    /// When the span ended, in nanoseconds since the Unix epoch.
    pub end_unix_nano: u64,
    /// The span's attributes.
    pub attributes: Attributes,
    /// The span's events, in order.
    pub events: Vec<SpanEvent>,
    /// How the span ended.
    pub status: Status,
}

/// One log record, as it was exported.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    /// The line the record was exported in, counted from 1.
    pub line: usize,
    /// The resource of the record's export.
    pub resource: Attributes,
    /// The scope of the record's export.
    pub scope: Scope,
    /// The name of the event the record is of.
    pub event_name: String,
    /// The severity, as OpenTelemetry numbers it: 9 is `INFO`, 13 `WARN`.
    pub severity_number: i32,
    /// The severity, as text.
    pub severity_text: String,
    /// When what the record is of happened, in nanoseconds since the Unix
    /// epoch.
    pub time_unix_nano: u64,
    /// When the record was made, in nanoseconds since the Unix epoch.
    pub observed_time_unix_nano: u64,
    /// The trace the record belongs to, in hex; empty when it names none.
    pub trace_id: String,
    /// The span the record belongs to, in hex; empty when it names none.
    pub span_id: String,
    /// The W3C trace flags.
    pub flags: u32,
    /// The record's attributes.
    pub attributes: Attributes,
    /// The record's body, when it has one.
    pub body: Option<Value>,
}

/// Everything a run of lines exported.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Exported {
    /// How many lines there were, each of which was one export.
    pub lines: usize,
    /// The spans, in the order they were exported.
    pub spans: Vec<Span>,
    /// The log records, in the order they were exported.
    pub records: Vec<LogRecord>,
}

impl Span {
    /// How long the span lasted, in whole milliseconds; none for a span
    /// that ends before it starts.
    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.end_unix_nano.saturating_sub(self.start_unix_nano) / 1_000_000
    }
}

impl Exported {
    /// Reads the file at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError::Unreadable`] when the file can't be read, and
    /// what [`Exported::parse`] returns for what it holds.
    pub fn read(path: &Path) -> Result<Self, ReadError> {
        let text = std::fs::read_to_string(path).map_err(|error| ReadError::Unreadable {
            path: path.to_owned(),
            reason: error.to_string(),
        })?;
        Self::parse(&text)
    }

    /// Reads `text`, which is lines as the file exporter writes them.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError::Line`] for the first line that isn't one
    /// export: a line that's empty or isn't a JSON object, that holds any
    /// key but the one that says what it exports, that doesn't read as the
    /// request its key names, or that gives a span, a record, an event, a
    /// resource or a nested value one key twice. The last line ends in a
    /// newline like the rest, so text that ends elsewhere was cut short.
    pub fn parse(text: &str) -> Result<Self, ReadError> {
        let mut exported = Self::default();
        let mut rest = text;
        while !rest.is_empty() {
            let line = exported.lines + 1;
            let fault = |reason: String| ReadError::Line { line, reason };
            let Some((written, after)) = rest.split_once('\n') else {
                return Err(fault(
                    "it doesn't end in a newline, so the export was cut short".to_owned(),
                ));
            };
            exported.lines = line;
            exported.read_line(line, written).map_err(fault)?;
            rest = after;
        }
        Ok(exported)
    }

    fn read_line(&mut self, line: usize, written: &str) -> Result<(), String> {
        let value: Value =
            serde_json::from_str(written).map_err(|error| format!("it isn't JSON: {error}"))?;
        let keys: Vec<&str> = value
            .as_object()
            .ok_or("it isn't a JSON object")?
            .keys()
            .map(String::as_str)
            .collect();
        match keys[..] {
            [SPANS] => self.read_spans(line, value),
            [LOGS] => self.read_logs(line, value),
            _ => Err(format!(
                "its keys are {keys:?}, and an export has one, `{SPANS}` or `{LOGS}`"
            )),
        }
    }

    fn read_spans(&mut self, line: usize, value: Value) -> Result<(), String> {
        let request: ExportTraceServiceRequest = serde_json::from_value(value)
            .map_err(|error| format!("it isn't an export of spans: {error}"))?;
        for of_resource in request.resource_spans {
            let resource = resource(of_resource.resource)?;
            for of_scope in of_resource.scope_spans {
                let scope = scope(of_scope.scope, of_scope.schema_url);
                for span in of_scope.spans {
                    let events = span
                        .events
                        .into_iter()
                        .map(|event| {
                            Ok(SpanEvent {
                                name: event.name,
                                time_unix_nano: event.time_unix_nano,
                                attributes: attributes(event.attributes)?,
                            })
                        })
                        .collect::<Result<_, String>>()?;
                    self.spans.push(Span {
                        line,
                        resource: resource.clone(),
                        scope: scope.clone(),
                        trace_id: hex(&span.trace_id),
                        span_id: hex(&span.span_id),
                        parent_span_id: Some(hex(&span.parent_span_id))
                            .filter(|parent| !parent.is_empty()),
                        flags: span.flags,
                        kind: kind(span.kind)?,
                        name: span.name,
                        start_unix_nano: span.start_time_unix_nano,
                        end_unix_nano: span.end_time_unix_nano,
                        attributes: attributes(span.attributes)?,
                        events,
                        status: span.status.map_or(Ok(Status::Unset), |status| {
                            ended(status.code, status.message)
                        })?,
                    });
                }
            }
        }
        Ok(())
    }

    fn read_logs(&mut self, line: usize, value: Value) -> Result<(), String> {
        let request: ExportLogsServiceRequest = serde_json::from_value(value)
            .map_err(|error| format!("it isn't an export of log records: {error}"))?;
        for of_resource in request.resource_logs {
            let resource = resource(of_resource.resource)?;
            for of_scope in of_resource.scope_logs {
                let scope = scope(of_scope.scope, of_scope.schema_url);
                for record in of_scope.log_records {
                    self.records.push(LogRecord {
                        line,
                        resource: resource.clone(),
                        scope: scope.clone(),
                        event_name: record.event_name,
                        severity_number: record.severity_number,
                        severity_text: record.severity_text,
                        time_unix_nano: record.time_unix_nano,
                        observed_time_unix_nano: record.observed_time_unix_nano,
                        trace_id: hex(&record.trace_id),
                        span_id: hex(&record.span_id),
                        flags: record.flags,
                        attributes: attributes(record.attributes)?,
                        body: record.body.map(json).transpose()?,
                    });
                }
            }
        }
        Ok(())
    }

    /// The spans whose name is `operation` and then what the operation was
    /// on, as `chat` names every span of a provider call.
    #[must_use]
    pub fn spans_of(&self, operation: &str) -> Vec<&Span> {
        self.spans
            .iter()
            .filter(|span| {
                span.name
                    .strip_prefix(operation)
                    .is_some_and(|rest| rest.starts_with(' '))
            })
            .collect()
    }

    /// The log records of the event `event_name`.
    #[must_use]
    pub fn records_of(&self, event_name: &str) -> Vec<&LogRecord> {
        self.records
            .iter()
            .filter(|record| record.event_name == event_name)
            .collect()
    }

    /// The spans and the records as multisets: each without the line it
    /// was exported in, and sorted, so what two destinations hold of one
    /// run compares equal however each batched it.
    #[must_use]
    pub fn ungrouped(&self) -> Ungrouped {
        let mut spans: Vec<Span> = self
            .spans
            .iter()
            .cloned()
            .map(|span| Span { line: 0, ..span })
            .collect();
        spans.sort_by(|a, b| {
            (&a.trace_id, &a.span_id, &a.name).cmp(&(&b.trace_id, &b.span_id, &b.name))
        });
        let mut records: Vec<LogRecord> = self
            .records
            .iter()
            .cloned()
            .map(|record| LogRecord { line: 0, ..record })
            .collect();
        records.sort_by(|a, b| {
            (&a.trace_id, &a.span_id, &a.event_name, a.time_unix_nano).cmp(&(
                &b.trace_id,
                &b.span_id,
                &b.event_name,
                b.time_unix_nano,
            ))
        });
        Ungrouped { spans, records }
    }
}

/// What a run of lines exported, as multisets: see [`Exported::ungrouped`].
#[derive(Debug, Clone, PartialEq)]
pub struct Ungrouped {
    /// The spans, sorted by trace, span and name, with no line.
    pub spans: Vec<Span>,
    /// The log records, sorted by trace, span, event and time, with no
    /// line.
    pub records: Vec<LogRecord>,
}

/// Lower-case hex, two digits to a byte.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::new(), |mut hex, byte| {
        // Writing to a `String` can't fail, so there's no error to report.
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

fn resource(resource: Option<Resource>) -> Result<Attributes, String> {
    attributes(
        resource
            .map(|resource| resource.attributes)
            .unwrap_or_default(),
    )
}

fn scope(scope: Option<InstrumentationScope>, schema_url: String) -> Scope {
    let (name, version) = scope
        .map(|scope| (scope.name, scope.version))
        .unwrap_or_default();
    Scope {
        name,
        version,
        schema_url,
    }
}

fn kind(kind: i32) -> Result<SpanKind, String> {
    match span::SpanKind::try_from(kind) {
        Ok(span::SpanKind::Unspecified) => Ok(SpanKind::Unspecified),
        Ok(span::SpanKind::Internal) => Ok(SpanKind::Internal),
        Ok(span::SpanKind::Server) => Ok(SpanKind::Server),
        Ok(span::SpanKind::Client) => Ok(SpanKind::Client),
        Ok(span::SpanKind::Producer) => Ok(SpanKind::Producer),
        Ok(span::SpanKind::Consumer) => Ok(SpanKind::Consumer),
        Err(_) => Err(format!("{kind} isn't a kind of span")),
    }
}

fn ended(code: i32, message: String) -> Result<Status, String> {
    match status::StatusCode::try_from(code) {
        Ok(status::StatusCode::Unset) => Ok(Status::Unset),
        Ok(status::StatusCode::Ok) => Ok(Status::Ok),
        Ok(status::StatusCode::Error) => Ok(Status::Error(message)),
        Err(_) => Err(format!("{code} isn't a status of a span")),
    }
}

/// The attributes by key, which refuses a key that's there twice: a map
/// would keep one of the two values and say nothing.
fn attributes(pairs: Vec<KeyValue>) -> Result<Attributes, String> {
    let mut attributes = Attributes::new();
    for KeyValue { key, value, .. } in pairs {
        let value = value.map_or(Ok(Value::Null), json)?;
        if attributes.insert(key.clone(), value).is_some() {
            return Err(format!("the key `{key}` is there twice"));
        }
    }
    Ok(attributes)
}

fn json(value: AnyValue) -> Result<Value, String> {
    Ok(match value.value {
        None => Value::Null,
        Some(any_value::Value::StringValue(text)) => Value::String(text),
        Some(any_value::Value::BoolValue(flag)) => Value::Bool(flag),
        Some(any_value::Value::IntValue(number)) => Value::from(number),
        Some(any_value::Value::DoubleValue(number)) => Value::from(number),
        Some(any_value::Value::ArrayValue(array)) => Value::Array(
            array
                .values
                .into_iter()
                .map(json)
                .collect::<Result<_, _>>()?,
        ),
        Some(any_value::Value::KvlistValue(list)) => {
            Value::Object(attributes(list.values)?.into_iter().collect())
        }
        Some(any_value::Value::BytesValue(bytes)) => Value::String(hex(&bytes)),
        Some(any_value::Value::StringValueStrindex(_)) => {
            return Err(
                "a value refers to a table of strings, which an export has none of".to_owned(),
            );
        }
    })
}

#[cfg(test)]
mod tests;
