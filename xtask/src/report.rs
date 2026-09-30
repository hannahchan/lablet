//! A gate's closing report: the verdict line and, when a step failed, a table
//! of every step with the failed ones first.

use std::fmt::{self, Write as _};

/// What a passing step wants known, or how to re-run a failed one.
#[derive(Debug, PartialEq, Eq)]
pub enum Note {
    /// Shown when a run lists its passes.
    Info(String),
    /// Why a pass judged less than it is for, as an unresolvable base has the
    /// changelog gate do (spec §8). Shown however quiet the run, and counted
    /// on a green gate's line.
    Warning(String),
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info(text) => f.write_str(text),
            Self::Warning(text) => write!(f, "warning: {text}"),
        }
    }
}

/// One step's outcome.
#[derive(Debug)]
pub struct Row {
    /// The step's label.
    pub name: String,
    /// How long it ran, in seconds.
    pub elapsed: f64,
    /// Whether it passed.
    pub ok: bool,
    /// What a passing step wants known, or how to re-run a failed one.
    pub note: Option<Note>,
}

/// The report for `target`, or nothing when no step ran.
pub fn render(target: &str, rows: &[Row]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let total: f64 = rows.iter().map(|row| row.elapsed).sum();
    let ran = rows.len();
    let steps = if ran == 1 { "step" } else { "steps" };
    let failed = rows.iter().filter(|row| !row.ok).count();
    if failed == 0 {
        let warned = rows
            .iter()
            .filter(|row| matches!(row.note, Some(Note::Warning(_))))
            .count();
        return if warned == 0 {
            format!("{target} · {ran} {steps} ok · {total:.1}s\n")
        } else {
            format!("{target} · {ran} {steps} ok · {warned} warned · {total:.1}s\n")
        };
    }

    let width = rows.iter().map(|row| row.name.len()).max().unwrap_or(0);
    let rule = "─".repeat(width + 22);
    let mut out = format!("{target} · {failed} of {ran} {steps} failed · {total:.1}s\n{rule}\n");
    let (red, green): (Vec<&Row>, Vec<&Row>) = rows.iter().partition(|row| !row.ok);
    for row in red.into_iter().chain(green) {
        let status = if row.ok { "ok" } else { "FAIL" };
        let seconds = format!("{:.1}s", row.elapsed);
        let _ = write!(out, "{:<width$}   {status:<4}   {seconds:>9}", row.name);
        if let Some(note) = &row.note {
            let _ = write!(out, "   {note}");
        }
        out.push('\n');
    }
    out.push_str(&rule);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, elapsed: f64, ok: bool, note: Option<&str>) -> Row {
        Row {
            name: name.to_owned(),
            elapsed,
            ok,
            note: note.map(|text| Note::Info(text.to_owned())),
        }
    }

    #[test]
    fn a_gate_that_ran_nothing_reports_nothing() {
        assert_eq!(render("pre-commit", &[]), "");
    }

    #[test]
    fn a_green_gate_is_one_line() {
        let rows = [
            row("fmt", 0.42, true, None),
            row("changelog", 0.04, true, Some("no contract file changed")),
        ];
        assert_eq!(render("pre-push", &rows), "pre-push · 2 steps ok · 0.5s\n");
        assert_eq!(render("ci", &rows[..1]), "ci · 1 step ok · 0.4s\n");
    }

    #[test]
    fn a_green_gate_whose_step_warned_says_so_on_its_one_line() {
        let warned = Row {
            note: Some(Note::Warning("skipped".to_owned())),
            ..row("changelog", 0.04, true, None)
        };
        let rows = [row("fmt", 0.42, true, None), warned];
        assert_eq!(
            render("pre-push", &rows),
            "pre-push · 2 steps ok · 1 warned · 0.5s\n"
        );
    }

    #[test]
    fn a_red_gate_lists_every_step_with_the_failed_ones_first() {
        let rows = [
            row("fmt", 1.0, true, None),
            row("clippy", 2.0, false, Some("re-run: cargo xtask clippy")),
            Row {
                note: Some(Note::Warning("skipped".to_owned())),
                ..row("changelog", 0.04, true, None)
            },
        ];
        let rule = "─".repeat(31);
        assert_eq!(
            render("pre-commit", &rows),
            format!(
                "pre-commit · 1 of 3 steps failed · 3.0s\n{rule}\n\
                 clippy      FAIL        2.0s   re-run: cargo xtask clippy\n\
                 fmt         ok          1.0s\n\
                 changelog   ok          0.0s   warning: skipped\n\
                 {rule}\n"
            )
        );
    }
}
