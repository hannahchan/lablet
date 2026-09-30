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
