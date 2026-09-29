//! The coverage and mutation floors, as data (quality-bar item 12).
//!
//! Coverage is held to a percentage of lines, regions, and branches. Mutation
//! testing is held to an exact floor: every viable mutant is caught, except
//! the ones [`EQUIVALENT_MUTANTS`] names.
//!
//! Three things keep a floor honest without parsing Rust. A crate may measure
//! nothing only while its `src/` defines no function. Test code is told from
//! production code by file name: in a floor crate unit tests live in a
//! sibling file declared as `#[cfg(test)] mod tests;`, which coverage leaves
//! out with [`TEST_FILES`]. [`crates`] fails a crate whose production files
//! carry any other attribute that mentions `test`, since those lines would
//! count as covered production code. And [`crates`] fails a production file
//! that mentions `mutants::skip`, since cargo-mutants generates no mutant for
//! what it marks and the exact floor would pass without judging that code.

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
    /// The least share of regions, in percent, its tests must execute.
    ///
    /// A region is a span of source with its own counter, so a match arm, an
    /// `else` a line never spells out, and the far side of a `&&` each count
    /// on their own. Lines are derived from regions and can only be kinder:
    /// one line holding three arms is covered when any one of them runs.
    pub region_coverage: u64,
    /// The least share of branches, in percent, its tests must take.
    ///
    /// A branch is one outcome of a condition, so an `if` has two. Regions
    /// can't stand in for them: when one side of a `&&` or `||` went the same
    /// way every time while the whole condition went both ways, every region
    /// still ran.
    pub branch_coverage: u64,
}

/// Every crate with a floor. Changing a number or the list is a decision:
/// it is stated in contributing/README.md and product/quality-bar.md too.
///
/// All three are held to every line, region, and branch. They have no
/// unreachable code, and a branch that no test can take is the signal the
/// floor exists to raise: it means a type allows a state its caller has
/// already ruled out, and the fix is to make the type rule it out instead.
/// A check a second caller can genuinely fail stays, and that caller's tests
/// cover it.
pub const FLOORS: &[Floor] = &[
    Floor {
        package: "lablet-model",
        line_coverage: 100,
        region_coverage: 100,
        branch_coverage: 100,
    },
    Floor {
        package: "lablet-policy",
        line_coverage: 100,
        region_coverage: 100,
        branch_coverage: 100,
    },
    Floor {
        package: "lablet-run",
        line_coverage: 100,
        region_coverage: 100,
        branch_coverage: 100,
    },
];

/// A mutant no test can catch, because the code it leaves behaves as the code
/// it replaced.
#[derive(Debug, Clone, Copy)]
pub struct Equivalent {
    /// The package name, as in the crate's `Cargo.toml`.
    pub package: &'static str,
    /// The file, relative to the workspace root.
    pub file: &'static str,
    /// What the mutant does: the text cargo-mutants gives after `line:col: `.
    /// The line and column aren't part of the name, since they move with
    /// every edit to the file. An entry excuses one mutant, so two that read
    /// the same are two entries.
    pub mutant: &'static str,
    /// Why no test can catch it.
    pub reason: &'static str,
}

/// The mutants the exact floor excuses. Adding one is a decision, as changing
/// [`FLOORS`] is.
///
/// The list exists because whether a mutant is equivalent isn't decidable by
/// a tool, so each survivor is judged once by a person and written down here
/// with the reason. cargo-mutants still generates and tests every one, so a
/// full run fails on an entry that no longer matches a missed mutant: the code
/// it named is gone, or a test now catches it.
pub const EQUIVALENT_MUTANTS: &[Equivalent] = &[Equivalent {
    package: "lablet-run",
    file: "crates/application/run/src/observer.rs",
    mutant: "replace RunObserver::trace_context -> Option<TraceContext> with None",
    reason: "the default body of `RunObserver::trace_context` is already `None`",
}];

/// A `tests.rs` file or anything under a `tests/` directory, as a regex.
pub const TEST_FILES: &str = r"(^|/)tests(\.rs|/)";

/// How one crate fared against one floor.
#[derive(Debug, PartialEq, Eq)]
pub enum Standing {
    /// At or above the floor.
    Met,
    /// Nothing to measure in a crate that defines no function yet. It passes,
    /// with a note, so a scaffold can run the gate.
    NothingToMeasure,
    /// Nothing was measured in a crate that defines a function: a failure,
    /// since the floor was not applied at all.
    NothingMeasured,
    /// Below the floor.
    Below,
}

/// One crate's result line in a floor report.
#[derive(Debug)]
pub struct Line {
    /// The package name.
    pub package: &'static str,
    /// How many units (lines, regions, branches) counted towards the floor.
    pub hit: u64,
    /// How many units there were.
    pub total: u64,
    /// The floor, in percent.
    pub floor: u64,
    /// What `hit` counts, for the wording: "production lines covered".
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
            Standing::NothingMeasured => f.write_str(&nothing_measured(
                package,
                none_of,
                &format!("{floor}% floor"),
            )),
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

/// The line for a crate that defines a function and had nothing measured.
/// `floor` names the floor that went unapplied: "100% floor", "exact floor".
pub fn nothing_measured(package: &str, none_of: &str, floor: &str) -> String {
    format!(
        "{package}: NOTHING MEASURED (no {none_of}) though its src/ defines a function, so the \
         {floor} was not applied. Either something hides the crate from the tool (a filter, an \
         ignore pattern, rewritten paths), or no function in src/ is one the tool can measure \
         yet (trait methods without a body, only unviable mutants): land the first function \
         with a body together with its test"
    )
}

/// A floor report as a step result: every line is shown either way, and any
/// floor not met, or unmeasured though the crate defines a function, fails.
///
/// The count is of floors rather than crates, because a crate can be held to
/// more than one: coverage judges its lines and its regions apart.
pub fn conclude(what: &str, lines: &[Line]) -> crate::gates::CheckResult {
    let report = report(lines);
    let count = |standings: &[Standing]| {
        lines
            .iter()
            .filter(|line| standings.contains(&line.standing()))
            .count()
    };
    let failed = count(&[Standing::Below, Standing::NothingMeasured]);
    if failed > 0 {
        return Err(format!(
            "{what}: {failed} floor(s) below or unmeasured\n\n{report}"
        ));
    }
    println!("{report}");
    let unmeasured = count(&[Standing::NothingToMeasure]);
    Ok((unmeasured > 0).then(|| {
        format!(
            "{unmeasured} of {} floor(s) had nothing to measure yet",
            lines.len()
        )
    }))
}

/// One indented line per entry, as every floor report lists its crates.
pub fn report<L: fmt::Display>(lines: &[L]) -> String {
    lines
        .iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `target/xtask` under the workspace, where the floor checks keep their
/// tools' reports. Created here: on a fresh clone `target/` does not exist
/// yet, and cargo-mutants does not create the parent of its output directory.
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

/// Every floor crate, read from disk. A floor naming a crate the workspace
/// does not have is an error, so the list cannot rot silently; so is test
/// code in a production file, and so is `mutants::skip` in one.
pub fn crates(workspace: &Workspace) -> Result<Vec<FloorCrate>, String> {
    let mut crates = Vec::new();
    let mut misplaced = Vec::new();
    let mut skipped = Vec::new();
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
            if let Some(line) = skips_mutants(&source) {
                let relative = file.strip_prefix(&workspace.root).unwrap_or(&file);
                skipped.push(format!("  {}:{line}", relative.display()));
            }
        }
        crates.push(FloorCrate {
            floor,
            directory,
            holds_code,
        });
    }
    if !skipped.is_empty() {
        return Err(format!(
            "`mutants::skip` in a production file of a crate with a mutation floor:\n\n{}\n\n\
             cargo-mutants generates no mutant for what it marks, whatever a `cfg_attr` around it \
             says, so the floor would pass without judging that code. Catch its mutants with a \
             test, or name one no test can catch in EQUIVALENT_MUTANTS, with the reason.",
            skipped.join("\n")
        ));
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

/// The `.rs` files under `src` that [`TEST_FILES`] does not name, sorted.
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

/// `fn`, a name, then `(` or `<`, outside a line comment. A string that reads
/// like one counts too, which only asks for a measurement sooner.
fn defines_a_function(line: &str) -> bool {
    let line = line.trim_start();
    !line.starts_with("//")
        && line.match_indices("fn ").any(|(at, _)| {
            let rest = &line[at + 3..];
            let after = rest.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_');
            after.len() < rest.len() && after.starts_with(['(', '<'])
        })
}

/// The 1-based line of the first mention of `mutants::skip`, spaced or not,
/// comments included: a mention in prose only asks for a rewording.
fn skips_mutants(source: &str) -> Option<usize> {
    source
        .lines()
        .position(|line| {
            let unspaced: String = line.split_whitespace().collect();
            unspaced.contains("mutants::skip")
        })
        .map(|at| at + 1)
}

/// The 1-based line of the first attribute that mentions `test` and is not
/// `#[cfg(test)]` directly on `mod tests;`. An attribute mentions `test` when
/// that is one of its words, so `#[tokio::test]` and `#[cfg(all(test, unix))]`
/// do and `#[serde(rename = "latest")]` does not. The item is the rest of the
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
    fn the_floor_crates_are_held_to_every_line_region_and_branch() {
        let coverage: Vec<(&str, u64, u64, u64)> = FLOORS
            .iter()
            .map(|floor| {
                (
                    floor.package,
                    floor.line_coverage,
                    floor.region_coverage,
                    floor.branch_coverage,
                )
            })
            .collect();
        assert_eq!(
            coverage,
            [
                ("lablet-model", 100, 100, 100),
                ("lablet-policy", 100, 100, 100),
                ("lablet-run", 100, 100, 100),
            ]
        );
    }

    #[test]
    fn every_mutant_named_as_equivalent_is_in_a_floor_crate_and_says_why() {
        let workspace = Workspace::load(&crate::workspace::workspace_root()).unwrap();
        for named in EQUIVALENT_MUTANTS {
            let Equivalent {
                package,
                file,
                mutant,
                reason,
            } = named;
            assert!(
                FLOORS.iter().any(|floor| floor.package == *package),
                "{package} has no floor"
            );
            let member = workspace.member_named(package).unwrap();
            assert!(
                file.starts_with(&format!("{}/src/", member.path)),
                "{file} is not a source file of {package}"
            );
            assert!(workspace.root.join(file).is_file(), "{file} is gone");
            // A position would go stale with the next edit to the file.
            assert!(mutant.starts_with("replace ") || mutant.starts_with("delete "));
            assert!(!reason.is_empty(), "{mutant} gives no reason");
        }
        let names: Vec<&str> = EQUIVALENT_MUTANTS
            .iter()
            .map(|named| named.mutant)
            .collect();
        assert_eq!(
            names,
            ["replace RunObserver::trace_context -> Option<TraceContext> with None"]
        );
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
            Some("1 of 2 floor(s) had nothing to measure yet")
        );
        assert_eq!(conclude("coverage", &[line(95, 100, 90)]).unwrap(), None);

        let hidden = Line {
            holds_code: true,
            ..line(0, 0, 90)
        };
        assert_eq!(hidden.standing(), Standing::NothingMeasured);
        let error = conclude("mutants", &[line(0, 0, 90), hidden]).unwrap_err();
        assert!(
            error.starts_with("mutants: 1 floor(s) below or unmeasured"),
            "{error}"
        );
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
            error.starts_with("coverage: 1 floor(s) below or unmeasured"),
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

    #[test]
    fn a_production_file_never_skips_mutants() {
        assert_eq!(skips_mutants("pub fn add() {}\n"), None);
        for (skipping, line) in [
            ("#[mutants::skip]\nfn a() {}\n", 1),
            (
                "fn a() {}\n#[cfg_attr(windows, mutants::skip)]\nfn b() {}\n",
                2,
            ),
            (
                "#[cfg_attr(\n    any(),\n    mutants :: skip\n)]\nfn a() {}\n",
                3,
            ),
            ("#[::mutants::skip]\nfn a() {}\n", 1),
        ] {
            assert_eq!(skips_mutants(skipping), Some(line), "{skipping}");
        }
        let skipped = crates_with(&[(
            "skip.rs",
            "#[cfg_attr(windows, mutants::skip)]\nfn a() {}\n",
        )])
        .unwrap_err();
        assert!(
            skipped.contains("crates/domain/model/src/skip.rs:1"),
            "{skipped}"
        );
    }

    /// The floor crates, `lablet-model` with `files` under its `src/`.
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
