//! The end-of-gate report: a verdict line, then one row per step with its
//! status, its duration, and any note it left.

use std::fmt::Write as _;

/// One step's outcome.
#[derive(Debug)]
pub struct Row {
    /// The step's label.
    pub name: String,
    /// How long it ran, in seconds.
    pub elapsed: f64,
    /// Whether it passed.
    pub ok: bool,
    /// What a passing step wants known: a skipped comparison, an empty crate.
    pub note: Option<String>,
}

/// The report for `target`, or nothing when no step ran.
pub fn render(target: &str, rows: &[Row]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let total: f64 = rows.iter().map(|row| row.elapsed).sum();
    let ran = rows.len();
    let failed = rows.iter().filter(|row| !row.ok).count();
    let verdict = if failed == 0 {
        format!("{ran} step(s) ok")
    } else {
        format!("{failed} of {ran} step(s) failed")
    };

    // Width fits the longest step name.
    let width = rows.iter().map(|row| row.name.len()).max().unwrap_or(0);
    let rule = "─".repeat(width + 22);
    let mut out = format!("{target}  ·  {verdict}  ·  {total:.1}s\n{rule}\n");
    for row in rows {
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
            note: note.map(str::to_owned),
        }
    }

    #[test]
    fn a_gate_that_ran_nothing_reports_nothing() {
        assert_eq!(render("pre-commit", &[]), "");
    }

    #[test]
    fn a_green_gate_lists_every_step_with_its_time_and_note() {
        let rows = [
            row("fmt", 0.42, true, None),
            row("changelog", 0.04, true, Some("no contract file changed")),
        ];
        let rule = "─".repeat(31);
        assert_eq!(
            render("pre-push", &rows),
            format!(
                "pre-push  ·  2 step(s) ok  ·  0.5s\n{rule}\n\
                 fmt         ok          0.4s\n\
                 changelog   ok          0.0s   no contract file changed\n\
                 {rule}\n"
            )
        );
    }

    #[test]
    fn a_red_gate_counts_its_failures_and_marks_each_one() {
        let rows = [
            row("fmt", 1.0, false, None),
            row("clippy", 2.0, true, None),
            row("lint-layers", 0.0, false, None),
        ];
        let report = render("pre-commit", &rows);
        assert!(
            report.starts_with("pre-commit  ·  2 of 3 step(s) failed  ·  3.0s\n"),
            "{report}"
        );
        assert_eq!(report.matches("FAIL").count(), 2, "{report}");
    }
}
