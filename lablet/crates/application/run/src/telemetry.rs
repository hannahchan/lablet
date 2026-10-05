//! Lablet's telemetry, hand-written once: what every span and log record
//! the loop emits is built from.
//!
//! A generated module lists each span and event as a struct whose
//! `attributes()` is pairs of key and value, with no branch, so the floors
//! hold it through the ordinary tests that record one. Everything that could
//! be got wrong is here, shared by every struct: an [`Attribute`] with no
//! value is left out of a span and a record, text is cut to
//! [`ATTRIBUTE_MAX_BYTES`], a list becomes a span's array or a record's list,
//! a [`Record`] becomes a log record with its event name, severity, the time
//! the loop measured and the span it belongs to, and [`Logger`] is the
//! object-safe logger for the loop to hold beside its tracer.
//!
//! [`generated`] is the module `cargo xtask weaver generate` writes from the
//! registry folder `lablet/telemetry/registry/application/run/`,
//! [`spellings`] holds the domain's enums to its enums, and the crate's own
//! `conversation` module rebuilds the content the records carry from what
//! the loop saw.

pub(crate) mod conversation;
pub mod generated;
pub mod spellings;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::SystemTime;

use opentelemetry::logs::{AnyValue, LogRecord as _, Severity};
use opentelemetry::trace::SpanContext;
use opentelemetry::{Array, KeyValue, StringValue};

/// `count` as the wire carries it: an `i64`, of which one too large is the
/// largest the wire can say.
///
/// The loop counts in `u64`, and a generated struct's `int` field is `i64`,
/// so a call site passes each count through here.
#[must_use]
pub fn count_of(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// The longest text an attribute holds, in bytes: 1 MiB.
///
/// A collector refuses a request over 4 MiB by default, and a record of
/// captured content holds the conversation so far. No record holds more than
/// three such values, so none outgrows a request with this as the limit of
/// each.
pub const ATTRIBUTE_MAX_BYTES: usize = 1 << 20;

/// `text`, cut to [`ATTRIBUTE_MAX_BYTES`] at the last character boundary the
/// limit allows.
fn bounded(mut text: String) -> String {
    text.truncate(text.floor_char_boundary(ATTRIBUTE_MAX_BYTES));
    text
}

/// The value of one attribute.
///
/// The variants are the types the registry gives lablet's attributes. A
/// count is held as the `i64` the wire carries, and one too large for it is
/// the largest the wire can say.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// One text.
    Text(String),
    /// A list of texts, an array on a span and a list on a record.
    Texts(Vec<String>),
    /// A whole number, which is what every count becomes.
    Int(i64),
    /// A number with a fraction.
    Float(f64),
    /// Yes or no.
    Flag(bool),
}

impl From<&str> for Value {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<String> for Value {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<Vec<String>> for Value {
    fn from(texts: Vec<String>) -> Self {
        Self::Texts(texts)
    }
}

impl From<u64> for Value {
    fn from(count: u64) -> Self {
        Self::Int(count_of(count))
    }
}

impl From<u32> for Value {
    fn from(count: u32) -> Self {
        Self::Int(i64::from(count))
    }
}

impl From<u16> for Value {
    fn from(count: u16) -> Self {
        Self::Int(i64::from(count))
    }
}

impl From<i64> for Value {
    fn from(number: i64) -> Self {
        Self::Int(number)
    }
}

impl From<f64> for Value {
    fn from(number: f64) -> Self {
        Self::Float(number)
    }
}

impl From<bool> for Value {
    fn from(flag: bool) -> Self {
        Self::Flag(flag)
    }
}

impl Value {
    /// The value with its text at or under the limit.
    fn bounded(self) -> Self {
        match self {
            Self::Text(text) => Self::Text(bounded(text)),
            Self::Texts(texts) => Self::Texts(texts.into_iter().map(bounded).collect()),
            held @ (Self::Int(_) | Self::Float(_) | Self::Flag(_)) => held,
        }
    }
}

/// One attribute of a span or a record: a key, and the value it holds if it
/// holds one.
///
/// A generated struct lists every field as one of these, so a field with no
/// value is an attribute with none. It's left out of a span and a record by
/// [`span_attributes`] and [`Record::emit_through`], and never written as a
/// placeholder.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// A constant of a generated module, which is why it's `'static`, or a
    /// template's prefix joined with a suffix by a dot.
    key: Cow<'static, str>,
    value: Option<Value>,
}

impl Attribute {
    /// `key`, holding `value` if there is one.
    #[must_use]
    pub fn new(key: &'static str, value: Option<Value>) -> Self {
        Self {
            key: Cow::Borrowed(key),
            value,
        }
    }

    /// `key`, holding `value`.
    #[must_use]
    pub fn of(key: &'static str, value: impl Into<Value>) -> Self {
        Self::new(key, Some(value.into()))
    }

    /// The key of the template `prefix` for `suffix`, holding `value`. A
    /// template's key is its prefix, a dot, and the suffix.
    #[must_use]
    pub fn under(prefix: &'static str, suffix: &str, value: impl Into<Value>) -> Self {
        Self {
            key: Cow::Owned(format!("{prefix}.{suffix}")),
            value: Some(value.into()),
        }
    }

    /// The key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// What it holds; `None` when it holds nothing.
    #[must_use]
    pub fn value(&self) -> Option<&Value> {
        self.value.as_ref()
    }
}

/// One attribute for each entry of `values`, under the template `prefix`:
/// what a generated struct's template field becomes, with no branch of its
/// own.
#[must_use]
pub fn each_under<V: Into<Value> + Clone>(
    prefix: &'static str,
    values: &BTreeMap<String, V>,
) -> Vec<Attribute> {
    values
        .iter()
        .map(|(suffix, value)| Attribute::under(prefix, suffix, value.clone()))
        .collect()
}

/// The attributes that hold a value, each with its text at or under the
/// limit. It's the only path from an [`Attribute`] to a span or a record, so
/// no text gets past the limit, and the limit is held in one place.
fn held(attributes: Vec<Attribute>) -> impl Iterator<Item = (Cow<'static, str>, Value)> {
    attributes
        .into_iter()
        .filter_map(|attribute| Some((attribute.key, attribute.value?.bounded())))
}

/// `attributes` as a span carries them: the ones with a value, each text cut
/// to [`ATTRIBUTE_MAX_BYTES`], and a list as an array of strings.
#[must_use]
pub fn span_attributes(attributes: Vec<Attribute>) -> Vec<KeyValue> {
    held(attributes)
        .map(|(key, value)| {
            let value = match value {
                Value::Text(text) => opentelemetry::Value::String(text.into()),
                Value::Texts(texts) => opentelemetry::Value::Array(Array::String(
                    texts.into_iter().map(StringValue::from).collect(),
                )),
                Value::Int(number) => opentelemetry::Value::I64(number),
                Value::Float(number) => opentelemetry::Value::F64(number),
                Value::Flag(flag) => opentelemetry::Value::Bool(flag),
            };
            KeyValue::new(key, value)
        })
        .collect()
}

/// One log record, in the context of the span it belongs to, at the time the
/// loop measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// The event's name in the registry.
    pub name: &'static str,
    /// Its severity; the record carries the number and the name.
    pub severity: Severity,
    /// When what it reports happened.
    pub at: SystemTime,
    /// The span it belongs to.
    pub span: SpanContext,
    /// Its attributes, of which the ones with a value reach the record.
    pub attributes: Vec<Attribute>,
}

impl Record {
    /// Emits this through `logger`, the one path from a [`Record`] to the
    /// Logs Bridge API.
    ///
    /// The time it was observed is the time it happened. The SDK would
    /// otherwise read its own clock, and nothing in a run's record is timed
    /// by the exporter.
    pub fn emit_through<L: opentelemetry::logs::Logger>(self, logger: &L) {
        let mut record = logger.create_log_record();
        record.set_event_name(self.name);
        record.set_severity_number(self.severity);
        record.set_severity_text(self.severity.name());
        record.set_timestamp(self.at);
        record.set_observed_timestamp(self.at);
        record.set_trace_context(
            self.span.trace_id(),
            self.span.span_id(),
            Some(self.span.trace_flags()),
        );
        for (key, value) in held(self.attributes) {
            let value = match value {
                Value::Text(text) => AnyValue::String(text.into()),
                Value::Texts(texts) => AnyValue::ListAny(Box::new(
                    texts
                        .into_iter()
                        .map(|text| AnyValue::String(text.into()))
                        .collect(),
                )),
                Value::Int(number) => AnyValue::Int(number),
                Value::Float(number) => AnyValue::Double(number),
                Value::Flag(flag) => AnyValue::Boolean(flag),
            };
            record.add_attribute(key, value);
        }
        logger.emit(record);
    }
}

/// Lablet's logger, for the loop to hold as a `Box<dyn Logger>` beside its
/// `BoxedTracer`.
///
/// The API's `Logger` has an associated record type, so it can't be a trait
/// object; this one takes a whole [`Record`] instead, and [`Bridge`] is its
/// implementation over any logger of the API.
pub trait Logger: Send + Sync {
    /// Emits `record`.
    fn emit(&self, record: Record);
}

/// A [`Logger`] over any logger of the API, which emits each record through
/// [`Record::emit_through`].
///
/// A test wanting nothing emitted wraps the logger of the API's
/// [`opentelemetry::logs::NoopLoggerProvider`].
#[derive(Debug, Clone)]
pub struct Bridge<L>(L);

impl<L> Bridge<L> {
    /// A bridge over `logger`.
    #[must_use]
    pub fn new(logger: L) -> Self {
        Self(logger)
    }
}

impl<L: opentelemetry::logs::Logger + Send + Sync> Logger for Bridge<L> {
    fn emit(&self, record: Record) {
        record.emit_through(&self.0);
    }
}

#[cfg(test)]
mod tests;
