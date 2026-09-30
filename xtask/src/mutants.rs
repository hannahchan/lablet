//! `cargo xtask mutants`: mutation testing from cargo-mutants, held to the
//! exact floor in [`crate::floors`]: every viable mutant of a floor crate is
//! caught, except the ones [`EQUIVALENT_MUTANTS`] names. Each mutant runs its
//! own package's tests, so a crate is judged on the tests it carries.
//!
//! Viable means every mutant but the ones cargo-mutants calls unviable,
//! which did not build. A timeout, or any other outcome without a verdict,
//! counts as not caught because nothing showed the tests would have failed,
//! and for the same reason a name excuses only a mutant the tests ran against
//! and missed.
//!
//! A full run is what the floor means. `--changed` is the quick answer for
//! whoever is editing a floor crate: it tests only the mutants in code that
//! differs from the merge-base with `origin/main`. It can't say that the list
//! of equivalent mutants is current, or that a crate was measured at all, so
//! it judges neither.

use serde::Deserialize;
use std::fmt::{self, Write as _};
use std::path::Path;
use std::process::ExitStatus;

use crate::error::{Error, Verb};
use crate::floors::{self, EQUIVALENT_MUTANTS, Equivalent, FLOORS, FloorCrate, Standing};
use crate::gates::{CheckResult, Failure};
use crate::process;
use crate::report::Note;
use crate::workspace::{self, Workspace, workspace_root};

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
    /// `file:line:column: ` and then what the mutant does.
    name: String,
    package: String,
    /// Relative to the workspace root.
    file: String,
}

impl Mutant {
    /// What the mutant does: its name less the `file:line:column: ` ahead of
    /// it. `None` when the name isn't shaped that way, and then no entry of
    /// the list matches it, which fails loudly rather than excusing by guess.
    fn description(&self) -> Option<&str> {
        let position = self.name.strip_prefix(self.file.as_str())?;
        let (line, rest) = position.strip_prefix(':')?.split_once(':')?;
        let (column, description) = rest.split_once(": ")?;
        let is_number = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
        (is_number(line) && is_number(column)).then_some(description)
    }

    /// Never by line or column, which move with every edit to the file.
    fn is_named_by(&self, named: &Equivalent) -> bool {
        self.package == named.package
            && self.file == named.file
            && self.description() == Some(named.mutant)
    }
}

/// How much of the floor crates a run tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Every mutant of every floor crate.
    Full,
    /// Only the mutants in what changed; see [`check_changed`].
    Changed,
}

/// One crate's result line in a mutation report.
#[derive(Debug)]
struct Tally {
    package: &'static str,
    caught: u64,
    /// Every mutant but the unviable ones.
    viable: u64,
    /// Missed, and named as equivalent: the entry, once for each mutant.
    excused: Vec<Equivalent>,
    /// Missed, timed out, or ended without a verdict, and not named: each by
    /// its full name, so the reader can go to the line, and with which it was.
    escaped: Vec<String>,
    /// Whether the crate must have a viable mutant; see
    /// [`FloorCrate::holds_code`]. Never in a scoped run, where a crate
    /// nothing changed in has none.
    holds_code: bool,
    scope: Scope,
}

impl Tally {
    fn standing(&self) -> Standing {
        if self.viable == 0 && self.holds_code {
            Standing::NothingMeasured
        } else if self.viable == 0 {
            Standing::NothingToMeasure
        } else if self.escaped.is_empty() {
            Standing::Met
        } else {
            Standing::Below
        }
    }
}

impl fmt::Display for Tally {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            package,
            caught,
            viable,
            excused,
            escaped,
            holds_code: _,
            scope,
        } = self;
        match self.standing() {
            Standing::NothingToMeasure if *scope == Scope::Changed => {
                write!(f, "{package}: no viable mutant in what changed")
            }
            Standing::NothingToMeasure => write!(
                f,
                "{package}: nothing to measure yet (no viable mutants); the floor is exact"
            ),
            Standing::NothingMeasured => f.write_str(&floors::nothing_measured(
                package,
                "viable mutants",
                "exact floor",
            )),
            // A scoped run tested a part of the crate, so it says what that
            // part came to and leaves the verdict on the floor to the full run.
            Standing::Met if *scope == Scope::Changed => write!(
                f,
                "{package}: {caught} of {viable} viable mutants in what changed caught, {} \
                 excused by name",
                excused.len()
            ),
            Standing::Below if *scope == Scope::Changed => write!(
                f,
                "{package}: {caught} of {viable} viable mutants in what changed caught, {} \
                 excused by name, {} not caught",
                excused.len(),
                escaped.len()
            ),
            Standing::Met => write!(
                f,
                "{package}: {caught} of {viable} viable mutants caught, {} excused by name, \
                 meets the exact floor",
                excused.len()
            ),
            Standing::Below => write!(
                f,
                "{package}: {caught} of {viable} viable mutants caught, {} excused by name, {} \
                 not caught, BELOW the exact floor",
                excused.len(),
                escaped.len()
            ),
        }
    }
}

/// What a run came to: a line per floor crate, and the entries of the list
/// that excused nothing.
#[derive(Debug)]
struct Verdict {
    tallies: Vec<Tally>,
    /// Only a full run can find one: a scoped run tests the named mutants
    /// only when their code changed.
    stale: Vec<Equivalent>,
}

fn judge(
    outcomes: &Outcomes,
    crates: &[FloorCrate],
    named: &[Equivalent],
    scope: Scope,
) -> Verdict {
    let mut used = vec![false; named.len()];
    let tallies = crates
        .iter()
        .map(|krate| {
            let mut tally = Tally {
                package: krate.floor.package,
                caught: 0,
                viable: 0,
                excused: Vec::new(),
                escaped: Vec::new(),
                holds_code: krate.holds_code && scope == Scope::Full,
                scope,
            };
            for outcome in &outcomes.outcomes {
                let Scenario::Mutant(mutant) = &outcome.scenario else {
                    continue;
                };
                if mutant.package != tally.package {
                    continue;
                }
                match outcome.summary.as_str() {
                    "CaughtMutant" => {
                        tally.caught += 1;
                        tally.viable += 1;
                    }
                    "MissedMutant" => {
                        tally.viable += 1;
                        // An entry stands for one mutant. Two operators of a
                        // kind in one function read the same without their
                        // positions, and the judgement of one isn't of both.
                        let entry = named
                            .iter()
                            .zip(&used)
                            .position(|(entry, used)| !used && mutant.is_named_by(entry));
                        match entry {
                            Some(entry) => {
                                used[entry] = true;
                                tally.excused.push(named[entry]);
                            }
                            None => tally.escaped.push(format!("{} (missed)", mutant.name)),
                        }
                    }
                    "Timeout" => {
                        tally.viable += 1;
                        tally.escaped.push(format!("{} (timed out)", mutant.name));
                    }
                    // The mutant did not build, so it tested nothing.
                    "Unviable" => {}
                    // "Failure" (cargo itself ended by a signal) and any summary a
                    // later version adds: nothing showed the tests would have failed.
                    other => {
                        tally.viable += 1;
                        tally.escaped.push(format!("{} ({other})", mutant.name));
                    }
                }
            }
            tally
        })
        .collect();
    let stale = named
        .iter()
        .zip(used)
        .filter(|(_, used)| scope == Scope::Full && !used)
        .map(|(entry, _)| *entry)
        .collect();
    Verdict { tallies, stale }
}

/// A mutation report as a step result: every line is shown either way, and
/// the run fails on a crate below the floor or unmeasured, and on a stale
/// entry in the list. What was excused is shown with its reason, so that a
/// green run still says what it let through and why.
fn conclude(verdict: &Verdict) -> CheckResult {
    let Verdict { tallies, stale } = verdict;
    let mut report = floors::report(tallies);
    let excused: Vec<&Equivalent> = tallies.iter().flat_map(|tally| &tally.excused).collect();
    if !excused.is_empty() {
        report.push_str("\n\nExcused by name, each with why no test can catch it:\n");
        for Equivalent {
            file,
            mutant,
            reason,
            ..
        } in excused
        {
            let _ = write!(report, "\n  {file}: {mutant}\n    {reason}");
        }
    }
    let count = |standings: &[Standing]| {
        tallies
            .iter()
            .filter(|tally| standings.contains(&tally.standing()))
            .count()
    };
    let failed = count(&[Standing::Below, Standing::NothingMeasured]);
    if failed == 0 && stale.is_empty() {
        println!("{report}");
        let unmeasured = count(&[Standing::NothingToMeasure]);
        let scoped = tallies.iter().any(|tally| tally.scope == Scope::Changed);
        return Ok((unmeasured > 0 && !scoped).then(|| {
            Note::Info(format!(
                "{unmeasured} of {} floor(s) had nothing to measure yet",
                tallies.len()
            ))
        }));
    }

    let mut headline = Vec::new();
    if failed > 0 {
        headline.push(format!("{failed} floor(s) below or unmeasured"));
    }
    if !stale.is_empty() {
        headline.push(format!(
            "{} mutant(s) named as equivalent matched nothing",
            stale.len()
        ));
    }
    let mut message = format!("mutants: {}\n\n{report}", headline.join(", "));
    let escaped: Vec<&String> = tallies.iter().flat_map(|tally| &tally.escaped).collect();
    if !escaped.is_empty() {
        message.push_str("\n\nNot caught, and not named as equivalent in xtask/src/floors.rs:\n\n");
        for name in escaped {
            let _ = writeln!(message, "  {name}");
        }
        message.push_str(
            "\nCatch each with a test. Only a mutant no test can catch is added to \
             EQUIVALENT_MUTANTS, with the reason.",
        );
    }
    if !stale.is_empty() {
        message.push_str(
            "\n\nNamed as equivalent in xtask/src/floors.rs, but no mutant that was missed \
             matches:\n\n",
        );
        for entry in stale {
            let Equivalent {
                package,
                file,
                mutant,
                reason: _,
            } = entry;
            let _ = writeln!(message, "  {package}, {file}: {mutant}");
        }
        message.push_str(
            "\nThe code an entry names is gone, or a test now catches the mutant. Remove the \
             entry, or correct it if the code was renamed or moved.",
        );
    }
    Err(Failure::Verdict(message))
}

/// cargo-mutants' exit codes that still leave a full set of outcomes: all
/// caught, some missed, some timed out. Anything else means it did not test.
const EXITS_WITH_OUTCOMES: [i32; 3] = [0, 2, 3];

/// cargo-mutants hands `--cargo-arg` to every cargo command it runs, so the
/// builds in its copies of the tree hold to the committed lockfile too. The
/// named mutants aren't excluded: testing them is what shows an entry of the
/// list is still true. Nor is a `.cargo/mutants.toml` read: it could stop
/// mutants from being generated (`exclude_re`, `exclude_globs`, `examine_*`,
/// `skip_calls`) or have other crates' tests run against them
/// (`test_workspace`, `test_package`), and the floor would pass on less than
/// it says.
fn mutants_args<'a>(output: &'a str, in_diff: Option<&'a str>) -> Vec<&'a str> {
    let mut args = vec![
        "mutants",
        "--no-config",
        "--cargo-arg=--locked",
        "--output",
        output,
    ];
    if let Some(diff) = in_diff {
        args.extend(["--in-diff", diff]);
    }
    for floor in FLOORS {
        args.extend(["--package", floor.package]);
    }
    args
}

/// Runs cargo-mutants through `run`, which takes cargo's arguments, and reads
/// what it found. `output` is the directory it writes `mutants.out` under.
fn test_mutants(
    output: &Path,
    in_diff: Option<&Path>,
    run: impl FnOnce(&[&str]) -> Result<ExitStatus, Error>,
) -> Result<Outcomes, Failure> {
    let output_arg = output.display().to_string();
    let diff_arg = in_diff.map(|diff| diff.display().to_string());
    let args = mutants_args(&output_arg, diff_arg.as_deref());
    let report = output.join("mutants.out").join("outcomes.json");
    forget_last_report(&report)?;
    let status = run(&args)?;
    if !status
        .code()
        .is_some_and(|code| EXITS_WITH_OUTCOMES.contains(&code))
    {
        return Err(Failure::Verdict(format!(
            "{}; no mutants were tested (the tests must pass unmutated first)",
            process::command_failed("cargo", &args)
        )));
    }
    Ok(read_outcomes(&report)?)
}

/// How [`check`] and [`check_changed`] run cargo-mutants: through cargo
/// itself, its output streamed.
fn cargo(args: &[&str]) -> Result<ExitStatus, Error> {
    process::stream("cargo", args, &[])
}

/// Runs cargo-mutants over the floor crates and judges the exact floor.
pub fn check() -> CheckResult {
    let workspace = Workspace::load(&workspace_root())?;
    let crates = floors::crates(&workspace)?;
    let output = floors::output_directory(&workspace.root)?;
    let outcomes = test_mutants(&output, None, cargo)?;
    conclude(&judge(&outcomes, &crates, EQUIVALENT_MUTANTS, Scope::Full))
}

/// Runs cargo-mutants over what changed in the floor crates since the
/// merge-base with `origin/main`, and holds every mutant it tested to being
/// caught or named. It reads the repository and writes only under
/// `target/xtask`: the index, the working tree, and git's configuration are
/// left as they were.
pub fn check_changed() -> CheckResult {
    let workspace = Workspace::load(&workspace_root())?;
    let crates = floors::crates(&workspace)?;
    let (base, source) = merge_base(&workspace.root)?;
    let since = format!("{} ({source})", short(&base));

    let diff = changed_diff(&workspace.root, &base, &pathspecs(&workspace.root, &crates))?;
    let output = floors::output_directory(&workspace.root)?;
    test_changed(&diff, &since, &crates, &output, cargo)
}

/// [`check_changed`], from the diff it found and with `run` in place of
/// cargo. `output` is where a full run writes; an empty diff runs nothing.
fn test_changed(
    diff: &str,
    since: &str,
    crates: &[FloorCrate],
    output: &Path,
    run: impl FnOnce(&[&str]) -> Result<ExitStatus, Error>,
) -> CheckResult {
    if diff.is_empty() {
        return Ok(Some(Note::Info(format!(
            "nothing to test: no Rust file of a floor crate changed since {since}"
        ))));
    }
    let diff_path = output.join("changed.diff");
    std::fs::write(&diff_path, diff).map_err(Error::file(Verb::Write, &diff_path))?;
    // Apart from the full run's, so the quick answer doesn't displace the
    // report the floor was last judged on.
    let output = output.join("changed");
    std::fs::create_dir_all(&output).map_err(Error::file(Verb::Create, &output))?;

    let outcomes = test_mutants(&output, Some(&diff_path), run)?;
    let verdict = judge(&outcomes, crates, EQUIVALENT_MUTANTS, Scope::Changed);
    if verdict.tallies.iter().all(|tally| tally.viable == 0) {
        return Ok(Some(Note::Info(format!(
            "nothing to test: what changed since {since} holds no viable mutant"
        ))));
    }
    conclude(&verdict)?;
    Ok(Some(Note::Info(format!(
        "only the mutants in what changed since {since}; the full run judges the floor"
    ))))
}

/// The merge-base with `origin/main`, else with `main` for a clone that has
/// no remote, and which of the two it was. Git failing to answer is its own
/// error, not a missing merge-base.
fn merge_base(directory: &Path) -> Result<(String, String), Error> {
    for branch in ["origin/main", "main"] {
        if let Some(revision) = process::git_merge_base(directory, branch)? {
            return Ok((revision, format!("merge-base with {branch}")));
        }
    }
    Err(Error::Missing {
        what: "no merge-base with origin/main or main, so there is nothing to scope the run to"
            .to_owned(),
        remedy: "Fetch origin, or run the full `cargo xtask mutants`".to_owned(),
    })
}

/// A full commit id cut to what a person reads.
fn short(revision: &str) -> &str {
    revision.get(..12).unwrap_or(revision)
}

/// The Rust files of the floor crates, as git pathspecs relative to the
/// workspace root. cargo-mutants reads only Rust from a diff, and a diff of
/// anything else (a binary file, a lockfile) is one more thing to parse.
fn pathspecs(workspace_root: &Path, crates: &[FloorCrate]) -> Vec<String> {
    crates
        .iter()
        .map(|krate| {
            let directory = krate
                .directory
                .strip_prefix(workspace_root)
                .unwrap_or(&krate.directory);
            format!(":(glob){}/**/*.rs", directory.display())
        })
        .collect()
}

/// Git options that leave the repository as it was and the diff as
/// cargo-mutants reads it, whatever the developer has configured. `git diff`
/// would otherwise rewrite the index to refresh a file whose timestamp moved
/// and whose text didn't, which `--no-optional-locks` doesn't stop. A colour,
/// an external diff tool, or other prefixes would not parse, and a text
/// conversion would show other text than the source. Nor would git's default
/// quoting parse, which writes a name that isn't ASCII in octal escapes that
/// cargo-mutants' diff parser rejects. `-c` sets each for this command alone.
const GIT_DIFF: [&str; 12] = [
    "-c",
    "diff.autoRefreshIndex=false",
    "-c",
    "core.quotePath=false",
    "diff",
    "--relative",
    "--no-renames",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

/// Untracked files, less the ignored ones.
const GIT_UNTRACKED: [&str; 5] = [
    "--no-optional-locks",
    "ls-files",
    "--others",
    "--exclude-standard",
    "-z",
];

/// Everything under `pathspecs` that differs between `base` and the working
/// tree, as one diff: what is committed on the branch, staged, or neither,
/// and the files git doesn't track. A builder never commits, so its new
/// module is an untracked file, which `git diff` alone leaves out.
///
/// Git runs in the workspace root with `--relative`, because cargo-mutants
/// matches a diff's paths against paths relative to the directory it runs
/// in, and that is one level below the repository root. Rename detection is
/// off so that a moved file counts as changed throughout: its tests may not
/// have moved with it.
fn changed_diff(workspace_root: &Path, base: &str, pathspecs: &[String]) -> Result<String, Error> {
    let git = |options: &[&str], revision: Option<&str>| {
        let mut args = options.to_vec();
        args.extend(revision);
        args.push("--");
        args.extend(pathspecs.iter().map(String::as_str));
        process::capture_in(workspace_root, "git", &args)
    };
    let mut diff = git(&GIT_DIFF, Some(base))?;
    let untracked = git(&GIT_UNTRACKED, None)?;
    for path in untracked.split('\0').filter(|path| !path.is_empty()) {
        let text = workspace::read(&workspace_root.join(path))?;
        diff.push_str(&added_file(path, &text));
    }
    Ok(diff)
}

/// The diff that adds a whole file, as `git diff` writes one for a file it
/// tracks. An empty file has no line to change and so no diff. A name that
/// holds a space ends in a tab on the `+++` line, as git ends it, because a
/// diff parser otherwise takes the name to end at the space.
fn added_file(path: &str, text: &str) -> String {
    let lines = text.split_inclusive('\n').count();
    if lines == 0 {
        return String::new();
    }
    let tab = if path.contains(' ') { "\t" } else { "" };
    let mut diff = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}{tab}\n\
         @@ -0,0 +1,{lines} @@\n"
    );
    for line in text.split_inclusive('\n') {
        diff.push('+');
        diff.push_str(line);
    }
    if !text.ends_with('\n') {
        diff.push_str("\n\\ No newline at end of file\n");
    }
    diff
}

/// Removes the report a previous run left, so the one read after a run is
/// that run's. cargo-mutants leaves the last run's `outcomes.json` in place
/// when a diff holds no mutant, and a file's modification time can't tell the
/// two apart: Linux stamps a file from a clock that may read earlier than the
/// one a run's start was read from.
fn forget_last_report(path: &Path) -> Result<(), Error> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(Error::file(Verb::Remove, path)(e))
        }
        _ => Ok(()),
    }
}

/// cargo-mutants writes no `outcomes.json` when it finds nothing to mutate,
/// so a missing file is an empty run and not an error; the floors then fail
/// any crate that must be measured.
fn read_outcomes(path: &Path) -> Result<Outcomes, Error> {
    if !path.is_file() {
        return Ok(Outcomes {
            outcomes: Vec::new(),
        });
    }
    workspace::read_json(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::chain;
    use crate::workspace::fixture::{TempDir, defy_git_defaults, scratch_git};
    use std::cell::RefCell;
    use std::os::unix::process::ExitStatusExt as _;

    const MODEL: &str = "crates/domain/model/src/lib.rs";
    const POLICY: &str = "crates/domain/policy/src/lib.rs";
    const RUN: &str = "crates/application/run/src/lib.rs";
    const OBSERVER: &str = "crates/application/run/src/observer.rs";
    /// In a crate without a floor.
    const OTHER: &str = "crates/adapters/other/src/lib.rs";

    const TRACE_CONTEXT: &str =
        "replace RunObserver::trace_context -> Option<TraceContext> with None";
    const ON_TURN: &str = "replace RunObserver::on_turn with ()";
    const WHOLE_MS: &str = "replace whole_ms -> u64 with 0";

    const NAMED: [Equivalent; 1] = [Equivalent {
        package: "lablet-run",
        file: OBSERVER,
        mutant: TRACE_CONTEXT,
        reason: "the default body is already `None`",
    }];

    /// One entry of `outcomes.json`, with the keys cargo-mutants 27 writes.
    /// The package is the one whose directory holds the file.
    fn mutant(file: &str, position: &str, does: &str, summary: &str) -> String {
        let package = match file {
            MODEL => "lablet-model",
            POLICY => "lablet-policy",
            RUN | OBSERVER => "lablet-run",
            _ => "lablet-other",
        };
        format!(
            r#"{{"scenario": {{"Mutant": {{
                 "name": "{file}:{position}: {does}", "package": "{package}", "file": "{file}",
                 "function": {{"function_name": "f", "return_type": "-> u8"}},
                 "replacement": "0", "genre": "FnValue"}}}},
               "summary": "{summary}", "log_path": "log/a.log"}}"#
        )
    }

    fn outcomes(mutants: &[String]) -> Outcomes {
        let baseline = r#"{"scenario": "Baseline", "summary": "Success"}"#;
        let mut all = vec![baseline.to_owned()];
        all.extend_from_slice(mutants);
        let text = format!(
            r#"{{"outcomes": [{}], "total_mutants": {}, "cargo_mutants_version": "27.1.0"}}"#,
            all.join(","),
            mutants.len()
        );
        serde_json::from_str(&text).unwrap()
    }

    /// Every floor crate caught all it had, and `lablet-run` missed the one
    /// mutant the list names.
    fn green() -> Vec<String> {
        vec![
            mutant(MODEL, "69:5", WHOLE_MS, "CaughtMutant"),
            mutant(MODEL, "72:9", "delete ! in whole_ms", "CaughtMutant"),
            mutant(MODEL, "12:9", "replace new -> Self with Self", "Unviable"),
            mutant(POLICY, "7:5", "replace allows with true", "CaughtMutant"),
            mutant(RUN, "39:5", "replace run -> u8 with 0", "CaughtMutant"),
            mutant(OBSERVER, "190:9", TRACE_CONTEXT, "MissedMutant"),
            // Not a floor crate, so nothing it missed is counted.
            mutant(OTHER, "1:1", "replace f -> u8 with 0", "MissedMutant"),
        ]
    }

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

    fn full(mutants: &[String]) -> Verdict {
        judge(&outcomes(mutants), &crates(true), &NAMED, Scope::Full)
    }

    fn scoped(mutants: &[String]) -> Verdict {
        judge(&outcomes(mutants), &crates(true), &NAMED, Scope::Changed)
    }

    /// Per crate: caught, viable, excused, and the standing.
    fn seen(verdict: &Verdict) -> Vec<(&str, u64, u64, usize, Standing)> {
        let seen = |tally: &Tally| {
            let standing = tally.standing();
            (
                tally.package,
                tally.caught,
                tally.viable,
                tally.excused.len(),
                standing,
            )
        };
        verdict.tallies.iter().map(seen).collect()
    }

    #[test]
    fn every_viable_mutant_caught_or_named_meets_the_exact_floor() {
        let verdict = full(&green());
        assert_eq!(
            seen(&verdict),
            [
                // 2 of 2: the unviable mutant is left out.
                ("lablet-model", 2, 2, 0, Standing::Met),
                ("lablet-policy", 1, 1, 0, Standing::Met),
                // 1 of 2: the other is the one the list names.
                ("lablet-run", 1, 2, 1, Standing::Met),
            ]
        );
        assert!(verdict.stale.is_empty());
        assert_eq!(conclude(&verdict).unwrap(), None);
        assert_eq!(
            verdict.tallies[2].to_string(),
            "lablet-run: 1 of 2 viable mutants caught, 1 excused by name, meets the exact floor"
        );
    }

    #[test]
    fn a_missed_mutant_the_list_does_not_name_fails_and_is_listed_by_its_full_name() {
        let mut mutants = green();
        mutants.push(mutant(OBSERVER, "201:9", ON_TURN, "MissedMutant"));
        let verdict = full(&mutants);
        assert_eq!(seen(&verdict)[2], ("lablet-run", 1, 3, 1, Standing::Below));

        let error = conclude(&verdict).unwrap_err().into_verdict();
        assert!(
            error.starts_with("mutants: 1 floor(s) below or unmeasured\n"),
            "{error}"
        );
        for phrase in [
            "  lablet-model: 2 of 2 viable mutants caught, 0 excused by name, meets the exact \
             floor\n",
            "  lablet-run: 1 of 3 viable mutants caught, 1 excused by name, 1 not caught, BELOW \
             the exact floor\n",
            "  crates/application/run/src/observer.rs:201:9: replace RunObserver::on_turn with \
             () (missed)\n",
            "EQUIVALENT_MUTANTS",
            // What was excused, without the position it happened to have.
            "  crates/application/run/src/observer.rs: replace RunObserver::trace_context -> \
             Option<TraceContext> with None\n    the default body is already `None`\n",
        ] {
            assert!(error.contains(phrase), "{error}");
        }
        // The excused mutant is not among the ones to go and catch.
        assert!(!error.contains("190:9"), "{error}");
    }

    /// The floor is a count, not a share: 99 caught of 100 met the floor of
    /// 80% and doesn't meet this one.
    #[test]
    fn one_mutant_not_caught_among_many_caught_is_below_the_floor() {
        let mut mutants: Vec<String> = (1..=99)
            .map(|line| mutant(POLICY, &format!("{line}:5"), "delete !", "CaughtMutant"))
            .collect();
        mutants.push(mutant(POLICY, "100:5", "delete !", "MissedMutant"));
        let verdict = full(&mutants);
        assert_eq!(
            seen(&verdict)[1],
            ("lablet-policy", 99, 100, 0, Standing::Below)
        );
        let error = conclude(&verdict).unwrap_err().into_verdict();
        let listed = format!("  {POLICY}:100:5: delete ! (missed)\n");
        assert!(error.contains(&listed), "{error}");
    }

    #[test]
    fn a_timeout_fails_and_a_name_does_not_excuse_one() {
        let mut mutants = green();
        mutants.push(mutant(MODEL, "80:5", WHOLE_MS, "Timeout"));
        let verdict = full(&mutants);
        assert_eq!(
            seen(&verdict)[0],
            ("lablet-model", 2, 3, 0, Standing::Below)
        );
        let error = conclude(&verdict).unwrap_err().into_verdict();
        let listed = format!("  {MODEL}:80:5: {WHOLE_MS} (timed out)\n");
        assert!(error.contains(&listed), "{error}");

        // Nothing showed the tests pass against a mutant that timed out, so it
        // is not the missed mutant the entry stands for.
        let verdict = full(&[mutant(OBSERVER, "190:9", TRACE_CONTEXT, "Timeout")]);
        assert_eq!(seen(&verdict)[2], ("lablet-run", 0, 1, 0, Standing::Below));
        assert_eq!(verdict.stale.len(), 1);
    }

    /// cargo-mutants 27 calls a mutant a `Failure` when cargo itself ended by
    /// a signal, and exits 0 for it: the tests gave no verdict on the mutant.
    #[test]
    fn a_mutant_that_ended_without_a_verdict_fails_as_not_caught() {
        let mut mutants = green();
        mutants.push(mutant(MODEL, "90:5", WHOLE_MS, "Failure"));
        let verdict = full(&mutants);
        assert_eq!(
            seen(&verdict)[0],
            ("lablet-model", 2, 3, 0, Standing::Below)
        );
        let error = conclude(&verdict).unwrap_err().into_verdict();
        let listed = format!("  {MODEL}:90:5: {WHOLE_MS} (Failure)\n");
        assert!(error.contains(&listed), "{error}");

        // A scoped run whose one mutant ended so has something to test.
        let verdict = scoped(&[mutant(MODEL, "90:5", WHOLE_MS, "Failure")]);
        assert_eq!(verdict.tallies[0].viable, 1);
        let error = conclude(&verdict).unwrap_err().into_verdict();
        assert!(error.contains(&listed), "{error}");
    }

    #[test]
    fn an_entry_that_matched_nothing_fails_a_full_run_and_names_the_entry() {
        // The code the entry named is gone, or a test now catches the mutant.
        for now in [None, Some("CaughtMutant")] {
            let mut mutants = green();
            mutants.retain(|mutant| !mutant.contains(TRACE_CONTEXT));
            mutants.extend(now.map(|summary| mutant(OBSERVER, "190:9", TRACE_CONTEXT, summary)));
            let verdict = full(&mutants);
            let standings = seen(&verdict);
            assert!(
                standings.iter().all(|seen| seen.4 == Standing::Met),
                "{now:?}"
            );

            let error = conclude(&verdict).unwrap_err().into_verdict();
            assert!(
                error.starts_with("mutants: 1 mutant(s) named as equivalent matched nothing\n"),
                "{error}"
            );
            let entry = format!("  lablet-run, {OBSERVER}: {TRACE_CONTEXT}\n");
            for phrase in [entry.as_str(), "Remove the entry", "xtask/src/floors.rs"] {
                assert!(error.contains(phrase), "{error}");
            }
        }
    }

    #[test]
    fn a_scoped_run_ignores_an_entry_that_matched_nothing_and_a_crate_it_did_not_test() {
        let changed = [mutant(MODEL, "69:5", WHOLE_MS, "CaughtMutant")];
        let verdict = scoped(&changed);
        assert_eq!(
            seen(&verdict),
            [
                ("lablet-model", 1, 1, 0, Standing::Met),
                ("lablet-policy", 0, 0, 0, Standing::NothingToMeasure),
                ("lablet-run", 0, 0, 0, Standing::NothingToMeasure),
            ]
        );
        assert!(verdict.stale.is_empty());
        assert_eq!(conclude(&verdict).unwrap(), None);
        assert_eq!(
            verdict.tallies[0].to_string(),
            "lablet-model: 1 of 1 viable mutants in what changed caught, 0 excused by name"
        );
        assert_eq!(
            verdict.tallies[1].to_string(),
            "lablet-policy: no viable mutant in what changed"
        );

        // The same outcomes fail a full run on both counts.
        let error = conclude(&full(&changed)).unwrap_err().into_verdict();
        assert!(
            error.starts_with(
                "mutants: 2 floor(s) below or unmeasured, 1 mutant(s) named as equivalent \
                 matched nothing\n"
            ),
            "{error}"
        );
    }

    #[test]
    fn a_scoped_run_still_fails_on_a_mutant_it_tested_and_did_not_catch() {
        let verdict = scoped(&[
            mutant(OBSERVER, "190:9", TRACE_CONTEXT, "MissedMutant"),
            mutant(OBSERVER, "201:9", ON_TURN, "MissedMutant"),
        ]);
        assert_eq!(seen(&verdict)[2], ("lablet-run", 0, 2, 1, Standing::Below));
        assert_eq!(
            verdict.tallies[2].to_string(),
            "lablet-run: 0 of 2 viable mutants in what changed caught, 1 excused by name, 1 not \
             caught"
        );
        let error = conclude(&verdict).unwrap_err().into_verdict();
        let listed = format!("  {OBSERVER}:201:9: {ON_TURN} (missed)\n");
        assert!(error.contains(&listed), "{error}");
    }

    #[test]
    fn a_named_mutant_whose_line_and_column_moved_is_still_excused() {
        for position in ["190:9", "7:1", "1204:33"] {
            for verdict in [
                full(&[mutant(OBSERVER, position, TRACE_CONTEXT, "MissedMutant")]),
                scoped(&[mutant(OBSERVER, position, TRACE_CONTEXT, "MissedMutant")]),
            ] {
                assert_eq!(
                    seen(&verdict)[2],
                    ("lablet-run", 0, 1, 1, Standing::Met),
                    "{position}"
                );
                assert!(verdict.stale.is_empty(), "{position}");
            }
        }
    }

    /// Without its position, a second `+` in a function reads as the first.
    #[test]
    fn an_entry_excuses_one_mutant_and_not_a_second_that_reads_the_same() {
        let twice = [
            mutant(OBSERVER, "190:9", TRACE_CONTEXT, "MissedMutant"),
            mutant(OBSERVER, "198:9", TRACE_CONTEXT, "MissedMutant"),
        ];
        let verdict = full(&twice);
        assert_eq!(seen(&verdict)[2], ("lablet-run", 0, 2, 1, Standing::Below));
        let error = conclude(&verdict).unwrap_err().into_verdict();
        let listed = format!("  {OBSERVER}:198:9: {TRACE_CONTEXT} (missed)\n");
        assert!(error.contains(&listed), "{error}");

        // Judged twice and written down twice, both are excused.
        let named = [NAMED[0], NAMED[0]];
        let verdict = judge(&outcomes(&twice), &crates(true), &named, Scope::Full);
        assert_eq!(seen(&verdict)[2], ("lablet-run", 0, 2, 2, Standing::Met));
        assert!(verdict.stale.is_empty());
        let verdict = judge(&outcomes(&twice[..1]), &crates(true), &named, Scope::Full);
        assert_eq!(verdict.stale.len(), 1);
    }

    #[test]
    fn a_mutant_is_named_by_its_package_its_file_and_what_it_does() {
        let of = |package: &str, file: &str, name: String| Mutant {
            name,
            package: package.to_owned(),
            file: file.to_owned(),
        };
        let name = format!("{OBSERVER}:190:9: {TRACE_CONTEXT}");
        let named = of("lablet-run", OBSERVER, name.clone());
        assert_eq!(named.description(), Some(TRACE_CONTEXT));
        assert!(named.is_named_by(&NAMED[0]));

        for other in [
            // The same words in another package or another file, and other
            // words that begin or end with the same ones.
            of("lablet-model", OBSERVER, name),
            of("lablet-run", RUN, format!("{RUN}:190:9: {TRACE_CONTEXT}")),
            of(
                "lablet-run",
                OBSERVER,
                format!("{OBSERVER}:190:9: {TRACE_CONTEXT}::new()"),
            ),
            of(
                "lablet-run",
                OBSERVER,
                format!("{OBSERVER}:190:9: not {TRACE_CONTEXT}"),
            ),
        ] {
            assert!(!other.is_named_by(&NAMED[0]), "{}", other.name);
        }
        for shapeless in [
            TRACE_CONTEXT.to_owned(),
            format!("{OBSERVER}: {TRACE_CONTEXT}"),
            format!("{OBSERVER}:190: {TRACE_CONTEXT}"),
            format!("{OBSERVER}:190:x: {TRACE_CONTEXT}"),
            format!("{OBSERVER}::9: {TRACE_CONTEXT}"),
            format!("src/observer.rs:190:9: {TRACE_CONTEXT}"),
        ] {
            let mutant = of("lablet-run", OBSERVER, shapeless);
            assert_eq!(mutant.description(), None, "{}", mutant.name);
            assert!(!mutant.is_named_by(&NAMED[0]), "{}", mutant.name);
        }
    }

    /// An outcome as cargo-mutants 27.1.0 wrote it, against the real list.
    #[test]
    fn the_list_excuses_the_mutant_it_names_as_cargo_mutants_reports_it() {
        let reported = r#"{"outcomes": [{
          "scenario": {"Mutant": {
            "name": "crates/application/run/src/observer.rs:190:9: replace RunObserver::trace_context -> Option<TraceContext> with None",
            "package": "lablet-run",
            "file": "crates/application/run/src/observer.rs",
            "function": {"function_name": "RunObserver::trace_context",
                         "return_type": "-> Option<TraceContext>",
                         "span": {"start": {"line": 185, "column": 5},
                                  "end": {"line": 192, "column": 6}}},
            "span": {"start": {"line": 190, "column": 9}, "end": {"line": 191, "column": 13}},
            "replacement": "None",
            "genre": "FnValue"}},
          "summary": "MissedMutant",
          "log_path": "log/crates__application__run__src__observer.rs_line_190_col_9.log",
          "diff_path": "diff/crates__application__run__src__observer.rs_line_190_col_9.diff",
          "phase_results": []}],
          "total_mutants": 1, "missed": 1, "caught": 0, "timeout": 0, "unviable": 0}"#;
        let outcomes: Outcomes = serde_json::from_str(reported).unwrap();
        let verdict = judge(&outcomes, &crates(true), EQUIVALENT_MUTANTS, Scope::Full);
        assert_eq!(seen(&verdict)[2], ("lablet-run", 0, 1, 1, Standing::Met));
        assert!(verdict.stale.is_empty());
    }

    #[test]
    fn a_full_run_that_found_no_mutants_passes_only_for_crates_that_define_no_function() {
        let missing = Path::new("/nonexistent/mutants.out/outcomes.json");
        let outcomes = read_outcomes(missing).unwrap();
        for (holds_code, standing) in [
            (false, Standing::NothingToMeasure),
            (true, Standing::NothingMeasured),
        ] {
            let verdict = judge(&outcomes, &crates(holds_code), &[], Scope::Full);
            let tallies = &verdict.tallies;
            assert!(tallies.iter().all(|tally| tally.standing() == standing));
            assert_eq!(conclude(&verdict).is_ok(), !holds_code);
        }

        let verdict = judge(&outcomes, &crates(false), &[], Scope::Full);
        assert_eq!(
            conclude(&verdict).unwrap(),
            Some(Note::Info(
                "3 of 3 floor(s) had nothing to measure yet".to_owned()
            ))
        );
        assert_eq!(
            verdict.tallies[0].to_string(),
            "lablet-model: nothing to measure yet (no viable mutants); the floor is exact"
        );
        let verdict = judge(&outcomes, &crates(true), &[], Scope::Full);
        let error = conclude(&verdict).unwrap_err().into_verdict();
        for phrase in [
            "mutants: 3 floor(s) below or unmeasured",
            "NOTHING MEASURED (no viable mutants)",
            "so the exact floor was not applied",
        ] {
            assert!(error.contains(phrase), "{error}");
        }
    }

    /// An exit with `code`, as the wait status a child leaves.
    fn exited(code: i32) -> ExitStatus {
        ExitStatus::from_raw(code << 8)
    }

    /// A stand-in for cargo-mutants that ends with `status`. It refuses to
    /// start while the last run's report is still there, keeps the arguments
    /// it was given in `args`, and writes `found` as its report when given.
    fn cargo_mutants<'a>(
        args: &'a RefCell<Vec<String>>,
        found: Option<&'a [String]>,
        status: ExitStatus,
    ) -> impl FnOnce(&[&str]) -> Result<ExitStatus, Error> + 'a {
        move |given: &[&str]| {
            *args.borrow_mut() = given.iter().map(|arg| (*arg).to_owned()).collect();
            let output = given.iter().position(|arg| *arg == "--output").unwrap() + 1;
            let report = Path::new(given[output]).join("mutants.out/outcomes.json");
            assert!(
                !report.exists(),
                "cargo-mutants started with the last run's report still at {}",
                report.display()
            );
            if let Some(found) = found {
                let text = format!(r#"{{"outcomes": [{}]}}"#, found.join(","));
                std::fs::create_dir_all(report.parent().unwrap()).unwrap();
                std::fs::write(&report, text).unwrap();
            }
            Ok(status)
        }
    }

    /// With `--in-diff` and no mutant in the diff, cargo-mutants exits 0 and
    /// leaves the last run's report where it was.
    #[test]
    fn a_report_a_previous_run_left_is_gone_before_the_next_run_reads_one() {
        let dir = TempDir::new("mutants-outcomes");
        let path = dir.path().join("mutants.out/outcomes.json");
        let last = format!(r#"{{"outcomes": [{}]}}"#, green().join(","));
        dir.write("mutants.out/outcomes.json", &last);
        assert_eq!(read_outcomes(&path).unwrap().outcomes.len(), 7);

        let args = RefCell::new(Vec::new());
        let outcomes = test_mutants(dir.path(), None, cargo_mutants(&args, None, exited(0)));
        assert_eq!(outcomes.unwrap().outcomes.len(), 0);
        assert_eq!(
            args.borrow()[3..5],
            ["--output".to_owned(), dir.path().display().to_string()]
        );

        dir.write("mutants.out/outcomes.json", "not JSON");
        let error = read_outcomes(&path).unwrap_err();
        assert!(
            matches!(&error, Error::Parse { path: named, .. } if *named == path),
            "{error:?}"
        );
        let prefix = format!("could not parse {}: ", path.display());
        assert!(chain(&error).starts_with(&prefix), "{}", chain(&error));
    }

    /// cargo-mutants exits 1 on bad arguments, 4 when the tests fail
    /// unmutated, and 70 on an internal error, and a signal leaves no code.
    #[test]
    fn only_exit_codes_0_2_and_3_are_a_run_with_outcomes_to_judge() {
        let args = RefCell::new(Vec::new());
        let found = green();
        for code in [0, 2, 3] {
            let dir = TempDir::new("mutants-exit");
            let run = cargo_mutants(&args, Some(&found), exited(code));
            let outcomes = test_mutants(dir.path(), None, run).unwrap();
            assert_eq!(outcomes.outcomes.len(), 7, "{code}");
        }
        for status in [exited(1), exited(4), exited(70), ExitStatus::from_raw(9)] {
            let dir = TempDir::new("mutants-exit");
            let run = cargo_mutants(&args, Some(&found), status);
            let error = test_mutants(dir.path(), None, run)
                .unwrap_err()
                .into_verdict();
            assert_eq!(
                error,
                format!(
                    "command failed (in lablet/): cargo mutants --no-config --cargo-arg=--locked \
                     --output {} --package lablet-model --package lablet-policy --package \
                     lablet-run; no mutants were tested (the tests must pass unmutated first)",
                    dir.path().display()
                ),
                "{status}"
            );
        }
    }

    #[test]
    fn a_scoped_run_tests_nothing_on_an_empty_diff_and_otherwise_says_it_judged_only_the_diff() {
        let since = "a1a1a1a1a1a1 (merge-base with origin/main)";
        let dir = TempDir::new("mutants-changed");
        let untested = |_: &[&str]| -> Result<ExitStatus, Error> {
            unreachable!("cargo-mutants ran on an empty diff")
        };
        assert_eq!(
            test_changed("", since, &crates(true), dir.path(), untested).unwrap(),
            Some(Note::Info(format!(
                "nothing to test: no Rust file of a floor crate changed since {since}"
            )))
        );
        assert!(!dir.path().join("changed.diff").exists());

        let diff = "diff --git a/crates/domain/model/src/lib.rs b/crates/domain/model/src/lib.rs\n";
        let args = RefCell::new(Vec::new());
        let scoped = |found: Option<&[String]>, status: ExitStatus| {
            let run = cargo_mutants(&args, found, status);
            test_changed(diff, since, &crates(true), dir.path(), run)
        };
        // No mutant in what changed, so cargo-mutants wrote no report.
        assert_eq!(
            scoped(None, exited(0)).unwrap(),
            Some(Note::Info(format!(
                "nothing to test: what changed since {since} holds no viable mutant"
            )))
        );
        let changed = dir.path().join("changed");
        let diff_path = dir.path().join("changed.diff");
        assert_eq!(
            args.borrow()[3..7],
            [
                "--output".to_owned(),
                changed.display().to_string(),
                "--in-diff".to_owned(),
                diff_path.display().to_string(),
            ]
        );
        assert_eq!(std::fs::read_to_string(&diff_path).unwrap(), diff);

        let caught = [mutant(MODEL, "69:5", WHOLE_MS, "CaughtMutant")];
        assert_eq!(
            scoped(Some(&caught), exited(0)).unwrap(),
            Some(Note::Info(format!(
                "only the mutants in what changed since {since}; the full run judges the floor"
            )))
        );
        let missed = [mutant(OBSERVER, "201:9", ON_TURN, "MissedMutant")];
        let error = scoped(Some(&missed), exited(2)).unwrap_err().into_verdict();
        assert!(
            error.starts_with("mutants: 1 floor(s) below or unmeasured\n"),
            "{error}"
        );
    }

    #[test]
    fn only_the_floor_crates_are_mutated_against_the_committed_lockfile() {
        let packages = [
            "--package",
            "lablet-model",
            "--package",
            "lablet-policy",
            "--package",
            "lablet-run",
        ];
        let full = mutants_args("/out", None);
        assert_eq!(
            full[..5],
            [
                "mutants",
                "--no-config",
                "--cargo-arg=--locked",
                "--output",
                "/out"
            ]
        );
        assert_eq!(full[5..], packages);

        let scoped = mutants_args("/out/changed", Some("/out/changed.diff"));
        assert_eq!(
            scoped[3..7],
            ["--output", "/out/changed", "--in-diff", "/out/changed.diff"]
        );
        assert_eq!(scoped[..3], full[..3]);
        assert_eq!(scoped[7..], packages);
        // The named mutants are tested like any other, or the list would rot.
        for args in [full, scoped] {
            let excludes = args.iter().any(|arg| arg.starts_with("--exclude"));
            assert!(!excludes, "{args:?}");
        }
    }

    #[test]
    fn the_scoped_diff_reads_the_rust_files_of_the_floor_crates() {
        let workspace = Workspace::load(&workspace_root()).unwrap();
        let crates = floors::crates(&workspace).unwrap();
        assert_eq!(
            pathspecs(&workspace.root, &crates),
            [
                ":(glob)crates/domain/model/**/*.rs",
                ":(glob)crates/domain/policy/**/*.rs",
                ":(glob)crates/application/run/**/*.rs",
            ]
        );
    }

    #[test]
    fn a_file_git_does_not_track_is_the_diff_that_adds_every_line_of_it() {
        assert_eq!(
            added_file(
                "crates/model/src/new.rs",
                "pub fn four() -> u8 {\n    4\n}\n"
            ),
            "diff --git a/crates/model/src/new.rs b/crates/model/src/new.rs\n\
             new file mode 100644\n\
             --- /dev/null\n\
             +++ b/crates/model/src/new.rs\n\
             @@ -0,0 +1,3 @@\n\
             +pub fn four() -> u8 {\n\
             +    4\n\
             +}\n"
        );
        assert_eq!(
            added_file("src/one.rs", "pub mod one;"),
            "diff --git a/src/one.rs b/src/one.rs\nnew file mode 100644\n--- /dev/null\n\
             +++ b/src/one.rs\n@@ -0,0 +1,1 @@\n+pub mod one;\n\\ No newline at end of file\n"
        );
        assert_eq!(added_file("src/empty.rs", ""), "");

        // As git writes it: a tab after a name that holds a space, on the
        // `+++` line alone.
        let spaced = added_file("crates/x/src/new one.rs", "pub mod one;\n");
        assert!(
            spaced.starts_with(
                "diff --git a/crates/x/src/new one.rs b/crates/x/src/new one.rs\n\
                 new file mode 100644\n--- /dev/null\n+++ b/crates/x/src/new one.rs\t\n"
            ),
            "{spaced}"
        );
        assert_eq!(spaced.matches('\t').count(), 1, "{spaced}");
    }

    /// A builder never commits, so what it wrote is staged, unstaged, or not
    /// tracked at all, and a new module is the last of these. A file whose
    /// timestamp moved and whose text didn't is none of them.
    #[test]
    fn the_scoped_diff_holds_what_is_committed_staged_unstaged_moved_and_untracked() {
        let dir = TempDir::new("mutants-git");
        let git = |args: &[&str]| scratch_git(dir.path(), args);
        let body = |value: u8| format!("pub fn value() -> u8 {{\n    {value}\n}}\n");
        let model = "lablet/crates/domain/model";
        let other = "lablet/crates/adapters/other";
        for file in ["committed", "staged", "unstaged", "untouched"] {
            dir.write(&format!("{model}/src/{file}.rs"), &body(1));
        }
        dir.write(&format!("{model}/src/moved.rs"), &body(9));
        dir.write(&format!("{model}/Cargo.toml"), "[package]\n");
        dir.write(&format!("{other}/src/lib.rs"), &body(1));
        dir.write(".gitignore", "ignored.rs\n");
        git(&["init", "--quiet", "--initial-branch=main"]);
        defy_git_defaults(dir.path(), "diff.noprefix");
        let resolved = git(&["rev-parse", "--show-toplevel"]);
        assert_eq!(
            std::fs::canonicalize(resolved.trim()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap(),
            "git resolved outside the scratch repository"
        );
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message=base"]);
        let base = git(&["rev-parse", "HEAD"]);

        dir.write(&format!("{model}/src/committed.rs"), &body(2));
        git(&["commit", "--quiet", "--all", "--message=on the branch"]);
        dir.write(&format!("{model}/src/staged.rs"), &body(3));
        git(&["add", &format!("{model}/src/staged.rs")]);
        dir.write(&format!("{model}/src/unstaged.rs"), &body(4));
        dir.write(&format!("{model}/src/untracked/new.rs"), &body(5));
        // None of these is a floor crate's Rust, tracked or not.
        dir.write(&format!("{model}/src/ignored.rs"), &body(6));
        dir.write(&format!("{model}/Cargo.toml"), "[package]\nname = \"m\"\n");
        dir.write(&format!("{model}/notes.md"), "Notes.\n");
        dir.write(&format!("{other}/src/lib.rs"), &body(7));
        dir.write(&format!("{other}/src/new.rs"), &body(8));
        let moved = [
            format!("{model}/src/moved.rs"),
            format!("{model}/src/renamed.rs"),
        ];
        git(&["mv", &moved[0], &moved[1]]);
        let untouched = dir.path().join(format!("{model}/src/untouched.rs"));
        let untouched = std::fs::File::options()
            .write(true)
            .open(untouched)
            .unwrap();
        let long_ago = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        untouched.set_modified(long_ago).unwrap();

        let index = || std::fs::read(dir.path().join(".git/index")).unwrap();
        let status = || git(&["--no-optional-locks", "status", "--porcelain"]);
        let (index_before, status_before) = (index(), status());

        let workspace = dir.path().join("lablet");
        let pathspecs = [":(glob)crates/domain/model/**/*.rs".to_owned()];
        let diff = changed_diff(&workspace, base.trim(), &pathspecs).unwrap();

        // Relative to the workspace root, as cargo-mutants names a file, and
        // the moved file gone from where it was and whole where it is.
        let files: Vec<&str> = diff
            .lines()
            .filter(|line| line.starts_with("--- ") || line.starts_with("+++ "))
            .collect();
        assert_eq!(
            files,
            [
                "--- a/crates/domain/model/src/committed.rs",
                "+++ b/crates/domain/model/src/committed.rs",
                "--- a/crates/domain/model/src/moved.rs",
                "+++ /dev/null",
                "--- /dev/null",
                "+++ b/crates/domain/model/src/renamed.rs",
                "--- a/crates/domain/model/src/staged.rs",
                "+++ b/crates/domain/model/src/staged.rs",
                "--- a/crates/domain/model/src/unstaged.rs",
                "+++ b/crates/domain/model/src/unstaged.rs",
                "--- /dev/null",
                "+++ b/crates/domain/model/src/untracked/new.rs",
            ]
        );
        let values: Vec<&str> = diff
            .lines()
            .filter(|line| line.starts_with("+    "))
            .collect();
        assert_eq!(values, ["+    2", "+    9", "+    3", "+    4", "+    5"]);
        for value in [9, 5] {
            let whole = format!("@@ -0,0 +1,3 @@\n+pub fn value() -> u8 {{\n+    {value}\n+}}\n");
            assert!(diff.contains(&whole), "{diff}");
        }

        assert!(index() == index_before, "the index was rewritten");
        assert_eq!(status(), status_before);

        // Nothing changed is an empty diff, and a run with nothing to test.
        let untouched = [":(glob)crates/domain/policy/**/*.rs".to_owned()];
        let diff = changed_diff(&workspace, base.trim(), &untouched).unwrap();
        assert_eq!(diff, "");
    }

    /// cargo-mutants' diff parser takes a name to end at the first space
    /// unless a tab ends it, and refuses git's octal quoting of a name that
    /// isn't ASCII. Either name needs `#[path]`, which a floor crate may use.
    #[test]
    fn a_name_with_a_space_or_beyond_ascii_reaches_the_diff_as_it_is() {
        let dir = TempDir::new("mutants-names");
        let git = |args: &[&str]| scratch_git(dir.path(), args);
        let body = |value: u8| format!("pub fn value() -> u8 {{\n    {value}\n}}\n");
        let model = "lablet/crates/domain/model";
        dir.write(&format!("{model}/src/café.rs"), &body(1));
        git(&["init", "--quiet", "--initial-branch=main"]);
        defy_git_defaults(dir.path(), "diff.mnemonicPrefix");
        let resolved = git(&["rev-parse", "--show-toplevel"]);
        assert_eq!(
            std::fs::canonicalize(resolved.trim()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap(),
            "git resolved outside the scratch repository"
        );
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message=base"]);
        let base = git(&["rev-parse", "HEAD"]);

        dir.write(&format!("{model}/src/café.rs"), &body(2));
        git(&["add", &format!("{model}/src/café.rs")]);
        dir.write(&format!("{model}/src/new one.rs"), &body(3));

        let workspace = dir.path().join("lablet");
        let pathspecs = [":(glob)crates/domain/model/**/*.rs".to_owned()];
        let diff = changed_diff(&workspace, base.trim(), &pathspecs).unwrap();
        let files: Vec<&str> = diff
            .lines()
            .filter(|line| line.starts_with("--- ") || line.starts_with("+++ "))
            .collect();
        assert_eq!(
            files,
            [
                "--- a/crates/domain/model/src/café.rs",
                "+++ b/crates/domain/model/src/café.rs",
                "--- /dev/null",
                "+++ b/crates/domain/model/src/new one.rs\t",
            ]
        );
    }

    #[test]
    fn a_last_report_that_cannot_be_removed_stops_the_run_before_it_starts() {
        let dir = TempDir::new("mutants-stuck");
        // A directory can't be removed as a file, and isn't missing either.
        let report = dir.path().join("mutants.out/outcomes.json");
        std::fs::create_dir_all(&report).unwrap();
        let args = RefCell::new(Vec::new());
        let run = cargo_mutants(&args, None, exited(0));
        let error = test_mutants(dir.path(), None, run)
            .unwrap_err()
            .into_error();
        let Error::File { verb, path, source } = &error else {
            panic!("{error:?}");
        };
        assert_eq!((*verb, path), (Verb::Remove, &report));
        let expected = format!("could not remove {}: {source}", report.display());
        assert_eq!(chain(&error), expected);
        assert_eq!(*args.borrow(), Vec::<String>::new(), "cargo-mutants ran");
    }

    /// cargo prints its own error for the manifest, which names it.
    #[test]
    fn the_cargo_a_run_goes_through_reports_how_cargo_exited() {
        let dir = TempDir::new("mutants-cargo");
        let absent = dir
            .path()
            .join("xtask-test-manifest-that-is-not-there/Cargo.toml");
        let absent = absent.display().to_string();
        let status = cargo(&["locate-project", "--manifest-path", &absent]).unwrap();
        assert_eq!(status.code(), Some(101));
    }

    #[test]
    fn the_scoped_run_starts_from_the_merge_base_with_origin_main_else_with_main() {
        let dir = TempDir::new("mutants-base");
        let git = |args: &[&str]| scratch_git(dir.path(), args).trim().to_owned();
        git(&["init", "--quiet", "--initial-branch=main"]);
        assert_eq!(
            std::fs::canonicalize(git(&["rev-parse", "--show-toplevel"])).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap(),
            "git resolved outside the scratch repository"
        );
        git(&["commit", "--quiet", "--allow-empty", "--message=base"]);
        let base = git(&["rev-parse", "HEAD"]);
        git(&["switch", "--quiet", "--create", "topic"]);
        git(&["commit", "--quiet", "--allow-empty", "--message=topic"]);
        let topic = git(&["rev-parse", "HEAD"]);
        let found = |revision: &str, branch: &str| {
            (revision.to_owned(), format!("merge-base with {branch}"))
        };
        assert_eq!(merge_base(dir.path()).unwrap(), found(&base, "main"));

        git(&["update-ref", "refs/remotes/origin/main", &topic]);
        assert_eq!(
            merge_base(dir.path()).unwrap(),
            found(&topic, "origin/main")
        );

        git(&["update-ref", "-d", "refs/remotes/origin/main"]);
        git(&["branch", "--quiet", "--delete", "--force", "main"]);
        let error = merge_base(dir.path()).unwrap_err();
        assert!(matches!(error, Error::Missing { .. }), "{error:?}");
        assert_eq!(
            chain(&error),
            "no merge-base with origin/main or main, so there is nothing to scope the run to. \
             Fetch origin, or run the full `cargo xtask mutants`"
        );

        // A `.git` file naming no repository stops git's search upward, so
        // what lies above the temporary directory doesn't matter.
        let outside = TempDir::new("mutants-not-a-repository");
        outside.write(
            ".git",
            &format!("gitdir: {}\n", outside.path().join("none").display()),
        );
        let error = merge_base(outside.path()).unwrap_err();
        let Error::Failed { stderr, .. } = &error else {
            panic!("{error:?}");
        };
        assert!(stderr.contains("not a git repository"), "{stderr}");
    }

    #[test]
    fn a_commit_id_is_shortened_for_the_note() {
        assert_eq!(short(&"a1".repeat(20)), "a1a1a1a1a1a1");
        assert_eq!(short("a1a1"), "a1a1");
    }
}
