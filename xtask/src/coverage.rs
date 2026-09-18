//! `cargo xtask coverage`: line coverage from cargo-llvm-cov, held to the
//! floors in [`crate::floors`]. The whole workspace's tests run once; each
//! floor crate is then judged on the lines of its own source files.

use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::floors::{self, Floor, Line};
use crate::gates::CheckResult;
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
/// lines.
fn lines_by_crate(export: &Export, crates: &[(&'static Floor, PathBuf)]) -> Vec<Line> {
    crates
        .iter()
        .map(|(floor, directory)| {
            let (hit, total) = export
                .data
                .iter()
                .flat_map(|data| &data.files)
                .filter(|file| file.filename.starts_with(directory))
                .fold((0, 0), |(hit, total), file| {
                    (
                        hit + file.summary.lines.covered,
                        total + file.summary.lines.count,
                    )
                });
            Line {
                package: floor.package,
                hit,
                total,
                floor: floor.line_coverage,
                unit: "lines covered",
                none_of: "coverable lines",
            }
        })
        .collect()
}

/// Runs the workspace's tests under cargo-llvm-cov and judges the floors.
pub fn check() -> CheckResult {
    let workspace = Workspace::load(&workspace_root()).map_err(|e| e.to_string())?;
    let crates: Vec<(&'static Floor, PathBuf)> = floors::crate_directories(&workspace)?
        .into_iter()
        // llvm-cov reports resolved paths, so compare against resolved ones.
        .map(|(floor, directory)| {
            let resolved = directory.canonicalize().unwrap_or(directory);
            (floor, resolved)
        })
        .collect();

    let report = report_path(&workspace.root)?;
    let report_arg = report.display().to_string();
    let args = [
        "llvm-cov",
        "--workspace",
        "--json",
        "--summary-only",
        "--output-path",
        &report_arg,
    ];
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

/// Where the JSON report goes: under the workspace's `target/`, which git
/// ignores and `cargo clean` removes.
fn report_path(workspace_root: &Path) -> Result<PathBuf, String> {
    let directory = workspace_root.join("target").join("xtask");
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("could not create {}: {e}", directory.display()))?;
    Ok(directory.join("coverage.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::floors::Standing;

    /// A cut-down cargo-llvm-cov 0.9 report: two files in one crate, one in
    /// another, one outside every floor crate.
    const REPORT: &str = r#"{
      "data": [{
        "files": [
          {"filename": "/ws/crates/domain/model/src/lib.rs",
           "summary": {"lines": {"count": 60, "covered": 57, "percent": 95.0},
                       "functions": {"count": 9, "covered": 9, "percent": 100.0}}},
          {"filename": "/ws/crates/domain/model/src/usage.rs",
           "summary": {"lines": {"count": 40, "covered": 30, "percent": 75.0}}},
          {"filename": "/ws/crates/domain/model-extras/src/lib.rs",
           "summary": {"lines": {"count": 500, "covered": 0, "percent": 0.0}}},
          {"filename": "/ws/crates/application/run/src/lib.rs",
           "summary": {"lines": {"count": 10, "covered": 9, "percent": 90.0}}}
        ],
        "totals": {"lines": {"count": 610, "covered": 96, "percent": 15.7}}
      }],
      "type": "llvm.coverage.json.export",
      "version": "3.1.0"
    }"#;

    fn floor(package: &'static str) -> &'static Floor {
        floors::FLOORS
            .iter()
            .find(|floor| floor.package == package)
            .unwrap()
    }

    #[test]
    fn a_crate_is_judged_on_the_files_under_its_own_directory() {
        let export: Export = serde_json::from_str(REPORT).unwrap();
        let crates = vec![
            (
                floor("lablet-model"),
                PathBuf::from("/ws/crates/domain/model"),
            ),
            (
                floor("lablet-policy"),
                PathBuf::from("/ws/crates/domain/policy"),
            ),
            (
                floor("lablet-run"),
                PathBuf::from("/ws/crates/application/run"),
            ),
        ];
        let lines = lines_by_crate(&export, &crates);
        let seen: Vec<(&str, u64, u64, Standing)> = lines
            .iter()
            .map(|line| (line.package, line.hit, line.total, line.standing()))
            .collect();
        assert_eq!(
            seen,
            [
                // 87 of 100: `model-extras` shares a name prefix, not a directory.
                ("lablet-model", 87, 100, Standing::Below),
                ("lablet-policy", 0, 0, Standing::NothingToMeasure),
                ("lablet-run", 9, 10, Standing::Met),
            ]
        );
    }

    #[test]
    fn an_empty_workspace_report_has_nothing_to_measure_anywhere() {
        let export: Export =
            serde_json::from_str(r#"{"data":[{"files":[],"totals":{}}],"version":"3.1.0"}"#)
                .unwrap();
        let crates = vec![(
            floor("lablet-model"),
            PathBuf::from("/ws/crates/domain/model"),
        )];
        let lines = lines_by_crate(&export, &crates);
        assert_eq!(lines[0].standing(), Standing::NothingToMeasure);
    }
}
