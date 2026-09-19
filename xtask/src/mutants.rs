//! `cargo xtask mutants`: mutation testing from cargo-mutants, held to the
//! floors in [`crate::floors`]. Each mutant runs its own package's tests, so a
//! crate is judged on the tests it carries.
//!
//! The score is caught over viable: caught, missed, and timed-out mutants
//! count; a mutant that does not build does not. A timeout counts against the
//! score because nothing showed the tests would have failed.

use serde::Deserialize;
use std::path::Path;

use crate::floors::{self, FLOORS, FloorCrate, Line};
use crate::gates::CheckResult;
use crate::process;
use crate::workspace::{Workspace, workspace_root};

/// The part of cargo-mutants' `outcomes.json` that is read.
#[derive(Debug, Deserialize)]
struct Outcomes {
    outcomes: Vec<Outcome>,
}

#[derive(Debug, Deserialize)]
struct Outcome {
    scenario: Scenario,
    summary: String,
}

/// `"Baseline"` for the unmutated run, `{"Mutant": {..}}` for each mutant.
#[derive(Debug, Deserialize)]
enum Scenario {
    Baseline,
    Mutant(Mutant),
}

#[derive(Debug, Deserialize)]
struct Mutant {
    package: String,
}

fn lines_by_crate(outcomes: &Outcomes, crates: &[FloorCrate]) -> Vec<Line> {
    crates
        .iter()
        .map(|krate| {
            let floor = krate.floor;
            let mut caught = 0;
            let mut viable = 0;
            for outcome in &outcomes.outcomes {
                let Scenario::Mutant(mutant) = &outcome.scenario else {
                    continue;
                };
                if mutant.package != floor.package {
                    continue;
                }
                match outcome.summary.as_str() {
                    "CaughtMutant" => {
                        caught += 1;
                        viable += 1;
                    }
                    "MissedMutant" | "Timeout" => viable += 1,
                    // "Unviable": the mutant did not build, so it tested nothing.
                    _ => {}
                }
            }
            Line {
                package: floor.package,
                hit: caught,
                total: viable,
                floor: floor.mutants_caught,
                unit: "mutants caught",
                none_of: "viable mutants",
                holds_code: krate.holds_code,
            }
        })
        .collect()
}

/// cargo-mutants' exit codes that still leave a full set of outcomes: all
/// caught, some missed, some timed out. Anything else means it did not test.
const EXITS_WITH_OUTCOMES: [i32; 3] = [0, 2, 3];

/// cargo-mutants hands `--cargo-arg` to every cargo command it runs, so the
/// builds in its copies of the tree hold to the committed lockfile too.
fn mutants_args(output: &str) -> Vec<&str> {
    let mut args = vec!["mutants", "--cargo-arg=--locked", "--output", output];
    for floor in FLOORS {
        args.extend(["--package", floor.package]);
    }
    args
}

/// Runs cargo-mutants over the floor crates and judges the floors.
pub fn check() -> CheckResult {
    let workspace = Workspace::load(&workspace_root())?;
    let crates = floors::crates(&workspace)?;

    let output = floors::output_directory(&workspace.root)?;
    let output_arg = output.display().to_string();
    let args = mutants_args(&output_arg);
    let status = process::stream("cargo", &args, &[])?;
    if !status
        .code()
        .is_some_and(|code| EXITS_WITH_OUTCOMES.contains(&code))
    {
        return Err(format!(
            "{}; no mutants were tested (the tests must pass unmutated first)",
            process::command_failed("cargo", &args)
        ));
    }
    let outcomes = read_outcomes(&output.join("mutants.out").join("outcomes.json"))?;
    floors::conclude("mutants", &lines_by_crate(&outcomes, &crates))
}

/// cargo-mutants writes no `outcomes.json` when it finds nothing to mutate,
/// so a missing file is an empty run, not an error; the floors then fail any
/// crate that defines a function.
fn read_outcomes(path: &Path) -> Result<Outcomes, String> {
    if !path.is_file() {
        return Ok(Outcomes {
            outcomes: Vec::new(),
        });
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("could not parse {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::floors::Standing;

    const OUTCOMES: &str = r#"{
      "outcomes": [
        {"scenario": "Baseline", "summary": "Success", "log_path": "log/baseline.log"},
        {"scenario": {"Mutant": {"name": "a", "package": "lablet-model", "file": "src/lib.rs",
                                 "genre": "FnValue"}}, "summary": "CaughtMutant"},
        {"scenario": {"Mutant": {"name": "b", "package": "lablet-model"}}, "summary": "CaughtMutant"},
        {"scenario": {"Mutant": {"name": "c", "package": "lablet-model"}}, "summary": "CaughtMutant"},
        {"scenario": {"Mutant": {"name": "d", "package": "lablet-model"}}, "summary": "CaughtMutant"},
        {"scenario": {"Mutant": {"name": "e", "package": "lablet-model"}}, "summary": "MissedMutant"},
        {"scenario": {"Mutant": {"name": "f", "package": "lablet-model"}}, "summary": "Unviable"},
        {"scenario": {"Mutant": {"name": "g", "package": "lablet-run"}}, "summary": "CaughtMutant"},
        {"scenario": {"Mutant": {"name": "h", "package": "lablet-run"}}, "summary": "Timeout"},
        {"scenario": {"Mutant": {"name": "i", "package": "lablet-other"}}, "summary": "MissedMutant"}
      ],
      "total_mutants": 9, "missed": 2, "caught": 5, "timeout": 1, "unviable": 1, "success": 0
    }"#;

    fn crates(holds_code: bool) -> Vec<FloorCrate> {
        FLOORS
            .iter()
            .map(|floor| FloorCrate {
                floor,
                directory: std::path::PathBuf::new(),
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
    fn the_score_is_caught_over_viable_per_crate() {
        let outcomes: Outcomes = serde_json::from_str(OUTCOMES).unwrap();
        assert_eq!(
            seen(&lines_by_crate(&outcomes, &crates(true))),
            [
                // 4 of 5: the unviable mutant is left out, and 80% meets 80%.
                ("lablet-model", 4, 5, Standing::Met),
                // It defines a function, and no mutant of it was tested.
                ("lablet-policy", 0, 0, Standing::NothingMeasured),
                // 1 of 2: the timeout counts against the score.
                ("lablet-run", 1, 2, Standing::Below),
            ]
        );
    }

    #[test]
    fn only_the_floor_crates_are_mutated_against_the_committed_lockfile() {
        let args = mutants_args("/out");
        assert_eq!(
            args[..4],
            ["mutants", "--cargo-arg=--locked", "--output", "/out"]
        );
        let packages: Vec<&str> = args[4..].chunks(2).map(|pair| pair[1]).collect();
        assert_eq!(packages, ["lablet-model", "lablet-policy", "lablet-run"]);
        assert!(args[4..].chunks(2).all(|pair| pair[0] == "--package"));
    }

    #[test]
    fn a_run_that_found_no_mutants_passes_only_for_crates_that_define_no_function() {
        let outcomes = read_outcomes(Path::new("/nonexistent/mutants.out/outcomes.json")).unwrap();
        for (holds_code, standing) in [
            (false, Standing::NothingToMeasure),
            (true, Standing::NothingMeasured),
        ] {
            let lines = lines_by_crate(&outcomes, &crates(holds_code));
            assert!(lines.iter().all(|line| line.standing() == standing));
        }
    }
}
