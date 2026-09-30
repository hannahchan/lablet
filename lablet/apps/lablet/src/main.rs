//! The `lablet` command line: `init`, `run`, `check`, and `schema` over the
//! library in this package.
//!
//! This file only does effects. What it decides is in `cli`, as functions a
//! test calls.

mod cli;

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;
use lablet::Config;
use tracing_subscriber::EnvFilter;

use cli::args::{Cli, Command, InitArgs, RunArgs};
use cli::refusal::Refusal;
use cli::request::Source;
use cli::{diagnostics, init, report, request};

fn main() -> ExitCode {
    // Before anything else, so no other process of the user can read the
    // environment, which holds the key, through `/proc`, and no core dump
    // holds it either.
    #[cfg(target_os = "linux")]
    {
        if let Err(error) = nix::sys::prctl::set_dumpable(false) {
            say(&format!(
                "lablet: the process's dumpable flag couldn't be cleared, so other processes \
                 could read its environment: {error}"
            ));
            return ExitCode::from(report::NOT_STARTED);
        }
    }
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // clap writes help to standard output and a refusal to standard
            // error, and nothing is left to do if neither can be written.
            let _ = error.print();
            return ExitCode::from(report::usage_exit_code(&error));
        }
    };
    ExitCode::from(match cli.command {
        Command::Init(args) => init_command(&args),
        Command::Run(args) => run_command(args),
        Command::Check(_) => refuse(&Refusal::NotBuilt("lablet check")),
        Command::Schema => refuse(&Refusal::NotBuilt("lablet schema")),
    })
}

fn init_command(args: &InitArgs) -> u8 {
    log(diagnostics::filter(rust_log().as_deref(), false));
    match init::write(args.provider, &args.path) {
        Ok(paths) => {
            say(&init::written(&paths, &args.path));
            0
        }
        Err(refusal) => refuse(&refusal),
    }
}

fn run_command(args: RunArgs) -> u8 {
    let RunArgs {
        config,
        prompt,
        names,
        overrides,
        quiet,
    } = args;
    if !overrides.set.is_empty() {
        return refuse(&Refusal::NotBuilt("lablet run --set"));
    }
    let config = match Config::from_path(&config) {
        Ok(config) => config,
        Err(error) => return refuse(&error.into()),
    };
    let on_stderr = diagnostics::telemetry_on_stderr(&config);
    log(diagnostics::filter(rust_log().as_deref(), on_stderr));

    let source = Source::from(prompt);
    if source == Source::Stdin {
        tracing::info!("reading the task prompt from standard input");
    }
    let request = match request::read(&source, io::stdin().lock())
        .and_then(|prompt| request::request(prompt, &source, names))
    {
        Ok(request) => request,
        Err(refusal) => return refuse(&refusal),
    };

    let budget = config.run.max_total_tokens;
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            say(&format!("lablet: the runtime couldn't be started: {error}"));
            return report::NOT_STARTED;
        }
    };
    let code = runtime.block_on(async {
        let mut lablet = match lablet::build(config).await {
            Ok(lablet) => lablet,
            Err(error) => return refuse(&error.into()),
        };
        let finished = lablet.run(request).await;
        let code = report::exit_code(finished.summary.outcome.stop_reason());
        let summary = report::summary(&finished, budget);
        print_outcome(finished.summary.outcome);
        lablet.shutdown().await;
        if report::summary_wanted(quiet, on_stderr) {
            say(&summary);
        }
        code
    });
    // The telemetry's shutdown has had its bound, and what it left
    // exporting would otherwise hold the process past it.
    runtime.shutdown_background();
    code
}

/// Prints the outcome document on standard output, as one line.
#[expect(
    clippy::print_stdout,
    reason = "the outcome document is what `lablet run` writes on standard output"
)]
fn print_outcome(outcome: lablet_model::RunOutcome) {
    match serde_json::to_string(&lablet::OutcomeDocument::from(outcome)) {
        Ok(document) => println!("{document}"),
        Err(error) => say(&format!("lablet: the outcome couldn't be written: {error}")),
    }
}

/// Installs the diagnostic log on standard error, when there's one.
fn log(filter: Option<EnvFilter>) {
    let Some(filter) = filter else {
        return;
    };
    // Only fails when a log is installed already, which is then the log.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .with_ansi(io::stderr().is_terminal())
        .try_init();
}

fn rust_log() -> Option<OsString> {
    std::env::var_os("RUST_LOG")
}

fn refuse(refusal: &Refusal) -> u8 {
    say(&refusal.to_string());
    report::NOT_STARTED
}

/// Writes `line` on standard error. There's nowhere left to say that it
/// couldn't be written.
fn say(line: &str) {
    let _ = writeln!(io::stderr().lock(), "{line}");
}
