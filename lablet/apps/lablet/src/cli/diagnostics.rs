//! The diagnostic log, which is about lablet, apart from the telemetry,
//! which is about the run.

use std::ffi::OsStr;
use std::path::Path;

use lablet::Config;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

/// Whether the config sends the telemetry to standard error, which a file
/// path of `-` does.
pub(crate) fn telemetry_on_stderr(config: &Config) -> bool {
    config.telemetry.file.path.as_deref() == Some(Path::new("-"))
}

/// What the diagnostic log shows: what `rust_log`, the value of `RUST_LOG`,
/// asks for, and warnings when it's unset or empty. `None` is no log at
/// all, which is what a run whose telemetry is on standard error has
/// unless `RUST_LOG` asks for one, so that nothing of lablet's own lands
/// between two OTLP lines. Whoever set `RUST_LOG` can route what it asks
/// for.
pub(crate) fn filter(rust_log: Option<&OsStr>, telemetry_on_stderr: bool) -> Option<EnvFilter> {
    let asked = rust_log
        .map(OsStr::to_string_lossy)
        .filter(|directives| !directives.is_empty());
    if asked.is_none() && telemetry_on_stderr {
        return None;
    }
    Some(
        EnvFilter::builder()
            .with_default_directive(LevelFilter::WARN.into())
            .parse_lossy(asked.unwrap_or_default()),
    )
}

#[cfg(test)]
mod tests;
