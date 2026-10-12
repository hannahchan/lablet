//! [`Secret`]: a credential the config holds, which nothing of lablet's
//! writes out.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A credential: the model's key, a header's value, or a value given to a
/// tool under `tools.builtin.credentials`.
///
/// Its `Debug` and `Display` forms and its serialisation are the marker
/// `[secret]`, so no message, log line or resolved config holds the value,
/// and the marker doesn't read back: a config that writes it is refused,
/// since a secret set in code has no written form. Two are equal when their
/// values are.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// What stands for a secret wherever a config is written out.
    pub const MARKER: &'static str = "[secret]";

    /// A secret holding `value`, as a host sets one in code.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one place that uses it: the adapter that sends
    /// it, or the environment a tool is given it in.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::MARKER)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::MARKER)
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(Self::MARKER)
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text == Self::MARKER {
            return Err(D::Error::custom(
                "it's how lablet writes a secret set in code; set it with Secret::new",
            ));
        }
        Ok(Self(text))
    }
}

/// A text, as the schema says: what the config writes there is a `${VAR}`.
impl JsonSchema for Secret {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "string".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        String::json_schema(generator)
    }
}

#[cfg(test)]
mod tests;
