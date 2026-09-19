//! The steps, the commands and gates built from them, and the runner.
//!
//! A command such as `clippy` streams its steps' output; a gate such as
//! `pre-push` holds each step's output back and shows it only on failure.
//! Either way every step runs, so one run reports every failure.

use std::fmt::Write as _;
use std::io::{IsTerminal, Read as _, Write as _};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Instant;

use crate::report::{self, Row};
use crate::workspace::{Workspace, repo_root, workspace_root, xtask_manifest};
use crate::{changelog, coverage, generated, lint_layers, lint_manifests, mutants, process};

/// `Ok(None)` is a pass, `Ok(Some)` a pass with a note for the report, `Err`
/// the whole diagnostic of a failure.
pub type CheckResult = Result<Option<String>, String>;

/// One named unit of a command or gate.
pub struct Step {
    /// The task a gate's report says re-runs the step, then an optional
    /// ` (qualifier)`; see [`task_of`].
    pub label: &'static str,
    action: Action,
    hint: Option<&'static str>,
}

enum Action {
    Command {
        program: &'static str,
        args: Vec<String>,
        env: &'static [(&'static str, &'static str)],
    },
    Check(fn() -> CheckResult),
}

impl Step {
    fn command(label: &'static str, program: &'static str, args: &[&str]) -> Self {
        Self {
            label,
            action: Action::Command {
                program,
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                env: &[],
            },
            hint: None,
        }
    }

    /// `--locked` follows every subcommand but `fmt`, which resolves nothing
    /// and rejects it: a gate judges the committed lockfiles, and without the
    /// flag cargo would quietly re-resolve a stale one and leave the tree dirty.
    fn cargo(label: &'static str, args: &[&str]) -> Self {
        let mut step = Self::command(label, "cargo", args);
        if let Action::Command { args, .. } = &mut step.action
            && args.first().is_some_and(|subcommand| subcommand != "fmt")
        {
            args.insert(1, LOCKED.to_owned());
        }
        step
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

    fn with_env(mut self, variables: &'static [(&'static str, &'static str)]) -> Self {
        if let Action::Command { env, .. } = &mut self.action {
            *env = variables;
        }
        self
    }
}

/// The cargo flag that refuses to touch a lockfile; see [`Step::cargo`].
pub const LOCKED: &str = "--locked";

const WORKSPACE: &[&str] = &["--workspace"];
const DENY_WARNINGS: &[&str] = &["--all-targets", "--", "-D", "warnings"];
const FMT_HINT: &str = "fix with: cargo xtask fmt";
const VALE_CONFIG: &str = ".vale.ini";
/// Where `vale sync` puts the package `.vale.ini` names.
const VALE_STYLE: &str = ".vale/styles/Microsoft";
/// `--no-global` keeps a developer's own Vale config out of the result.
const VALE_SYNC: &[&str] = &["--no-global", "--config", VALE_CONFIG, "sync"];

/// One cargo subcommand over the workspace (`selector` picks its packages),
/// then over xtask, which a cargo run in `lablet/` reaches only by manifest.
fn both(label: [&'static str; 2], subcommand: &str, selector: &[&str], rest: &[&str]) -> [Step; 2] {
    let manifest = xtask_manifest().display().to_string();
    let workspace = [&[subcommand], selector, rest].concat();
    let xtask = [&[subcommand, "--manifest-path", &manifest], rest].concat();
    [
        Step::cargo(label[0], &workspace),
        Step::cargo(label[1], &xtask),
    ]
}

/// `cargo check` over every target.
pub fn check_steps() -> Vec<Step> {
    let rest = &["--all-targets"];
    both(["check", "check (xtask)"], "check", WORKSPACE, rest).into()
}

/// `cargo build` over the workspace.
pub fn build_steps(release: bool) -> Vec<Step> {
    let mut args = vec!["build", "--workspace"];
    if release {
        args.push("--release");
    }
    vec![Step::cargo("build", &args)]
}

/// The `lablet` binary, with `args` handed to it.
pub fn run_steps(args: &[String]) -> Vec<Step> {
    let mut cargo = vec!["run", "--bin", "lablet", "--"];
    cargo.extend(args.iter().map(String::as_str));
    vec![Step::cargo("run", &cargo)]
}

/// rustfmt for Rust and dprint for JSON, TOML, Markdown, and YAML: a rewrite,
/// or with `check` a verification.
pub fn fmt_steps(check: bool) -> Vec<Step> {
    let label = ["fmt", "fmt (xtask)"];
    let mut steps: Vec<Step> = if check {
        let steps = both(label, "fmt", &["--all"], &["--", "--check"]);
        steps.map(|step| step.with_hint(FMT_HINT)).into()
    } else {
        both(label, "fmt", &["--all"], &[]).into()
    };
    let dprint = Step::command(
        "fmt (dprint)",
        "dprint",
        &[if check { "check" } else { "fmt" }],
    );
    steps.push(if check {
        dprint.with_hint(FMT_HINT)
    } else {
        dprint
    });
    steps
}

/// Clippy's machine-applicable fixes, then rustfmt. A tree being fixed is
/// dirty by definition, hence the two `--allow` flags.
pub fn fix_steps() -> Vec<Step> {
    const FIX: &[&str] = &["--fix", "--allow-dirty", "--allow-staged", "--all-targets"];
    let mut steps = Vec::from(both(["fix", "fix (xtask)"], "clippy", WORKSPACE, FIX));
    steps.extend(fmt_steps(false));
    steps
}

/// Clippy over every target, warnings denied.
pub fn clippy_steps() -> Vec<Step> {
    let label = ["clippy", "clippy (xtask)"];
    both(label, "clippy", WORKSPACE, DENY_WARNINGS).into()
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

/// The prose vale reads, relative to the repository root. A directory is read
/// whole, except `product`: its `research/` notes are not held to the style.
const PROSE: [&str; 7] = [
    "README.md",
    "CLAUDE.md",
    "CHANGELOG.md",
    "contributing",
    "product",
    "lablet/README.md",
    "lablet/docs",
];

/// A frozen record of what was run, kept out of the linters.
const RESEARCH: &str = "product/research/";

/// Upstream's files, which stay as upstream wrote them.
const VENDORED: &str = "lablet/telemetry/deps/";

/// shellcheck over every tracked shell script.
pub fn lint_shell_steps() -> Vec<Step> {
    match shell_scripts(&repo_root()) {
        Ok(scripts) => {
            let mut args = vec!["--external-sources"];
            args.extend(scripts.iter().map(String::as_str));
            vec![Step::command("lint-shell", "shellcheck", &args)]
        }
        Err(_) => vec![Step::check("lint-shell", || {
            shell_scripts(&repo_root()).map(|_| None)
        })],
    }
}

/// Tracked files that are shell: a `.sh` file, or one with no extension whose
/// first line is a shell shebang, since the git hooks have none.
fn shell_scripts(root: &Path) -> Result<Vec<String>, String> {
    let listed = process::capture_in(root, "git", &["ls-files", "-z"])?;
    Ok(listed
        .split('\0')
        .filter(|path| !path.is_empty())
        .filter(|path| !path.starts_with(RESEARCH) && !path.starts_with(VENDORED))
        .filter(|path| match Path::new(path).extension() {
            Some(extension) => extension.eq_ignore_ascii_case("sh"),
            None => std::fs::read_to_string(root.join(path))
                .is_ok_and(|text| text.lines().next().is_some_and(is_shell_shebang)),
        })
        .map(str::to_owned)
        .collect())
}

fn is_shell_shebang(line: &str) -> bool {
    line.strip_prefix("#!").is_some_and(|interpreter| {
        ["sh", "bash", "dash", "ksh"]
            .iter()
            .any(|shell| interpreter.split(['/', ' ']).any(|word| word == *shell))
    })
}

/// Vale over the project's prose: errors only, or with `all` every alert.
pub fn lint_prose_steps(all: bool) -> Vec<Step> {
    let root = repo_root();
    if !root.join(VALE_CONFIG).is_file() {
        return vec![Step::check("lint-prose", || {
            Err(format!(
                "{VALE_CONFIG} is missing from the repository root, so vale has no style to apply"
            ))
        })];
    }
    let args = vale_args(&root, all);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    prose_steps(root.join(VALE_STYLE).is_dir(), &args)
}

/// The styles are downloaded, not committed, so a fresh clone syncs first.
fn prose_steps(styles_present: bool, args: &[&str]) -> Vec<Step> {
    let mut steps = Vec::new();
    if !styles_present {
        steps.push(Step::command("lint-prose (sync)", "vale", VALE_SYNC));
    }
    steps.push(Step::command("lint-prose", "vale", args));
    steps
}

fn vale_args(root: &Path, all: bool) -> Vec<String> {
    let mut args = vec![
        "--no-global".to_owned(),
        "--config".to_owned(),
        VALE_CONFIG.to_owned(),
    ];
    if !all {
        args.extend(["--minAlertLevel".to_owned(), "error".to_owned()]);
    }
    for path in PROSE {
        if path == "product" {
            let mut files: Vec<String> = std::fs::read_dir(root.join(path))
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "md"))
                .map(|name| format!("{path}/{name}"))
                .collect();
            files.sort();
            args.extend(files);
        } else if root.join(path).exists() {
            args.push(path.to_owned());
        }
    }
    args
}

/// Paths are relative to the repository root, where weaver runs: it resolves
/// the registry manifest's dependency paths against its working directory.
/// `--v2` because the policies read the v2 form of the resolved registry.
/// `--quiet` still prints every diagnostic.
const WEAVER_CHECK: [&str; 12] = [
    "registry",
    "check",
    "--v2",
    "--quiet",
    "--registry",
    "lablet/telemetry/registry",
    "--policy",
    "lablet/telemetry/policies",
    "--policy",
    "lablet/telemetry/deps/weaver-packages/policies/check/naming_conventions",
    "--policy",
    "lablet/telemetry/deps/weaver-packages/policies/check/stability",
];

const WEAVER_DIAGNOSTICS: &str = "lablet/telemetry/templates/diagnostics";

const WEAVER_VENDOR: &str = "lablet/telemetry/vendor.sh";

/// The telemetry registry against the lablet policies and the vendored naming
/// and stability policies. It reads only the working tree, never the network.
pub fn weaver_check_steps() -> Vec<Step> {
    let args = weaver_check_args(in_ci());
    vec![Step::command("weaver check", "weaver", &args)]
}

fn weaver_check_args(github: bool) -> Vec<&'static str> {
    let mut args = WEAVER_CHECK.to_vec();
    args.extend(diagnostic_args(github));
    args
}

/// The diagnostic formats are lablet's own templates; `github` renders
/// workflow commands, which Actions turns into annotations.
const fn diagnostic_args(github: bool) -> [&'static str; 4] {
    let format = if github { "github" } else { "text" };
    [
        "--diagnostic-template",
        WEAVER_DIAGNOSTICS,
        "--diagnostic-format",
        format,
    ]
}

/// How every weaver command here reports: as text, or for GitHub in CI.
pub fn weaver_diagnostic_args() -> [&'static str; 4] {
    diagnostic_args(in_ci())
}

/// Renders the `lablet-telemetry-registry` sources and the telemetry reference
/// from the registry, or with `check` fails when the tree differs from that
/// rendering; see [`generated`].
pub fn weaver_generate_steps(check: bool) -> Vec<Step> {
    let run = if check {
        generated::check
    } else {
        generated::write
    };
    vec![Step::check("weaver generate", run)]
}

/// Replaces the vendored upstream trees from the pins in the vendor script,
/// or with `check` fetches them again and fails on any difference. Both need
/// the network, so neither is part of a gate.
pub fn weaver_vendor_steps(check: bool) -> Vec<Step> {
    let mut args = vec![WEAVER_VENDOR];
    if check {
        args.push("--check");
    }
    vec![Step::command("weaver vendor", "bash", &args)]
}

/// cargo-deny under `lablet/deny.toml`, over the workspace and over xtask,
/// which has its own lockfile. `cargo tree -d` recovers the suppressed graphs.
pub fn deny_steps() -> Vec<Step> {
    // The policy is written for the workspace, so entries that match nothing
    // in xtask's small graph are expected there.
    const UNMATCHED: &str = "-A advisory-not-detected -A license-not-encountered -A unmatched-skip";
    let config = workspace_root().join("deny.toml").display().to_string();
    let manifest = xtask_manifest().display().to_string();
    // cargo-deny takes `--config` ahead of `check`, not after it.
    let check = ["--config", &config, "check", "--hide-inclusion-graph"];
    let workspace = [&["deny"], &check[..]].concat();
    let mut xtask = [&["deny", "--manifest-path", &manifest], &check[..]].concat();
    xtask.extend(UNMATCHED.split(' '));
    vec![
        Step::cargo("deny", &workspace),
        Step::cargo("deny (xtask)", &xtask),
    ]
}

/// rustdoc without dependencies, warnings denied: the only step that evaluates
/// the `rustdoc` lints. `cargo doc` skips a binary named like its package's
/// library, so `apps/lablet/src/main.rs` is not covered.
pub fn doc_steps() -> Vec<Step> {
    both(["doc", "doc (xtask)"], "doc", WORKSPACE, &["--no-deps"])
        .map(|step| step.with_env(&[("RUSTDOCFLAGS", "-D warnings")]))
        .into()
}

/// The workspace's tests, doctests included, and xtask's own.
pub fn test_steps() -> Vec<Step> {
    both(["test", "test (xtask)"], "test", WORKSPACE, &[]).into()
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

/// scripts/setup.sh: the pinned toolchain, the pinned tools, the git hooks.
pub fn setup_steps() -> Vec<Step> {
    vec![Step::command("setup", "bash", &["scripts/setup.sh"])]
}

/// `cargo clean` in both workspaces, which takes the coverage and mutation
/// reports under `lablet/target` too. xtask goes last: this binary runs from
/// its target directory.
pub fn clean_steps() -> Vec<Step> {
    both(["clean", "clean (xtask)"], "clean", &[], &[]).into()
}

/// The fast gate: what judges a commit without building the whole tree.
pub fn pre_commit_steps() -> Vec<Step> {
    let mut steps = fmt_steps(true);
    steps.extend(clippy_steps());
    steps.extend(lint_layers_steps());
    steps.extend(lint_manifests_steps());
    steps.extend(weaver_check_steps());
    steps.extend(weaver_generate_steps(true));
    steps.extend(lint_shell_steps());
    steps.extend(lint_prose_steps(false));
    steps
}

/// The full local gate, cheap steps first; `ci` runs the same list.
/// `coverage` and `mutants` are CI jobs of their own.
pub fn pre_push_steps() -> Vec<Step> {
    let mut steps = pre_commit_steps();
    steps.extend(deny_steps());
    steps.extend(changelog_steps());
    steps.extend(doc_steps());
    steps.extend(test_steps());
    steps
}

/// How a list of steps is run.
#[derive(Clone, Copy)]
pub enum Mode {
    /// Output streams, and every step's outcome gets a line.
    Command,
    /// Output streams and a pass adds nothing: the program's output is the point.
    Passthrough,
    /// Output is shown only for a failed step, and a report closes the run.
    Gate(&'static str),
}

/// Runs every step, in order, whatever fails. Returns whether all passed.
pub fn run(mode: Mode, steps: &[Step]) -> bool {
    let capture = matches!(mode, Mode::Gate(_));
    let lists_passes = match mode {
        Mode::Command => true,
        Mode::Passthrough => false,
        Mode::Gate(_) => verbose(),
    };
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
        let elapsed = start.elapsed().as_secs_f64();
        if capture && on_terminal() {
            print!("\r");
            let _ = std::io::stdout().flush();
        }
        match &result {
            Ok(_) if !lists_passes => {}
            Ok(None) => println!("[ok] {} ({elapsed:.1}s)", step.label),
            Ok(Some(note)) => println!("[ok] {} ({elapsed:.1}s): {note}", step.label),
            Err(diagnostic) => {
                let diagnostic = with_hint(diagnostic, step.hint);
                eprintln!("[FAIL] {} ({elapsed:.1}s)\n\n{diagnostic}\n", step.label);
            }
        }
        rows.push(Row {
            name: step.label.to_owned(),
            elapsed,
            ok: result.is_ok(),
            note: result.unwrap_or_else(|_| Some(rerun(step.label))),
        });
    }

    let failed = rows.iter().filter(|row| !row.ok).count();
    match mode {
        Mode::Gate(name) => {
            // A failed step ends in a blank line of its own.
            if lists_passes && rows.last().is_some_and(|row| row.ok) {
                println!();
            }
            print!("{}", report::render(name, &rows));
        }
        Mode::Command if failed > 0 && rows.len() > 1 => {
            eprintln!("error: {failed} of {} step(s) failed", rows.len());
        }
        Mode::Command | Mode::Passthrough => {}
    }
    failed == 0
}

/// The task that runs a step alone: its label up to any ` (qualifier)`, so
/// `clippy (xtask)` is `clippy` and `weaver check` is itself.
pub fn task_of(label: &str) -> &str {
    label.split_once(" (").map_or(label, |(task, _)| task)
}

/// The command that runs a failed gate step alone, as a gate runs it.
fn rerun(label: &str) -> String {
    let task = task_of(label);
    let verifies = ["fmt", "weaver generate"].contains(&task);
    let check = if verifies { " --check" } else { "" };
    format!("re-run: cargo xtask {task}{check}")
}

/// Streamed, a failure is one line, since the output is already on screen.
/// Captured, both streams share one pipe so their interleaving survives, and
/// the failure carries the capture.
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

fn with_hint(diagnostic: &str, hint: Option<&str>) -> String {
    match hint {
        Some(hint) => format!("{diagnostic}\n{hint}"),
        None => diagnostic.to_owned(),
    }
}

/// Both streams: progress goes to stdout, a failure to stderr. Decided once;
/// the streams do not change under a run.
fn on_terminal() -> bool {
    static TERMINAL: LazyLock<bool> =
        LazyLock::new(|| std::io::stdout().is_terminal() && std::io::stderr().is_terminal());
    *TERMINAL
}

/// Whether a green gate lists its steps: on a terminal, under `CI=true` where
/// the log is the only record, or under `XTASK_VERBOSE=1`. A hook under an
/// agent or a piped run gets one line.
fn verbose() -> bool {
    static VERBOSE: LazyLock<bool> = LazyLock::new(|| {
        std::io::stdout().is_terminal()
            || in_ci()
            || std::env::var("XTASK_VERBOSE").is_ok_and(|v| v == "1")
    });
    *VERBOSE
}

fn in_ci() -> bool {
    std::env::var("CI").is_ok_and(|v| v == "true")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(steps: &[Step]) -> String {
        let labels: Vec<&str> = steps.iter().map(|step| step.label).collect();
        labels.join(", ")
    }

    #[test]
    fn pre_commit_is_fmt_clippy_the_lints_and_the_registry_checks() {
        let steps: Vec<Step> = pre_commit_steps()
            .into_iter()
            .filter(|step| step.label != "lint-prose (sync)")
            .collect();
        assert_eq!(
            labels(&steps),
            "fmt, fmt (xtask), fmt (dprint), clippy, clippy (xtask), lint-layers, lint-manifests, weaver check, weaver generate, lint-shell, lint-prose"
        );
    }

    #[test]
    fn the_shell_scripts_are_the_sh_files_and_the_extensionless_hooks() {
        let scripts = shell_scripts(&repo_root()).unwrap();
        for expected in [
            "scripts/setup.sh",
            "scripts/hooks/pre-commit",
            "scripts/hooks/pre-push",
            ".claude/hooks/session-start.sh",
        ] {
            assert!(scripts.iter().any(|path| path == expected), "{expected}");
        }
        assert!(scripts.iter().all(|path| !path.starts_with(RESEARCH)));
        assert!(scripts.iter().all(|path| !path.starts_with(VENDORED)));
        assert!(is_shell_shebang("#!/usr/bin/env bash"));
        assert!(is_shell_shebang("#!/bin/sh"));
        assert!(!is_shell_shebang("#!/usr/bin/env python3"));
        assert!(!is_shell_shebang("# not a shebang"));
    }

    #[test]
    fn prose_styles_are_synced_first_only_when_they_are_missing() {
        assert_eq!(labels(&prose_steps(true, &[])), "lint-prose");
        assert_eq!(
            labels(&prose_steps(false, &[])),
            "lint-prose (sync), lint-prose"
        );
    }

    #[test]
    fn pre_push_is_pre_commit_then_deny_changelog_doc_and_test() {
        let pre_commit = labels(&pre_commit_steps());
        assert_eq!(
            labels(&pre_push_steps()),
            format!(
                "{pre_commit}, deny, deny (xtask), changelog, doc, doc (xtask), test, test (xtask)"
            )
        );
    }

    #[test]
    fn a_failed_formatting_check_says_how_to_fix_it_and_how_to_run_it_again() {
        for step in fmt_steps(true) {
            assert_eq!(
                step.hint,
                Some("fix with: cargo xtask fmt"),
                "{}",
                step.label
            );
            assert_eq!(rerun(step.label), "re-run: cargo xtask fmt --check");
        }
        for step in fmt_steps(false) {
            assert_eq!(step.hint, None, "{}", step.label);
        }
        assert_eq!(
            with_hint("error: command failed", Some(FMT_HINT)),
            "error: command failed\nfix with: cargo xtask fmt"
        );
        assert_eq!(with_hint("error", None), "error");
        assert_eq!(rerun("clippy (xtask)"), "re-run: cargo xtask clippy");
    }

    #[test]
    fn a_failed_step_of_a_two_word_task_is_re_run_by_both_words() {
        assert_eq!(task_of("weaver check"), "weaver check");
        assert_eq!(task_of("lint-prose (sync)"), "lint-prose");
        assert_eq!(rerun("weaver check"), "re-run: cargo xtask weaver check");
    }

    #[test]
    fn a_gate_compares_the_generated_files_and_never_writes_them() {
        let steps = pre_commit_steps();
        let generate = steps.iter().find(|step| step.label == "weaver generate");
        let Some(Action::Check(run)) = generate.map(|step| &step.action) else {
            panic!("pre-commit has no `weaver generate` check");
        };
        assert!(std::ptr::fn_addr_eq(
            *run,
            generated::check as fn() -> CheckResult
        ));
        assert_eq!(
            rerun("weaver generate"),
            "re-run: cargo xtask weaver generate --check"
        );
    }

    #[test]
    fn weaver_check_reads_the_working_tree_and_renders_for_github_only_in_ci() {
        let local = weaver_check_args(false);
        assert_eq!(local[..4], ["registry", "check", "--v2", "--quiet"]);
        assert_eq!(local[local.len() - 2..], ["--diagnostic-format", "text"]);
        let ci = weaver_check_args(true);
        assert_eq!(ci[ci.len() - 2..], ["--diagnostic-format", "github"]);
        assert_eq!(local[..local.len() - 1], ci[..ci.len() - 1]);

        // A git URL in place of any of these would be cloned on every run.
        let root = repo_root();
        let paths: Vec<&str> = local
            .iter()
            .copied()
            .filter(|arg| !arg.starts_with("--") && arg.contains('/'))
            .collect();
        assert_eq!(paths.len(), 5, "{paths:?}");
        for path in &paths {
            assert!(root.join(path).is_dir(), "{path}");
        }
        for format in ["text", "github"] {
            let template = format!("{WEAVER_DIAGNOSTICS}/{format}/weaver.yaml");
            assert!(root.join(&template).is_file(), "{template}");
        }
    }

    #[test]
    fn weaver_vendor_runs_the_script_that_holds_the_pins() {
        let args = |check: bool| match &weaver_vendor_steps(check)[0].action {
            Action::Command { program, args, .. } => format!("{program} {}", args.join(" ")),
            Action::Check(_) => String::new(),
        };
        assert_eq!(args(false), "bash lablet/telemetry/vendor.sh");
        assert_eq!(args(true), "bash lablet/telemetry/vendor.sh --check");
        assert!(repo_root().join(WEAVER_VENDOR).is_file());
    }

    #[test]
    fn every_cargo_step_that_resolves_dependencies_refuses_to_touch_the_lockfile() {
        let mut steps = pre_push_steps();
        for more in [check_steps(), build_steps(true), fix_steps(), clean_steps()] {
            steps.extend(more);
        }
        steps.extend(run_steps(&["--version".to_owned()]));
        for step in &steps {
            let Action::Command { program, args, .. } = &step.action else {
                continue;
            };
            if *program == "cargo" {
                let locked = args.get(1).map(String::as_str) == Some("--locked");
                assert_eq!(locked, args[0] != "fmt", "{}: {args:?}", step.label);
            }
        }
    }

    #[test]
    fn the_thin_tasks_run_the_cargo_commands_a_developer_expects() {
        let args = |steps: &[Step]| -> Vec<String> {
            steps
                .iter()
                .map(|step| match &step.action {
                    Action::Command { args, .. } => args.join(" "),
                    Action::Check(_) => String::new(),
                })
                .collect()
        };
        let build = args(&build_steps(true));
        assert_eq!(build, ["build --locked --workspace --release"]);
        let deny = args(&deny_steps());
        assert!(deny[0].ends_with("deny.toml check --hide-inclusion-graph"));
        assert!(deny[1].ends_with("--hide-inclusion-graph -A advisory-not-detected -A license-not-encountered -A unmatched-skip"));
        assert_eq!(
            args(&run_steps(&["--help".to_owned()])),
            ["run --locked --bin lablet -- --help"]
        );
        assert_eq!(args(&fmt_steps(false))[0], "fmt --all");
        assert_eq!(args(&fmt_steps(true))[0], "fmt --all -- --check");
        let fix = args(&fix_steps());
        assert_eq!(
            fix[0],
            "clippy --locked --workspace --fix --allow-dirty --allow-staged --all-targets"
        );
        assert_eq!(fix[2], "fmt --all");
        let clean = args(&clean_steps());
        assert_eq!(clean[0], "clean --locked");
        assert!(clean[1].starts_with("clean --locked --manifest-path "));
    }

    #[test]
    fn lint_prose_reads_the_project_prose_but_not_the_research_notes() {
        let root = repo_root();
        let errors = vale_args(&root, false);
        assert_eq!(
            errors[..5],
            [
                "--no-global",
                "--config",
                ".vale.ini",
                "--minAlertLevel",
                "error"
            ]
        );
        let all = vale_args(&root, true);
        assert_eq!(all[..3], errors[..3]);
        assert_eq!(all[3..], errors[5..]);
        let paths = &all[3..];
        for expected in ["README.md", "contributing", "product/spec.md"] {
            assert!(paths.iter().any(|path| path == expected), "{expected}");
        }
        for path in paths {
            assert!(root.join(path).exists(), "{path}");
            assert!(path != "product" && !path.contains("research"), "{path}");
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
