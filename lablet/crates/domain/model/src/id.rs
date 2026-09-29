//! Identifiers: string newtypes that hold only values that passed validation.

use serde::Serialize;

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

/// Declares an identifier newtype whose only way in is `$validate`.
macro_rules! id {
    ($(#[$attribute:meta])* $name:ident, $validate:path) => {
        $(#[$attribute])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    ///
    /// It serialises, as a bare string, because a [`crate::Message`] holds
    /// one and the loop measures a request by serialising its messages.
    #[derive(Serialize)]
    ToolCallId, tool_call_id
}

id! {
    /// The name a tool is offered and called under: 1 to [`ToolName::MAX_LEN`]
    /// ASCII letters, digits, `_`, or `-`, which is what both the Anthropic and
    /// the OpenAI tool-calling APIs accept.
    ///
    /// It serialises, as a bare string, because a [`crate::ToolSpec`] and a
    /// [`crate::ToolUse`] hold one and the loop measures with both.
    #[derive(Serialize)]
    ToolName, tool_name
}

impl RunId {
    /// The digits a ULID is written in, which are Crockford's base 32.
    const ULID_DIGITS: &'static [u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

    /// The run id that's the ULID `value`, written as its 26 digits.
    ///
    /// Every ULID is a run id, so whoever generates one needs no error path
    /// for an id that can't be refused. The domain reads no clock and draws
    /// no random number, so the value is the caller's to make.
    #[must_use]
    pub fn ulid(value: u128) -> Self {
        let digit = |place: u32| {
            let [index, ..] = ((value >> (5 * place)) & 31).to_le_bytes();
            char::from(Self::ULID_DIGITS[usize::from(index)])
        };
        Self((0..26).rev().map(digit).collect())
    }

    /// A number that's a pure function of the run, the turn and the attempt,
    /// which is what the jitter of a retry is read from: two runs that fail
    /// together wait differently, and one run waits the same every time it's
    /// replayed.
    ///
    /// It's FNV-1a over the id's bytes and then the two numbers',
    /// little-endian. The hash is written out here because the standard
    /// library's hasher may change from one release of Rust to the next, and
    /// a replayed run's waits would change with it.
    #[must_use]
    pub fn salt(&self, turn: u32, attempt: u32) -> u64 {
        const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        self.0
            .bytes()
            .chain(turn.to_le_bytes())
            .chain(attempt.to_le_bytes())
            .fold(OFFSET_BASIS, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(PRIME)
            })
    }
}

impl ToolName {
    /// The longest tool name both provider APIs accept.
    pub const MAX_LEN: usize = 64;

    /// The name of the tool a run that completes explicitly ends by calling.
    /// It's spelled here, where the rule for a name is, so the module that
    /// reads completion modes depends on this one and never the reverse.
    pub const TASK_COMPLETE: &'static str = "task_complete";

    /// The tool a run that completes explicitly ends by calling.
    ///
    /// Built here rather than validated at the call site: this module owns
    /// the rule and can see that the name keeps it, so the loop needs no
    /// error path for a name that can't be refused. A test holds the two to
    /// each other.
    #[must_use]
    pub fn task_complete() -> Self {
        Self(Self::TASK_COMPLETE.to_owned())
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
