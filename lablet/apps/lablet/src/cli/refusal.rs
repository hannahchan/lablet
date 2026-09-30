//! Why a command stopped before it did what it was asked, as the line it
//! prints. Each class of message has a prefix of its own, so a script can
//! tell them apart.

use lablet::{BuildError, ConfigError, ErrorClass};

/// Why a command stopped before it did what it was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Refusal {
    /// What the command was given can't be used: the config, a flag, the
    /// task prompt, or a file `init` would write.
    #[error("config: {0}")]
    Config(String),
    /// An MCP server couldn't be started.
    #[error("mcp: {0}")]
    Mcp(String),
    /// The provider refused what it was asked.
    #[error("provider: {0}")]
    Provider(String),
    /// The command, or a flag of it, isn't built yet.
    #[error("{0} isn't built yet")]
    NotBuilt(&'static str),
}

impl From<ConfigError> for Refusal {
    fn from(error: ConfigError) -> Self {
        Self::Config(error.to_string())
    }
}

impl From<BuildError> for Refusal {
    /// The library says which class a failure is, so the prefix follows
    /// its class and not the CLI's reading of the variant. The match names
    /// each class, so a class the library gains can't take another's prefix
    /// unnoticed.
    fn from(error: BuildError) -> Self {
        match error.class() {
            ErrorClass::Config => Self::Config(error.to_string()),
            ErrorClass::Mcp => Self::Mcp(error.to_string()),
            // A build never reaches the provider; a rejected key is a run
            // that began, and its outcome says so.
            ErrorClass::Provider => Self::Provider(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests;
