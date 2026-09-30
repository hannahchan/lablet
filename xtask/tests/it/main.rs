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

    let passed = lint_layers(Path::new(env!("CARGO_MANIFEST_DIR")));
    let stderr = String::from_utf8_lossy(&passed.stderr);
    assert_eq!(passed.status.code(), Some(0), "{stderr}");
}
