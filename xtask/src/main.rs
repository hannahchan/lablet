//! Lablet's gate runner, invoked as `cargo xtask <task>` through the alias in
//! the root `.cargo/config.toml`. `cargo xtask help` lists the tasks.
//!
//! Every rule in contributing/README.md is enforced here or is labelled a
//! review convention there.

use std::process::ExitCode;

mod changelog;
mod coverage;
mod error;
mod floors;
mod gates;
mod generated;
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
  check                    Type-check every target
  build [--release]        Build the workspace
  run [-- <args>]          Run the lablet binary, passing <args> to it
  test                     Run tests, doctests included
  doc                      Build rustdoc with warnings denied

Telemetry contract:
  weaver check             Registry against the lablet, naming, and stability policies
  weaver generate [--check]
                           Render the registry crate and docs (--check: compare only)
  weaver vendor [--check]  Fetch the pinned upstream registries (--check: compare only)

Quality checks:
  fmt [--check]            Format with rustfmt + dprint (--check: verify only)
  fix                      Apply clippy's machine-applicable fixes, then fmt
  clippy                   Lint every target with warnings denied
  lint-layers              Layer dependency rules
  lint-manifests           Manifest rules: inheritance, exact pins, xtask's lint copy
  lint-shell               Shell scripts with shellcheck
  lint-prose [--all]       Prose style with vale (--all: warnings and suggestions too)
  deny                     Licences, advisories, bans, sources
  changelog                A contract change has an entry under Unreleased

Quality gates:
  pre-commit               fmt --check + clippy + lint-layers + lint-manifests + weaver check
                           + weaver generate --check + lint-shell + lint-prose
  pre-push                 pre-commit + deny + changelog + doc + test + mutants --changed
  ci                       pre-push

Analysis:
  coverage [--branch]      Line and region coverage floors (--branch: branches, on nightly)
  mutants [--changed]      The exact mutation floor (--changed: only what changed)

Project:
  setup                    Install the pinned toolchain, tools, and git hooks
  clean                    Remove build, coverage, and mutation output

Tasks cover the lablet/ workspace and, where it applies, xtask. A gate runs
every step even after one fails. On a terminal it lists each step; off one (a
hook, a pipe) a green gate prints one line, and XTASK_VERBOSE=1 or CI=true
lists the steps anyway. Pinned tools come from mise.toml.
";

const WEAVER_USAGE: &str = "`weaver` takes `check`, `generate [--check]`, or `vendor [--check]`";

/// Why the arguments name no task to run.
#[derive(Debug, thiserror::Error)]
enum Usage {
    #[error("unknown task `{0}`")]
    UnknownTask(String),
    #[error("`{task}` takes no arguments; got `{got}`")]
    NoArguments { task: String, got: String },
    #[error("`{task}` takes only `{flag}`")]
    OnlyFlag { task: String, flag: String },
    #[error("{WEAVER_USAGE}")]
    Weaver,
    #[error("`run` takes the binary's arguments after `--`")]
    Run,
}

/// A task's steps and how to run them, or why the arguments are wrong.
fn plan(task: &str, args: &[String]) -> Result<(Mode, Vec<Step>), Usage> {
    let flag_of = |task: &str, args: &[String], name: &str| match args {
        [] => Ok(false),
        [arg] if arg == name => Ok(true),
        _ => Err(Usage::OnlyFlag {
            task: task.to_owned(),
            flag: name.to_owned(),
        }),
    };
    let flag = |name: &str| flag_of(task, args, name);
    let with_arguments = match (task, args) {
        ("build", _) => Some((Mode::Command, gates::build_steps(flag("--release")?))),
        ("fmt", _) => Some((Mode::Command, gates::fmt_steps(flag("--check")?))),
        ("lint-prose", _) => Some((Mode::Command, gates::lint_prose_steps(flag("--all")?))),
        ("coverage", _) => Some((Mode::Command, gates::coverage_steps(flag("--branch")?))),
        ("mutants", _) => Some((Mode::Command, gates::mutants_steps(flag("--changed")?))),
        ("weaver", [subtask]) if subtask == "check" => {
            Some((Mode::Command, gates::weaver_check_steps()))
        }
        ("weaver", [subtask, rest @ ..]) if subtask == "generate" => {
            let check = flag_of("weaver generate", rest, "--check")?;
            Some((Mode::Command, gates::weaver_generate_steps(check)))
        }
        ("weaver", [subtask, rest @ ..]) if subtask == "vendor" => {
            let check = flag_of("weaver vendor", rest, "--check")?;
            Some((Mode::Command, gates::weaver_vendor_steps(check)))
        }
        ("weaver", _) => return Err(Usage::Weaver),
        ("run", []) => Some((Mode::Passthrough, gates::run_steps(&[]))),
        ("run", [dashes, rest @ ..]) if dashes == "--" => {
            Some((Mode::Passthrough, gates::run_steps(rest)))
        }
        ("run", _) => return Err(Usage::Run),
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
        "lint-shell" => (Mode::Command, gates::lint_shell_steps()),
        "deny" => (Mode::Command, gates::deny_steps()),
        "changelog" => (Mode::Command, gates::changelog_steps()),
        "pre-commit" => (Mode::Gate("pre-commit"), gates::pre_commit_steps()),
        "pre-push" => (Mode::Gate("pre-push"), gates::pre_push_steps()),
        "ci" => (Mode::Gate("ci"), gates::pre_push_steps()),
        "setup" => (Mode::Command, gates::setup_steps()),
        "clean" => (Mode::Command, gates::clean_steps()),
        other => return Err(Usage::UnknownTask(other.to_owned())),
    };
    match args.first() {
        Some(arg) => Err(Usage::NoArguments {
            task: task.to_owned(),
            got: arg.clone(),
        }),
        None => Ok(planned),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    exit_code(&args, gates::run)
}

/// What `cargo xtask <args>` exits with, `run` running the steps a task
/// plans and saying whether all passed. Success is help, or a run in which
/// every step passed; a usage error runs nothing.
fn exit_code(args: &[String], run: impl FnOnce(Mode, &[Step]) -> bool) -> ExitCode {
    let Some((task, args)) = args.split_first() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if matches!(task.as_str(), "help" | "--help" | "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match plan(task, args) {
        Ok((mode, steps)) if run(mode, &steps) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(usage) => {
            eprintln!("error: {}\n", error::chain(&usage));
            eprint!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Task lines are the ones indented by exactly two spaces. A task is the
    /// words ahead of the two-space gap, less any `[argument]`.
    fn documented_tasks() -> Vec<String> {
        USAGE
            .lines()
            .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
            .filter_map(|line| line.trim_start().split("  ").next())
            .map(|usage| usage.split(" [").next().unwrap_or(usage).to_owned())
            .collect()
    }

    /// A task as typed after `cargo xtask`, without arguments of its own.
    fn plan_of(task: &str) -> Result<(Mode, Vec<Step>), Usage> {
        let words: Vec<String> = task.split(' ').map(str::to_owned).collect();
        plan(&words[0], &words[1..])
    }

    #[test]
    fn the_usage_text_groups_every_task_in_the_order_a_developer_works() {
        assert_eq!(
            documented_tasks().join(", "),
            "check, build, run, test, doc, weaver check, weaver generate, weaver vendor, fmt, fix, clippy, \
             lint-layers, lint-manifests, lint-shell, lint-prose, deny, changelog, pre-commit, \
             pre-push, ci, coverage, mutants, setup, clean"
        );
        for task in documented_tasks() {
            assert!(
                plan_of(&task).is_ok(),
                "`{task}` is documented but not dispatched"
            );
        }
    }

    #[test]
    fn a_failed_gate_step_names_a_task_that_runs_it_alone() {
        for step in gates::pre_push_steps() {
            assert!(
                plan_of(gates::task_of(step.label)).is_ok(),
                "{}",
                step.label
            );
        }
    }

    #[test]
    fn an_unknown_task_and_a_stray_argument_are_usage_errors() {
        let error = |task: &str, args: &[&str]| {
            let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            plan(task, &args).err().map(|usage| usage.to_string())
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
        assert_eq!(
            error("coverage", &["--branches"]).as_deref(),
            Some("`coverage` takes only `--branch`")
        );
        assert_eq!(
            error("mutants", &["--changed", "--branch"]).as_deref(),
            Some("`mutants` takes only `--changed`")
        );
        for args in [vec![], vec!["live-check"], vec!["check", "--v2"]] {
            assert_eq!(
                error("weaver", &args).as_deref(),
                Some(WEAVER_USAGE),
                "{args:?}"
            );
        }
        assert_eq!(
            error("weaver", &["generate", "--force"]).as_deref(),
            Some("`weaver generate` takes only `--check`")
        );
        assert_eq!(
            error("weaver", &["vendor", "--force"]).as_deref(),
            Some("`weaver vendor` takes only `--check`")
        );
        for (task, args) in [
            ("fmt", vec!["--check"]),
            ("build", vec!["--release"]),
            ("lint-prose", vec!["--all"]),
            ("coverage", vec![]),
            ("coverage", vec!["--branch"]),
            ("mutants", vec![]),
            ("mutants", vec!["--changed"]),
            ("run", vec!["--", "--version"]),
            ("weaver", vec!["check"]),
            ("weaver", vec!["generate"]),
            ("weaver", vec!["generate", "--check"]),
            ("weaver", vec!["vendor"]),
            ("weaver", vec!["vendor", "--check"]),
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
    fn a_failed_run_exits_non_zero_and_a_passed_one_zero() {
        let words = |line: &str| -> Vec<String> { line.split(' ').map(str::to_owned).collect() };
        let ran = std::cell::RefCell::new(Vec::new());
        let run = |passed: bool| {
            let ran = &ran;
            move |_: Mode, steps: &[Step]| {
                ran.borrow_mut().extend(steps.iter().map(|step| step.label));
                passed
            }
        };
        assert_eq!(
            exit_code(&words("lint-layers"), run(true)),
            ExitCode::SUCCESS
        );
        assert_eq!(
            exit_code(&words("lint-layers"), run(false)),
            ExitCode::FAILURE
        );
        assert_eq!(*ran.borrow(), ["lint-layers", "lint-layers"]);

        // Help runs nothing and succeeds; a usage error runs nothing and fails.
        assert_eq!(exit_code(&words("help"), run(false)), ExitCode::SUCCESS);
        assert_eq!(exit_code(&[], run(true)), ExitCode::FAILURE);
        assert_eq!(exit_code(&words("fnt"), run(true)), ExitCode::FAILURE);
        assert_eq!(
            exit_code(&words("clippy --fix"), run(true)),
            ExitCode::FAILURE
        );
        assert_eq!(ran.borrow().len(), 2);
    }

    #[test]
    fn ci_runs_the_pre_push_steps_as_a_gate() {
        let (mode, steps) = plan("ci", &[]).unwrap();
        assert!(matches!(mode, Mode::Gate("ci")));
        let labels = |steps: &[Step]| steps.iter().map(|step| step.label).collect::<Vec<_>>();
        assert_eq!(labels(&steps), labels(&gates::pre_push_steps()));
    }

    /// A clone whose `core.hooksPath` is absolute runs the main checkout's
    /// hooks for a commit in any of its worktrees, so a hook that found the
    /// repository from its own location gated the wrong files. A stand-in
    /// `cargo` records where each hook ran it.
    #[test]
    fn a_hook_gates_the_worktree_it_runs_for_not_the_checkout_holding_it() {
        use std::os::unix::fs::PermissionsExt as _;
        use workspace::fixture::scratch_git_with;

        let dir = workspace::fixture::TempDir::new("hooks");
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let (main, worktree) = (root.join("main"), root.join("worktree"));
        let record = root.join("record");
        dir.write(
            "cargo-home/bin/cargo",
            "#!/bin/sh\necho \"$(pwd -P) $*\" >> \"$XTASK_HOOK_RECORD\"\n",
        );
        let fake = root.join("cargo-home/bin/cargo");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let hooks = main.join("scripts/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        for hook in ["pre-commit", "pre-push"] {
            std::fs::copy(
                workspace::repo_root().join("scripts/hooks").join(hook),
                hooks.join(hook),
            )
            .unwrap();
        }

        // What the hooks read, beside what pins git to each repository.
        let cargo_home = root.join("cargo-home");
        let env = [
            ("HOME", root.as_path()),
            ("CARGO_HOME", cargo_home.as_path()),
            ("XTASK_HOOK_RECORD", record.as_path()),
        ];
        let git = |directory: &std::path::Path, args: &[&str]| {
            scratch_git_with(directory, args, &env).trim().to_owned()
        };
        git(&main, &["init", "--quiet", "--initial-branch=main"]);
        assert_eq!(
            git(&main, &["rev-parse", "--show-toplevel"]),
            main.to_str().unwrap(),
            "git resolved outside the scratch checkout"
        );
        git(
            &main,
            &["commit", "--quiet", "--allow-empty", "--message=base"],
        );
        git(
            &main,
            &["worktree", "add", "--quiet", "-b", "topic", "../worktree"],
        );
        assert_eq!(
            git(&worktree, &["rev-parse", "--show-toplevel"]),
            worktree.to_str().unwrap(),
            "git resolved outside the scratch worktree"
        );
        // Not bare, since scratch_git names a repository by its working tree.
        let remote = root.join("remote");
        std::fs::create_dir_all(&remote).unwrap();
        git(&remote, &["init", "--quiet"]);
        git(
            &main,
            &["config", "core.hooksPath", hooks.to_str().unwrap()],
        );

        git(
            &worktree,
            &["commit", "--quiet", "--allow-empty", "--message=topic"],
        );
        git(&worktree, &["push", "--quiet", "../remote", "topic"]);

        let ran = std::fs::read_to_string(&record).unwrap();
        let worktree = worktree.display();
        assert_eq!(
            ran,
            format!("{worktree} xtask pre-commit\n{worktree} xtask pre-push\n")
        );
    }
}
