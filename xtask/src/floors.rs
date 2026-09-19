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
    /// Whether the crate is still a shell with no function in it. Only then
    /// may a run measure nothing and pass; for a crate that holds code,
    /// nothing measured means a filter or a path remap hid it, which fails.
    /// A test fails once a `may_be_empty` crate gains a function, so the flag
    /// is set to `false` in the commit that adds the first one.
    pub may_be_empty: bool,
}

/// Every crate with a floor. Changing a number or the list is a decision:
/// it is stated in contributing/README.md and product/quality-bar.md too.
pub const FLOORS: &[Floor] = &[
    Floor {
        package: "lablet-model",
        line_coverage: 90,
        mutants_caught: 80,
        may_be_empty: true,
    },
    Floor {
        package: "lablet-policy",
        line_coverage: 90,
        mutants_caught: 80,
        may_be_empty: true,
    },
    Floor {
        package: "lablet-run",
        line_coverage: 90,
        mutants_caught: 80,
        may_be_empty: true,
    },
];

/// How one crate fared against one floor.
#[derive(Debug, PartialEq, Eq)]
pub enum Standing {
    /// At or above the floor.
    Met,
    /// Nothing to measure: no coverable lines, or no viable mutants, in a
    /// crate that is still an empty shell ([`Floor::may_be_empty`]). It
    /// passes, with a note, so the scaffold can run the gate.
    NothingToMeasure,
    /// Nothing was measured in a crate that holds code, so the floor was not
    /// applied at all. A failure: a filter or a path remap hid the crate.
    NothingMeasured,
    /// Below the floor.
    Below,
}

impl Standing {
    /// Whether this standing fails the step.
    pub fn fails(&self) -> bool {
        matches!(self, Self::NothingMeasured | Self::Below)
    }
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
    /// Whether `total` may be zero; see [`Floor::may_be_empty`].
    pub may_be_empty: bool,
}

impl Line {
    /// Judges `hit` of `total` against the floor in exact integer arithmetic.
    pub fn standing(&self) -> Standing {
        if self.total == 0 && self.may_be_empty {
            Standing::NothingToMeasure
        } else if self.total == 0 {
            Standing::NothingMeasured
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
            may_be_empty: _,
        } = self;
        match self.standing() {
            Standing::NothingToMeasure => {
                write!(
                    f,
                    "{package}: nothing to measure yet (no {none_of}); floor {floor}%"
                )
            }
            Standing::NothingMeasured => {
                write!(
                    f,
                    "{package}: NOTHING MEASURED (no {none_of}) in a crate that holds code, so \
                     the {floor}% floor was not applied; look for a filter that hides the crate \
                     (.cargo/mutants.toml, an ignore regex) or a path remap \
                     (--remap-path-prefix)"
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
/// and any crate below its floor, or unmeasured when it holds code, fails the
/// step.
pub fn conclude(what: &str, lines: &[Line]) -> crate::gates::CheckResult {
    let report = lines
        .iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let failed = lines.iter().filter(|line| line.standing().fails()).count();
    if failed > 0 {
        return Err(format!(
            "{what}: {failed} crate(s) below the floor\n\n{report}"
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

/// `target/xtask` under the workspace, created: where the floor checks keep
/// their tools' reports. Git ignores it and `cargo clean` removes it. On a
/// fresh clone or a CI runner `target/` does not exist yet, and cargo-mutants
/// does not create the parent of its output directory.
pub fn output_directory(workspace_root: &std::path::Path) -> Result<PathBuf, String> {
    let directory = workspace_root.join("target").join("xtask");
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("could not create {}: {e}", directory.display()))?;
    Ok(directory)
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
            may_be_empty: true,
        }
    }

    /// The same result in a crate that holds code.
    fn holding_code(hit: u64, total: u64, floor: u64) -> Line {
        Line {
            may_be_empty: false,
            ..line(hit, total, floor)
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
    fn nothing_measured_in_a_crate_that_holds_code_fails() {
        let unmeasured = holding_code(0, 0, 90);
        assert_eq!(unmeasured.standing(), Standing::NothingMeasured);
        assert!(unmeasured.to_string().contains("NOTHING MEASURED"));
        let error = conclude("mutants", &[line(0, 0, 90), unmeasured]).unwrap_err();
        assert!(
            error.starts_with("mutants: 1 crate(s) below the floor"),
            "{error}"
        );
        // With something to measure the flag changes nothing.
        assert_eq!(holding_code(9, 10, 90).standing(), Standing::Met);
        assert_eq!(holding_code(8, 10, 90).standing(), Standing::Below);
    }

    /// The allowance expires by itself: the first function in a floor crate
    /// fails this test until the crate's `may_be_empty` is set to `false`,
    /// after which a run that measures nothing for it fails the gate.
    #[test]
    fn a_crate_that_may_be_empty_holds_no_function() {
        let workspace = Workspace::load(&crate::workspace::workspace_root()).unwrap();
        for (floor, directory) in crate_directories(&workspace).unwrap() {
            if !floor.may_be_empty {
                continue;
            }
            let mut pending = vec![directory.join("src")];
            while let Some(path) = pending.pop() {
                if path.is_dir() {
                    pending.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    let function = text
                        .lines()
                        .map(str::trim_start)
                        .find(|line| !line.starts_with("//") && line.contains("fn "));
                    assert_eq!(
                        function,
                        None,
                        "{} holds a function, so `{}` is no longer an empty shell: set its \
                         `may_be_empty` to false in xtask/src/floors.rs",
                        path.display(),
                        floor.package
                    );
                }
            }
        }
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
