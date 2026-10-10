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

use crate::error::{Error, chain};
use crate::report::{self, Note, Row};
use crate::scope::{self, Scope};
use crate::workspace::{Workspace, repo_root, workspace_root, xtask_manifest};
use crate::{
    changelog, coverage, generated, lint_layers, lint_manifests, live_check, mutants, process,
};

/// `Ok(None)` is a pass, `Ok(Some)` a pass with a note for the report.
pub type CheckResult = Result<Option<Note>, Failure>;

/// Why a step did not pass: it ran and judged against the tree, or it could
/// not run to a judgement.
#[derive(Debug, thiserror::Error)]
pub enum Failure {
    /// The step's own verdict, as its whole diagnostic: a lint's findings, a
    /// floor report, a changelog failure, a command's output.
    #[error("{0}")]
    Verdict(String),
    /// Why the step reached no verdict.
    #[error(transparent)]
    Error(#[from] Error),
}

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
        /// The prefixes of the variables the command doesn't inherit.
        without: &'static [&'static str],
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
                without: &[],
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

    /// Runs the command with every variable whose name begins with one of
    /// `prefixes` taken off its environment.
    fn without(mut self, prefixes: &'static [&'static str]) -> Self {
        if let Action::Command { without, .. } = &mut self.action {
            *without = prefixes;
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

/// The cargo arguments that run the `lablet` binary, with `args` handed to
/// it. cargo takes the place of xtask to run them: see `main`.
pub fn run_args(args: &[String]) -> Vec<String> {
    let cargo = ["run", LOCKED, "--bin", "lablet", "--"];
    cargo
        .into_iter()
        .map(str::to_owned)
        .chain(args.iter().cloned())
        .collect()
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
    steps.push(dprint_step(check));
    steps
}

/// dprint alone, which starts no compiler: a rewrite, or with `check` a
/// verification.
fn dprint_step(check: bool) -> Step {
    let dprint = Step::command(
        "fmt (dprint)",
        "dprint",
        &[if check { "check" } else { "fmt" }],
    );
    if check {
        dprint.with_hint(FMT_HINT)
    } else {
        dprint
    }
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
    Err(Failure::Verdict(message))
}

/// The prose vale reads, relative to the repository root. A directory is read
/// whole, except `product`: its `research/` notes are not held to the style.
const PROSE: [&str; 8] = [
    "README.md",
    "CLAUDE.md",
    "CHANGELOG.md",
    "contributing",
    "product",
    "lablet/README.md",
    "lablet/docs",
    "lablet/examples",
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
        // Listing the scripts failed while planning the gate. Report that when
        // the step runs, rather than listing again and passing green having
        // linted nothing.
        Err(_) => vec![Step::check("lint-shell", || {
            let error = shell_scripts(&repo_root())
                .err()
                .unwrap_or_else(|| Error::Missing {
                    what: "the tracked file list couldn't be read when the gate was planned"
                        .to_owned(),
                    remedy: "Run the gate again".to_owned(),
                });
            Err(error.into())
        })],
    }
}

/// Tracked files that are shell: a `.sh` file, or one with no extension whose
/// first line is a shell shebang, since the git hooks have none.
fn shell_scripts(root: &Path) -> Result<Vec<String>, Error> {
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
    lint_prose_steps_in(&repo_root(), all)
}

/// [`lint_prose_steps`], for the repository at `root`.
fn lint_prose_steps_in(root: &Path, all: bool) -> Vec<Step> {
    if !root.join(VALE_CONFIG).is_file() {
        return vec![Step::check("lint-prose", || {
            Err(Error::Missing {
                what: format!(
                    "{VALE_CONFIG} is missing from the repository root, so vale has no style to \
                     apply"
                ),
                remedy: "Restore it from git, which tracks it".to_owned(),
            }
            .into())
        })];
    }
    let args = vale_args(root, all);
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

/// Renders each crate's telemetry module and the telemetry reference from the
/// registry, or with `check` fails when the tree differs from that rendering;
/// see [`generated`].
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

/// What fake-provider runs emit, checked against the registry by weaver's
/// live checker over OTLP; see [`live_check`]. It builds lablet, binds ports
/// and takes some seconds of weaver, so it's a CI job of its own and not a
/// gate step.
pub fn weaver_live_check_steps() -> Vec<Step> {
    vec![Step::check("weaver live-check", live_check::check)]
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
/// the `rustdoc` lints. The binary `lablet`, `apps/lablet-cli/src/main.rs`, is
/// not covered: it has `doc = false`, since rustdoc would write it to
/// `target/doc/lablet`, where the library root `lablet` goes, a collision
/// cargo warns of.
pub fn doc_steps() -> Vec<Step> {
    both(["doc", "doc (xtask)"], "doc", WORKSPACE, &["--no-deps"])
        .map(|step| step.with_env(&[("RUSTDOCFLAGS", "-D warnings")]))
        .into()
}

/// The workspace's tests, doctests included, and xtask's own, with
/// [`process::KEPT_FROM_TESTS`] kept from them, as the coverage and mutation
/// runs keep it.
pub fn test_steps() -> Vec<Step> {
    both(["test", "test (xtask)"], "test", WORKSPACE, &[])
        .map(|step| step.without(process::KEPT_FROM_TESTS))
        .into()
}

/// The line and region floors, or with `branch` the branch floors, which are
/// measured on a nightly toolchain; see [`coverage`].
pub fn coverage_steps(branch: bool) -> Vec<Step> {
    vec![if branch {
        Step::check("coverage (branch)", coverage::check_branches)
    } else {
        Step::check("coverage", coverage::check)
    }]
}

/// The exact mutation floor, or with `changed` the same judgement of only
/// the mutants in what changed; see [`mutants`].
pub fn mutants_steps(changed: bool) -> Vec<Step> {
    vec![if changed {
        Step::check("mutants (changed)", mutants::check_changed)
    } else {
        Step::check("mutants", mutants::check)
    }]
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

/// The full local gate, cheap steps first; `ci` runs the same list. The
/// floors are not in it: `coverage` is a CI job of its own on every push, and
/// `mutants` and `coverage --branch` run daily.
pub fn pre_push_steps() -> Vec<Step> {
    let mut steps = pre_commit_steps();
    steps.extend(deny_steps());
    steps.extend(changelog_steps());
    steps.extend(doc_steps());
    steps.extend(test_steps());
    // Last, since it builds the floor crates once more for each mutant, and
    // a push that changes none of them passes it at once.
    steps.extend(mutants_steps(true));
    steps
}

/// The CI gate: a step that says which scope [`scope`] decided, then the
/// pre-push steps, or only the documentation steps when every path changed
/// since the base is documentation.
pub fn ci_steps(scope: Scope) -> Vec<Step> {
    let mut steps = vec![Step::check("scope", || Ok(Some(scope::note())))];
    steps.extend(match scope {
        Scope::Full => pre_push_steps(),
        Scope::Docs => docs_steps(),
    });
    steps
}

/// The steps that read documentation and start no compiler. The changelog
/// gate reads git alone, and runs so that a reduced gate judges nothing the
/// full one would pass.
fn docs_steps() -> Vec<Step> {
    let mut steps = vec![dprint_step(true)];
    steps.extend(lint_prose_steps(false));
    steps.extend(changelog_steps());
    steps
}

/// How a list of steps is run.
#[derive(Clone, Copy)]
pub enum Mode {
    /// Output streams, and every step's outcome gets a line.
    Command,
    /// Output is shown only for a failed step, and a report closes the run.
    Gate(&'static str),
}

/// Runs every step, in order, whatever fails. Returns whether all passed.
pub fn run(mode: Mode, steps: &[Step]) -> bool {
    let capture = matches!(mode, Mode::Gate(_));
    let lists_passes = match mode {
        Mode::Command => true,
        Mode::Gate(_) => verbose(),
    };
    // A quiet gate says it started, so a hook's log isn't blank while a long
    // step runs.
    if let Mode::Gate(name) = mode
        && !lists_passes
    {
        println!("{name}: running...");
    }
    let mut rows = Vec::new();
    for step in steps {
        if capture && on_terminal() {
            print!("[..] {}", step.label);
            let _ = std::io::stdout().flush();
        }
        let start = Instant::now();
        let result = match &step.action {
            Action::Command {
                program,
                args,
                env,
                without,
            } => {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                run_command(program, &args, env, without, capture)
            }
            Action::Check(check) => check(),
        };
        let elapsed = start.elapsed().as_secs_f64();
        if capture && on_terminal() {
            print!("\r");
            let _ = std::io::stdout().flush();
        }
        match &result {
            Ok(None) if lists_passes => println!("[ok] {} ({elapsed:.1}s)", step.label),
            Ok(Some(note)) if lists_passes || matches!(note, Note::Warning(_)) => {
                println!("[ok] {} ({elapsed:.1}s): {note}", step.label);
            }
            Ok(_) => {}
            Err(failure) => {
                let diagnostic = diagnostic(failure, step.hint);
                eprintln!("[FAIL] {} ({elapsed:.1}s)\n\n{diagnostic}\n", step.label);
            }
        }
        rows.push(Row {
            name: step.label.to_owned(),
            elapsed,
            ok: result.is_ok(),
            note: result.unwrap_or_else(|_| Some(Note::Info(rerun(step.label)))),
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
        Mode::Command => {}
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
    let flag = if ["fmt", "weaver generate"].contains(&task) {
        " --check"
    } else if label == "mutants (changed)" {
        " --changed"
    } else {
        ""
    };
    format!("re-run: cargo xtask {task}{flag}")
}

/// Streamed, a failure is one line, since the output is already on screen.
/// Captured, both streams share one pipe so their interleaving survives, and
/// the failure carries the capture.
fn run_command(
    program: &str,
    args: &[&str],
    env: &[(&str, &str)],
    without: &[&str],
    capture: bool,
) -> CheckResult {
    let invocation = process::invocation(program, args);
    let failed = || format!("error: {}", invocation.failed());
    let could_not_run = invocation.not_started();
    let mut command = process::command(program, args)?;
    command.envs(env.iter().copied());
    process::stripped(
        &mut command,
        std::env::vars_os().map(|(name, _)| name),
        without,
    );
    if !capture {
        let status = command.status().map_err(could_not_run)?;
        return if status.success() {
            Ok(None)
        } else {
            Err(Failure::Verdict(failed()))
        };
    }

    let (mut reader, writer) = std::io::pipe().map_err(&could_not_run)?;
    command
        .stdout(writer.try_clone().map_err(&could_not_run)?)
        .stderr(writer);
    if on_terminal() {
        command.env("CARGO_TERM_COLOR", "always");
        command.env("CLICOLOR_FORCE", "1");
    }
    let mut child = command.spawn().map_err(&could_not_run)?;
    // The command holds the write ends; drop it or the reader never sees EOF.
    drop(command);
    let mut output = Vec::new();
    let _ = reader.read_to_end(&mut output);
    let status = child.wait().map_err(could_not_run)?;
    if status.success() {
        Ok(None)
    } else {
        Err(Failure::Verdict(format!(
            "{}\n{}",
            String::from_utf8_lossy(&output).trim_end(),
            failed()
        )))
    }
}

/// A failed step's diagnostic. A step's hint fixes what its verdict found,
/// so it follows only a verdict: an error means the step never judged.
fn diagnostic(failure: &Failure, hint: Option<&str>) -> String {
    let told = chain(failure);
    match (failure, hint) {
        (Failure::Verdict(_), Some(hint)) => format!("{told}\n{hint}"),
        _ => told,
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
/// agent or a piped run gets one line, and a line for each step that warned.
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
impl Failure {
    /// The verdict, where one is expected.
    #[track_caller]
    pub fn into_verdict(self) -> String {
        match self {
            Self::Verdict(verdict) => verdict,
            Self::Error(error) => panic!("no verdict, but an error: {}", chain(&error)),
        }
    }

    /// The error, where one is expected.
    #[track_caller]
    pub fn into_error(self) -> Error {
        match self {
            Self::Error(error) => error,
            Self::Verdict(verdict) => panic!("no error, but a verdict: {verdict}"),
        }
    }
}

#[cfg(test)]
mod policies;

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

    /// A step's command as typed, or `None` for a check.
    fn command_line(step: &Step) -> Option<String> {
        match &step.action {
            Action::Command { program, args, .. } => Some(format!("{program} {}", args.join(" "))),
            Action::Check(_) => None,
        }
    }

    /// The prefixes a step keeps from its command's environment.
    fn kept_from(step: &Step) -> Vec<&'static str> {
        match &step.action {
            Action::Command { without, .. } => without.to_vec(),
            Action::Check(_) => Vec::new(),
        }
    }

    #[test]
    fn the_test_steps_keep_every_otel_variable_from_the_tests_and_the_other_steps_keep_none() {
        for step in test_steps() {
            assert_eq!(
                kept_from(&step),
                [
                    "OTEL_",
                    "TRACEPARENT",
                    "TRACESTATE",
                    "BAGGAGE",
                    "B3",
                    "X_B3_"
                ],
                "{}",
                step.label
            );
        }
        for step in check_steps().iter().chain(doc_steps().iter()) {
            assert!(kept_from(step).is_empty(), "{}", step.label);
        }
    }

    #[test]
    fn prose_is_linted_under_the_repository_config_and_refused_without_one() {
        let steps = lint_prose_steps(false);
        let vale = steps.last().and_then(command_line).unwrap_or_default();
        assert!(
            vale.starts_with("vale --no-global --config .vale.ini --minAlertLevel error "),
            "{vale}"
        );

        let bare = crate::workspace::fixture::TempDir::new("no-vale-config");
        let steps = lint_prose_steps_in(bare.path(), false);
        let [step] = steps.as_slice() else {
            panic!("{}", labels(&steps));
        };
        let Action::Check(check) = step.action else {
            panic!("`{}` runs vale with no config to run it under", step.label);
        };
        assert_eq!(step.label, "lint-prose");
        let error = check().unwrap_err().into_error();
        assert!(matches!(error, Error::Missing { .. }), "{error:?}");
        assert_eq!(
            chain(&error),
            ".vale.ini is missing from the repository root, so vale has no style to apply. \
             Restore it from git, which tracks it"
        );
    }

    #[test]
    fn check_type_checks_every_target_of_both_workspaces_and_setup_runs_the_script() {
        let lines: Vec<Option<String>> = check_steps().iter().map(command_line).collect();
        let manifest = xtask_manifest().display().to_string();
        assert_eq!(
            lines,
            [
                Some("cargo check --locked --workspace --all-targets".to_owned()),
                Some(format!(
                    "cargo check --locked --manifest-path {manifest} --all-targets"
                )),
            ]
        );
        let lines: Vec<Option<String>> = setup_steps().iter().map(command_line).collect();
        assert_eq!(lines, [Some("bash scripts/setup.sh".to_owned())]);
        assert!(repo_root().join("scripts/setup.sh").is_file());
    }

    /// `sh` is on every host the gates run on and pinned by nothing, so it
    /// runs from PATH.
    #[test]
    fn a_failed_step_of_a_gate_carries_what_it_printed_and_one_of_a_command_only_the_command() {
        let script = ["-c", "echo held for the report; exit 3"];
        let failed = "error: command failed (in the repository root): sh -c echo held for the \
                      report; exit 3";
        assert_eq!(
            run_command("sh", &script, &[], &[], true)
                .unwrap_err()
                .into_verdict(),
            format!("held for the report\n{failed}")
        );
        let silent = ["-c", "exit 3"];
        assert_eq!(
            run_command("sh", &silent, &[], &[], false)
                .unwrap_err()
                .into_verdict(),
            "error: command failed (in the repository root): sh -c exit 3"
        );
        assert_eq!(
            run_command("sh", &["-c", "true"], &[], &[], true).unwrap(),
            None
        );
        assert_eq!(
            run_command("sh", &["-c", "true"], &[], &[], false).unwrap(),
            None
        );

        // One that can't start reached no verdict, captured or not.
        let missing = "lablet-xtask-no-such-program";
        let why = std::io::Error::from_raw_os_error(2);
        for capture in [true, false] {
            let error = run_command(missing, &[], &[], &[], capture).unwrap_err();
            assert_eq!(
                chain(&error.into_error()),
                format!("could not run `{missing}` (in the repository root): {why}")
            );
        }
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
    fn pre_push_is_pre_commit_then_deny_changelog_doc_test_and_the_mutants_of_what_changed() {
        let pre_commit = labels(&pre_commit_steps());
        assert_eq!(
            labels(&pre_push_steps()),
            format!(
                "{pre_commit}, deny, deny (xtask), changelog, doc, doc (xtask), test, test (xtask), \
                 mutants (changed)"
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
        let verdict = || Failure::Verdict("error: command failed".to_owned());
        assert_eq!(
            diagnostic(&verdict(), Some(FMT_HINT)),
            "error: command failed\nfix with: cargo xtask fmt"
        );
        assert_eq!(diagnostic(&verdict(), None), "error: command failed");
        assert_eq!(rerun("clippy (xtask)"), "re-run: cargo xtask clippy");
    }

    /// `cargo xtask fmt` fails the same way when the formatter can't start.
    #[test]
    fn a_step_that_could_not_run_gets_no_hint_to_fix_what_it_never_judged() {
        let error = Failure::Error(Error::Missing {
            what: "dprint is not installed".to_owned(),
            remedy: "Run `cargo xtask setup`".to_owned(),
        });
        assert_eq!(
            diagnostic(&error, Some(FMT_HINT)),
            "dprint is not installed. Run `cargo xtask setup`"
        );
    }

    #[test]
    fn a_failed_scoped_mutation_step_is_re_run_scoped() {
        assert_eq!(
            rerun("mutants (changed)"),
            "re-run: cargo xtask mutants --changed"
        );
        assert_eq!(rerun("mutants"), "re-run: cargo xtask mutants");
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

        // Every other weaver command reports as `weaver check` does here.
        let here = weaver_check_args(in_ci());
        assert_eq!(weaver_diagnostic_args(), here[here.len() - 4..]);
    }

    #[test]
    fn weaver_live_check_is_one_check_outside_the_gates_re_run_by_both_words() {
        let steps = weaver_live_check_steps();
        let [step] = steps.as_slice() else {
            panic!("{}", labels(&steps));
        };
        let Action::Check(run) = step.action else {
            panic!("`{}` is not a check", step.label);
        };
        assert_eq!(step.label, "weaver live-check");
        assert!(std::ptr::fn_addr_eq(
            run,
            live_check::check as fn() -> CheckResult
        ));
        assert_eq!(task_of("weaver live-check"), "weaver live-check");
        assert_eq!(
            rerun("weaver live-check"),
            "re-run: cargo xtask weaver live-check"
        );
        for gate in [pre_commit_steps(), pre_push_steps()] {
            assert!(
                gate.iter().all(|step| step.label != "weaver live-check"),
                "{}",
                labels(&gate)
            );
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
        for step in &steps {
            let Action::Command { program, args, .. } = &step.action else {
                continue;
            };
            if *program == "cargo" {
                let locked = args.get(1).map(String::as_str) == Some("--locked");
                assert_eq!(locked, args[0] != "fmt", "{}: {args:?}", step.label);
            }
        }
        assert_eq!(run_args(&["--version".to_owned()])[1], "--locked");
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
            run_args(&["--help".to_owned()]).join(" "),
            "run --locked --bin lablet -- --help"
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
        for expected in [
            "README.md",
            "contributing",
            "product/spec.md",
            "lablet/examples",
        ] {
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
    fn a_floor_task_with_its_flag_is_the_same_task_judging_something_else() {
        let check = |steps: Vec<Step>| match steps.as_slice() {
            [step] => match step.action {
                Action::Check(check) => (step.label, check),
                Action::Command { .. } => panic!("{} is not a check", step.label),
            },
            _ => panic!("a floor task is one step"),
        };
        let same = |ran: fn() -> CheckResult, expected: fn() -> CheckResult| {
            std::ptr::fn_addr_eq(ran, expected)
        };
        let (label, ran) = check(coverage_steps(false));
        assert!(label == "coverage" && same(ran, coverage::check));
        let (label, ran) = check(coverage_steps(true));
        assert!(label == "coverage (branch)" && same(ran, coverage::check_branches));
        let (label, ran) = check(mutants_steps(false));
        assert!(label == "mutants" && same(ran, mutants::check));
        let (label, ran) = check(mutants_steps(true));
        assert!(label == "mutants (changed)" && same(ran, mutants::check_changed));
        assert_eq!(task_of("coverage (branch)"), "coverage");
        assert_eq!(task_of("mutants (changed)"), "mutants");
    }

    #[test]
    fn a_lint_passes_with_no_finding_and_lists_every_finding_otherwise() {
        assert_eq!(listed(&[]).unwrap(), None);
        assert_eq!(
            listed(&["one".to_owned(), "two".to_owned()])
                .unwrap_err()
                .into_verdict(),
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
                Err(Failure::Verdict("first failed".to_owned()))
            }),
            Step::check("second", || {
                RAN.fetch_add(1, Ordering::SeqCst);
                Ok(Some(Note::Info("a note".to_owned())))
            }),
        ];
        let passed = run(Mode::Gate("test-gate"), &steps);
        assert_eq!(RAN.load(Ordering::SeqCst), 2);
        assert!(!passed);
    }

    /// Off a terminal a green gate says it started and ends in one line, which
    /// says a step warned, and the warning itself is shown between them. The gate runs in a child of this
    /// test binary whose output goes to a pipe, as a hook's or an agent's does.
    #[test]
    fn a_green_gate_off_a_terminal_still_shows_a_warning_and_says_a_step_warned() {
        const CHILD: &str = "XTASK_TEST_GATE_OFF_A_TERMINAL";
        if std::env::var_os(CHILD).is_some() {
            let steps = [
                Step::check("warns", || Ok(Some(Note::Warning("skipped".to_owned())))),
                Step::check("notes", || Ok(Some(Note::Info("held back".to_owned())))),
                Step::check("passes", || Ok(None)),
            ];
            assert!(run(Mode::Gate("gate"), &steps));
            return;
        }
        let name = "gates::tests::a_green_gate_off_a_terminal_still_shows_a_warning_and_says_a_step_warned";
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .env_remove("CI")
            .env_remove("XTASK_VERBOSE")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stdout}{stderr}");
        // What the gate printed, less the timings and what libtest printed,
        // which puts the test's name ahead of its first line.
        let printed: Vec<String> = stdout
            .lines()
            .filter_map(|line| {
                let at = line
                    .find("gate: ")
                    .or_else(|| line.find("[ok] "))
                    .or_else(|| line.find("gate · "))?;
                Some(&line[at..])
            })
            .map(|line| {
                let (before, after) = line.split_once(" (").unwrap_or((line, ""));
                let after = after.split_once("s)").map_or(after, |(_, rest)| rest);
                format!("{before}{after}")
            })
            .collect();
        assert_eq!(printed.len(), 3, "{stdout}");
        assert_eq!(printed[0], "gate: running...", "{stdout}");
        assert_eq!(printed[1], "[ok] warns: warning: skipped", "{stdout}");
        assert!(
            printed[2].starts_with("gate · 3 steps ok · 1 warned · "),
            "{stdout}"
        );
    }

    #[test]
    fn a_run_with_every_step_passing_succeeds() {
        let steps = [Step::check("only", || Ok(None))];
        assert!(run(Mode::Command, &steps));
    }
}
