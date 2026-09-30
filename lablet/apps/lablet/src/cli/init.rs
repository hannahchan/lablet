//! `lablet init`: the starter configs, which are files of this package, and
//! writing them where a user asks.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cli::refusal::Refusal;

/// The provider a starter config selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Starter {
    /// Anthropic's Messages API.
    Anthropic,
    /// OpenAI's Responses API.
    Openai,
    /// A script played in place of a model, which needs no key.
    Fake,
}

/// A file `init` writes: its name in the directory it's given, and what it
/// holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StarterFile {
    pub(crate) name: &'static str,
    pub(crate) text: &'static str,
}

/// The name every starter config has.
pub(crate) const CONFIG: &str = "lablet.yaml";

/// The script the `fake` starter plays, which its config names.
pub(crate) const SCRIPT: StarterFile = StarterFile {
    name: "lablet-script.yaml",
    text: include_str!("init/lablet-script.yaml"),
};

const ANTHROPIC: [StarterFile; 1] = [StarterFile {
    name: CONFIG,
    text: include_str!("init/anthropic.yaml"),
}];

const OPENAI: [StarterFile; 1] = [StarterFile {
    name: CONFIG,
    text: include_str!("init/openai.yaml"),
}];

const FAKE: [StarterFile; 2] = [
    StarterFile {
        name: CONFIG,
        text: include_str!("init/fake.yaml"),
    },
    SCRIPT,
];

impl Starter {
    /// The files the starter is made of, its config first.
    pub(crate) const fn files(self) -> &'static [StarterFile] {
        match self {
            Self::Anthropic => &ANTHROPIC,
            Self::Openai => &OPENAI,
            Self::Fake => &FAKE,
        }
    }
}

/// Writes the files of `starter` into `directory`, which is made when it's
/// missing, and returns their paths. Nothing is written over: when any of
/// the files is there already, none is written.
///
/// # Errors
///
/// Returns [`Refusal::Config`] when one of the files is there already, and
/// when the directory or a file can't be written.
pub(crate) fn write(starter: Starter, directory: &Path) -> Result<Vec<PathBuf>, Refusal> {
    let files = starter.files();
    let paths: Vec<PathBuf> = files.iter().map(|file| directory.join(file.name)).collect();
    if let Some(there) = paths.iter().find(|path| path.exists()) {
        return Err(Refusal::Config(format!(
            "{} is there already, and init writes over nothing",
            there.display()
        )));
    }
    std::fs::create_dir_all(directory).map_err(|error| {
        Refusal::Config(format!("{} can't be made: {error}", directory.display()))
    })?;
    for (file, path) in files.iter().zip(&paths) {
        // `create_new`, so a file made since the look above isn't lost.
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .and_then(|mut written| written.write_all(file.text.as_bytes()))
            .map_err(|error| {
                Refusal::Config(format!("{} can't be written: {error}", path.display()))
            })?;
    }
    Ok(paths)
}

/// What `init` says once it has written `paths` into `directory`: what it
/// wrote, and the command that runs the config. A relative path in a config
/// starts at the directory lablet runs in, so the command runs from
/// `directory`.
pub(crate) fn written(paths: &[PathBuf], directory: &Path) -> String {
    let shown: Vec<String> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let run = format!("lablet run --config {CONFIG} --prompt \"Say hello.\"");
    let from = if directory == Path::new(".") {
        String::new()
    } else {
        format!("cd {} && ", directory.display())
    };
    format!("wrote {}; run it with: {from}{run}", shown.join(" and "))
}

#[cfg(test)]
mod tests;
