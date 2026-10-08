//! The `lablet` command line: `init`, `run`, `check`, and `schema` over the
//! composition in this package's library.
//!
//! This file only does effects. What it decides is in `cli`, as functions a
//! test calls.

mod cli;

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;
use lablet_run_request::CancelHandle;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::util::SubscriberInitExt as _;

use cli::args::{CheckArgs, Cli, Command, InitArgs, RunArgs};
use cli::refusal::{self, Refusal};
use cli::request::Source;
use cli::{check, config, diagnostics, init, report, request, signals};

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
        Command::Check(args) => check_command(&args),
        Command::Schema => schema_command(),
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
    let config = match config::read(&config, &overrides.set) {
        Ok(config) => config,
        Err(refusal) => return refuse(&refusal),
    };
    let on_stderr = lablet_cli::compose::telemetry_on_stderr(&config);
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
    let config = match source {
        Source::File(file) => config.with_prompt_file(file),
        Source::Given(_) | Source::Stdin => config,
    };

    let budget = config.run.max_total_tokens;
    let Some(runtime) = runtime() else {
        return report::NOT_STARTED;
    };
    let code = runtime.block_on(async {
        // Caught before the build, so a stop that comes while it's under
        // way stops the run before its first call, and the run still has
        // its outcome and its record.
        let stop = CancelHandle::new();
        let fired = stop.clone();
        if let Err(error) = signals::on_stop(move |signal| {
            tracing::info!(%signal, "stopping the run");
            fired.cancel();
        }) {
            say(&format!(
                "lablet: Ctrl-C and SIGTERM couldn't be caught: {error}"
            ));
            return report::NOT_STARTED;
        }
        let mut lablet = match lablet_cli::compose::build(config).await {
            Ok(lablet) => lablet,
            Err(error) => return refuse(&error.into()),
        };
        let finished = lablet.run(request.cancellation(stop)).await;
        let code = report::exit_code(finished.summary.outcome.stop_reason());
        let summary = report::summary(&finished, budget);
        let failure = refusal::of_run(&finished.summary.outcome);
        print_outcome(finished.summary.outcome, on_stderr);
        lablet.shutdown().await;
        // What's said about the run, apart from the telemetry, which is
        // standard error's alone when it's there.
        if !on_stderr && let Some(failure) = failure {
            say(&failure);
        }
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

fn check_command(args: &CheckArgs) -> u8 {
    log(diagnostics::filter(rust_log().as_deref(), false));
    let config = match config::read(&args.config, &args.overrides.set) {
        Ok(config) => config,
        Err(refusal) => return refuse(&refusal),
    };
    let Some(runtime) = runtime() else {
        return report::NOT_STARTED;
    };
    let checked = runtime.block_on(lablet_cli::compose::check(&config));
    runtime.shutdown_background();
    let checked = match checked {
        Ok(checked) => checked,
        Err(error) => return refuse(&error.into()),
    };
    match check::answer(&checked, args.resolved) {
        Ok(answer) => {
            if !answer.is_empty() {
                print(&answer, false);
            }
            say(&check::summary(&checked));
            0
        }
        Err(error) => {
            say(&format!(
                "lablet: the resolved config couldn't be written: {error}"
            ));
            report::NOT_STARTED
        }
    }
}

fn schema_command() -> u8 {
    match serde_json::to_string_pretty(&lablet_config::schema()) {
        Ok(schema) => {
            print(&schema, false);
            0
        }
        Err(error) => {
            say(&format!("lablet: the schema couldn't be written: {error}"));
            report::NOT_STARTED
        }
    }
}

/// The runtime a command's library calls run on, or `None`, said on
/// standard error, when it couldn't be started.
fn runtime() -> Option<tokio::runtime::Runtime> {
    tokio::runtime::Runtime::new()
        .inspect_err(|error| say(&format!("lablet: the runtime couldn't be started: {error}")))
        .ok()
}

/// Prints the outcome document on standard output, as one line.
fn print_outcome(outcome: lablet_model::RunOutcome, telemetry_on_stderr: bool) {
    match serde_json::to_string(&lablet_documents::OutcomeDocument::from(outcome)) {
        Ok(document) => print(&document, telemetry_on_stderr),
        Err(error) => say(&format!("lablet: the outcome couldn't be written: {error}")),
    }
}

/// Writes `document` on standard output, ended by a newline: the one place
/// lablet writes there. A reader that closed its end, as `head` does, isn't
/// a failure of the command, so a write that fails is said on standard error
/// and the exit code stays what the command's work says, where `println!`
/// would panic and exit 101. When the telemetry is standard error's, it's
/// standard error's alone, and the failure goes unsaid.
fn print(document: &str, telemetry_on_stderr: bool) {
    let mut out = io::stdout().lock();
    if let Err(error) = writeln!(out, "{document}").and_then(|()| out.flush())
        && !telemetry_on_stderr
    {
        say(&format!(
            "lablet: standard output couldn't be written: {error}"
        ));
    }
}

/// Installs the diagnostic log on standard error, when there's one.
fn log(filter: Option<EnvFilter>) {
    let Some(filter) = filter else {
        return;
    };
    // Only fails when a log is installed already, which is then the log.
    let _ = diagnostics::subscriber(filter, io::stderr, io::stderr().is_terminal()).try_init();
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
