//! `cargo xtask coverage`: line coverage from cargo-llvm-cov, held to the
//! floors in [`crate::floors`]. The whole workspace's tests run once; each
//! floor crate is then judged on the lines of its own source files.
//!
//! The floor is on production lines. Test code is covered by construction, so
//! counting it would lift a crate over the floor with its production code well
//! under it. The report leaves out every file [`floors::TEST_FILES`] names,
//! and [`floors::crates`] fails a floor crate that keeps test code anywhere
//! else.

use serde::Deserialize;
use std::path::PathBuf;

use crate::floors::{self, FloorCrate, Line};
use crate::gates::{CheckResult, LOCKED};
use crate::process;
use crate::workspace::{Workspace, workspace_root};

/// The part of `llvm-cov export --summary-only` JSON that is read: per file,
/// its name and its line counts.
#[derive(Debug, Deserialize)]
struct Export {
    data: Vec<ExportData>,
}

#[derive(Debug, Deserialize)]
struct ExportData {
    files: Vec<ExportFile>,
}

#[derive(Debug, Deserialize)]
struct ExportFile {
    filename: PathBuf,
    summary: ExportSummary,
}

#[derive(Debug, Deserialize)]
struct ExportSummary {
    lines: ExportCounts,
}

#[derive(Debug, Deserialize)]
struct ExportCounts {
    count: u64,
    covered: u64,
}

/// One report line per floor crate: the lines of every file under the crate's
/// directory, summed. A crate with no file in the report has no coverable
/// lines, which passes only while it defines no function.
fn lines_by_crate(export: &Export, crates: &[FloorCrate]) -> Vec<Line> {
    crates
        .iter()
        .map(|krate| {
            let (hit, total) = export
                .data
                .iter()
                .flat_map(|data| &data.files)
                .filter(|file| file.filename.starts_with(&krate.directory))
                .fold((0, 0), |(hit, total), file| {
                    (
                        hit + file.summary.lines.covered,
                        total + file.summary.lines.count,
                    )
                });
            Line {
                package: krate.floor.package,
                hit,
                total,
                floor: krate.floor.line_coverage,
                unit: "production lines covered",
                none_of: "coverable lines",
                holds_code: krate.holds_code,
            }
        })
        .collect()
}

/// The cargo-llvm-cov command: every test of the workspace, the committed
/// lockfile, test files left out, a JSON summary at `report`.
fn llvm_cov_args(report: &str) -> [&str; 9] {
    [
        "llvm-cov",
        LOCKED,
        "--workspace",
        "--ignore-filename-regex",
        floors::TEST_FILES,
        "--json",
        "--summary-only",
        "--output-path",
        report,
    ]
}

/// Runs the workspace's tests under cargo-llvm-cov and judges the floors.
pub fn check() -> CheckResult {
    let workspace = Workspace::load(&workspace_root())?;
    let mut crates = floors::crates(&workspace)?;
    for krate in &mut crates {
        // llvm-cov reports resolved paths, so compare against resolved ones.
        if let Ok(resolved) = krate.directory.canonicalize() {
            krate.directory = resolved;
        }
    }

    let report = floors::output_directory(&workspace.root)?.join("coverage.json");
    let report_arg = report.display().to_string();
    let args = llvm_cov_args(&report_arg);
    let status = process::stream("cargo", &args, &[])?;
    if !status.success() {
        return Err(format!(
            "{}; no coverage was measured (a failing test fails coverage too)",
            process::command_failed("cargo", &args)
        ));
    }

    let text = std::fs::read_to_string(&report)
        .map_err(|e| format!("could not read {}: {e}", report.display()))?;
    let export: Export = serde_json::from_str(&text)
        .map_err(|e| format!("could not parse {}: {e}", report.display()))?;
    floors::conclude("coverage", &lines_by_crate(&export, &crates))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::floors::Standing;

    /// A cut-down cargo-llvm-cov 0.9 report: two files in one crate, one in a
    /// crate whose name shares a prefix, one in another floor crate.
    const REPORT: &str = r#"{
      "data": [{
        "files": [
          {"filename": "/ws/crates/domain/model/src/lib.rs",
           "summary": {"lines": {"count": 7, "covered": 6, "percent": 85.7},
                       "functions": {"count": 1, "covered": 1, "percent": 100.0}}},
          {"filename": "/ws/crates/domain/model/src/usage.rs",
           "summary": {"lines": {"count": 2, "covered": 1, "percent": 50.0}}},
          {"filename": "/ws/crates/domain/model-extras/src/lib.rs",
           "summary": {"lines": {"count": 3, "covered": 0, "percent": 0.0}}},
          {"filename": "/ws/crates/application/run/src/lib.rs",
           "summary": {"lines": {"count": 1, "covered": 1, "percent": 100.0}}}
        ],
        "totals": {"lines": {"count": 13, "covered": 8, "percent": 61.5}}
      }],
      "type": "llvm.coverage.json.export", "version": "2.0.1"
    }"#;

    /// The three floor crates under `/ws`, each defining a function or not.
    fn crates(holds_code: bool) -> Vec<FloorCrate> {
        [
            "/ws/crates/domain/model",
            "/ws/crates/domain/policy",
            "/ws/crates/application/run",
        ]
        .into_iter()
        .zip(floors::FLOORS)
        .map(|(directory, floor)| FloorCrate {
            floor,
            directory: PathBuf::from(directory),
            holds_code,
        })
        .collect()
    }

    fn seen(lines: &[Line]) -> Vec<(&str, u64, u64, Standing)> {
        lines
            .iter()
            .map(|line| (line.package, line.hit, line.total, line.standing()))
            .collect()
    }

    #[test]
    fn a_crate_is_judged_on_the_files_under_its_own_directory() {
        let export: Export = serde_json::from_str(REPORT).unwrap();
        assert_eq!(
            seen(&lines_by_crate(&export, &crates(false))),
            [
                // 7 of 9: `model-extras` shares a name prefix, not a directory.
                ("lablet-model", 7, 9, Standing::Below),
                ("lablet-policy", 0, 0, Standing::NothingToMeasure),
                ("lablet-run", 1, 1, Standing::Met),
            ]
        );
    }

    #[test]
    fn a_crate_that_defines_a_function_and_has_no_lines_in_the_report_fails() {
        let export: Export = serde_json::from_str(REPORT).unwrap();
        let lines = lines_by_crate(&export, &crates(true));
        assert_eq!(
            seen(&lines)[1],
            ("lablet-policy", 0, 0, Standing::NothingMeasured)
        );
        assert!(floors::conclude("coverage", &lines).is_err());
    }

    #[test]
    fn coverage_runs_every_test_against_the_lockfile_and_leaves_test_files_out() {
        let args = llvm_cov_args("/out/coverage.json");
        assert_eq!(args[..3], ["llvm-cov", "--locked", "--workspace"]);
        let regex = args
            .iter()
            .position(|arg| *arg == "--ignore-filename-regex");
        assert_eq!(regex.map(|at| args[at + 1]), Some(r"(^|/)tests(\.rs|/)"));
        assert!(args.ends_with(&["--output-path", "/out/coverage.json"]));
    }
}
