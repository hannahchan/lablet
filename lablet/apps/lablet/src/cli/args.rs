//! The arguments, as `clap` reads them.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::cli::init::Starter;

/// A lightweight, instrumented agent loop.
#[derive(Debug, Parser)]
#[command(name = "lablet", version, about, arg_required_else_help = true)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// What `lablet` is asked to do.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Write a starter config that runs as it's written, and a script for
    /// `fake`.
    Init(InitArgs),
    /// Run one task, print its outcome as JSON, and exit 0 when it
    /// completed, 2 when it stopped any other way, and 1 when it never
    /// started.
    Run(RunArgs),
    /// Validate a config, start its MCP servers and list the tools, without
    /// calling the model.
    Check(CheckArgs),
    /// Print the JSON Schema of the config.
    Schema,
}

/// The arguments of `lablet init`.
#[derive(Debug, Args)]
pub(crate) struct InitArgs {
    /// The provider the config selects.
    #[arg(long, value_enum, default_value_t = Starter::Anthropic)]
    pub(crate) provider: Starter,
    /// The directory the files are written to, which is made when it's
    /// missing.
    #[arg(value_name = "DIR", default_value = ".")]
    pub(crate) path: PathBuf,
}

/// The arguments of `lablet run`.
#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    /// The config, in YAML or JSON as its name says.
    #[arg(long, value_name = "FILE")]
    pub(crate) config: PathBuf,
    #[command(flatten)]
    pub(crate) prompt: PromptArgs,
    #[command(flatten)]
    pub(crate) names: Names,
    #[command(flatten)]
    pub(crate) overrides: Overrides,
    /// Print no summary line on standard error.
    #[arg(long, short)]
    pub(crate) quiet: bool,
}

/// Where the task prompt comes from: one of the two flags, or standard
/// input with neither.
#[derive(Debug, Args)]
#[group(multiple = false)]
pub(crate) struct PromptArgs {
    /// The task prompt.
    #[arg(long, value_name = "TEXT")]
    pub(crate) prompt: Option<String>,
    /// A file that holds the task prompt.
    #[arg(long, value_name = "FILE")]
    pub(crate) prompt_file: Option<PathBuf>,
}

/// What the run is known by, which its outcome and its telemetry carry.
#[derive(Debug, Args)]
pub(crate) struct Names {
    /// The run's id; a fresh ULID when it's left out.
    #[arg(long, value_name = "ID")]
    pub(crate) run_id: Option<String>,
    /// The task the run attempts.
    #[arg(long, value_name = "ID")]
    pub(crate) task: Option<String>,
    /// The experiment the run is part of.
    #[arg(long, value_name = "ID")]
    pub(crate) experiment: Option<String>,
    /// Which repetition of the task the run is.
    #[arg(long, value_name = "N")]
    pub(crate) trial: Option<String>,
}

/// Settings stated over the config's.
#[derive(Debug, Args)]
pub(crate) struct Overrides {
    /// A setting stated over the config's, as `run.max_turns=5`; the value
    /// is read as a YAML scalar.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub(crate) set: Vec<String>,
}

/// The arguments of `lablet check`.
#[derive(Debug, Args)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "they parse, so a script can be written against them, and nothing reads them until check is built"
    )
)]
pub(crate) struct CheckArgs {
    /// The config, in YAML or JSON as its name says.
    #[arg(long, value_name = "FILE")]
    pub(crate) config: PathBuf,
    /// Print the config with every default filled in.
    #[arg(long)]
    pub(crate) resolved: bool,
    #[command(flatten)]
    pub(crate) overrides: Overrides,
}

#[cfg(test)]
mod tests;
