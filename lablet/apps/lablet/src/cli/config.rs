//! The config a command reads: its file, with each `--set` stated over it.

use std::path::Path;

use lablet::{Config, RawConfig};

use crate::cli::refusal::Refusal;

/// The config in the file at `path`, with each of `overrides`, a
/// `key=value`, stated over it in turn, so a later one states a setting over
/// an earlier one.
///
/// # Errors
///
/// Returns a `config:` [`Refusal`] when the file can't be read as a config,
/// when an override can't be stated, and when what they come to isn't a
/// config.
pub(crate) fn read(path: &Path, overrides: &[String]) -> Result<Config, Refusal> {
    let mut raw = RawConfig::from_path(path)?;
    for key_value in overrides {
        raw.set(key_value)?;
    }
    Ok(raw.config()?)
}

#[cfg(test)]
mod tests;
