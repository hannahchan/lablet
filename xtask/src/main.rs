//! Lablet's gate runner, invoked as `cargo xtask <task>` through the alias in
//! the root `.cargo/config.toml`. `cargo xtask help` lists the tasks.
//!
//! Every rule in contributing/README.md is enforced here or is labelled a
//! review convention there.

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
Usage: cargo xtask <task> [args]

Development:
  check               Type-check every target
  build [--release]   Build the workspace
  run [-- <args>]     Run the lablet binary, passing <args> to it
  test                Run tests, doctests included
  doc                 Build rustdoc with warnings denied

Quality checks:
  fmt [--check]       Format with rustfmt (--check: verify only)
  fix                 Apply clippy's machine-applicable fixes, then fmt
  clippy              Lint every target with warnings denied
  lint-layers         Layer dependency rules
  lint-manifests      Manifest rules: inheritance, exact pins, xtask's lint copy
  lint-prose [--all]  Prose style with vale (--all: warnings and suggestions too)
  deny                Licences, advisories, bans, sources
  changelog           A contract change has an entry under Unreleased

Quality gates:
  pre-commit          fmt --check + clippy + lint-layers + lint-manifests + lint-prose
  pre-push            pre-commit + deny + changelog + doc + test
  ci                  pre-push

Analysis:
  coverage            Line coverage against the floors
  mutants             Mutation testing against the floors

Project:
  setup               Install the pinned toolchain, tools, and git hooks
  clean               Remove build, coverage, and mutation output

Tasks cover the lablet/ workspace and, where it applies, xtask. A gate runs
every step even after one fails. On a terminal it lists each step; off one (a
hook, a pipe) a green gate prints one line, and XTASK_VERBOSE=1 or CI=true
lists the steps anyway. Pinned tools come from mise.toml.
";

/// A task's steps and how to run them, or why the arguments are wrong.
fn plan(task: &str, args: &[String]) -> Result<(Mode, Vec<Step>), String> {
    let flag = |name: &str| match args {
        [] => Ok(false),
        [arg] if arg == name => Ok(true),
        _ => Err(format!("`{task}` takes only `{name}`")),
    };
    let with_arguments = match (task, args) {
        ("build", _) => Some((Mode::Command, gates::build_steps(flag("--release")?))),
        ("fmt", _) => Some((Mode::Command, gates::fmt_steps(flag("--check")?))),
        ("lint-prose", _) => Some((Mode::Command, gates::lint_prose_steps(flag("--all")?))),
        ("run", []) => Some((Mode::Passthrough, gates::run_steps(&[]))),
        ("run", [dashes, rest @ ..]) if dashes == "--" => {
            Some((Mode::Passthrough, gates::run_steps(rest)))
        }
        ("run", _) => return Err("`run` takes the binary's arguments after `--`".to_owned()),
        _ => None,
    };
    if let Some(planned) = with_arguments {
        return Ok(planned);
    }
    let planned = match task {
        "check" => (Mode::Command, gates::check_steps()),
        "test" => (Mode::Command, gates::test_steps()),
        "doc" => (Mode::Command, gates::doc_steps()),
        "fix" => (Mode::Command, gates::fix_steps()),
        "clippy" => (Mode::Command, gates::clippy_steps()),
        "lint-layers" => (Mode::Command, gates::lint_layers_steps()),
        "lint-manifests" => (Mode::Command, gates::lint_manifests_steps()),
        "deny" => (Mode::Command, gates::deny_steps()),
        "changelog" => (Mode::Command, gates::changelog_steps()),
        "pre-commit" => (Mode::Gate("pre-commit"), gates::pre_commit_steps()),
        "pre-push" => (Mode::Gate("pre-push"), gates::pre_push_steps()),
        "ci" => (Mode::Gate("ci"), gates::pre_push_steps()),
        "coverage" => (Mode::Command, gates::coverage_steps()),
        "mutants" => (Mode::Command, gates::mutants_steps()),
        "setup" => (Mode::Command, gates::setup_steps()),
        "clean" => (Mode::Command, gates::clean_steps()),
        other => return Err(format!("unknown task `{other}`")),
    };
    match args.first() {
        Some(arg) => Err(format!("`{task}` takes no arguments; got `{arg}`")),
        None => Ok(planned),
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

    /// Task lines are the ones indented by exactly two spaces.
    fn documented_tasks() -> Vec<&'static str> {
        USAGE
            .lines()
            .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
            .filter_map(|line| line.split_whitespace().next())
            .collect()
    }

    #[test]
    fn the_usage_text_groups_every_task_in_the_order_a_developer_works() {
        assert_eq!(
            documented_tasks().join(" "),
            "check build run test doc fmt fix clippy lint-layers lint-manifests lint-prose deny \
             changelog pre-commit pre-push ci coverage mutants setup clean"
        );
        for task in documented_tasks() {
            assert!(
                plan(task, &[]).is_ok(),
                "`{task}` is documented but not dispatched"
            );
        }
    }

    #[test]
    fn a_failed_gate_step_names_a_task_that_runs_it_alone() {
        for step in gates::pre_push_steps() {
            let task = step.label.split(' ').next().unwrap();
            assert!(plan(task, &[]).is_ok(), "{}", step.label);
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
            error("fmt", &["--fix"]).as_deref(),
            Some("`fmt` takes only `--check`")
        );
        assert_eq!(
            error("run", &["--version"]).as_deref(),
            Some("`run` takes the binary's arguments after `--`")
        );
        for (task, args) in [
            ("fmt", vec!["--check"]),
            ("build", vec!["--release"]),
            ("lint-prose", vec!["--all"]),
            ("run", vec!["--", "--version"]),
        ] {
            assert_eq!(error(task, &args), None, "{task}");
        }
    }

    #[test]
    fn the_xtask_alias_points_at_this_crate_from_both_roots() {
        let alias = |config: &std::path::Path| {
            assert!(
                config.is_file(),
                "{} is missing; `cargo xtask` needs the alias at the repository root and its \
                 copy in lablet/",
                config.display()
            );
            let text = std::fs::read_to_string(config).unwrap();
            let document: toml::Value = toml::from_str(&text).unwrap();
            document
                .get("alias")
                .and_then(|aliases| aliases.get("xtask"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        };
        let root = workspace::repo_root();
        assert_eq!(
            alias(&root.join(".cargo/config.toml")).as_deref(),
            Some("run -q --locked --manifest-path xtask/Cargo.toml --")
        );
        assert_eq!(
            alias(&workspace::workspace_root().join(".cargo/config.toml")).as_deref(),
            Some("run -q --locked --manifest-path ../xtask/Cargo.toml --"),
            "the workspace copy of the alias has drifted from the root one"
        );
    }

    #[test]
    fn ci_runs_the_pre_push_steps_as_a_gate() {
        let (mode, _) = plan("ci", &[]).unwrap();
        assert!(matches!(mode, Mode::Gate("ci")));
    }
}
