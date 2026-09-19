//! The coverage and mutation floors, as data (quality-bar item 12). Domain
//! and application crates carry the floors; adapters are held by conformance
//! suites and recorded HTTP tests instead of a number.
//!
//! Two things keep a floor honest without parsing Rust. A crate may measure
//! nothing only while its `src/` defines no function. And test code is told
//! from production code by file name: in a floor crate unit tests live in a
//! sibling file declared as `#[cfg(test)] mod tests;` (`src/foo.rs` and
//! `src/foo/tests.rs`, or `src/tests.rs`), which coverage leaves out with
//! [`TEST_FILES`]. [`crates`] fails a crate whose production files carry any
//! other attribute that mentions `test` (`#[test]`, `#[cfg(all(test, ..))]`),
//! since those lines would count as covered production code.

use std::fmt;
use std::path::{Path, PathBuf};

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

/// The files that hold test code, as a regex over a source path: a `tests.rs`
/// file, or anything under a `tests/` directory.
pub const TEST_FILES: &str = r"(^|/)tests(\.rs|/)";

/// How one crate fared against one floor.
#[derive(Debug, PartialEq, Eq)]
pub enum Standing {
    /// At or above the floor.
    Met,
    /// Nothing to measure (no coverable lines, no viable mutants) in a crate
    /// that defines no function yet. It passes, with a note, so the scaffold
    /// can run the gate.
    NothingToMeasure,
    /// Nothing was measured in a crate that defines a function, so the floor
    /// was not applied at all. A failure: something hid the crate, or no
    /// function in it has a body the tool can measure.
    NothingMeasured,
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
    /// Whether `total` must not be zero; see [`FloorCrate::holds_code`].
    pub holds_code: bool,
}

impl Line {
    /// Judges `hit` of `total` against the floor in exact integer arithmetic.
    pub fn standing(&self) -> Standing {
        if self.total == 0 && self.holds_code {
            Standing::NothingMeasured
        } else if self.total == 0 {
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
            holds_code: _,
        } = self;
        match self.standing() {
            Standing::NothingToMeasure => write!(
                f,
                "{package}: nothing to measure yet (no {none_of}); floor {floor}%"
            ),
            Standing::NothingMeasured => write!(
                f,
                "{package}: NOTHING MEASURED (no {none_of}) though its src/ defines a function, \
                 so the {floor}% floor was not applied. Either something hides the crate from \
                 the tool (a filter, an ignore pattern, rewritten paths), or no function in \
                 src/ is one the tool can measure yet (trait methods without a body, only \
                 unviable mutants): land the first function with a body together with its test"
            ),
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
/// and any crate below its floor, or unmeasured though it defines a function,
/// fails the step.
pub fn conclude(what: &str, lines: &[Line]) -> crate::gates::CheckResult {
    let report = lines
        .iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let count = |standings: &[Standing]| {
        lines
            .iter()
            .filter(|line| standings.contains(&line.standing()))
            .count()
    };
    let failed = count(&[Standing::Below, Standing::NothingMeasured]);
    if failed > 0 {
        return Err(format!(
            "{what}: {failed} crate(s) below the floor or unmeasured\n\n{report}"
        ));
    }
    println!("{report}");
    let unmeasured = count(&[Standing::NothingToMeasure]);
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
pub fn output_directory(workspace_root: &Path) -> Result<PathBuf, String> {
    let directory = workspace_root.join("target").join("xtask");
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("could not create {}: {e}", directory.display()))?;
    Ok(directory)
}

/// A floor crate as it stands in the workspace.
#[derive(Debug)]
pub struct FloorCrate {
    /// Its floors.
    pub floor: &'static Floor,
    /// Its directory.
    pub directory: PathBuf,
    /// Whether a production file under `src/` defines a function. From then
    /// on, a run that measures nothing for the crate fails.
    pub holds_code: bool,
}

/// Every floor crate, read from disk. The errors: a floor naming a crate the
/// workspace does not have (the list must not rot silently), a source tree
/// that cannot be read, and test code in a production file.
pub fn crates(workspace: &Workspace) -> Result<Vec<FloorCrate>, String> {
    let mut crates = Vec::new();
    let mut misplaced = Vec::new();
    for floor in FLOORS {
        let member = workspace.member_named(floor.package).ok_or_else(|| {
            format!(
                "xtask/src/floors.rs sets a floor for `{}`, but the workspace has no such package",
                floor.package
            )
        })?;
        let directory = workspace.root.join(&member.path);
        let mut holds_code = false;
        for file in production_sources(&directory.join("src"))? {
            let source = std::fs::read_to_string(&file)
                .map_err(|e| format!("could not read {}: {e}", file.display()))?;
            holds_code |= source.lines().any(defines_a_function);
            if let Some(line) = misplaced_test_code(&source) {
                let relative = file.strip_prefix(&workspace.root).unwrap_or(&file);
                misplaced.push(format!("  {}:{line}", relative.display()));
            }
        }
        crates.push(FloorCrate {
            floor,
            directory,
            holds_code,
        });
    }
    if misplaced.is_empty() {
        return Ok(crates);
    }
    Err(format!(
        "test code in a production file of a crate with a coverage floor:\n\n{}\n\nIn these \
         crates unit tests live in a sibling file (src/foo.rs and src/foo/tests.rs, or \
         src/tests.rs) declared as `#[cfg(test)] mod tests;`, with any helper modules below that \
         file. Every other attribute that mentions `test` is refused outside those files: \
         `#[test]`, `#[tokio::test]`, `#[cfg(test)]` on anything else (an inline module, a \
         `#[path]` module, an item), `#[cfg(all(test, ..))]`, `#[cfg_attr(test, ..)]`, \
         `#[cfg(not(test))]`. Coverage tells test code from production code by file name, so \
         such lines would count as covered production lines, or not be measured at all.",
        misplaced.join("\n")
    ))
}

/// The `.rs` files under the source directory `src` that [`TEST_FILES`] does
/// not name, in path order.
fn production_sources(src: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![src.to_path_buf()];
    while let Some(path) = pending.pop() {
        let name = path.file_name().unwrap_or_default();
        if name == "tests" || name == "tests.rs" {
            continue;
        }
        if path.is_dir() || path == src {
            let entries = std::fs::read_dir(&path)
                .map_err(|e| format!("could not read {}: {e}", path.display()))?;
            pending.extend(entries.flatten().map(|entry| entry.path()));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Whether a source line defines or declares a function: `fn`, a name, then
/// `(` or `<`, outside a line comment. A string that reads like one counts
/// too, which only asks for a measurement sooner.
fn defines_a_function(line: &str) -> bool {
    let line = line.trim_start();
    !line.starts_with("//")
        && line.match_indices("fn ").any(|(at, _)| {
            let rest = &line[at + 3..];
            let after = rest.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_');
            after.len() < rest.len() && after.starts_with(['(', '<'])
        })
}

/// The 1-based line of the first attribute that mentions `test` and is not
/// `#[cfg(test)]` directly on `mod tests;`. An attribute is a line that starts
/// with `#[` or `#![`, and it mentions `test` when that is one of its words, so
/// `#[test]`, `#[tokio::test]`, and `#[cfg(all(test, unix))]` do and
/// `#[serde(rename = "latest")]` does not. The item is the rest of the
/// attribute's line, or the next line that is not blank or a comment; another
/// attribute in between (`#[path = ".."]`) is refused with the rest.
fn misplaced_test_code(source: &str) -> Option<usize> {
    let mut lines = source.lines().map(str::trim).enumerate();
    while let Some((at, line)) = lines.next() {
        let mut words = line.split(|c: char| !(c.is_alphanumeric() || c == '_'));
        if !(line.starts_with("#[") || line.starts_with("#![")) || !words.any(|w| w == "test") {
            continue;
        }
        let item = match line.strip_prefix("#[cfg(test)]").map(str::trim) {
            Some("") => lines
                .by_ref()
                .map(|(_, next)| next)
                .find(|next| !(next.is_empty() || next.starts_with("//"))),
            rest => rest,
        };
        if item != Some("mod tests;") {
            return Some(at + 1);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::FixtureWorkspace;

    fn line(hit: u64, total: u64, floor: u64) -> Line {
        Line {
            package: "lablet-model",
            hit,
            total,
            floor,
            unit: "lines covered",
            none_of: "coverable lines",
            holds_code: false,
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
        // 899 of 1000 is 89.9%: no rounding up to the floor.
        assert_eq!(line(899, 1000, 90).standing(), Standing::Below);
        assert_eq!(line(7, 7, 90).standing(), Standing::Met);
    }

    #[test]
    fn nothing_measured_passes_only_while_the_crate_defines_no_function() {
        let empty = line(0, 0, 90);
        assert_eq!(empty.standing(), Standing::NothingToMeasure);
        let note = conclude("coverage", &[line(95, 100, 90), empty]).unwrap();
        assert_eq!(
            note.as_deref(),
            Some("1 of 2 crate(s) had nothing to measure yet")
        );
        assert_eq!(conclude("coverage", &[line(95, 100, 90)]).unwrap(), None);

        let hidden = Line {
            holds_code: true,
            ..line(0, 0, 90)
        };
        assert_eq!(hidden.standing(), Standing::NothingMeasured);
        let error = conclude("mutants", &[line(0, 0, 90), hidden]).unwrap_err();
        assert!(
            error.starts_with("mutants: 1 crate(s) below the floor or unmeasured"),
            "{error}"
        );
        // Both causes are named: a hidden crate, and nothing measurable yet.
        for phrase in ["NOTHING MEASURED", "hides the crate", "without a body"] {
            assert!(error.contains(phrase), "{error}");
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
    fn a_function_is_fn_then_a_name_then_its_parameters() {
        for line in [
            "fn add(a: u8) -> u8 {",
            "    pub(crate) const fn new() -> Self {",
            "pub async fn run<P: Provider>(",
            "    fn name(&self) -> &str;",
        ] {
            assert!(defines_a_function(line), "{line}");
        }
        for line in [
            "//! Types and pure functions only.",
            "    // fn later() {}",
            "/// Calls `fn main()`.",
            "pub struct Usage {",
            "    pub callback: fn (u8) -> u8,",
            "type Hook = Box<dyn Fn(u8)>;",
        ] {
            assert!(!defines_a_function(line), "{line}");
        }
    }

    #[test]
    fn an_attribute_that_mentions_test_belongs_on_mod_tests_and_nothing_else() {
        for fine in [
            "pub fn add() {}\n\n#[cfg(test)]\nmod tests;\n",
            "#[cfg(test)] mod tests;\n",
            "#[cfg(test)]\n// The unit tests.\n\nmod tests;\n",
            "#[derive(Debug)]\n#[serde(rename = \"latest\")]\nstruct Attested;\n",
            "// #[test]\nfn real() {}\n",
        ] {
            assert_eq!(misplaced_test_code(fine), None, "{fine}");
        }
        for (misplaced, line) in [
            ("pub fn add() {}\n\n#[cfg(test)]\nmod tests {\n}\n", 3),
            ("#[cfg(test)] mod tests {}\n", 1),
            // Not named `tests`, so coverage would count src/fixture.rs.
            ("#[cfg(test)]\nmod fixture;\n", 1),
            ("#[cfg(test)]\n#[path = \"unit.rs\"]\nmod tests;\n", 1),
            (
                "#[cfg(test)]\nmod tests;\n\n#[cfg(test)]\nfn helper() {}\n",
                4,
            ),
            ("#![cfg(test)]\n", 1),
            // Test code that never says `#[cfg(test)]`.
            ("pub fn add() {}\n\n#[test]\nfn adds() {}\n", 3),
            ("#[tokio::test]\nasync fn runs() {}\n", 1),
            ("#[cfg(all(test, unix))]\nmod unit {\n}\n", 1),
            ("#[cfg_attr(test, derive(PartialEq))]\nstruct Usage;\n", 1),
            // Code the tests never build is code coverage never sees.
            ("#[cfg(not(test))]\nfn real() {}\n", 1),
        ] {
            assert_eq!(misplaced_test_code(misplaced), Some(line), "{misplaced}");
        }
    }

    /// A workspace holding the three floor crates, `lablet-model` with `files`
    /// under its `src/`.
    fn crates_with(files: &[(&str, &str)]) -> Result<Vec<FloorCrate>, String> {
        let mut workspace = FixtureWorkspace::new("");
        for (path, name) in [
            ("crates/domain/model", "lablet-model"),
            ("crates/domain/policy", "lablet-policy"),
            ("crates/application/run", "lablet-run"),
        ] {
            workspace = workspace.member(path, name, "");
            workspace.write(&format!("{path}/src/lib.rs"), "//! An empty shell.\n");
        }
        for (file, source) in files {
            workspace.write(&format!("crates/domain/model/src/{file}"), source);
        }
        crates(&workspace.load())
    }

    #[test]
    fn a_crate_holds_code_once_a_production_file_defines_a_function() {
        let holds_code = |files| -> Vec<bool> {
            let crates = crates_with(files).unwrap();
            crates.iter().map(|krate| krate.holds_code).collect()
        };
        assert_eq!(holds_code(&[]), [false, false, false]);
        assert_eq!(
            holds_code(&[("usage/total.rs", "pub fn total() -> u64 {\n    0\n}\n")]),
            [true, false, false]
        );
        // Test files are not production code, wherever they sit.
        let test = "#[test]\nfn adds() {}\n";
        assert_eq!(
            holds_code(&[
                ("lib.rs", "//! Shell.\n#[cfg(test)]\nmod tests;\n"),
                ("tests.rs", test),
                ("tests/helpers.rs", test),
                ("usage/tests.rs", test),
            ]),
            [false, false, false]
        );
    }

    #[test]
    fn test_code_in_a_production_file_of_a_floor_crate_is_an_error_naming_each_line() {
        let error = crates_with(&[
            (
                "usage.rs",
                "pub fn total() -> u64 {\n    0\n}\n\n#[cfg(test)]\nmod tests {\n}\n",
            ),
            ("usage/total.rs", "#[test]\nfn adds() {}\n"),
        ])
        .unwrap_err();
        for phrase in [
            "  crates/domain/model/src/usage.rs:5\n",
            "  crates/domain/model/src/usage/total.rs:1\n",
            "`#[cfg(test)] mod tests;`",
            "`#[test]`",
        ] {
            assert!(error.contains(phrase), "{error}");
        }
    }

    #[test]
    fn a_floor_crate_without_a_source_directory_is_an_error() {
        let workspace = FixtureWorkspace::new("")
            .member("crates/domain/model", "lablet-model", "")
            .load();
        let error = crates(&workspace).unwrap_err();
        assert!(error.contains("crates/domain/model/src"), "{error}");
    }

    #[test]
    fn the_real_floor_crates_are_all_there_and_keep_test_code_in_test_files() {
        let workspace = Workspace::load(&crate::workspace::workspace_root()).unwrap();
        let crates = crates(&workspace).unwrap();
        assert_eq!(crates.len(), FLOORS.len());
    }
}
