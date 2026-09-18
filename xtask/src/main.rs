//! Lablet's gate runner, invoked as `cargo xtask <task>` through the alias in
//! the root `.cargo/config.toml`. `cargo xtask help` lists the tasks.
//!
//! Every rule in contributing/README.md is enforced here or is labelled a
//! review convention. xtask is its own workspace at the repository root, not a
//! member of `lablet/`, and finds the repository from its own location.

use std::process::ExitCode;

mod changelog;
mod coverage;
mod floors;
mod gates;
mod lint_layers;
mod lint_manifests;
mod mutants;
mod process;
mod report;
mod workspace;

use gates::{Mode, Step};

const USAGE: &str = "\
Usage: cargo xtask <task>

Checks:
  fmt [--fix]      rustfmt over the workspace and xtask (--fix: rewrite)
  clippy           clippy over every target, warnings denied
  lint-layers      Layer rules: which crate may depend on which, by ring
  lint-manifests   Manifest rules: inherited lints and package keys, exact
                   pins, a comment on every dependency, xtask's lint copy
  deny             cargo-deny under lablet/deny.toml: licences, advisories,
                   bans, sources
  doc              rustdoc without dependencies, warnings denied
  test             The workspace's tests and xtask's own
  changelog        A contract change needs an entry under Unreleased
                   (base: $LABLET_CHANGELOG_BASE, else the merge-base with main)

Floors (CI):
  coverage         Line coverage: 90% on lablet-model, lablet-policy, lablet-run
  mutants          Mutants caught: 80% on the same crates

Gates:
  pre-commit       fmt + clippy + lint-layers + lint-manifests
  pre-push         pre-commit + test + doc + deny + changelog
  ci               pre-push

  help             This text

Every task covers the lablet/ workspace and, where it applies, xtask itself.
A gate runs every step even after one fails, then reports them all. Pinned
tools come from mise.toml; scripts/setup.sh installs them.
";

/// A task's steps and how to run them, or why the arguments are wrong.
fn plan(task: &str, args: &[String]) -> Result<(Mode, Vec<Step>), String> {
    let (mode, steps) = match task {
        "fmt" => {
            let fix = match args {
                [] => false,
                [flag] if flag == "--fix" => true,
                _ => return Err("`fmt` takes only `--fix`".to_owned()),
            };
            return Ok((Mode::Command, gates::fmt_steps(fix)));
        }
        "clippy" => (Mode::Command, gates::clippy_steps()),
        "lint-layers" => (Mode::Command, gates::lint_layers_steps()),
        "lint-manifests" => (Mode::Command, gates::lint_manifests_steps()),
        "deny" => (Mode::Command, gates::deny_steps()),
        "doc" => (Mode::Command, gates::doc_steps()),
        "test" => (Mode::Command, gates::test_steps()),
        "coverage" => (Mode::Command, gates::coverage_steps()),
        "mutants" => (Mode::Command, gates::mutants_steps()),
        "changelog" => (Mode::Command, gates::changelog_steps()),
        "pre-commit" => (Mode::Gate("pre-commit"), gates::pre_commit_steps()),
        "pre-push" => (Mode::Gate("pre-push"), gates::pre_push_steps()),
        "ci" => (Mode::Gate("ci"), gates::pre_push_steps()),
        // Phase 1 extension point: "weaver" dispatches on its own subcommand
        // (vendor, check, generate [--check]; phase 6 adds live-check).
        // Phase 11 extension point: "bench" runs criterion and applies the
        // 20% regression threshold.
        other => return Err(format!("unknown task `{other}`")),
    };
    match args.first() {
        Some(arg) => Err(format!("`{task}` takes no arguments; got `{arg}`")),
        None => Ok((mode, steps)),
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(task) = args.next() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if matches!(task.as_str(), "help" | "--help" | "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let args: Vec<String> = args.collect();
    match plan(&task, &args) {
        Ok((mode, steps)) if gates::run(mode, &steps) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("error: {message}\n");
            eprint!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The task names the usage text documents: the first word of each
    /// indented line that is not a continuation.
    fn documented_tasks() -> Vec<&'static str> {
        USAGE
            .lines()
            .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
            .filter_map(|line| line.split_whitespace().next())
            .collect()
    }

    #[test]
    fn every_documented_task_is_dispatched() {
        let tasks = documented_tasks();
        for expected in [
            "fmt",
            "clippy",
            "lint-layers",
            "lint-manifests",
            "deny",
            "doc",
            "test",
            "coverage",
            "mutants",
            "changelog",
            "pre-commit",
            "pre-push",
            "ci",
            "help",
        ] {
            assert!(
                tasks.contains(&expected),
                "`{expected}` is not in the usage text"
            );
        }
        for task in tasks.into_iter().filter(|task| *task != "help") {
            assert!(
                plan(task, &[]).is_ok(),
                "`{task}` is documented but not dispatched"
            );
        }
    }

    #[test]
    fn an_unknown_task_and_a_stray_argument_are_usage_errors() {
        let error = |task: &str, args: &[&str]| {
            let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            plan(task, &args).err()
        };
        assert_eq!(error("fnt", &[]).as_deref(), Some("unknown task `fnt`"));
        assert_eq!(
            error("clippy", &["--fix"]).as_deref(),
            Some("`clippy` takes no arguments; got `--fix`")
        );
        assert_eq!(
            error("fmt", &["--check"]).as_deref(),
            Some("`fmt` takes only `--fix`")
        );
        assert_eq!(error("fmt", &["--fix"]), None);
    }

    #[test]
    fn the_xtask_alias_points_at_this_crate_from_both_roots() {
        let alias = |config: &std::path::Path| {
            let text = std::fs::read_to_string(config).unwrap();
            let document: toml::Value = toml::from_str(&text).unwrap();
            document["alias"]["xtask"].as_str().map(str::to_owned)
        };
        let root = workspace::repo_root();
        assert_eq!(
            alias(&root.join(".cargo/config.toml")).as_deref(),
            Some("run -q --manifest-path xtask/Cargo.toml --")
        );
        let copy = workspace::workspace_root().join(".cargo/config.toml");
        if !copy.is_file() {
            eprintln!("skipped: {} does not exist yet", copy.display());
            return;
        }
        assert_eq!(
            alias(&copy).as_deref(),
            Some("run -q --manifest-path ../xtask/Cargo.toml --"),
            "the workspace copy of the alias has drifted from the root one"
        );
    }

    #[test]
    fn ci_runs_the_pre_push_steps_as_a_gate() {
        let (mode, _) = plan("ci", &[]).unwrap();
        assert!(matches!(mode, Mode::Gate("ci")));
    }
}
