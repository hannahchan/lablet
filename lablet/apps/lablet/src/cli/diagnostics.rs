//! The diagnostic log, which is about lablet, apart from the telemetry,
//! which is about the run.

use std::ffi::OsStr;

use tracing::{Metadata, Subscriber};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::{LevelFilter, filter_fn};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt as _;

/// The crates whose own events are held to `warn` unless `RUST_LOG` names
/// one: at `debug` the exporter prints the endpoint it resolved, and the
/// network crates under it print the address they connect to, either of
/// which may hold what a variable holds, and a line of the log holds an
/// endpoint only as the config writes it.
const FLOORED: [&str; 10] = [
    "opentelemetry",
    "opentelemetry_sdk",
    "opentelemetry_otlp",
    "tonic",
    "tower",
    "hyper",
    "hyper_util",
    "h2",
    "reqwest",
    "rustls",
];

/// What the diagnostic log shows: what `rust_log`, the value of `RUST_LOG`,
/// asks for, with [`FLOORED`] held to `warn` unless it names one, and
/// warnings when it's unset or empty. `None` is no log at all, which is
/// what a run whose telemetry is on standard error has unless `RUST_LOG`
/// asks for one, so that nothing of lablet's own lands between two OTLP
/// lines. Whoever set `RUST_LOG` can route what it asks for.
pub(crate) fn filter(rust_log: Option<&OsStr>, telemetry_on_stderr: bool) -> Option<EnvFilter> {
    let asked = rust_log
        .map(OsStr::to_string_lossy)
        .filter(|directives| !directives.is_empty());
    if asked.is_none() && telemetry_on_stderr {
        return None;
    }
    // An empty `RUST_LOG` gets the default directive, `warn`, which floors
    // everything already; a directive beside it would unseat it.
    let directives = asked.map_or_else(String::new, |asked| floored(&asked));
    Some(
        EnvFilter::builder()
            .with_default_directive(LevelFilter::WARN.into())
            .parse_lossy(directives),
    )
}

/// The diagnostic log: what `filter` lets through but the SDK's warnings
/// about the configuration it read itself, written to `writer`, in colour
/// when `ansi` says so.
pub(crate) fn subscriber<W>(
    filter: EnvFilter,
    writer: W,
    ansi: bool,
) -> impl Subscriber + Send + Sync + 'static
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(ansi)
        .finish()
        .with(filter_fn(not_the_sdks_own_configuration))
}

/// Whether an event isn't one of the SDK's warnings about a setting it read
/// from the environment itself. Its tracer provider's builder reads the
/// sampler variables, case-sensitively, and warns of a value it doesn't
/// take, naming the sampler it falls back to. Lablet reads the same
/// variables through the seam and states the sampler on that builder, so
/// such a warning describes a sampler that decides nothing, and stands
/// beside lablet's own warning when the value is one lablet can't use
/// either.
fn not_the_sdks_own_configuration(metadata: &Metadata<'_>) -> bool {
    !(metadata.target() == "opentelemetry_sdk"
        && metadata.name().starts_with("TracerProvider.Config."))
}

/// `asked` with each of [`FLOORED`] that it doesn't name, as a target or as
/// a module within one, held to `warn`.
fn floored(asked: &str) -> String {
    let names = |target: &str| {
        asked.split(',').any(|directive| {
            let named = directive.split(['=', '[']).next().unwrap_or_default();
            named == target
                || named
                    .strip_prefix(target)
                    .is_some_and(|rest| rest.starts_with("::"))
        })
    };
    let mut directives = asked.to_owned();
    for target in FLOORED {
        if !names(target) {
            directives.push(',');
            directives.push_str(target);
            directives.push_str("=warn");
        }
    }
    directives
}

#[cfg(test)]
mod tests;
