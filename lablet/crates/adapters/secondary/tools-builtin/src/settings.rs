//! What an executor that serves tools is built from, and why one can't be
//! built.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

/// The variables a command starts with when lablet's own environment holds
/// them. Nothing else of lablet's environment reaches a command, so a key
/// that lablet was started with isn't there for the model to read.
pub const ENVIRONMENT: [&str; 8] = [
    "HOME", "PATH", "SHELL", "USER", "LANG", "TERM", "TMPDIR", "TZ",
];

/// One of the built-in tools.
///
/// The order is the order an executor offers them in, whatever order they
/// were asked for in, so the digest of a run's tool specs doesn't change with
/// how a config lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tool {
    /// Runs a command.
    Bash,
    /// Reads a file under the root.
    ReadFile,
    /// Writes a file under the root.
    WriteFile,
}

impl Tool {
    /// Every built-in tool.
    pub const ALL: [Self; 3] = [Self::Bash, Self::ReadFile, Self::WriteFile];

    /// The name the model calls the tool by.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
        }
    }
}

impl core::fmt::Display for Tool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// What an executor that serves tools is built from.
///
/// The root isn't optional, so no executor serves a tool without one. An
/// executor that serves nothing needs no settings:
/// [`BuiltinTools::default`](crate::BuiltinTools::default) is one.
///
/// Its `Debug` form names the variables and leaves their values out, since a
/// value may be a credential that a command needs and a log doesn't.
#[derive(Clone, PartialEq, Eq)]
pub struct Settings {
    /// The directory `bash` starts in and the file tools stay under. It has
    /// to exist.
    pub root: PathBuf,
    /// The tools to serve.
    pub enabled: BTreeSet<Tool>,
    /// The longest a call may take. A call's own deadline can only shorten
    /// it.
    pub timeout: Duration,
    /// Variables a command starts with beside the ones of [`ENVIRONMENT`].
    /// One that has the name of a variable of that list replaces it.
    pub env: BTreeMap<String, String>,
}

impl core::fmt::Debug for Settings {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let Self {
            root,
            enabled,
            timeout,
            env,
        } = self;
        f.debug_struct("Settings")
            .field("root", root)
            .field("enabled", enabled)
            .field("timeout", timeout)
            .field("env", &env.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Why an executor couldn't be built from its settings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// The root couldn't be resolved to a directory that exists.
    #[error("the root {root} can't be used: {reason}")]
    Root {
        /// The root as the settings gave it.
        root: String,
        /// What the system said of it.
        reason: String,
    },
    /// The root is something other than a directory.
    #[error("the root {root} isn't a directory")]
    RootIsNoDirectory {
        /// The root as the settings gave it.
        root: String,
    },
    /// A variable has a name or a value no process can be started with.
    #[error("the variable {name:?} can't be set: {reason}")]
    Variable {
        /// The variable's name as the settings gave it.
        name: String,
        /// Which rule it breaks.
        reason: &'static str,
    },
}

/// The environment a command starts with: what `held` answers for each name
/// of [`ENVIRONMENT`], then `added`, which wins where both name a variable.
///
/// `held` is lablet's environment, asked for as a function so that a test
/// can say what it holds.
pub(crate) fn environment(
    held: impl Fn(&str) -> Option<OsString>,
    added: BTreeMap<String, String>,
) -> Result<BTreeMap<OsString, OsString>, SettingsError> {
    let mut environment: BTreeMap<OsString, OsString> = ENVIRONMENT
        .into_iter()
        .filter_map(|name| Some((name.into(), held(name)?)))
        .collect();
    for (name, value) in added {
        let refused = if name.is_empty() {
            Some("its name is empty")
        } else if name.contains(['=', '\0']) {
            Some("its name holds `=` or a NUL")
        } else if value.contains('\0') {
            Some("its value holds a NUL")
        } else {
            None
        };
        if let Some(reason) = refused {
            return Err(SettingsError::Variable { name, reason });
        }
        environment.insert(name.into(), value.into());
    }
    Ok(environment)
}

#[cfg(test)]
mod tests;
