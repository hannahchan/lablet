//! Why a command didn't do what it was asked, as the line it prints: a
//! refusal before a run, or the provider's error that ended one. Each class
//! of message has a prefix of its own, which the library gives, so a script
//! can tell them apart.

use std::fmt;

use lablet_config::ConfigError;
use lablet_model::RunOutcome;
use lablet_prepare::{BuildError, ErrorClass};

/// Why a command stopped before it did what it was asked: the class of the
/// failure, whose prefix the line begins with, and what's wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    class: ErrorClass,
    message: String,
}

impl Refusal {
    /// What the command was given can't be used: the config, a flag, the
    /// task prompt, or a file `init` would write.
    pub(crate) fn config(message: impl Into<String>) -> Self {
        Self {
            class: ErrorClass::Config,
            message: message.into(),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.class.prefix(), self.message)
    }
}

impl From<ConfigError> for Refusal {
    fn from(error: ConfigError) -> Self {
        Self::config(error.to_string())
    }
}

impl From<BuildError> for Refusal {
    /// The library says which class a failure is, so the prefix follows
    /// its class and not the CLI's reading of the variant.
    fn from(error: BuildError) -> Self {
        Self {
            class: error.class(),
            message: error.to_string(),
        }
    }
}

/// The line a run that the provider's error ended prints beside its
/// outcome, with the prefix of that class, as `provider: 401 invalid
/// x-api-key`; `None` for a run that ended any other way, whose outcome
/// says why.
pub(crate) fn of_run(outcome: &RunOutcome) -> Option<String> {
    let (class, error) = ErrorClass::of_run(outcome).zip(outcome.error())?;
    Some(format!("{} {error}", class.prefix()))
}

#[cfg(test)]
mod tests;
