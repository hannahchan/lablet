//! Subprocess plumbing: where a command runs, and which copy of a pinned tool
//! it finds. Nothing here exits the process, because a gate keeps going after
//! a failure.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use std::sync::OnceLock;

use crate::error::{Error, NotStarted};
use crate::workspace::{repo_root, workspace_root};

/// A tool mise.toml pins.
pub struct Tool {
    /// The key in mise.toml's `[tools]` table.
    pub mise_name: &'static str,
    /// The executable, as cargo or the shell looks it up.
    pub bin: &'static str,
}

/// The pinned tools xtask shells out to.
pub const TOOLS: &[Tool] = &[
    Tool {
        mise_name: "cargo-deny",
        bin: "cargo-deny",
    },
    Tool {
        mise_name: "github:taiki-e/cargo-llvm-cov",
        bin: "cargo-llvm-cov",
    },
    Tool {
        mise_name: "cargo:cargo-mutants",
        bin: "cargo-mutants",
    },
    Tool {
        mise_name: "github:open-telemetry/weaver",
        bin: "weaver",
    },
    Tool {
        mise_name: "dprint",
        bin: "dprint",
    },
    Tool {
        mise_name: "shellcheck",
        bin: "shellcheck",
    },
    Tool {
        mise_name: "vale",
        bin: "vale",
    },
];

/// A command as a person types it, and the directory it runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    directory: PathBuf,
    program: String,
    args: Vec<String>,
}

impl Invocation {
    /// `program` with `args`, run in `directory`.
    pub fn new(directory: &Path, program: &str, args: &[&str]) -> Self {
        Self {
            directory: directory.to_path_buf(),
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    /// Where it runs, as a person finds it: the repository root, a
    /// directory below it, or anywhere else by its whole path.
    pub fn place(&self) -> String {
        let root = repo_root();
        match self.directory.strip_prefix(&root) {
            Ok(below) if below.as_os_str().is_empty() => "the repository root".to_owned(),
            Ok(below) => format!("{}/", below.display()),
            Err(_) => self.directory.display().to_string(),
        }
    }

    /// The line for a run that exited unsuccessfully. It names the
    /// directory: a cargo command as printed does not work from the
    /// repository root, where `cargo xtask` is started.
    pub fn failed(&self) -> String {
        format!("command failed (in {}): {self}", self.place())
    }

    /// For `map_err`: the system would not start it.
    pub fn not_started(&self) -> impl Fn(std::io::Error) -> Error {
        |source| Error::Start {
            command: self.clone(),
            source: NotStarted::Io(source),
        }
    }
}

impl fmt::Display for Invocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.program)?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

/// `program` with `args`, where [`command`] runs it.
pub fn invocation(program: &str, args: &[&str]) -> Invocation {
    Invocation::new(&working_directory(program), program, args)
}

/// The line for a subprocess [`command`] ran that exited unsuccessfully.
pub fn command_failed(program: &str, args: &[&str]) -> String {
    invocation(program, args).failed()
}

/// Cargo runs in the Rust workspace, everything else in the repository root.
fn working_directory(program: &str) -> PathBuf {
    if program == "cargo" {
        workspace_root()
    } else {
        repo_root()
    }
}

/// A subprocess with the pinned tools first on PATH. A pinned tool that mise
/// cannot provide is an error, not a run of whatever copy PATH holds.
pub fn command(program: &str, args: &[&str]) -> Result<Command, Error> {
    command_in(&working_directory(program), program, args)
}

/// [`command`], in a directory the caller names.
pub fn command_in(directory: &Path, program: &str, args: &[&str]) -> Result<Command, Error> {
    command_running(directory, program, args, runs(program, args).as_deref())
}

/// The binary `program` ends up running, which is `program` itself for all
/// but cargo.
fn runs(program: &str, args: &[&str]) -> Option<String> {
    if program == "cargo" {
        cargo_plugin(args)
    } else {
        Some(program.to_owned())
    }
}

/// `cargo deny ...` runs the binary `cargo-deny`.
fn cargo_plugin(args: &[&str]) -> Option<String> {
    args.first().map(|subcommand| format!("cargo-{subcommand}"))
}

/// [`command_in`], for a program that ends up running `bin`.
fn command_running(
    directory: &Path,
    program: &str,
    args: &[&str],
    bin: Option<&str>,
) -> Result<Command, Error> {
    let not_started = |source| Error::Start {
        command: Invocation::new(directory, program, args),
        source,
    };
    if !directory.is_dir() {
        return Err(not_started(NotStarted::NoDirectory));
    }
    let tools = tools();
    if let Some(refused) = refusal(bin, tools) {
        return Err(not_started(refused));
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(directory)
        .env("PATH", &tools.path);
    for variable in GIT_REPOSITORY_ENV {
        command.env_remove(variable);
    }
    Ok(command)
}

/// What `git rev-parse --local-env-vars` lists. Git sets these for a hook, and
/// a subprocess that inherits them acts on that repository rather than the one
/// in its working directory, so a test's scratch repository would commit here.
pub const GIT_REPOSITORY_ENV: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// Runs a subprocess with inherited output and returns its status.
pub fn stream(program: &str, args: &[&str], env: &[(&str, &str)]) -> Result<ExitStatus, Error> {
    let mut command = command(program, args)?;
    command.envs(env.iter().copied());
    command
        .status()
        .map_err(invocation(program, args).not_started())
}

/// `cargo <args>` on `toolchain`, as rustup is asked for it. By `rustup run`
/// and not `cargo +toolchain`: only rustup's proxy takes `+toolchain`, and
/// whether the `cargo` first on PATH is the proxy or a toolchain's own binary
/// depends on how the machine was set up.
fn on_toolchain<'a>(toolchain: &'a str, args: &[&'a str]) -> Vec<&'a str> {
    [&["run", toolchain, "cargo"], args].concat()
}

/// [`stream`], for cargo on a toolchain other than the one
/// `rust-toolchain.toml` pins. The caller checks that the toolchain is
/// installed first, so that a missing one fails with the command that
/// installs it.
pub fn stream_on(
    toolchain: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> Result<ExitStatus, Error> {
    let rustup = on_toolchain(toolchain, args);
    let plugin = cargo_plugin(args);
    let mut command = command_running(&workspace_root(), "rustup", &rustup, plugin.as_deref())?;
    command.envs(env.iter().copied());
    command
        .status()
        .map_err(Invocation::new(&workspace_root(), "rustup", &rustup).not_started())
}

/// [`command_failed`], for [`stream_on`].
pub fn command_failed_on(toolchain: &str, args: &[&str]) -> String {
    let rustup = on_toolchain(toolchain, args);
    Invocation::new(&workspace_root(), "rustup", &rustup).failed()
}

/// A subprocess's stdout. The error names the command and where it ran,
/// with what it wrote on stderr when it ran and failed.
pub fn capture_in(directory: &Path, program: &str, args: &[&str]) -> Result<String, Error> {
    let (invocation, output) = output_in(directory, program, args)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(failed(invocation, &output))
    }
}

/// Git's answer to a question about the repository at `directory`: its
/// stdout, or `None` when it exits with status 1, which is how
/// `rev-parse --verify --quiet` says a name is no commit and `merge-base`
/// says two commits share no history. Any other failure is git failing to
/// answer, as outside a repository, and an error.
fn git_answer(directory: &Path, args: &[&str]) -> Result<Option<String>, Error> {
    let (invocation, output) = output_in(directory, "git", args)?;
    if output.status.success() {
        Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
    } else if output.status.code() == Some(1) {
        Ok(None)
    } else {
        Err(failed(invocation, &output))
    }
}

/// The commit `revision` names in the repository at `directory`, or `None`
/// when it names none.
pub fn git_commit(directory: &Path, revision: &str) -> Result<Option<String>, Error> {
    let commit = format!("{revision}^{{commit}}");
    let answer = git_answer(directory, &["rev-parse", "--verify", "--quiet", &commit])?;
    Ok(answer.map(|out| out.trim().to_owned()))
}

/// The text of `path` at `revision` in the repository at `directory`, or
/// `None` when `revision` has no such path. `git show` exits alike for an
/// absent path and for an object it could not read, so the path is looked up
/// first and its object read apart.
pub fn git_file_at(directory: &Path, revision: &str, path: &str) -> Result<Option<String>, Error> {
    let name = format!("{revision}:{path}");
    let Some(blob) = git_answer(directory, &["rev-parse", "--verify", "--quiet", &name])? else {
        return Ok(None);
    };
    capture_in(directory, "git", &["cat-file", "blob", blob.trim()]).map(Some)
}

/// Replaces this process with `program`, run as [`command`] runs it, so its
/// exit status and the signals sent to this process are its own. Returns
/// only when it could not be started.
pub fn exec(program: &str, args: &[&str]) -> Error {
    use std::os::unix::process::CommandExt as _;
    match command(program, args) {
        Ok(mut command) => invocation(program, args).not_started()(command.exec()),
        Err(error) => error,
    }
}

/// The merge-base of HEAD and `branch` in the repository at `directory`, or
/// `None` when there is none: the clone has no `branch`, or not enough of
/// its history to reach one, as a shallow clone may not.
pub fn git_merge_base(directory: &Path, branch: &str) -> Result<Option<String>, Error> {
    // `merge-base` refuses a branch the clone lacks with the status it
    // exits with outside a repository, so the branch is asked about first.
    if git_commit(directory, branch)?.is_none() {
        return Ok(None);
    }
    let answer = git_answer(directory, &["merge-base", "HEAD", branch])?;
    Ok(answer.map(|out| out.trim().to_owned()))
}

/// Runs `program` to its end in `directory`, capturing both streams.
fn output_in(
    directory: &Path,
    program: &str,
    args: &[&str],
) -> Result<(Invocation, Output), Error> {
    let invocation = Invocation::new(directory, program, args);
    let output = command_in(directory, program, args)?
        .output()
        .map_err(invocation.not_started())?;
    Ok((invocation, output))
}

fn failed(command: Invocation, output: &Output) -> Error {
    Error::Failed {
        command,
        stderr: String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_owned(),
    }
}

/// Why a command that ends up running `bin` may not start, or `None` when it
/// may: a pinned tool runs only from a directory mise named for it, never as
/// whatever copy PATH holds.
fn refusal(bin: Option<&str>, tools: &Tools) -> Option<NotStarted> {
    let bin = bin.filter(|bin| TOOLS.iter().any(|tool| tool.bin == *bin))?;
    if tools.provides(bin) {
        return None;
    }
    let why = tools
        .failure
        .as_deref()
        .unwrap_or("it is not installed; run scripts/setup.sh (or `mise install`)");
    Some(NotStarted::NotProvided {
        bin: bin.to_owned(),
        why: why.to_owned(),
    })
}

/// `failure` is why `directories` may be empty: mise could not be run, or
/// refused.
struct Tools {
    directories: Vec<PathBuf>,
    path: OsString,
    failure: Option<String>,
}

impl Tools {
    fn provides(&self, bin: &str) -> bool {
        self.directories
            .iter()
            .any(|dir| is_executable(&dir.join(bin)))
    }

    /// The tools as `mise bin-paths` listed them, `mise` being the program
    /// that was run. A failure is kept for the first pinned tool that needs
    /// it; a command that needs none still runs. `cargo_bin` goes last if
    /// `inherited` lacks it: cargo would otherwise search it ahead of PATH
    /// and shadow the pinned cargo plugins.
    fn from_mise(
        listed: std::io::Result<Output>,
        mise: &str,
        inherited: Option<&OsStr>,
        cargo_bin: Option<PathBuf>,
    ) -> Self {
        let mut directories = Vec::new();
        let mut failure = None;
        match listed {
            Ok(output) if output.status.success() => directories.extend(
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(PathBuf::from),
            ),
            Ok(output) => {
                failure = Some(format!(
                    "`mise bin-paths` failed; run scripts/setup.sh, which trusts mise.toml and \
                     installs the pins:\n{}",
                    String::from_utf8_lossy(&output.stderr).trim_end()
                ));
            }
            Err(e) => {
                failure = Some(format!(
                    "`{mise}` could not be run ({e}); install mise from \
                     https://mise.jdx.dev/installing-mise.html, then run scripts/setup.sh"
                ));
            }
        }
        let mut path = directories.clone();
        if let Some(inherited) = inherited {
            path.extend(std::env::split_paths(inherited));
        }
        if let Some(cargo_bin) = cargo_bin
            && !path.contains(&cargo_bin)
        {
            path.push(cargo_bin);
        }
        let path = std::env::join_paths(&path).unwrap_or_else(|e| {
            failure.get_or_insert(format!("could not build PATH: {e}"));
            inherited.map(OsStr::to_os_string).unwrap_or_default()
        });
        Self {
            directories,
            path,
            failure,
        }
    }
}

/// `mise bin-paths` is asked for [`TOOLS`] by name, so a developer's global
/// mise config does not leak onto PATH.
fn tools() -> &'static Tools {
    static RESOLVED: OnceLock<Tools> = OnceLock::new();
    RESOLVED.get_or_init(|| {
        let mut mise = mise();
        mise.arg("bin-paths")
            .args(TOOLS.iter().map(|tool| tool.mise_name));
        let program = mise.get_program().to_string_lossy().into_owned();
        let cargo_bin = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cargo")))
            .map(|cargo_home| cargo_home.join("bin"));
        let inherited = std::env::var_os("PATH");
        Tools::from_mise(mise.output(), &program, inherited.as_deref(), cargo_bin)
    })
}

/// Where the mise.run installer puts mise, under `$HOME`.
const MISE_UNDER_HOME: &str = ".local/bin/mise";

/// Homebrew on Apple silicon, Homebrew on an Intel Mac, and Linuxbrew.
const MISE_FROM_A_PACKAGE_MANAGER: [&str; 3] = [
    "/opt/homebrew/bin/mise",
    "/usr/local/bin/mise",
    "/home/linuxbrew/.linuxbrew/bin/mise",
];

/// The mise on PATH, else an installed copy: a git hook or an IDE task runner
/// starts with a minimal PATH that holds none. Without any the bare name
/// stays, so the caller's error names mise.
fn mise() -> Command {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let installed = if on_path("mise", std::env::var_os("PATH").as_deref()) {
        None
    } else {
        home.map(|home| home.join(MISE_UNDER_HOME))
            .into_iter()
            .chain(MISE_FROM_A_PACKAGE_MANAGER.map(PathBuf::from))
            .find(|path| is_executable(path))
    };
    let mut command = installed.map_or_else(|| Command::new("mise"), Command::new);
    let root = repo_root();
    // A git worktree nested in another checkout would otherwise pick up that
    // checkout's mise.toml, which mise treats as a separate, untrusted config.
    if let Some(parent) = root.parent() {
        command.env("MISE_CEILING_PATHS", parent);
    }
    command.current_dir(root);
    command
}

/// Whether a directory of `path`, a PATH value, holds `bin` as an executable.
fn on_path(bin: &str, path: Option<&OsStr>) -> bool {
    path.is_some_and(|path| std::env::split_paths(path).any(|dir| is_executable(&dir.join(bin))))
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::chain;
    use crate::workspace::fixture::{TempDir, scratch_git};
    use std::collections::BTreeSet;
    use std::fs::Permissions;
    use std::os::unix::process::ExitStatusExt as _;

    /// A directory holding `weaver` as a file anyone may run, `dprint` as a
    /// file no one may, and `vale` as a directory anyone may enter.
    fn tool_directory() -> TempDir {
        let dir = TempDir::new("tools");
        dir.write("weaver", "#!/bin/sh\n");
        dir.write("dprint", "#!/bin/sh\n");
        std::fs::create_dir(dir.path().join("vale")).unwrap();
        for (name, mode) in [("weaver", 0o755), ("dprint", 0o644), ("vale", 0o755)] {
            std::fs::set_permissions(dir.path().join(name), Permissions::from_mode(mode)).unwrap();
        }
        dir
    }

    /// What `mise bin-paths` leaves when it ends with `code`.
    fn mise_output(code: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn not_installed(bin: &str) -> String {
        let why = "it is not installed; run scripts/setup.sh (or `mise install`)";
        format!("{bin} is pinned in mise.toml, but {why}")
    }

    /// Why a program that ends up running `bin` may not start, as told.
    fn refused_as(bin: Option<&str>, tools: &Tools) -> Option<String> {
        refusal(bin, tools).map(|why| why.to_string())
    }

    #[test]
    fn a_pinned_tool_runs_only_as_an_executable_file_in_a_directory_mise_listed() {
        let dir = tool_directory();
        let listed = mise_output(0, &format!("{}\n", dir.path().display()), "");
        let tools = Tools::from_mise(Ok(listed), "mise", None, None);
        assert_eq!(tools.failure, None);
        assert_eq!(refused_as(Some("weaver"), &tools), None);
        assert_eq!(
            refused_as(Some("dprint"), &tools),
            Some(not_installed("dprint"))
        );
        assert_eq!(
            refused_as(Some("vale"), &tools),
            Some(not_installed("vale"))
        );
        assert_eq!(
            refused_as(Some("shellcheck"), &tools),
            Some(not_installed("shellcheck"))
        );
        // What no pin names runs from PATH.
        assert_eq!(refused_as(Some("git"), &tools), None);
        assert_eq!(refused_as(None, &tools), None);

        // A cargo subcommand is its plugin, and any other program itself.
        let refused =
            |program: &str, args: &[&str]| refused_as(runs(program, args).as_deref(), &tools);
        assert_eq!(
            refused("cargo", &["deny", "check"]),
            Some(not_installed("cargo-deny"))
        );
        assert_eq!(refused("cargo", &["check"]), None);
        assert_eq!(refused("cargo", &[]), None);
        assert_eq!(refused("dprint", &["check"]), Some(not_installed("dprint")));
        assert_eq!(refused("weaver", &["mutants"]), None);
    }

    #[test]
    fn a_pinned_tool_refused_because_mise_failed_says_how_it_failed() {
        let failed = mise_output(1, "", "mise ERROR mise.toml is not trusted\n");
        let tools = Tools::from_mise(Ok(failed), "mise", None, None);
        assert_eq!(tools.directories, Vec::<PathBuf>::new());
        assert_eq!(
            refused_as(Some("weaver"), &tools).as_deref(),
            Some(
                "weaver is pinned in mise.toml, but `mise bin-paths` failed; run scripts/setup.sh, \
                 which trusts mise.toml and installs the pins:\nmise ERROR mise.toml is not trusted"
            )
        );
        let absent = std::io::Error::from(std::io::ErrorKind::NotFound);
        let tools = Tools::from_mise(Err(absent), "/opt/homebrew/bin/mise", None, None);
        assert_eq!(
            refused_as(Some("vale"), &tools).as_deref(),
            Some(
                "vale is pinned in mise.toml, but `/opt/homebrew/bin/mise` could not be run \
                 (entity not found); install mise from https://mise.jdx.dev/installing-mise.html, \
                 then run scripts/setup.sh"
            )
        );
        // A command that needs no pinned tool still runs.
        assert_eq!(refused_as(Some("git"), &tools), None);
    }

    #[test]
    fn the_directories_mise_lists_come_first_on_path_and_cargo_bin_comes_last_once() {
        let listed = || mise_output(0, "/pins/weaver\n/pins/vale\n", "");
        let cargo_bin = || Some(PathBuf::from("/home/x/.cargo/bin"));
        let inherited = OsString::from("/usr/bin:/bin");
        let tools = Tools::from_mise(Ok(listed()), "mise", Some(&inherited), cargo_bin());
        assert_eq!(
            tools.directories,
            [Path::new("/pins/weaver"), Path::new("/pins/vale")]
        );
        assert_eq!(
            tools.path,
            "/pins/weaver:/pins/vale:/usr/bin:/bin:/home/x/.cargo/bin"
        );
        let inherited = OsString::from("/home/x/.cargo/bin:/usr/bin");
        let tools = Tools::from_mise(Ok(listed()), "mise", Some(&inherited), cargo_bin());
        assert_eq!(
            tools.path,
            "/pins/weaver:/pins/vale:/home/x/.cargo/bin:/usr/bin"
        );
    }

    #[test]
    fn a_program_is_on_path_when_a_directory_there_holds_it_as_an_executable_file() {
        let (tools, empty) = (tool_directory(), TempDir::new("empty"));
        let path = std::env::join_paths([empty.path(), tools.path()]).unwrap();
        assert!(on_path("weaver", Some(&path)));
        assert!(!on_path("dprint", Some(&path)));
        assert!(!on_path("vale", Some(&path)));
        assert!(!on_path("mise", Some(&path)));
        assert!(!on_path("weaver", None));
    }

    /// `sh` is pinned by nothing, so it runs from PATH.
    #[test]
    fn a_command_reports_how_it_ended_and_one_that_cannot_start_is_named() {
        assert_eq!(
            stream("sh", &["-c", "exit 3"], &[]).unwrap().code(),
            Some(3)
        );
        let probe = ["-c", "test \"$XTASK_PROBE\" = given"];
        assert_eq!(
            stream("sh", &probe, &[("XTASK_PROBE", "given")])
                .unwrap()
                .code(),
            Some(0)
        );
        let capture = |program: &str, args: &[&str]| capture_in(&repo_root(), program, args);
        assert_eq!(
            capture("sh", &["-c", "echo out; echo err >&2"]).unwrap(),
            "out\n"
        );

        let missing = "lablet-xtask-no-such-program";
        let why = std::io::Error::from_raw_os_error(2);
        let could_not = format!("could not run `{missing}` (in the repository root): {why}");
        assert_eq!(chain(&capture(missing, &[]).unwrap_err()), could_not);
        assert_eq!(chain(&stream(missing, &[], &[]).unwrap_err()), could_not);
    }

    #[test]
    fn a_captured_command_that_fails_is_named_with_where_it_ran_and_what_it_wrote_on_stderr() {
        let failing = ["-c", "echo out; echo err >&2; echo more >&2; exit 1"];
        let error = capture_in(&repo_root(), "sh", &failing).unwrap_err();
        let Error::Failed { command, stderr } = &error else {
            panic!("{error:?}");
        };
        assert_eq!(*command, Invocation::new(&repo_root(), "sh", &failing));
        assert_eq!(stderr, "err\nmore");
        assert_eq!(
            chain(&error),
            "command failed (in the repository root): sh -c echo out; echo err >&2; echo more \
             >&2; exit 1\nerr\nmore"
        );

        // A command that wrote nothing is still named, and where it ran.
        let error = capture_in(&workspace_root(), "sh", &["-c", "exit 1"]).unwrap_err();
        assert_eq!(chain(&error), "command failed (in lablet/): sh -c exit 1");
    }

    #[test]
    fn a_command_whose_directory_is_gone_is_named_and_never_started() {
        let dir = TempDir::new("gone");
        let gone = dir.path().join("gone");
        let error = capture_in(&gone, "sh", &["-c", "exit 0"]).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Start {
                    source: NotStarted::NoDirectory,
                    ..
                }
            ),
            "{error:?}"
        );
        assert_eq!(
            chain(&error),
            format!(
                "could not run `sh -c exit 0` (in {}): the directory does not exist",
                gone.display()
            )
        );
    }

    #[test]
    fn git_answering_no_is_none_and_git_failing_to_answer_is_an_error() {
        let dir = TempDir::new("git-answer");
        let git = |args: &[&str]| scratch_git(dir.path(), args).trim().to_owned();
        git(&["init", "--quiet", "--initial-branch=topic"]);
        assert_eq!(
            std::fs::canonicalize(git(&["rev-parse", "--show-toplevel"])).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap(),
            "git resolved outside the scratch repository"
        );
        git(&["commit", "--quiet", "--allow-empty", "--message=base"]);
        let base = git(&["rev-parse", "HEAD"]);
        assert_eq!(git_commit(dir.path(), "HEAD").unwrap(), Some(base.clone()));
        assert_eq!(git_commit(dir.path(), "main").unwrap(), None);
        assert_eq!(git_merge_base(dir.path(), "main").unwrap(), None);

        // History HEAD shares nothing with, as a shallow clone may hold.
        let tree = git(&["mktree"]);
        let unrelated = git(&["commit-tree", &tree, "-m", "unrelated"]);
        git(&["update-ref", "refs/heads/main", &unrelated]);
        assert_eq!(git_merge_base(dir.path(), "main").unwrap(), None);
        git(&["update-ref", "refs/heads/main", &base]);
        assert_eq!(git_merge_base(dir.path(), "main").unwrap(), Some(base));

        // A `.git` file naming no repository stops git's search upward, so
        // what lies above the temporary directory doesn't matter.
        let outside = TempDir::new("git-no-answer");
        let nowhere = outside.path().join("none");
        outside.write(".git", &format!("gitdir: {}\n", nowhere.display()));
        for error in [
            git_commit(outside.path(), "HEAD").unwrap_err(),
            git_merge_base(outside.path(), "main").unwrap_err(),
        ] {
            let Error::Failed { command, stderr } = &error else {
                panic!("{error:?}");
            };
            assert!(
                command.to_string().starts_with("git rev-parse"),
                "{command}"
            );
            assert!(stderr.contains("not a git repository"), "{stderr}");
        }
    }

    /// rustup prints its own error for the toolchain, which names it.
    #[test]
    fn cargo_on_a_toolchain_that_is_not_installed_is_a_failed_run() {
        let absent = "xtask-test-toolchain-that-is-not-installed";
        let status = stream_on(absent, &["--version"], &[]).unwrap();
        assert_eq!(status.code(), Some(1));
    }

    /// Every name the git on this machine lists, so a newer git's addition
    /// fails here rather than reaching a test that runs under a hook.
    #[test]
    fn a_subprocess_never_inherits_the_repository_git_names_for_a_hook() {
        let command = command_in(&repo_root(), "git", &["status"]).unwrap();
        let removed: BTreeSet<String> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        let listed = capture_in(&repo_root(), "git", &["rev-parse", "--local-env-vars"]).unwrap();
        let listed: BTreeSet<String> = listed.lines().map(str::to_owned).collect();
        assert_eq!(removed, listed);
    }

    #[test]
    fn every_pinned_tool_xtask_runs_is_pinned_in_mise_toml() {
        let path = repo_root().join("mise.toml");
        let config: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let pinned = config.get("tools").and_then(toml::Value::as_table).unwrap();
        for tool in TOOLS {
            assert!(
                pinned.contains_key(tool.mise_name),
                "xtask/src/process.rs TOOLS lists `{}`, but mise.toml does not pin it",
                tool.mise_name
            );
        }
    }

    /// `mise lock` skips a platform it cannot look up (a spent GitHub rate
    /// limit) and still exits 0, and a tool missing a host installs unverified
    /// there. The cargo backend builds from source and records no platforms.
    #[test]
    fn the_mise_lockfile_covers_every_supported_host_for_every_downloaded_tool() {
        const HOSTS: [&str; 4] = ["linux-x64", "linux-arm64", "macos-arm64", "macos-x64"];
        let path = repo_root().join("mise.lock");
        let lock: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let tools = lock.get("tools").and_then(toml::Value::as_table).unwrap();
        let mut downloaded = 0;
        for (name, entries) in tools {
            for entry in entries.as_array().unwrap() {
                let backend = entry.get("backend").and_then(toml::Value::as_str).unwrap();
                if backend.starts_with("cargo:") {
                    continue;
                }
                downloaded += 1;
                for host in HOSTS {
                    let checksum = entry
                        .get(format!("platforms.{host}"))
                        .and_then(|platform| platform.get("checksum"));
                    assert!(
                        checksum.is_some(),
                        "mise.lock has no checksum for `{name}` on {host}; rerun the `mise lock` \
                         command in mise.toml with GITHUB_TOKEN set"
                    );
                }
            }
        }
        assert!(downloaded > 0, "mise.lock lists no downloaded tool");
    }

    #[test]
    fn cargo_on_another_toolchain_runs_through_rustup_in_the_workspace() {
        let args = ["llvm-cov", "--locked"];
        assert_eq!(
            on_toolchain("nightly-2026-08-25", &args),
            ["run", "nightly-2026-08-25", "cargo", "llvm-cov", "--locked"]
        );
        assert_eq!(
            command_failed_on("nightly-2026-08-25", &args),
            "command failed (in lablet/): rustup run nightly-2026-08-25 cargo llvm-cov --locked"
        );
        // The plugin is a pinned tool on that toolchain as on any other.
        assert_eq!(cargo_plugin(&args).as_deref(), Some("cargo-llvm-cov"));
        assert!(TOOLS.iter().any(|tool| tool.bin == "cargo-llvm-cov"));
    }

    #[test]
    fn a_failed_command_is_named_with_its_arguments_and_where_it_ran() {
        assert_eq!(
            command_failed("cargo", &["deny", "check"]),
            "command failed (in lablet/): cargo deny check"
        );
        assert_eq!(
            command_failed("git", &["status"]),
            "command failed (in the repository root): git status"
        );
    }
}
