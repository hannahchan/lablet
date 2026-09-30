//! The gate runner as a developer or a hook starts it: the binary itself, and
//! the exit status it leaves.

use std::path::Path;
use std::process::Command;

/// The runner finds the repository through the manifest directory cargo
/// names at run time, so one that isn't there fails any gate that reads it.
#[test]
fn the_binary_exits_non_zero_for_a_failed_gate_and_zero_for_a_passed_one() {
    let lint_layers = |manifest_directory: &Path| {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("lint-layers")
            .env("CARGO_MANIFEST_DIR", manifest_directory)
            .output()
            .unwrap()
    };

    let absent = Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-repository/xtask");
    assert!(!absent.exists(), "{}", absent.display());
    let failed = lint_layers(&absent);
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert_eq!(failed.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("[FAIL] lint-layers"), "{stderr}");
    // The cause is told after the error it explains.
    let manifest = absent.with_file_name("lablet/Cargo.toml");
    let why = std::io::Error::from_raw_os_error(2);
    let told = format!("\n\ncould not read {}: {why}\n", manifest.display());
    assert!(stderr.contains(&told), "{stderr}");

    let passed = lint_layers(Path::new(env!("CARGO_MANIFEST_DIR")));
    let stderr = String::from_utf8_lossy(&passed.stderr);
    assert_eq!(passed.status.code(), Some(0), "{stderr}");
}

#[test]
fn a_task_that_does_not_exist_is_told_before_the_usage_and_exits_non_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("fnt")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.starts_with("error: unknown task `fnt`\n\nUsage: cargo xtask <task>"),
        "{stderr}"
    );
}

/// `cargo xtask run` becomes cargo, as cargo becomes lablet, so lablet's exit
/// status and the signals sent to xtask are lablet's: 2 for a run that began
/// and stopped short, and a Ctrl-C that lablet cancels on. A stand-in `cargo`
/// first on PATH says which process it runs as, then exits 2.
#[test]
fn run_hands_its_process_to_cargo_so_the_exit_status_is_what_cargo_ran() {
    use std::os::unix::fs::PermissionsExt as _;

    let bin = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("stand-in-cargo-{}", std::process::id()));
    std::fs::create_dir_all(&bin).unwrap();
    let cargo = bin.join("cargo");
    let script = "#!/bin/sh\necho \"$$ ${GIT_DIR-unset} $(pwd -P) $*\"\nexit 2\n";
    std::fs::write(&cargo, script).unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path =
        std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(&inherited)))
            .unwrap();

    let manifest_directory = Path::new(env!("CARGO_MANIFEST_DIR"));
    let xtask = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["run", "--", "--config", "lablet.toml"])
        .env("CARGO_MANIFEST_DIR", manifest_directory)
        .env("PATH", path)
        // What git sets for a hook, which the command must not inherit.
        .env("GIT_DIR", "/nonexistent")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let xtask_pid = xtask.id();
    let output = xtask.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&bin);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert_eq!(stderr, "", "xtask reported on a run that was cargo's");
    let workspace = std::fs::canonicalize(manifest_directory.join("../lablet")).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "{xtask_pid} unset {} run --locked --bin lablet -- --config lablet.toml\n",
            workspace.display()
        )
    );
}
