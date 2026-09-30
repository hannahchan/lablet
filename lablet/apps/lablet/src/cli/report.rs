//! What a command reports: its exit code, and the summary line a run ends
//! with.

use lablet::{FinishedRun, StopReason};
use lablet_model::Usage;

/// The exit code of a command that never started a run, or of `init`
/// when it wrote nothing.
pub(crate) const NOT_STARTED: u8 = 1;

/// The exit code of a run that ended with `reason`: 0 when it completed,
/// and 2 for any other stop reason, so a script tells a run that never
/// started, with 1, from one that did and didn't complete.
pub(crate) fn exit_code(reason: StopReason) -> u8 {
    if reason == StopReason::Completed {
        0
    } else {
        2
    }
}

/// The exit code of arguments that `clap` answered instead of the command:
/// 0 for the help and the version, and 1 for arguments it refused, which
/// `clap` would give 2, the code of a run that started.
pub(crate) fn usage_exit_code(error: &clap::Error) -> u8 {
    if error.use_stderr() { NOT_STARTED } else { 0 }
}

/// Whether a run ends with its summary line: not with `--quiet`, and not
/// when the telemetry is on standard error, where the line would land
/// among the OTLP lines.
pub(crate) const fn summary_wanted(quiet: bool, telemetry_on_stderr: bool) -> bool {
    !quiet && !telemetry_on_stderr
}

/// The line a run ends with on standard error: the stop reason, the turns,
/// the tokens, the tool calls and the duration. `budget` is
/// `run.max_total_tokens`: a run that reached it gives both numbers, and
/// its count is what the budget counts, the attempts that failed included.
pub(crate) fn summary(run: &FinishedRun, budget: Option<u64>) -> String {
    let outcome = &run.summary.outcome;
    let reason = outcome.stop_reason();
    let tokens = match (reason, budget) {
        (StopReason::MaxTotalTokens, Some(budget)) => {
            let failed = run.summary.failed_usage.as_ref().map_or(0, Usage::total);
            format!(
                "token budget reached: {} of {} tokens",
                grouped(outcome.usage.total().saturating_add(failed)),
                grouped(budget)
            )
        }
        _ => counted(outcome.usage.total(), "token", "tokens"),
    };
    format!(
        "{reason}: {}, {tokens}, {}, {}",
        counted(u64::from(outcome.turns), "turn", "turns"),
        counted(outcome.tool_calls, "tool call", "tool calls"),
        duration(outcome.duration_ms)
    )
}

/// `count` and the name of what it counts, as one or as many.
fn counted(count: u64, one: &str, many: &str) -> String {
    format!("{} {}", grouped(count), if count == 1 { one } else { many })
}

/// `number` with its digits in groups of three, as `100,412`.
fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// `ms` as a person reads a duration: milliseconds under a second, tenths
/// of a second under a minute, and minutes and seconds after that.
fn duration(ms: u64) -> String {
    match ms {
        0..1_000 => format!("{ms}ms"),
        1_000..60_000 => format!("{}.{}s", ms / 1_000, ms % 1_000 / 100),
        _ => format!("{}m {}s", ms / 60_000, ms % 60_000 / 1_000),
    }
}

#[cfg(test)]
mod tests;
