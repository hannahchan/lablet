//! What an executor that serves tools is built from, and why one can't be
//! built.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use lablet_model::Secrets;

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

lablet_model::every_variant!(Tool::ALL = [Bash, ReadFile, WriteFile]);

impl Tool {
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
    /// Variables a command starts with on top of the ones it inherits. One
    /// that has the name of an inherited variable replaces it, and one that
    /// has the name of a withheld variable passes it on.
    pub env: BTreeMap<String, String>,
    /// lablet's own secrets, which no command inherits and no result shows.
    pub withheld: Withheld,
}

/// lablet's own secrets: the variables lablet reads them from, which no
/// command inherits, and their values, which are cut out of every result.
///
/// A command can find a value some other way than its environment, in a
/// file or in lablet's own environment through the process table, as
/// `ps eww -p $PPID` and `/proc/<pid>/environ` read it. So a value is cut
/// out of every result however the command came by it, as long as it's
/// written as lablet holds it.
///
/// Its `Debug` form names the variables and says nothing of the values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Withheld {
    /// Variables of lablet's environment that no command inherits.
    pub variables: BTreeSet<String>,
    /// Values that no result shows.
    pub values: Secrets,
}

impl core::fmt::Debug for Settings {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let Self {
            root,
            enabled,
            timeout,
            env,
            withheld,
        } = self;
        f.debug_struct("Settings")
            .field("root", root)
            .field("enabled", enabled)
            .field("timeout", timeout)
            .field("env", &env.keys().collect::<Vec<_>>())
            .field("withheld", withheld)
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

/// The environment a command starts with: `inherited` less the variables
/// that `withheld` names, then `added`, which wins where it names a variable
/// of either.
///
/// `inherited` is lablet's environment, asked for as a list so that a test
/// can say what it holds.
pub(crate) fn environment(
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
    withheld: &BTreeSet<String>,
    added: BTreeMap<String, String>,
) -> Result<BTreeMap<OsString, OsString>, SettingsError> {
    let mut environment: BTreeMap<OsString, OsString> = inherited
        .into_iter()
        .filter(|(name, _)| !name.to_str().is_some_and(|name| withheld.contains(name)))
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
