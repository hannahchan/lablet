use std::path::Path;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

use super::{Cli, Command};
use crate::cli::init::Starter;

fn parsed(args: &[&str]) -> Command {
    Cli::try_parse_from(["lablet"].iter().chain(args))
        .unwrap()
        .command
}

fn refused(args: &[&str]) -> ErrorKind {
    Cli::try_parse_from(["lablet"].iter().chain(args))
        .unwrap_err()
        .kind()
}

#[test]
fn the_command_line_is_one_clap_accepts() {
    Cli::command().debug_assert();
}

#[test]
fn a_run_takes_its_config_its_prompt_its_names_and_its_overrides() {
    let Command::Run(run) = parsed(&[
        "run",
        "--config",
        "lablet.yaml",
        "--prompt",
        "Fix the test.",
        "--run-id",
        "r",
        "--task",
        "t",
        "--experiment",
        "e",
        "--trial",
        "3",
        "--set",
        "run.max_turns=5",
        "--set",
        "model.name=other",
        "--quiet",
    ]) else {
        panic!("not a run");
    };
    assert_eq!(run.config, Path::new("lablet.yaml"));
    assert_eq!(run.prompt.prompt.as_deref(), Some("Fix the test."));
    assert_eq!(run.prompt.prompt_file, None);
    assert_eq!(run.names.run_id.as_deref(), Some("r"));
    assert_eq!(run.names.task.as_deref(), Some("t"));
    assert_eq!(run.names.experiment.as_deref(), Some("e"));
    assert_eq!(run.names.trial.as_deref(), Some("3"));
    assert_eq!(run.overrides.set, ["run.max_turns=5", "model.name=other"]);
    assert!(run.quiet);
}

#[test]
fn a_run_with_neither_prompt_flag_names_nothing_and_is_quiet_only_when_asked() {
    let Command::Run(run) = parsed(&["run", "--config", "lablet.yaml"]) else {
        panic!("not a run");
    };
    assert_eq!((run.prompt.prompt, run.prompt.prompt_file), (None, None));
    assert_eq!(
        (
            run.names.run_id,
            run.names.task,
            run.names.experiment,
            run.names.trial
        ),
        (None, None, None, None)
    );
    assert!(run.overrides.set.is_empty());
    assert!(!run.quiet);
}

#[test]
fn a_run_is_given_one_prompt_at_most() {
    let both = [
        "run",
        "--config",
        "lablet.yaml",
        "--prompt",
        "Fix the test.",
        "--prompt-file",
        "task.md",
    ];
    assert_eq!(refused(&both), ErrorKind::ArgumentConflict);
}

#[test]
fn a_run_needs_a_config() {
    assert_eq!(
        refused(&["run", "--prompt", "Fix the test."]),
        ErrorKind::MissingRequiredArgument
    );
}

#[test]
fn init_writes_a_config_for_anthropic_in_the_working_directory_unless_told_otherwise() {
    let Command::Init(init) = parsed(&["init"]) else {
        panic!("not an init");
    };
    assert_eq!(
        (init.provider, init.path.as_path()),
        (Starter::Anthropic, Path::new("."))
    );

    let Command::Init(init) = parsed(&["init", "--provider", "fake", "trial"]) else {
        panic!("not an init");
    };
    assert_eq!(
        (init.provider, init.path.as_path()),
        (Starter::Fake, Path::new("trial"))
    );
}

#[test]
fn init_knows_the_three_providers_and_no_other() {
    for (name, starter) in [
        ("anthropic", Starter::Anthropic),
        ("openai", Starter::Openai),
        ("fake", Starter::Fake),
    ] {
        let Command::Init(init) = parsed(&["init", "--provider", name]) else {
            panic!("not an init");
        };
        assert_eq!(init.provider, starter);
    }
    assert_eq!(
        refused(&["init", "--provider", "ollama"]),
        ErrorKind::InvalidValue
    );
}

#[test]
fn check_takes_a_config_the_resolved_flag_and_overrides() {
    let Command::Check(check) = parsed(&[
        "check",
        "--config",
        "lablet.yaml",
        "--resolved",
        "--set",
        "run.max_turns=5",
    ]) else {
        panic!("not a check");
    };
    assert_eq!(check.config, Path::new("lablet.yaml"));
    assert!(check.resolved);
    assert_eq!(check.overrides.set, ["run.max_turns=5"]);

    let Command::Check(check) = parsed(&["check", "--config", "lablet.yaml"]) else {
        panic!("not a check");
    };
    assert!(!check.resolved);
}

#[test]
fn schema_takes_nothing() {
    assert!(matches!(parsed(&["schema"]), Command::Schema));
    assert_eq!(
        refused(&["schema", "--config", "x"]),
        ErrorKind::UnknownArgument
    );
}
