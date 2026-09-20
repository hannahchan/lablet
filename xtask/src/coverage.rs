//! `cargo xtask coverage`: line and region coverage from cargo-llvm-cov, held
//! to the floors in [`crate::floors`]. The whole workspace's tests run once;
//! each floor crate is then judged twice, on the lines and on the regions of
//! its own source files.
//!
//! Regions are the finer of the two and the reason both are reported. A region
//! is a span of source with its own counter, so each match arm, each `else` a
//! line never spells out, and the far side of a `&&` are counted apart; a line
//! is covered when any region touching it ran. So 100% regions implies 100%
//! lines, and the gap between them is where a line hides an arm nothing took.
//! Branch coverage would be finer again in the other direction — it alone sees
//! a condition that was never false — but it needs a nightly toolchain, and
//! this workspace pins a stable one.
//!
//! The floors are on production code. Test code is covered by construction, so
//! counting it would lift a crate over the floor with its production code well
//! under it; the report leaves out every file [`floors::TEST_FILES`] names.

use serde::Deserialize;
use std::path::PathBuf;

use crate::floors::{self, FloorCrate, Line};
use crate::gates::{CheckResult, LOCKED};
use crate::process;
use crate::workspace::{Workspace, workspace_root};

/// The part of `llvm-cov export --summary-only` JSON that is read.
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
    regions: ExportCounts,
}

#[derive(Debug, Deserialize)]
struct ExportCounts {
    count: u64,
    covered: u64,
}

/// What a crate is judged on: which counts to read, and how to say so.
struct Measure {
    /// Reads this measure's counts out of one file's summary.
    counts: fn(&ExportSummary) -> &ExportCounts,
    /// The floor for this measure, out of the crate's floors.
    floor: fn(&floors::Floor) -> u64,
    unit: &'static str,
    none_of: &'static str,
}

/// Lines first, then regions, so each crate reads from the kinder measure to
/// the stricter one and the gap between them is on adjacent report lines.
const MEASURES: [Measure; 2] = [
    Measure {
        counts: |summary| &summary.lines,
        floor: |floor| floor.line_coverage,
        unit: "production lines covered",
        none_of: "coverable lines",
    },
    Measure {
        counts: |summary| &summary.regions,
        floor: |floor| floor.region_coverage,
        unit: "production regions covered",
        none_of: "coverable regions",
    },
];

/// Two report lines per floor crate, one per measure: the counts of every file
/// under its directory, summed. A crate with no file in the report has nothing
/// coverable, which the standing reads as measuring nothing.
fn lines_by_crate(export: &Export, crates: &[FloorCrate]) -> Vec<Line> {
    crates
        .iter()
        .flat_map(|krate| {
            MEASURES.iter().map(move |measure| {
                let (hit, total) = export
                    .data
                    .iter()
                    .flat_map(|data| &data.files)
                    .filter(|file| file.filename.starts_with(&krate.directory))
                    .fold((0, 0), |(hit, total), file| {
                        let counts = (measure.counts)(&file.summary);
                        (hit + counts.covered, total + counts.count)
                    });
                Line {
                    package: krate.floor.package,
                    hit,
                    total,
                    floor: (measure.floor)(krate.floor),
                    unit: measure.unit,
                    none_of: measure.none_of,
                    holds_code: krate.holds_code,
                }
            })
        })
        .collect()
}

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

    /// Two files in one crate, one in a crate whose name shares a prefix, one
    /// in another floor crate. Every file's regions differ from its lines, so
    /// a measure that read the wrong counts could not pass.
    const REPORT: &str = r#"{
      "data": [{
        "files": [
          {"filename": "/ws/crates/domain/model/src/lib.rs",
           "summary": {"lines": {"count": 7, "covered": 6, "percent": 85.7},
                       "regions": {"count": 8, "covered": 5, "percent": 62.5},
                       "functions": {"count": 1, "covered": 1, "percent": 100.0}}},
          {"filename": "/ws/crates/domain/model/src/usage.rs",
           "summary": {"lines": {"count": 2, "covered": 1, "percent": 50.0},
                       "regions": {"count": 3, "covered": 1, "percent": 33.3}}},
          {"filename": "/ws/crates/domain/model-extras/src/lib.rs",
           "summary": {"lines": {"count": 3, "covered": 0, "percent": 0.0},
                       "regions": {"count": 4, "covered": 0, "percent": 0.0}}},
          {"filename": "/ws/crates/application/run/src/lib.rs",
           "summary": {"lines": {"count": 1, "covered": 1, "percent": 100.0},
                       "regions": {"count": 2, "covered": 2, "percent": 100.0}}}
        ],
        "totals": {"lines": {"count": 13, "covered": 8, "percent": 61.5}}
      }],
      "type": "llvm.coverage.json.export", "version": "2.0.1"
    }"#;

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

    /// The unit's middle word, which is the measure: "lines" or "regions".
    fn seen(lines: &[Line]) -> Vec<(&str, &str, u64, u64, Standing)> {
        lines
            .iter()
            .map(|line| {
                let measure = line.unit.split(' ').nth(1).unwrap_or(line.unit);
                (line.package, measure, line.hit, line.total, line.standing())
            })
            .collect()
    }

    #[test]
    fn a_crate_is_judged_on_the_files_under_its_own_directory_once_per_measure() {
        let export: Export = serde_json::from_str(REPORT).unwrap();
        assert_eq!(
            seen(&lines_by_crate(&export, &crates(false))),
            [
                // 7 of 9, 6 of 11: `model-extras` shares a name prefix, not a
                // directory, so neither measure counts it.
                ("lablet-model", "lines", 7, 9, Standing::Below),
                ("lablet-model", "regions", 6, 11, Standing::Below),
                ("lablet-policy", "lines", 0, 0, Standing::NothingToMeasure),
                ("lablet-policy", "regions", 0, 0, Standing::NothingToMeasure),
                ("lablet-run", "lines", 1, 1, Standing::Met),
                ("lablet-run", "regions", 2, 2, Standing::Met),
            ]
        );
    }

    /// Regions are the stricter measure, so a crate can meet the line floor
    /// and miss the region one. That gap is the whole reason both are read.
    #[test]
    fn a_crate_over_the_line_floor_can_still_be_under_the_region_floor() {
        const REPORT: &str = r#"{
          "data": [{
            "files": [
              {"filename": "/ws/crates/application/run/src/lib.rs",
               "summary": {"lines": {"count": 10, "covered": 10, "percent": 100.0},
                           "regions": {"count": 10, "covered": 8, "percent": 80.0}}}
            ]
          }],
          "type": "llvm.coverage.json.export", "version": "2.0.1"
        }"#;
        let export: Export = serde_json::from_str(REPORT).unwrap();
        let lines = lines_by_crate(&export, &crates(true));

        assert_eq!(
            seen(&lines)[4..],
            [
                ("lablet-run", "lines", 10, 10, Standing::Met),
                ("lablet-run", "regions", 8, 10, Standing::Below),
            ]
        );
        assert!(floors::conclude("coverage", &lines).is_err());
    }

    #[test]
    fn a_crate_that_defines_a_function_and_has_no_lines_in_the_report_fails() {
        let export: Export = serde_json::from_str(REPORT).unwrap();
        let lines = lines_by_crate(&export, &crates(true));
        assert_eq!(
            seen(&lines)[2..4],
            [
                ("lablet-policy", "lines", 0, 0, Standing::NothingMeasured),
                ("lablet-policy", "regions", 0, 0, Standing::NothingMeasured),
            ]
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
