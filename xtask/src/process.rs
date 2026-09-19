//! Subprocess plumbing: where a command runs, and which copy of a pinned tool
//! it finds. Nothing here exits the process, because a gate keeps going after
//! a failure.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::OnceLock;

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

/// The line for a subprocess that could not start.
pub fn could_not_run(program: &str, e: &std::io::Error) -> String {
    format!("could not run `{program}`: {e}")
}

/// The line for a subprocess that exited non-zero. It names the directory:
/// a cargo command as printed does not work from the repository root, where
/// `cargo xtask` is started.
pub fn command_failed(program: &str, args: &[&str]) -> String {
    let (_, place) = working_directory(program);
    format!("command failed (in {place}): {program} {}", args.join(" "))
}

/// Cargo runs in the Rust workspace, everything else in the repository root.
fn working_directory(program: &str) -> (PathBuf, &'static str) {
    if program == "cargo" {
        (workspace_root(), "lablet/")
    } else {
        (repo_root(), "the repository root")
    }
}

/// A subprocess with the pinned tools first on PATH. A pinned tool that mise
/// cannot provide is an error, not a run of whatever copy PATH holds.
pub fn command(program: &str, args: &[&str]) -> Result<Command, String> {
    command_in(&working_directory(program).0, program, args)
}

/// [`command`], in a directory the caller names.
fn command_in(directory: &Path, program: &str, args: &[&str]) -> Result<Command, String> {
    if !directory.is_dir() {
        return Err(format!(
            "{} does not exist, so `{program}` has nowhere to run",
            directory.display()
        ));
    }
    let tools = tools();
    // `cargo deny ...` runs the binary `cargo-deny`.
    let bin = if program == "cargo" {
        args.first().map(|subcommand| format!("cargo-{subcommand}"))
    } else {
        Some(program.to_owned())
    };
    if let Some(bin) = bin
        && TOOLS.iter().any(|tool| tool.bin == bin)
        && !tools.provides(&bin)
    {
        let why = tools
            .failure
            .as_deref()
            .unwrap_or("it is not installed; run scripts/setup.sh (or `mise install`)");
        return Err(format!("{bin} is pinned in mise.toml, but {why}"));
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
pub fn stream(program: &str, args: &[&str], env: &[(&str, &str)]) -> Result<ExitStatus, String> {
    let mut command = command(program, args)?;
    command.envs(env.iter().copied());
    command.status().map_err(|e| could_not_run(program, &e))
}

/// A subprocess's stdout, or on failure its stderr or why it could not start.
pub fn capture(program: &str, args: &[&str]) -> Result<String, String> {
    capture_in(&working_directory(program).0, program, args)
}

/// [`capture`], in a directory the caller names.
pub fn capture_in(directory: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    let output = command_in(directory, program, args)?
        .output()
        .map_err(|e| could_not_run(program, &e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_owned())
    }
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
}

/// `mise bin-paths` is asked for [`TOOLS`] by name, so a developer's global
/// mise config does not leak onto PATH. A failure is kept for the first pinned
/// tool that needs it; a command that needs none still runs. `$CARGO_HOME/bin`
/// goes last if absent: cargo would otherwise search it ahead of PATH and
/// shadow the pinned cargo plugins.
fn tools() -> &'static Tools {
    static RESOLVED: OnceLock<Tools> = OnceLock::new();
    RESOLVED.get_or_init(|| {
        let mut directories = Vec::new();
        let mut failure = None;
        let mut mise = mise();
        mise.arg("bin-paths")
            .args(TOOLS.iter().map(|tool| tool.mise_name));
        match mise.output() {
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
                    "`{}` could not be run ({e}); install mise from \
                     https://mise.jdx.dev/installing-mise.html, then run scripts/setup.sh",
                    mise.get_program().to_string_lossy()
                ));
            }
        }
        let mut path = directories.clone();
        if let Some(inherited) = std::env::var_os("PATH") {
            path.extend(std::env::split_paths(&inherited));
        }
        let cargo_bin = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cargo")))
            .map(|cargo_home| cargo_home.join("bin"));
        if let Some(cargo_bin) = cargo_bin
            && !path.contains(&cargo_bin)
        {
            path.push(cargo_bin);
        }
        let path = std::env::join_paths(&path).unwrap_or_else(|e| {
            failure.get_or_insert(format!("could not build PATH: {e}"));
            std::env::var_os("PATH").unwrap_or_default()
        });
        Tools {
            directories,
            path,
            failure,
        }
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
    let installed = if on_path("mise") {
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

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| is_executable(&dir.join(bin))))
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subprocess_never_inherits_the_repository_git_names_for_a_hook() {
        let command = command_in(&repo_root(), "git", &["status"]).unwrap();
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
        ] {
            assert!(removed.iter().any(|key| key == variable), "{variable}");
        }
        assert_eq!(removed.len(), GIT_REPOSITORY_ENV.len());
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
