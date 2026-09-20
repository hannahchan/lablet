//! Identifiers: string newtypes that hold only values that passed validation,
//! whether built in code or deserialised.

use serde::{Deserialize, Serialize};

/// Why a string was refused as an identifier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// The value was the empty string.
    #[error("{kind} is empty")]
    Empty {
        /// Which identifier was being built, for the message.
        kind: &'static str,
    },
    /// The value began or ended with whitespace.
    #[error("{kind} {value:?} has leading or trailing whitespace")]
    SurroundingWhitespace {
        /// Which identifier was being built, for the message.
        kind: &'static str,
        /// The refused value.
        value: String,
    },
    /// A tool name held a character a provider API refuses.
    #[error("tool name {value:?} has a character other than an ASCII letter, a digit, `_`, or `-`")]
    ToolNameCharacter {
        /// The refused value.
        value: String,
    },
    /// A tool name was longer than [`ToolName::MAX_LEN`].
    #[error("tool name {value:?} is longer than {max} characters", max = ToolName::MAX_LEN)]
    ToolNameTooLong {
        /// The refused value.
        value: String,
    },
}

/// Declares an identifier newtype whose only way in, serde included, is
/// `$validate`.
macro_rules! id {
    ($(#[$doc:meta])* $name:ident, $validate:path) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validates `value` and wraps it.
            ///
            /// # Errors
            ///
            /// Returns the [`IdError`] for the first rule `value` breaks.
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                $validate(value.into()).map(Self)
            }

            /// The identifier as a string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, IdError> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

id! {
    /// Identifies one run. By convention a ULID, which the composition root
    /// generates; the domain only requires it to be non-empty and trimmed.
    /// The leniency is for [`ToolCallId`], whose value a provider chooses; a
    /// run id is lablet's own and is always a ULID in practice.
    RunId, run_id
}

id! {
    /// Identifies one tool call within a conversation, as the provider issued it.
    ToolCallId, tool_call_id
}

id! {
    /// The name a tool is offered and called under: 1 to [`ToolName::MAX_LEN`]
    /// ASCII letters, digits, `_`, or `-`, which is what both the Anthropic and
    /// the OpenAI tool-calling APIs accept.
    ToolName, tool_name
}

impl ToolName {
    /// The longest tool name both provider APIs accept.
    pub const MAX_LEN: usize = 64;

    /// The tool a run in [`crate::CompletionMode::Explicit`] ends by calling.
    ///
    /// Built here rather than validated at the call site: this module owns
    /// the rule and can see that the name keeps it, so the loop needs no
    /// error path for a name that can't be refused. A test holds the two to
    /// each other.
    #[must_use]
    pub fn task_complete() -> Self {
        Self(crate::CompletionMode::TASK_COMPLETE.to_owned())
    }
}

display_as_str!(RunId, ToolCallId, ToolName);

fn run_id(value: String) -> Result<String, IdError> {
    trimmed("run id", value)
}

fn tool_call_id(value: String) -> Result<String, IdError> {
    trimmed("tool call id", value)
}

fn tool_name(value: String) -> Result<String, IdError> {
    let value = trimmed("tool name", value)?;
    // Characters before length, so the length reported is a character count.
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(IdError::ToolNameCharacter { value });
    }
    if value.len() > ToolName::MAX_LEN {
        return Err(IdError::ToolNameTooLong { value });
    }
    Ok(value)
}

fn trimmed(kind: &'static str, value: String) -> Result<String, IdError> {
    if value.is_empty() {
        return Err(IdError::Empty { kind });
    }
    if value.trim() != value {
        return Err(IdError::SurroundingWhitespace { kind, value });
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
