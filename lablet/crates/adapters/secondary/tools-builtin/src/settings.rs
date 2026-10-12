//! What an executor that serves tools is built from, the environment its
//! commands start from, and why one can't be built.

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
/// Its `Debug` form names the variables of the environment and leaves their
/// values out, since a value may be a credential that a command needs and a
/// log doesn't.
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
    /// The environment every command starts from, computed once by
    /// [`environment`]: what was handed in, less what's dropped, with the
    /// config's own variables and credentials on top. The context
    /// variables are the exception: each command gets its tool span's as it
    /// starts.
    pub environment: BTreeMap<OsString, OsString>,
    /// The names no context is injected into: the config's own variables
    /// and credentials, which win over a context variable that's injected,
    /// and the names dropped, which stay dropped. The values of what's
    /// dropped aren't here, since a command can find a value some other way
    /// than its environment, in a file or in lablet's own environment
    /// through the process table, as `ps eww -p $PPID` and
    /// `/proc/<pid>/environ` read it. So the values come with every call, as
    /// [`lablet_run::ToolCall::secrets`], and are cut out of every result
    /// however the command came by them.
    pub kept: BTreeSet<String>,
}

impl core::fmt::Debug for Settings {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let Self {
            root,
            enabled,
            timeout,
            environment,
            kept,
        } = self;
        f.debug_struct("Settings")
            .field("root", root)
            .field("enabled", enabled)
            .field("timeout", timeout)
            .field("environment", &environment.keys().collect::<Vec<_>>())
            .field("kept", kept)
            .finish()
    }
}

/// Why an executor couldn't be built from its settings, or a command's
/// environment from what it starts from.
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
    /// A variable of the config's own has a name or a value no process can
    /// be started with.
    #[error("the variable {name:?} can't be set: {reason}")]
    Variable {
        /// The variable's name as the config gave it.
        name: String,
        /// Which rule it breaks.
        reason: &'static str,
    },
    /// A credential given to the command has a name or a value no process
    /// can be started with.
    #[error("the credential {name:?} can't be set: {reason}")]
    Credential {
        /// The credential's name as the config gave it.
        name: String,
        /// Which rule it breaks.
        reason: &'static str,
    },
}

/// The environment a command starts from, and what was dropped from the
/// base on the way to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    /// The environment: the base less the drops, then the config's own
    /// variables, then its credentials. Its values aren't shown by its
    /// `Debug` form's holder, [`Settings`].
    pub environment: BTreeMap<OsString, OsString>,
    /// The variables of the base dropped by name: those that filled an own
    /// or given field.
    pub by_name: BTreeSet<String>,
    /// The variables of the base dropped by value: those holding an own or
    /// given value, or its percent-encoded form, under any name.
    pub by_value: BTreeSet<String>,
}

/// The environment a command starts from: `base`, less every variable
/// `dropped_names` names and every variable whose value `secrets` would
/// cut, as it is or percent-decoded, then `env`, then `credentials`, each
/// of which wins where it names a variable of what came before.
///
/// The drop by value is the one place lablet matches values, over the
/// environment it was handed and against the values it holds: it's the only
/// guard against a copy of a credential under a name the config never
/// wrote, a key set in code among them. A value is matched as the cut
/// matches it, so a value under the cut's floor is never dropped for it.
///
/// `base` is asked for as a list so that a caller says what it holds: the
/// command line hands in its own environment, a host what it chooses.
///
/// # Errors
///
/// Returns [`SettingsError::Variable`] or [`SettingsError::Credential`] when
/// an entry of `env` or `credentials` has a name or a value no process can
/// be started with.
pub fn environment(
    base: impl IntoIterator<Item = (OsString, OsString)>,
    dropped_names: &BTreeSet<String>,
    secrets: &Secrets,
    env: BTreeMap<String, String>,
    credentials: BTreeMap<String, String>,
) -> Result<Dropped, SettingsError> {
    let mut environment = BTreeMap::new();
    let mut by_name = BTreeSet::new();
    let mut by_value = BTreeSet::new();
    for (name, value) in base {
        // A name that isn't text filled nothing, since a config writes text.
        if name
            .to_str()
            .is_some_and(|shown| dropped_names.contains(shown))
        {
            by_name.insert(name.to_string_lossy().into_owned());
        } else if holds_a_secret(secrets, &value) {
            by_value.insert(name.to_string_lossy().into_owned());
        } else {
            environment.insert(name, value);
        }
    }
    for (name, value) in env {
        let (name, value) = checked(name, value)
            .map_err(|(name, reason)| SettingsError::Variable { name, reason })?;
        environment.insert(name, value);
    }
    for (name, value) in credentials {
        let (name, value) = checked(name, value)
            .map_err(|(name, reason)| SettingsError::Credential { name, reason })?;
        environment.insert(name, value);
    }
    Ok(Dropped {
        environment,
        by_name,
        by_value,
    })
}

/// Whether `secrets` would cut `value`, as it is or percent-decoded, read
/// as a command's output is read.
fn holds_a_secret(secrets: &Secrets, value: &OsString) -> bool {
    if secrets.is_empty() {
        return false;
    }
    let text = value.to_string_lossy();
    let decoded = percent_decoded(&text);
    secrets.redacted(&text) != text || secrets.redacted(&decoded) != decoded
}

/// `name` and `value` as a process can be started with them, or why not.
fn checked(name: String, value: String) -> Result<(OsString, OsString), (String, &'static str)> {
    let refused = if name.is_empty() {
        Some("its name is empty")
    } else if name.contains(['=', '\0']) {
        Some("its name holds `=` or a NUL")
    } else if value.contains('\0') {
        Some("its value holds a NUL")
    } else {
        None
    };
    match refused {
        Some(reason) => Err((name, reason)),
        None => Ok((name.into(), value.into())),
    }
}

/// `text` with each `%XX` read as the byte it encodes, and a `%` that
/// begins no such pair kept as it is, read as a command's output is read.
/// The adapter's own, since a kernel's isn't its to use.
fn percent_decoded(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some(at) = rest.iter().position(|&byte| byte == b'%') {
        bytes.extend_from_slice(&rest[..at]);
        let encoded = rest
            .get(at + 1..at + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = encoded {
            bytes.push(byte);
            rest = &rest[at + 3..];
        } else {
            bytes.push(b'%');
            rest = &rest[at + 1..];
        }
    }
    bytes.extend_from_slice(rest);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests;
