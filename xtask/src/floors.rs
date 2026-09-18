//! The coverage and mutation floors, as data (quality-bar item 12). Domain
//! and application crates carry the floors; adapters are held by conformance
//! suites and recorded HTTP tests instead of a number.

use std::fmt;
use std::path::PathBuf;

use crate::workspace::Workspace;

/// The floors one crate is held to.
#[derive(Debug, Clone, Copy)]
pub struct Floor {
    /// The package name, as in the crate's `Cargo.toml`.
    pub package: &'static str,
    /// The least share of lines, in percent, its tests must execute.
    pub line_coverage: u64,
    /// The least share of viable mutants, in percent, its tests must catch.
    pub mutants_caught: u64,
}

/// Every crate with a floor. Changing a number or the list is a decision:
/// it is stated in contributing/README.md and product/quality-bar.md too.
pub const FLOORS: &[Floor] = &[
    Floor {
        package: "lablet-model",
        line_coverage: 90,
        mutants_caught: 80,
    },
    Floor {
        package: "lablet-policy",
        line_coverage: 90,
        mutants_caught: 80,
    },
    Floor {
        package: "lablet-run",
        line_coverage: 90,
        mutants_caught: 80,
    },
];

/// How one crate fared against one floor.
#[derive(Debug, PartialEq, Eq)]
pub enum Standing {
    /// At or above the floor.
    Met,
    /// Nothing to measure: no coverable lines, or no viable mutants. An empty
    /// crate passes, with a note, so the scaffold can run the gate.
    NothingToMeasure,
    /// Below the floor.
    Below,
}

/// One crate's result line in a floor report.
#[derive(Debug)]
pub struct Line {
    /// The package name.
    pub package: &'static str,
    /// How many units (lines, mutants) counted towards the floor.
    pub hit: u64,
    /// How many units there were.
    pub total: u64,
    /// The floor, in percent.
    pub floor: u64,
    /// What `hit` counts, for the wording: "lines covered", "mutants caught".
    pub unit: &'static str,
    /// What there was none of when `total` is zero: "coverable lines".
    pub none_of: &'static str,
}

impl Line {
    /// Judges `hit` of `total` against the floor in exact integer arithmetic.
    pub fn standing(&self) -> Standing {
        if self.total == 0 {
            Standing::NothingToMeasure
        } else if self.hit * 100 >= self.floor * self.total {
            Standing::Met
        } else {
            Standing::Below
        }
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            package,
            hit,
            total,
            floor,
            unit,
            none_of,
        } = self;
        match self.standing() {
            Standing::NothingToMeasure => {
                write!(
                    f,
                    "{package}: nothing to measure yet (no {none_of}); floor {floor}%"
                )
            }
            standing => {
                // Tenths of a percent, rounded down so a miss never reads as the floor.
                let tenths = hit * 1000 / total;
                let verdict = if standing == Standing::Met {
                    "meets"
                } else {
                    "BELOW"
                };
                write!(
                    f,
                    "{package}: {}.{}% ({hit} of {total} {unit}), {verdict} the {floor}% floor",
                    tenths / 10,
                    tenths % 10
                )
            }
        }
    }
}

/// Turns a floor report into a step result: every line is shown either way,
/// and any crate below its floor fails the step.
pub fn conclude(what: &str, lines: &[Line]) -> crate::gates::CheckResult {
    let report = lines
        .iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let below = lines
        .iter()
        .filter(|line| line.standing() == Standing::Below)
        .count();
    if below > 0 {
        return Err(format!(
            "{what}: {below} crate(s) below the floor\n\n{report}"
        ));
    }
    println!("{report}");
    let unmeasured = lines
        .iter()
        .filter(|line| line.standing() == Standing::NothingToMeasure)
        .count();
    Ok((unmeasured > 0).then(|| {
        format!(
            "{unmeasured} of {} crate(s) had nothing to measure yet",
            lines.len()
        )
    }))
}

/// Each floor crate's directory in the workspace. A floor naming a crate the
/// workspace does not have is an error: the list must not rot silently.
pub fn crate_directories(workspace: &Workspace) -> Result<Vec<(&'static Floor, PathBuf)>, String> {
    FLOORS
        .iter()
        .map(|floor| {
            workspace
                .member_named(floor.package)
                .map(|member| (floor, workspace.root.join(&member.path)))
                .ok_or_else(|| {
                    format!(
                        "xtask/src/floors.rs sets a floor for `{}`, but the workspace has no \
                         such package",
                        floor.package
                    )
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(hit: u64, total: u64, floor: u64) -> Line {
        Line {
            package: "lablet-model",
            hit,
            total,
            floor,
            unit: "lines covered",
            none_of: "coverable lines",
        }
    }

    #[test]
    fn the_floors_are_ninety_and_eighty_on_the_domain_and_application_crates() {
        let packages: Vec<&str> = FLOORS.iter().map(|floor| floor.package).collect();
        assert_eq!(packages, ["lablet-model", "lablet-policy", "lablet-run"]);
        assert!(FLOORS.iter().all(|floor| floor.line_coverage == 90));
        assert!(FLOORS.iter().all(|floor| floor.mutants_caught == 80));
    }

    #[test]
    fn exactly_the_floor_meets_it_and_one_unit_less_does_not() {
        assert_eq!(line(90, 100, 90).standing(), Standing::Met);
        assert_eq!(line(89, 100, 90).standing(), Standing::Below);
        assert_eq!(line(9, 10, 90).standing(), Standing::Met);
        // 899 of 1000 is 89.9%: no rounding up to the floor.
        assert_eq!(line(899, 1000, 90).standing(), Standing::Below);
        assert_eq!(line(7, 7, 90).standing(), Standing::Met);
    }

    #[test]
    fn nothing_to_measure_is_its_own_standing() {
        assert_eq!(line(0, 0, 90).standing(), Standing::NothingToMeasure);
    }

    #[test]
    fn a_line_reads_as_a_percentage_with_its_counts_and_the_floor() {
        assert_eq!(
            line(899, 1000, 90).to_string(),
            "lablet-model: 89.9% (899 of 1000 lines covered), BELOW the 90% floor"
        );
        assert_eq!(
            line(2, 3, 50).to_string(),
            "lablet-model: 66.6% (2 of 3 lines covered), meets the 50% floor"
        );
        assert_eq!(
            line(0, 0, 90).to_string(),
            "lablet-model: nothing to measure yet (no coverable lines); floor 90%"
        );
    }

    #[test]
    fn one_crate_below_its_floor_fails_the_report_and_every_line_is_shown() {
        let error = conclude("coverage", &[line(95, 100, 90), line(10, 100, 90)]).unwrap_err();
        assert!(
            error.starts_with("coverage: 1 crate(s) below the floor"),
            "{error}"
        );
        assert!(error.contains("95.0%"), "{error}");
        assert!(error.contains("10.0%"), "{error}");
    }

    #[test]
    fn an_empty_crate_passes_with_a_note() {
        let note = conclude("coverage", &[line(95, 100, 90), line(0, 0, 90)]).unwrap();
        assert_eq!(
            note.as_deref(),
            Some("1 of 2 crate(s) had nothing to measure yet")
        );
        assert_eq!(conclude("coverage", &[line(95, 100, 90)]).unwrap(), None);
    }
}
