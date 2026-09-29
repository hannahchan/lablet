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
//!
//! Branch coverage is finer again in the other direction: it alone sees a
//! side of a `&&` or `||` that always went the same way while the whole
//! condition went both ways. The compiler measures it only on a nightly
//! toolchain, so `cargo xtask coverage --branch` is a run of its own, on the
//! nightly [`NIGHTLY`] pins, that judges the branches and nothing else. The
//! workspace, and the lines and regions, stay on the stable toolchain
//! `rust-toolchain.toml` pins.
//!
//! The floors are on production code. Test code is covered by construction, so
//! counting it would lift a crate over the floor with its production code well
//! under it; the report leaves out every file [`floors::TEST_FILES`] names.

use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::floors::{self, FloorCrate, Line};
use crate::gates::{CheckResult, LOCKED};
use crate::process;
use crate::workspace::{Workspace, repo_root, workspace_root};

/// The toolchain branch coverage is measured on, pinned by date so that every
/// run measures with the same compiler. .github/workflows/floors.yml installs
/// it by the same name; a test holds the two in step.
pub const NIGHTLY: &str = "nightly-2026-08-25";

/// What cargo-llvm-cov needs of the toolchain it runs on.
const LLVM_TOOLS: &str = "llvm-tools-preview";

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
    /// All zeros unless the run was one with `--branch`.
    #[serde(default)]
    branches: ExportCounts,
}

#[derive(Debug, Default, Deserialize)]
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
    /// Whether every function has some. A function has a line and a region,
    /// but only one with a condition has a branch, so a crate that holds code
    /// and has no branch was not missed by the tool, as long as the report
    /// has its lines. A crate with no line in the report was hidden from the
    /// tool, and fails as nothing measured whatever the measure.
    in_every_function: bool,
}

/// Lines first, then regions, so each crate reads from the kinder measure to
/// the stricter one and the gap between them is on adjacent report lines.
const MEASURES: [Measure; 2] = [
    Measure {
        counts: |summary| &summary.lines,
        floor: |floor| floor.line_coverage,
        unit: "production lines covered",
        none_of: "coverable lines",
        in_every_function: true,
    },
    Measure {
        counts: |summary| &summary.regions,
        floor: |floor| floor.region_coverage,
        unit: "production regions covered",
        none_of: "coverable regions",
        in_every_function: true,
    },
];

/// What a run with `--branch` is judged on. Its lines and regions are not:
/// they are held to their floors on the stable toolchain, on every push.
const BRANCHES: [Measure; 1] = [Measure {
    counts: |summary| &summary.branches,
    floor: |floor| floor.branch_coverage,
    unit: "production branches taken",
    none_of: "branches",
    in_every_function: false,
}];

/// A report line per floor crate and measure: the counts of every file under
/// the crate's directory, summed. A crate with no file in the report has
/// nothing coverable, which the standing reads as measuring nothing.
fn lines_by_crate(export: &Export, crates: &[FloorCrate], measures: &[Measure]) -> Vec<Line> {
    crates
        .iter()
        .flat_map(|krate| {
            measures.iter().map(move |measure| {
                let (hit, total, lines) = export
                    .data
                    .iter()
                    .flat_map(|data| &data.files)
                    .filter(|file| file.filename.starts_with(&krate.directory))
                    .fold((0, 0, 0), |(hit, total, lines), file| {
                        let counts = (measure.counts)(&file.summary);
                        (
                            hit + counts.covered,
                            total + counts.count,
                            lines + file.summary.lines.count,
                        )
                    });
                Line {
                    package: krate.floor.package,
                    hit,
                    total,
                    floor: (measure.floor)(krate.floor),
                    unit: measure.unit,
                    none_of: measure.none_of,
                    holds_code: krate.holds_code && (measure.in_every_function || lines == 0),
                }
            })
        })
        .collect()
}

fn llvm_cov_args(report: &str, branch: bool) -> Vec<&str> {
    let mut args = vec!["llvm-cov", LOCKED, "--workspace"];
    if branch {
        args.push("--branch");
    }
    args.extend([
        "--ignore-filename-regex",
        floors::TEST_FILES,
        "--json",
        "--summary-only",
        "--output-path",
        report,
    ]);
    args
}

/// The floor crates, with their directories as llvm-cov reports a path.
fn floor_crates(workspace: &Workspace) -> Result<Vec<FloorCrate>, String> {
    let mut crates = floors::crates(workspace)?;
    for krate in &mut crates {
        // llvm-cov reports resolved paths, so compare against resolved ones.
        if let Ok(resolved) = krate.directory.canonicalize() {
            krate.directory = resolved;
        }
    }
    Ok(crates)
}

fn read_report(report: &Path) -> Result<Export, String> {
    let text = std::fs::read_to_string(report)
        .map_err(|e| format!("could not read {}: {e}", report.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("could not parse {}: {e}", report.display()))
}

const NOT_MEASURED: &str = "no coverage was measured (a failing test fails coverage too)";

/// Runs the workspace's tests under cargo-llvm-cov and judges the line and
/// region floors.
pub fn check() -> CheckResult {
    let workspace = Workspace::load(&workspace_root())?;
    let crates = floor_crates(&workspace)?;

    let report = floors::output_directory(&workspace.root)?.join("coverage.json");
    let report_arg = report.display().to_string();
    let args = llvm_cov_args(&report_arg, false);
    let status = process::stream("cargo", &args, &[])?;
    if !status.success() {
        return Err(format!(
            "{}; {NOT_MEASURED}",
            process::command_failed("cargo", &args)
        ));
    }
    let export = read_report(&report)?;
    floors::conclude("coverage", &lines_by_crate(&export, &crates, &MEASURES))
}

/// Runs the workspace's tests under cargo-llvm-cov on the pinned nightly and
/// judges the branch floors. It builds in a target directory of its own: in
/// the one the stable run uses, each run would rebuild what the other built.
pub fn check_branches() -> CheckResult {
    let rustup = |args: &[&str]| process::capture_in(&repo_root(), "rustup", args);
    let toolchains = rustup(&["toolchain", "list"])?;
    if !lists_toolchain(&toolchains, NIGHTLY) {
        return Err(not_installed("isn't installed"));
    }
    let components = rustup(&["component", "list", "--toolchain", NIGHTLY, "--installed"])?;
    if !lists_llvm_tools(&components) {
        return Err(not_installed(&format!("is installed without {LLVM_TOOLS}")));
    }

    let workspace = Workspace::load(&workspace_root())?;
    let crates = floor_crates(&workspace)?;
    let report = floors::output_directory(&workspace.root)?.join("branch-coverage.json");
    let report_arg = report.display().to_string();
    let target = workspace.root.join("target").join("branch-coverage");
    let target_arg = target.display().to_string();
    let args = llvm_cov_args(&report_arg, true);
    let env = [("CARGO_TARGET_DIR", target_arg.as_str())];
    let status = process::stream_on(NIGHTLY, &args, &env)?;
    if !status.success() {
        return Err(format!(
            "{}; {NOT_MEASURED}",
            process::command_failed_on(NIGHTLY, &args)
        ));
    }
    let export = read_report(&report)?;
    conclude_branches(&lines_by_crate(&export, &crates, &BRANCHES))
}

/// One crate without a branch has no condition in it. All of them without one
/// is a run that did not measure branches at all, whatever was asked of it.
fn conclude_branches(lines: &[Line]) -> CheckResult {
    if lines.iter().all(|line| line.total == 0) {
        return Err(format!(
            "coverage: no floor crate has a branch in the report, so branches were not \
             measured and no floor was applied. cargo-llvm-cov was run with `--branch` on \
             {NIGHTLY}; either the flag no longer takes there, or something hides the crates \
             from the report\n\n{}",
            floors::report(lines)
        ));
    }
    floors::conclude("coverage", lines)
}

/// Whether `rustup toolchain list` names the toolchain: by itself, or with
/// the host after it, and then with whatever rustup says of it in brackets.
fn lists_toolchain(toolchains: &str, toolchain: &str) -> bool {
    toolchains
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .any(|name| {
            name.strip_prefix(toolchain)
                .is_some_and(|host| host.is_empty() || host.starts_with('-'))
        })
}

/// Whether `rustup component list --installed` names the LLVM tools, which it
/// lists without the `-preview` and with the host.
fn lists_llvm_tools(components: &str) -> bool {
    let name = LLVM_TOOLS.trim_end_matches("-preview");
    components.lines().any(|line| line.starts_with(name))
}

/// The command that installs the pinned nightly with what coverage needs of
/// it, and nothing more. floors.yml runs the same one.
fn install_command() -> String {
    format!("rustup toolchain install {NIGHTLY} --profile minimal --component {LLVM_TOOLS}")
}

/// The failure for a nightly that can't be used. xtask installs no toolchain
/// itself: the workspace's own is pinned in `rust-toolchain.toml`, and a
/// second one on the machine is the developer's to add.
fn not_installed(what: &str) -> String {
    format!(
        "branch coverage is measured on the toolchain {NIGHTLY}, which {what}. Install it \
         with:\n\n  {}\n\nOnly this task runs on it; the workspace stays on the toolchain \
         rust-toolchain.toml pins.",
        install_command()
    )
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
            seen(&lines_by_crate(&export, &crates(false), &MEASURES)),
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
        let lines = lines_by_crate(&export, &crates(true), &MEASURES);

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
        let lines = lines_by_crate(&export, &crates(true), &MEASURES);
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
        for branch in [false, true] {
            let args = llvm_cov_args("/out/coverage.json", branch);
            assert_eq!(args[..3], ["llvm-cov", "--locked", "--workspace"]);
            assert_eq!(args.contains(&"--branch"), branch);
            let regex = args
                .iter()
                .position(|arg| *arg == "--ignore-filename-regex");
            assert_eq!(regex.map(|at| args[at + 1]), Some(r"(^|/)tests(\.rs|/)"));
            assert!(args.ends_with(&["--output-path", "/out/coverage.json"]));
        }
    }

    /// As [`REPORT`], from a run with `--branch`. `policy` has code and no
    /// condition in it; `run` has a condition that was never false.
    const BRANCH_REPORT: &str = r#"{
      "data": [{
        "files": [
          {"filename": "/ws/crates/domain/model/src/lib.rs",
           "summary": {"lines": {"count": 7, "covered": 6, "percent": 85.7},
                       "regions": {"count": 8, "covered": 5, "percent": 62.5},
                       "branches": {"count": 4, "covered": 4, "notcovered": 0, "percent": 100.0},
                       "mcdc": {"count": 0, "covered": 0, "notcovered": 0, "percent": 0.0}}},
          {"filename": "/ws/crates/domain/model/src/usage.rs",
           "summary": {"lines": {"count": 2, "covered": 2, "percent": 100.0},
                       "regions": {"count": 3, "covered": 3, "percent": 100.0},
                       "branches": {"count": 2, "covered": 2, "notcovered": 0, "percent": 100.0}}},
          {"filename": "/ws/crates/domain/policy/src/lib.rs",
           "summary": {"lines": {"count": 3, "covered": 3, "percent": 100.0},
                       "regions": {"count": 3, "covered": 3, "percent": 100.0},
                       "branches": {"count": 0, "covered": 0, "notcovered": 0, "percent": 0.0}}},
          {"filename": "/ws/crates/application/run/src/lib.rs",
           "summary": {"lines": {"count": 5, "covered": 5, "percent": 100.0},
                       "regions": {"count": 6, "covered": 6, "percent": 100.0},
                       "branches": {"count": 2, "covered": 1, "notcovered": 1, "percent": 50.0}}}
        ]
      }],
      "type": "llvm.coverage.json.export", "version": "3.1.0"
    }"#;

    /// Branches see what regions can't: every line and region of `run` ran,
    /// and one of its two branches was never taken.
    #[test]
    fn a_branch_run_judges_the_branches_and_nothing_else() {
        let export: Export = serde_json::from_str(BRANCH_REPORT).unwrap();
        let lines = lines_by_crate(&export, &crates(true), &BRANCHES);
        assert_eq!(
            seen(&lines),
            [
                // Below the line and region floors, which this run doesn't judge.
                ("lablet-model", "branches", 6, 6, Standing::Met),
                (
                    "lablet-policy",
                    "branches",
                    0,
                    0,
                    Standing::NothingToMeasure
                ),
                ("lablet-run", "branches", 1, 2, Standing::Below),
            ]
        );
        assert_eq!(
            lines[0].to_string(),
            "lablet-model: 100.0% (6 of 6 production branches taken), meets the 100% floor"
        );
        let error = conclude_branches(&lines).unwrap_err();
        assert!(
            error.starts_with("coverage: 1 floor(s) below or unmeasured"),
            "{error}"
        );
        assert!(
            error.contains("lablet-run: 50.0% (1 of 2 production branches taken), BELOW"),
            "{error}"
        );
    }

    #[test]
    fn a_crate_without_a_branch_passes_but_a_run_without_any_did_not_measure_them() {
        let export: Export = serde_json::from_str(BRANCH_REPORT).unwrap();
        let mut lines = lines_by_crate(&export, &crates(true), &BRANCHES);
        lines.pop();
        assert_eq!(
            conclude_branches(&lines).unwrap().as_deref(),
            Some("1 of 2 floor(s) had nothing to measure yet")
        );

        // A report from a run without `--branch` has no branch counts at all.
        let export: Export = serde_json::from_str(REPORT).unwrap();
        let lines = lines_by_crate(&export, &crates(true), &BRANCHES);
        assert!(lines.iter().all(|line| line.total == 0));
        let error = conclude_branches(&lines).unwrap_err();
        for phrase in [
            "no floor crate has a branch in the report",
            "`--branch` on nightly-",
            "  lablet-run: nothing to measure yet (no branches); floor 100%",
        ] {
            assert!(error.contains(phrase), "{error}");
        }
    }

    /// A crate with no line in the report was hidden from the tool, which a
    /// branch run must not take for a crate without a condition.
    #[test]
    fn a_crate_missing_from_a_branch_report_is_nothing_measured() {
        let mut export: Export = serde_json::from_str(BRANCH_REPORT).unwrap();
        export.data[0]
            .files
            .retain(|file| !file.filename.starts_with("/ws/crates/application/run"));
        let lines = lines_by_crate(&export, &crates(true), &BRANCHES);
        assert_eq!(
            seen(&lines)[1..],
            [
                (
                    "lablet-policy",
                    "branches",
                    0,
                    0,
                    Standing::NothingToMeasure
                ),
                ("lablet-run", "branches", 0, 0, Standing::NothingMeasured),
            ]
        );
        let error = conclude_branches(&lines).unwrap_err();
        assert!(
            error.contains("lablet-run: NOTHING MEASURED (no branches)"),
            "{error}"
        );
    }

    #[test]
    fn the_pinned_nightly_is_found_by_name_whatever_host_follows_it() {
        let installed = "stable-aarch64-apple-darwin (default)\n\
                         nightly-aarch64-apple-darwin\n\
                         nightly-2026-08-25-aarch64-apple-darwin\n\
                         1.98.1-aarch64-apple-darwin (active)\n";
        assert!(lists_toolchain(installed, "nightly-2026-08-25"));
        assert!(lists_toolchain(installed, "nightly"));
        assert!(lists_toolchain(installed, "1.98.1"));
        assert!(lists_toolchain(
            "nightly-2026-08-25 (active)\n",
            "nightly-2026-08-25"
        ));
        // Another day's nightly, and a name this one only begins.
        assert!(!lists_toolchain(installed, "nightly-2026-08-2"));
        assert!(!lists_toolchain(installed, "nightly-2026-08-26"));
        assert!(!lists_toolchain(installed, "1.98"));
        assert!(!lists_toolchain(
            "no installed toolchains\n",
            "nightly-2026-08-25"
        ));

        let components = "cargo-aarch64-apple-darwin\nllvm-tools-aarch64-apple-darwin\n\
                          rust-std-aarch64-apple-darwin\nrustc-aarch64-apple-darwin\n";
        assert!(lists_llvm_tools(components));
        assert!(lists_llvm_tools(
            "llvm-tools-preview-x86_64-unknown-linux-gnu\n"
        ));
        assert!(!lists_llvm_tools(
            "cargo-aarch64-apple-darwin\nrustc-aarch64-apple-darwin\n"
        ));
    }

    #[test]
    fn a_missing_nightly_fails_with_the_command_that_installs_it() {
        assert_eq!(
            not_installed("isn't installed"),
            "branch coverage is measured on the toolchain nightly-2026-08-25, which isn't \
             installed. Install it with:\n\n  rustup toolchain install nightly-2026-08-25 \
             --profile minimal --component llvm-tools-preview\n\nOnly this task runs on it; the \
             workspace stays on the toolchain rust-toolchain.toml pins."
        );
        let partly = not_installed("is installed without llvm-tools-preview");
        assert!(
            partly.contains(&format!("\n  {}\n", install_command())),
            "{partly}"
        );
    }

    /// The nightly is named in two places, here and in the workflow that
    /// measures branches daily. A pin bumped in one alone would have CI
    /// install a toolchain xtask then says is missing.
    #[test]
    fn the_floors_workflow_installs_the_nightly_this_crate_pins() {
        let path = repo_root().join(".github/workflows/floors.yml");
        let workflow = std::fs::read_to_string(&path).unwrap();
        let pins: Vec<&str> = workflow
            .lines()
            .filter_map(|line| line.trim().strip_prefix("NIGHTLY: "))
            .collect();
        assert_eq!(
            pins,
            [NIGHTLY],
            "floors.yml sets NIGHTLY once, to the toolchain xtask/src/coverage.rs pins"
        );
        let install = install_command().replace(NIGHTLY, "\"$NIGHTLY\"");
        assert!(
            workflow
                .lines()
                .any(|line| line.trim() == format!("run: {install}")),
            "floors.yml does not run `{install}`"
        );
        // A second name for it, spelt out, would not move with the first.
        for line in workflow.lines().filter(|line| line.contains("nightly-")) {
            assert_eq!(line.trim(), format!("NIGHTLY: {NIGHTLY}"), "{line}");
        }
        let date = NIGHTLY.strip_prefix("nightly-").unwrap_or_default();
        let digits: Vec<&str> = date.split('-').collect();
        assert!(
            matches!(digits[..], [year, month, day] if year.len() == 4 && month.len() == 2 && day.len() == 2)
                && date.bytes().all(|b| b.is_ascii_digit() || b == b'-'),
            "{NIGHTLY} is not a nightly pinned by date"
        );
    }
}
