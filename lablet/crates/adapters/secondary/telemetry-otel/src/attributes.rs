//! The attributes of one span or log record, held to the one limit on how
//! long a value may be.

use std::borrow::Cow;

use opentelemetry::logs::{AnyValue, LogRecord as _};
use opentelemetry::{Array, KeyValue, StringValue, Value};
use opentelemetry_sdk::logs::SdkLogRecord;

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
pub(crate) enum Held {
    Text(String),
    Texts(Vec<String>),
    Int(i64),
    Float(f64),
    Flag(bool),
}

impl From<&str> for Held {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<String> for Held {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<Vec<String>> for Held {
    fn from(texts: Vec<String>) -> Self {
        Self::Texts(texts)
    }
}

impl From<u64> for Held {
    fn from(count: u64) -> Self {
        Self::Int(i64::try_from(count).unwrap_or(i64::MAX))
    }
}

impl From<u32> for Held {
    fn from(count: u32) -> Self {
        Self::Int(i64::from(count))
    }
}

impl From<u16> for Held {
    fn from(count: u16) -> Self {
        Self::Int(i64::from(count))
    }
}

impl From<i64> for Held {
    fn from(number: i64) -> Self {
        Self::Int(number)
    }
}

impl From<f64> for Held {
    fn from(number: f64) -> Self {
        Self::Float(number)
    }
}

impl From<bool> for Held {
    fn from(flag: bool) -> Self {
        Self::Flag(flag)
    }
}

impl Held {
    /// The value with its text under the limit. [`Attributes::with`] is the
    /// only way a value joins a signal, so no text gets past this.
    fn bounded(self) -> Self {
        match self {
            Self::Text(text) => Self::Text(bounded(text)),
            Self::Texts(texts) => Self::Texts(texts.into_iter().map(bounded).collect()),
            held @ (Self::Int(_) | Self::Float(_) | Self::Flag(_)) => held,
        }
    }
}

/// The attributes of one signal, in the order they were given.
///
/// A key is a constant of the registry crate, which is why it's `'static`,
/// or such a constant and a suffix, when the constant is a template's.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Attributes(Vec<(Cow<'static, str>, Held)>);

impl Attributes {
    /// These and `key`, holding `value`.
    pub(crate) fn with(mut self, key: &'static str, value: impl Into<Held>) -> Self {
        self.0.push((Cow::Borrowed(key), value.into().bounded()));
        self
    }

    /// These and the key of the template `prefix` for `suffix`, holding
    /// `value`. A template's key is its prefix, a dot, and the suffix.
    pub(crate) fn with_under(
        mut self,
        prefix: &'static str,
        suffix: &str,
        value: impl Into<Held>,
    ) -> Self {
        let key = format!("{prefix}.{suffix}");
        self.0.push((Cow::Owned(key), value.into().bounded()));
        self
    }

    /// These and `key`, when there's a value for it to hold. An attribute
    /// without a value is left out and never written as a placeholder.
    pub(crate) fn with_any(self, key: &'static str, value: Option<impl Into<Held>>) -> Self {
        match value {
            Some(value) => self.with(key, value),
            None => self,
        }
    }

    /// These and then `more`.
    pub(crate) fn and(mut self, more: Self) -> Self {
        self.0.extend(more.0);
        self
    }

    /// As a span carries them.
    pub(crate) fn into_key_values(self) -> Vec<KeyValue> {
        self.0
            .into_iter()
            .map(|(key, held)| {
                let value = match held {
                    Held::Text(text) => Value::String(text.into()),
                    Held::Texts(texts) => Value::Array(Array::String(
                        texts.into_iter().map(StringValue::from).collect(),
                    )),
                    Held::Int(number) => Value::I64(number),
                    Held::Float(number) => Value::F64(number),
                    Held::Flag(flag) => Value::Bool(flag),
                };
                KeyValue::new(key, value)
            })
            .collect()
    }

    /// Onto a log record.
    pub(crate) fn onto(self, record: &mut SdkLogRecord) {
        for (key, held) in self.0 {
            let value = match held {
                Held::Text(text) => AnyValue::String(text.into()),
                Held::Texts(texts) => AnyValue::ListAny(Box::new(
                    texts
                        .into_iter()
                        .map(|text| AnyValue::String(text.into()))
                        .collect(),
                )),
                Held::Int(number) => AnyValue::Int(number),
                Held::Float(number) => AnyValue::Double(number),
                Held::Flag(flag) => AnyValue::Boolean(flag),
            };
            record.add_attribute(key, value);
        }
    }
}

#[cfg(test)]
impl Attributes {
    /// The keys, in the order they were given, each as often as it was.
    pub(crate) fn keys(&self) -> Vec<&str> {
        self.0.iter().map(|(key, _)| key.as_ref()).collect()
    }

    /// What `key` holds; `None` when it's not among these.
    pub(crate) fn held(&self, key: &str) -> Option<&Held> {
        self.0
            .iter()
            .find_map(|(held, value)| (*held == key).then_some(value))
    }

    /// Each key and what it holds, in the order they were given.
    pub(crate) fn pairs(&self) -> Vec<(&str, &Held)> {
        self.0
            .iter()
            .map(|(key, held)| (key.as_ref(), held))
            .collect()
    }
}

#[cfg(test)]
mod tests;
