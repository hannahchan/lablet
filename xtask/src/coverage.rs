//! `cargo xtask coverage`: line coverage from cargo-llvm-cov, held to the
//! floors in [`crate::floors`]. The whole workspace's tests run once; each
//! floor crate is then judged on the lines of its own source files.
//!
//! The floor is on production lines. Unit tests are inline (spec §9), and the
//! lines of a `#[cfg(test)] mod` are covered by construction: counted, they
//! would lift a crate over the floor with its production code well under it.
//! So the report is read line by line (lcov), and a file's lines from its
//! test module on are left out.

use std::path::{Path, PathBuf};

use crate::floors::{self, Floor, Line};
use crate::gates::{CheckResult, LOCKED};
use crate::process;
use crate::workspace::{Workspace, workspace_root};

/// One source file of an lcov report: its `SF:` path and, from its `DA:`
/// records, each instrumented line with its execution count.
#[derive(Debug, PartialEq, Eq)]
struct FileLines {
    path: PathBuf,
    lines: Vec<(u64, u64)>,
}

/// Reads the `SF:` and `DA:` records of an lcov tracefile; the rest (function
/// and branch records, totals) is not needed.
fn parse_lcov(text: &str) -> Result<Vec<FileLines>, String> {
    let mut files: Vec<FileLines> = Vec::new();
    for record in text.lines() {
        if let Some(path) = record.strip_prefix("SF:") {
            files.push(FileLines {
                path: PathBuf::from(path),
                lines: Vec::new(),
            });
        } else if let Some(data) = record.strip_prefix("DA:") {
            let mut fields = data.split(',');
            let line = fields.next().and_then(|field| field.parse().ok());
            let count = fields.next().and_then(|field| field.parse().ok());
            let (Some(file), Some(line), Some(count)) = (files.last_mut(), line, count) else {
                return Err(format!("unreadable lcov record `{record}`"));
            };
            file.lines.push((line, count));
        }
    }
    Ok(files)
}

/// The 1-based line a file's inline test module starts on: the first
/// `#[cfg(test)]` that stands directly above a `mod name {`. Clippy's
/// `items_after_test_module` keeps an inline test module last, so everything
/// from there to the end of the file is test code. A `mod name;` declaration
/// is not a start: production code may follow it.
fn test_module_start(source: &str) -> Option<u64> {
    let mut lines = (1..).zip(source.lines().map(str::trim)).peekable();
    while let Some((number, line)) = lines.next() {
        let next = lines.peek().map_or("", |(_, next)| *next);
        let inline_module =
            (next.starts_with("mod ") || next.starts_with("pub mod ")) && next.ends_with('{');
        if line == "#[cfg(test)]" && inline_module {
            return Some(number);
        }
    }
    None
}

/// The first file of the report that is not under the workspace root. Every
/// file llvm-cov reports is a workspace source file, so one outside it means
/// the paths were rewritten (`--remap-path-prefix` in RUSTFLAGS or a cargo
/// config), and matching crates by directory would find no lines at all.
fn outside_the_workspace<'a>(files: &'a [FileLines], workspace_root: &Path) -> Option<&'a Path> {
    files
        .iter()
        .map(|file| file.path.as_path())
        .find(|path| !path.starts_with(workspace_root))
}

/// One report line per floor crate: the production lines of every file under
/// the crate's directory, summed. `test_start` gives the line a file's test
/// module starts on. A crate with no file in the report has no coverable
/// lines.
fn lines_by_crate(
    files: &[FileLines],
    crates: &[(&'static Floor, PathBuf)],
    test_start: impl Fn(&Path) -> Option<u64>,
) -> Vec<Line> {
    crates
        .iter()
        .map(|(floor, directory)| {
            let (mut hit, mut total) = (0, 0);
            for file in files.iter().filter(|file| file.path.starts_with(directory)) {
                let end = test_start(&file.path).unwrap_or(u64::MAX);
                for (_, count) in file.lines.iter().filter(|(line, _)| *line < end) {
                    total += 1;
                    hit += u64::from(*count > 0);
                }
            }
            Line {
                package: floor.package,
                hit,
                total,
                floor: floor.line_coverage,
                unit: "production lines covered",
                none_of: "coverable lines",
                may_be_empty: floor.may_be_empty,
            }
        })
        .collect()
}

/// The cargo-llvm-cov command: every test of the workspace, the committed
/// lockfile, an lcov tracefile at `report`.
fn llvm_cov_args(report: &str) -> [&str; 6] {
    [
        "llvm-cov",
        LOCKED,
        "--workspace",
        "--lcov",
        "--output-path",
        report,
    ]
}

/// Runs the workspace's tests under cargo-llvm-cov and judges the floors.
pub fn check() -> CheckResult {
    let workspace = Workspace::load(&workspace_root()).map_err(|e| e.to_string())?;
    // llvm-cov reports resolved paths, so compare against resolved ones.
    let resolved = |directory: PathBuf| directory.canonicalize().unwrap_or(directory);
    let crates: Vec<(&'static Floor, PathBuf)> = floors::crate_directories(&workspace)?
        .into_iter()
        .map(|(floor, directory)| (floor, resolved(directory)))
        .collect();

    let report = floors::output_directory(&workspace.root)?.join("coverage.lcov");
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
    let files = parse_lcov(&text).map_err(|e| format!("{}: {e}", report.display()))?;
    let root = resolved(workspace.root.clone());
    if let Some(stray) = outside_the_workspace(&files, &root) {
        return Err(format!(
            "the coverage report names {}, which is not under {}, so no crate's lines can be \
             found by directory; is --remap-path-prefix set in RUSTFLAGS or a cargo config?",
            stray.display(),
            root.display()
        ));
    }
    let lines = lines_by_crate(&files, &crates, |path| {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|source| test_module_start(&source))
    });
    floors::conclude("coverage", &lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::floors::Standing;

    /// A cut-down cargo-llvm-cov 0.9 lcov report: two files in one crate, one
    /// in a crate whose name shares a prefix, one in another floor crate.
    const REPORT: &str = "\
SF:/ws/crates/domain/model/src/lib.rs
FN:1,_RNvCs_5model3add
FNDA:4,_RNvCs_5model3add
FNF:1
FNH:1
DA:1,4
DA:2,4
DA:3,0
DA:4,4
DA:10,1
DA:11,1
DA:12,1
LF:7
LH:6
end_of_record
SF:/ws/crates/domain/model/src/usage.rs
DA:1,2
DA:2,0
end_of_record
SF:/ws/crates/domain/model-extras/src/lib.rs
DA:1,0
DA:2,0
DA:3,0
end_of_record
SF:/ws/crates/application/run/src/lib.rs
DA:5,1
end_of_record
";

    fn floor(package: &'static str) -> &'static Floor {
        floors::FLOORS
            .iter()
            .find(|floor| floor.package == package)
            .unwrap()
    }

    fn crates() -> Vec<(&'static Floor, PathBuf)> {
        [
            ("lablet-model", "/ws/crates/domain/model"),
            ("lablet-policy", "/ws/crates/domain/policy"),
            ("lablet-run", "/ws/crates/application/run"),
        ]
        .map(|(package, directory)| (floor(package), PathBuf::from(directory)))
        .into()
    }

    fn seen(lines: &[Line]) -> Vec<(&str, u64, u64, Standing)> {
        lines
            .iter()
            .map(|line| (line.package, line.hit, line.total, line.standing()))
            .collect()
    }

    #[test]
    fn a_crate_is_judged_on_the_files_under_its_own_directory() {
        let files = parse_lcov(REPORT).unwrap();
        let lines = lines_by_crate(&files, &crates(), |_| None);
        assert_eq!(
            seen(&lines),
            [
                // 7 of 9: `model-extras` shares a name prefix, not a directory.
                ("lablet-model", 7, 9, Standing::Below),
                ("lablet-policy", 0, 0, Standing::NothingToMeasure),
                ("lablet-run", 1, 1, Standing::Met),
            ]
        );
    }

    #[test]
    fn the_lines_of_an_inline_test_module_do_not_count() {
        let files = parse_lcov(REPORT).unwrap();
        // lib.rs: production lines 1 to 4, a test module from line 9 on.
        let lines = lines_by_crate(&files, &crates(), |path| {
            path.ends_with("model/src/lib.rs").then_some(9)
        });
        // 3 of 4 in lib.rs and 1 of 2 in usage.rs; the three covered test
        // lines would have made it 7 of 9.
        assert_eq!(seen(&lines)[0], ("lablet-model", 4, 6, Standing::Below));
    }

    #[test]
    fn a_test_module_starts_at_the_cfg_test_above_a_mod() {
        let source = "pub fn add() {}\n\n#[cfg(test)]\nfn helper() {}\n\n    #[cfg(test)]\n    mod tests {\n}\n";
        assert_eq!(test_module_start(source), Some(6));
        assert_eq!(
            test_module_start("#[cfg(test)]\npub mod fixture {\n}\n"),
            Some(1)
        );
        assert_eq!(test_module_start("pub fn add() {}\n"), None);
        // Production code may follow a declaration of an out-of-line module.
        assert_eq!(
            test_module_start("#[cfg(test)]\nmod tests;\n\npub fn add() {}\n"),
            None
        );
        assert_eq!(test_module_start("#[cfg(test)]"), None);
    }

    #[test]
    fn an_lcov_record_that_cannot_be_read_is_an_error() {
        assert_eq!(parse_lcov("").unwrap(), []);
        assert_eq!(
            parse_lcov("SF:/a.rs\nDA:3,2,abc\nend_of_record\n").unwrap(),
            [FileLines {
                path: PathBuf::from("/a.rs"),
                lines: vec![(3, 2)]
            }]
        );
        for broken in ["DA:1,1\n", "SF:/a.rs\nDA:one,1\n", "SF:/a.rs\nDA:1\n"] {
            assert!(parse_lcov(broken).is_err(), "{broken}");
        }
    }

    #[test]
    fn a_report_file_outside_the_workspace_is_found() {
        let files = parse_lcov(REPORT).unwrap();
        assert_eq!(outside_the_workspace(&files, Path::new("/ws")), None);
        // What `--remap-path-prefix=<workspace>=.` or `=/build` leaves behind.
        for remapped in [
            "crates/domain/model/src/lib.rs",
            "/build/crates/domain/model/src/lib.rs",
        ] {
            let files = parse_lcov(&format!("SF:{remapped}\nDA:1,0\nend_of_record\n")).unwrap();
            assert_eq!(
                outside_the_workspace(&files, Path::new("/ws")),
                Some(Path::new(remapped))
            );
        }
    }

    #[test]
    fn an_empty_report_has_nothing_to_measure_anywhere() {
        let lines = lines_by_crate(&[], &crates(), |_| None);
        assert!(
            lines
                .iter()
                .all(|line| line.standing() == Standing::NothingToMeasure)
        );
    }

    #[test]
    fn coverage_runs_every_workspace_test_against_the_committed_lockfile() {
        let args = llvm_cov_args("/out/coverage.lcov");
        assert_eq!(args[..2], ["llvm-cov", "--locked"]);
        assert!(args.contains(&"--workspace"));
        assert!(args.ends_with(&["--lcov", "--output-path", "/out/coverage.lcov"]));
    }
}
