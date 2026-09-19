//! The steps, the commands and gates built from them, and the runner.
//!
//! A step is a subprocess or an in-process check. A command such as `clippy`
//! is a short list of steps with their output streamed; a gate such as
//! `pre-push` is a longer list with each step's output held back and shown
//! only if it fails. Either way every step runs, so one run reports every
//! failure, and any failure makes `cargo xtask` exit non-zero.

use std::fmt::Write as _;
use std::io::{IsTerminal, Read as _, Write as _};
use std::sync::LazyLock;
use std::time::Instant;

use crate::report::{self, Row};
use crate::workspace::{Workspace, repo_root, workspace_root, xtask_manifest};
use crate::{changelog, coverage, lint_layers, lint_manifests, mutants, process};

/// What an in-process check returns: `Ok(None)` for a plain pass, `Ok(Some)`
/// for a pass with a note worth a line in the report, `Err` with the whole
/// diagnostic for a failure.
pub type CheckResult = Result<Option<String>, String>;

/// One named unit of a command or gate.
pub struct Step {
    label: &'static str,
    action: Action,
    /// What to run when the step fails, where one command is the remedy.
    hint: Option<&'static str>,
}

enum Action {
    /// A subprocess; the step passes when it exits zero.
    Command {
        program: &'static str,
        args: Vec<String>,
        env: &'static [(&'static str, &'static str)],
    },
    /// A check that runs in this process.
    Check(fn() -> CheckResult),
}

impl Step {
    fn cargo(label: &'static str, args: &[&str]) -> Self {
        Self::cargo_with_env(label, args, &[])
    }

    /// A cargo step. Every subcommand that resolves dependencies gets
    /// `--locked` straight after its name, where cargo's own subcommands and
    /// cargo-deny both take it: a gate judges the committed lockfiles, and
    /// without the flag cargo would quietly re-resolve a stale one, pass
    /// against a graph nobody committed, and leave the tree dirty. `cargo fmt`
    /// resolves nothing and rejects the flag.
    fn cargo_with_env(
        label: &'static str,
        args: &[&str],
        env: &'static [(&'static str, &'static str)],
    ) -> Self {
        let mut args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        if args.first().is_some_and(|subcommand| subcommand != "fmt") {
            args.insert(1, LOCKED.to_owned());
        }
        Self {
            label,
            action: Action::Command {
                program: "cargo",
                args,
                env,
            },
            hint: None,
        }
    }

    const fn check(label: &'static str, check: fn() -> CheckResult) -> Self {
        Self {
            label,
            action: Action::Check(check),
            hint: None,
        }
    }

    const fn with_hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
    }
}

/// The cargo flag that refuses to touch a lockfile; see [`Step::cargo_with_env`].
pub const LOCKED: &str = "--locked";

/// xtask's manifest as a cargo argument. Cargo commands run in `lablet/`, so
/// the passes over xtask itself name its manifest.
fn xtask_manifest_arg() -> String {
    xtask_manifest().display().to_string()
}

// --- The steps of each command ---

/// rustfmt over the workspace and xtask: a check, or with `fix` a rewrite.
pub fn fmt_steps(fix: bool) -> Vec<Step> {
    let manifest = xtask_manifest_arg();
    let mut workspace = vec!["fmt", "--all"];
    let mut xtask = vec!["fmt", "--manifest-path", &manifest];
    if !fix {
        workspace.extend(["--", "--check"]);
        xtask.extend(["--", "--check"]);
    }
    let steps = [
        Step::cargo("fmt", &workspace),
        Step::cargo("fmt (xtask)", &xtask),
    ];
    if fix {
        steps.into()
    } else {
        // The command a failed check echoes only reports; this one repairs.
        steps.map(|step| step.with_hint(FMT_HINT)).into()
    }
}

/// What a failed formatting check says to run.
const FMT_HINT: &str = "fix with: cargo xtask fmt --fix";

/// clippy over every target of the workspace and of xtask, warnings denied.
/// The lint set itself lives in the manifests (`[workspace.lints]`).
pub fn clippy_steps() -> Vec<Step> {
    const DENY_WARNINGS: [&str; 4] = ["--all-targets", "--", "-D", "warnings"];
    let manifest = xtask_manifest_arg();
    let mut workspace = vec!["clippy", "--workspace"];
    workspace.extend(DENY_WARNINGS);
    let mut xtask = vec!["clippy", "--manifest-path", &manifest];
    xtask.extend(DENY_WARNINGS);
    vec![
        Step::cargo("clippy", &workspace),
        Step::cargo("clippy (xtask)", &xtask),
    ]
}

/// The layer rules; see [`lint_layers`].
pub fn lint_layers_steps() -> Vec<Step> {
    vec![Step::check("lint-layers", || {
        let workspace = Workspace::load(&workspace_root())?;
        listed(&lint_layers::lint(&workspace))
    })]
}

/// The manifest rules; see [`lint_manifests`].
pub fn lint_manifests_steps() -> Vec<Step> {
    vec![Step::check("lint-manifests", || {
        let workspace = Workspace::load(&workspace_root())?;
        listed(&lint_manifests::lint(&workspace, &repo_root()))
    })]
}

/// A lint's result: a pass, or every finding in a paragraph of its own.
fn listed(findings: &[String]) -> CheckResult {
    if findings.is_empty() {
        return Ok(None);
    }
    let mut message = String::new();
    for finding in findings {
        let _ = writeln!(message, "  {finding}\n");
    }
    let _ = write!(message, "{} violation(s) found.", findings.len());
    Err(message)
}

/// cargo-deny under `lablet/deny.toml`, over the workspace and over xtask,
/// which has its own lockfile. `cargo tree -d` recovers the suppressed graphs.
pub fn deny_steps() -> Vec<Step> {
    let config = workspace_root().join("deny.toml").display().to_string();
    let manifest = xtask_manifest_arg();
    vec![
        // cargo-deny 0.20 takes `--config` ahead of `check`.
        Step::cargo(
            "deny",
            &[
                "deny",
                "--config",
                &config,
                "check",
                "--hide-inclusion-graph",
            ],
        ),
        // The policy is written for the workspace, so entries that match
        // nothing in xtask's small graph are expected here.
        Step::cargo(
            "deny (xtask)",
            &[
                "deny",
                "--manifest-path",
                &manifest,
                "--config",
                &config,
                "check",
                "--hide-inclusion-graph",
                "-A",
                "advisory-not-detected",
                "-A",
                "license-not-encountered",
                "-A",
                "unmatched-skip",
            ],
        ),
    ]
}

/// rustdoc over the workspace and xtask, without dependencies, any warning an
/// error: a broken intra-doc link or a missing doc fails the step. This is the
/// only step that evaluates the `rustdoc` lints; clippy never runs rustdoc.
/// `cargo doc` documents a package's library and skips a binary of the same
/// name, so `apps/lablet/src/main.rs` is not covered.
pub fn doc_steps() -> Vec<Step> {
    const DENY_WARNINGS: &[(&str, &str)] = &[("RUSTDOCFLAGS", "-D warnings")];
    let manifest = xtask_manifest_arg();
    vec![
        Step::cargo_with_env("doc", &["doc", "--workspace", "--no-deps"], DENY_WARNINGS),
        Step::cargo_with_env(
            "doc (xtask)",
            &["doc", "--manifest-path", &manifest, "--no-deps"],
            DENY_WARNINGS,
        ),
    ]
}

/// The workspace's tests, doctests included, and xtask's own.
pub fn test_steps() -> Vec<Step> {
    let manifest = xtask_manifest_arg();
    vec![
        Step::cargo("test", &["test", "--workspace"]),
        Step::cargo("test (xtask)", &["test", "--manifest-path", &manifest]),
    ]
}

/// The line coverage floors; see [`coverage`].
pub fn coverage_steps() -> Vec<Step> {
    vec![Step::check("coverage", coverage::check)]
}

/// The mutation floors; see [`mutants`].
pub fn mutants_steps() -> Vec<Step> {
    vec![Step::check("mutants", mutants::check)]
}

/// The changelog gate; see [`changelog`].
pub fn changelog_steps() -> Vec<Step> {
    vec![Step::check("changelog", changelog::check)]
}

/// The fast gate: what judges a commit and finishes quickly.
///
/// Phase 1 extension point: `weaver check` and the generated-files check
/// (`weaver generate --check`) join this list, as contributing/README.md
/// already says.
pub fn pre_commit_steps() -> Vec<Step> {
    let mut steps = fmt_steps(false);
    steps.extend(clippy_steps());
    steps.extend(lint_layers_steps());
    steps.extend(lint_manifests_steps());
    steps
}

/// The full local gate: pre-commit, plus what needs the whole tree built or
/// the whole history read. `ci` runs the same list.
///
/// `coverage` and `mutants` are not here: CI runs them as jobs of their own.
/// Phase 6 extension point: `weaver live-check` is a CI job too. Phase 11
/// extension point: so is `bench`, with its regression threshold.
pub fn pre_push_steps() -> Vec<Step> {
    let mut steps = pre_commit_steps();
    steps.extend(test_steps());
    steps.extend(doc_steps());
    steps.extend(deny_steps());
    steps.extend(changelog_steps());
    steps
}

// --- The runner ---

/// How a list of steps is run.
#[derive(Clone, Copy)]
pub enum Mode {
    /// A single command: subprocess output streams as it comes.
    Command,
    /// A named gate: subprocess output is shown only for a failed step, and a
    /// report closes the run.
    Gate(&'static str),
}

/// Runs every step, in order, whatever fails, and reports. Returns whether
/// every step passed.
pub fn run(mode: Mode, steps: &[Step]) -> bool {
    let capture = matches!(mode, Mode::Gate(_));
    let mut rows = Vec::new();
    for step in steps {
        if capture && on_terminal() {
            print!("[..] {}", step.label);
            let _ = std::io::stdout().flush();
        }
        let start = Instant::now();
        let result = match &step.action {
            Action::Command { program, args, env } => {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                run_command(program, &args, env, capture)
            }
            Action::Check(check) => check(),
        };
        let result = result.map_err(|diagnostic| with_hint(diagnostic, step.hint));
        let elapsed = start.elapsed().as_secs_f64();
        if capture && on_terminal() {
            print!("\r");
            let _ = std::io::stdout().flush();
        }
        match &result {
            Ok(None) => println!("[ok] {} ({elapsed:.1}s)", step.label),
            Ok(Some(note)) => println!("[ok] {} ({elapsed:.1}s): {note}", step.label),
            Err(diagnostic) => {
                eprintln!("[FAIL] {} ({elapsed:.1}s)\n\n{diagnostic}\n", step.label);
            }
        }
        rows.push(Row {
            name: step.label.to_owned(),
            elapsed,
            ok: result.is_ok(),
            note: result.ok().flatten(),
        });
    }

    let failed = rows.iter().filter(|row| !row.ok).count();
    match mode {
        Mode::Gate(name) => {
            // A failed step ends in a blank line of its own.
            if rows.last().is_some_and(|row| row.ok) {
                println!();
            }
            print!("{}", report::render(name, &rows));
        }
        Mode::Command if failed > 0 && rows.len() > 1 => {
            eprintln!("error: {failed} of {} step(s) failed", rows.len());
        }
        Mode::Command => {}
    }
    failed == 0
}

/// Runs a subprocess step. Streamed, the failure is one line, since the
/// output is already on screen. Captured, both streams share one pipe so
/// their interleaving survives, and the failure carries the capture.
fn run_command(program: &str, args: &[&str], env: &[(&str, &str)], capture: bool) -> CheckResult {
    let failed = || format!("error: {}", process::command_failed(program, args));
    if !capture {
        let status = process::stream(program, args, env)?;
        return if status.success() {
            Ok(None)
        } else {
            Err(failed())
        };
    }

    let could_not_run = |e: std::io::Error| process::could_not_run(program, &e);
    let (mut reader, writer) = std::io::pipe().map_err(could_not_run)?;
    let mut command = process::command(program, args)?;
    command
        .envs(env.iter().copied())
        .stdout(writer.try_clone().map_err(could_not_run)?)
        .stderr(writer);
    if on_terminal() {
        command.env("CARGO_TERM_COLOR", "always");
        command.env("CLICOLOR_FORCE", "1");
    }
    let mut child = command.spawn().map_err(could_not_run)?;
    // The command holds the write ends; drop it or the reader never sees EOF.
    drop(command);
    let mut output = Vec::new();
    let _ = reader.read_to_end(&mut output);
    let status = child.wait().map_err(could_not_run)?;
    if status.success() {
        Ok(None)
    } else {
        Err(format!(
            "{}\n{}",
            String::from_utf8_lossy(&output).trim_end(),
            failed()
        ))
    }
}

/// A failure's diagnostic, closed by the step's hint when it has one.
fn with_hint(diagnostic: String, hint: Option<&str>) -> String {
    match hint {
        Some(hint) => format!("{diagnostic}\n{hint}"),
        None => diagnostic,
    }
}

/// Both streams: progress goes to stdout, a failure to stderr. Decided once;
/// the streams do not change under a run.
fn on_terminal() -> bool {
    static TERMINAL: LazyLock<bool> =
        LazyLock::new(|| std::io::stdout().is_terminal() && std::io::stderr().is_terminal());
    *TERMINAL
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(steps: &[Step]) -> Vec<&'static str> {
        steps.iter().map(|step| step.label).collect()
    }

    #[test]
    fn pre_commit_is_fmt_clippy_and_the_two_lints() {
        assert_eq!(
            labels(&pre_commit_steps()),
            [
                "fmt",
                "fmt (xtask)",
                "clippy",
                "clippy (xtask)",
                "lint-layers",
                "lint-manifests"
            ]
        );
    }

    #[test]
    fn pre_push_is_pre_commit_then_test_doc_deny_and_changelog() {
        let pre_commit = labels(&pre_commit_steps());
        let pre_push = labels(&pre_push_steps());
        assert_eq!(pre_push[..pre_commit.len()], pre_commit);
        assert_eq!(
            pre_push[pre_commit.len()..],
            [
                "test",
                "test (xtask)",
                "doc",
                "doc (xtask)",
                "deny",
                "deny (xtask)",
                "changelog"
            ]
        );
    }

    #[test]
    fn a_failed_formatting_check_says_how_to_fix_it() {
        for step in fmt_steps(false) {
            assert_eq!(
                step.hint,
                Some("fix with: cargo xtask fmt --fix"),
                "{}",
                step.label
            );
        }
        for step in fmt_steps(true) {
            assert_eq!(step.hint, None, "{}", step.label);
        }
        assert_eq!(
            with_hint("error: command failed".to_owned(), Some(FMT_HINT)),
            "error: command failed\nfix with: cargo xtask fmt --fix"
        );
        assert_eq!(with_hint("error".to_owned(), None), "error");
    }

    #[test]
    fn every_cargo_step_that_resolves_dependencies_refuses_to_touch_the_lockfile() {
        let mut steps = pre_push_steps();
        steps.extend(fmt_steps(true));
        for step in &steps {
            let Action::Command { args, .. } = &step.action else {
                continue;
            };
            let locked = args.get(1).map(String::as_str) == Some("--locked");
            // `cargo fmt` resolves nothing and rejects the flag.
            assert_eq!(locked, args[0] != "fmt", "{}: {args:?}", step.label);
        }
    }

    #[test]
    fn one_clippy_configuration_lets_tests_unwrap_in_both_workspaces() {
        let root = crate::workspace::repo_root();
        let text = std::fs::read_to_string(root.join("clippy.toml")).unwrap();
        let config: toml::Value = toml::from_str(&text).unwrap();
        for key in ["allow-unwrap-in-tests", "allow-expect-in-tests"] {
            assert_eq!(
                config.get(key).and_then(toml::Value::as_bool),
                Some(true),
                "clippy.toml: `{key}` must be true, or a test that unwraps fails clippy"
            );
        }
        // Clippy takes the first file it finds walking up from a package, so a
        // copy at a workspace root would shadow the shared one.
        for shadow in ["lablet/clippy.toml", "xtask/clippy.toml"] {
            assert!(
                !root.join(shadow).exists(),
                "{shadow} shadows the repository's clippy.toml"
            );
        }
    }

    #[test]
    fn a_lint_passes_with_no_finding_and_lists_every_finding_otherwise() {
        assert_eq!(listed(&[]), Ok(None));
        assert_eq!(
            listed(&["one".to_owned(), "two".to_owned()]).unwrap_err(),
            "  one\n\n  two\n\n2 violation(s) found."
        );
    }

    #[test]
    fn a_failed_step_does_not_stop_the_steps_after_it() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static RAN: AtomicU32 = AtomicU32::new(0);
        let steps = [
            Step::check("first", || {
                RAN.fetch_add(1, Ordering::SeqCst);
                Err("first failed".to_owned())
            }),
            Step::check("second", || {
                RAN.fetch_add(1, Ordering::SeqCst);
                Ok(Some("a note".to_owned()))
            }),
        ];
        let passed = run(Mode::Gate("test-gate"), &steps);
        assert_eq!(RAN.load(Ordering::SeqCst), 2);
        assert!(!passed);
    }

    #[test]
    fn a_run_with_every_step_passing_succeeds() {
        let steps = [Step::check("only", || Ok(None))];
        assert!(run(Mode::Command, &steps));
    }
}
