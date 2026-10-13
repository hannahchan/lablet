//! The config a command reads: its file, with each `--set` stated over it,
//! each `${VAR}` filled from the process's environment, and each relative
//! path the file wrote placed at the file's directory.

use std::path::Path;

use lablet_config::{Config, ConfigError, RawConfig};
use serde_json::Value;

use crate::cli::refusal::Refusal;

/// The config in the file at `path`, with each of `overrides`, a
/// `key=value`, stated over it in turn, so a later one states a setting over
/// an earlier one, every `${VAR}` then filled from the process's
/// environment through the input module, and every relative path the file
/// wrote made absolute against the file's directory, where one an override
/// gave starts where lablet runs.
///
/// # Errors
///
/// Returns a `config:` [`Refusal`] when the file can't be read as a config,
/// when an override can't be stated, when a `${VAR}` can't be filled, when
/// what they come to isn't a config, and when the file's directory can't be
/// told from where lablet runs.
pub(crate) fn read(path: &Path, overrides: &[String]) -> Result<Config, Refusal> {
    read_with_tree(path, overrides).map(|(config, _)| config)
}

/// [`read`], with the tree the config was read from, each `--set` stated
/// over it and each `${VAR}` filled: what `check --resolved` tells an item
/// of a list by, where the config holds the list in an order of its own.
///
/// # Errors
///
/// Returns what [`read`] returns.
pub(crate) fn read_with_tree(
    path: &Path,
    overrides: &[String],
) -> Result<(Config, Value), Refusal> {
    let mut raw = RawConfig::from_path(path)?;
    for key_value in overrides {
        raw.set(key_value)?;
    }
    let raw = raw.substituted(lablet_cli::input::variable)?;
    let config = raw.config()?;
    // The file's directory, from where lablet runs, so a bare file name's
    // directory is the working directory and not nothing.
    let absolute = std::path::absolute(path).map_err(|error| ConfigError::Unreadable {
        path: path.display().to_string(),
        reason: format!("its directory couldn't be told from the working directory: {error}"),
    })?;
    let directory = absolute.parent().unwrap_or(&absolute);
    Ok((config.relative_to(directory), raw.tree().clone()))
}

#[cfg(test)]
mod tests;
