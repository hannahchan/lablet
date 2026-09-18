//! Subprocess plumbing: where a command runs, and which copy of a pinned tool
//! it finds. Nothing here exits the process; a gate keeps going after a
//! failure, so every problem comes back as an `Err` with its message.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::OnceLock;

use crate::workspace::{repo_root, workspace_root};

/// A tool mise.toml pins, by its mise name and the binary it provides.
pub struct Tool {
    /// The key in mise.toml's `[tools]` table.
    pub mise_name: &'static str,
    /// The executable, as cargo or the shell looks it up.
    pub bin: &'static str,
}

/// The pinned tools xtask shells out to. A test keeps the names in step with
/// mise.toml.
///
/// Phase 1 extension point: add `github:open-telemetry/weaver` (binary
/// `weaver`) here when the `weaver` commands land.
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
];

/// The line for a subprocess that could not start.
pub fn could_not_run(program: &str, e: &std::io::Error) -> String {
    format!("could not run `{program}`: {e}")
}

/// The line for a subprocess that exited non-zero.
pub fn command_failed(program: &str, args: &[&str]) -> String {
    format!("command failed: {program} {}", args.join(" "))
}

/// Cargo commands run in the Rust workspace; everything else (git, mise)
/// runs in the repository root.
fn working_directory(program: &str) -> PathBuf {
    if program == "cargo" {
        workspace_root()
    } else {
        repo_root()
    }
}

/// A subprocess in the directory [`working_directory`] picks, with the tools
/// pinned in mise.toml first on PATH. A pinned tool that mise cannot provide
/// is an error rather than a run of whatever copy PATH holds: the pin is the
/// point.
pub fn command(program: &str, args: &[&str]) -> Result<Command, String> {
    let directory = working_directory(program);
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
    Ok(command)
}

/// Runs a subprocess with inherited output and returns its status.
pub fn stream(program: &str, args: &[&str], env: &[(&str, &str)]) -> Result<ExitStatus, String> {
    let mut command = command(program, args)?;
    command.envs(env.iter().copied());
    command.status().map_err(|e| could_not_run(program, &e))
}

/// Runs a subprocess with piped output: its stdout on success, its stderr (or
/// the reason it could not start) on failure.
pub fn capture(program: &str, args: &[&str]) -> Result<String, String> {
    let output = command(program, args)?
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

/// Where the pinned tools live, and the PATH that puts them first. `failure`
/// is why `directories` may be empty: mise could not be run, or refused.
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

/// Resolved once per run. `mise bin-paths` is asked for [`TOOLS`] by name, so
/// a developer's global mise config does not leak onto PATH. A failure is
/// kept, not printed, and reported by the first pinned tool that needs it; a
/// command that needs none still runs. `$CARGO_HOME/bin` goes last if absent:
/// cargo would otherwise search it ahead of PATH and shadow the pinned cargo
/// plugins.
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

/// Where the mise.run installer puts mise, under `$HOME`. macOS shells do not
/// put that directory on PATH, so [`mise`] looks there when PATH has no mise.
const MISE_UNDER_HOME: &str = ".local/bin/mise";

/// A `mise` command in the repository root, where mise.toml is: the mise on
/// PATH, else the installer's copy. Without either the bare name stays, so the
/// caller's error names mise.
fn mise() -> Command {
    let installed = std::env::var_os("HOME").map(|home| Path::new(&home).join(MISE_UNDER_HOME));
    let mut command = match installed {
        Some(path) if !on_path("mise") && is_executable(&path) => Command::new(path),
        _ => Command::new("mise"),
    };
    command.current_dir(repo_root());
    command
}

/// Whether an executable `bin` is on PATH. No `.exe`: Windows is not supported.
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
    fn every_pinned_tool_xtask_runs_is_pinned_in_mise_toml() {
        let path = repo_root().join("mise.toml");
        if !path.is_file() {
            eprintln!("skipped: {} does not exist yet", path.display());
            return;
        }
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

    #[test]
    fn a_failed_command_is_named_with_its_arguments() {
        assert_eq!(
            command_failed("cargo", &["deny", "check"]),
            "command failed: cargo deny check"
        );
    }
}
