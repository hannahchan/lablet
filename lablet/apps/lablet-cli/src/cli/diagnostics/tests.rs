use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::sync::{Arc, Mutex};

use super::{FLOORED, filter, subscriber};

fn shown(rust_log: Option<&str>, telemetry_on_stderr: bool) -> Option<String> {
    filter(rust_log.map(OsStr::new), telemetry_on_stderr).map(|filter| filter.to_string())
}

/// The directives of a filter, in no order, since the filter orders them
/// its own way.
fn directives(shown: Option<String>) -> BTreeSet<String> {
    shown.unwrap().split(',').map(str::to_owned).collect()
}

/// `asked`, beside every floored crate held to `warn` but those `lifted`.
fn floored_beside(asked: &[&str], lifted: &[&str]) -> BTreeSet<String> {
    asked
        .iter()
        .map(|directive| (*directive).to_owned())
        .chain(
            FLOORED
                .iter()
                .filter(|target| !lifted.contains(target))
                .map(|target| format!("{target}=warn")),
        )
        .collect()
}

#[test]
fn the_log_shows_warnings_when_rust_log_asks_for_nothing() {
    assert_eq!(shown(None, false).as_deref(), Some("warn"));
    assert_eq!(shown(Some(""), false).as_deref(), Some("warn"));
}

/// The OpenTelemetry crates' targets, and the network crates' under them,
/// are held to `warn` beside what's asked for, so their events at `debug`
/// don't print an endpoint.
#[test]
fn the_log_shows_what_rust_log_asks_for_with_the_opentelemetry_and_network_crates_held_to_warn() {
    for crate_name in ["opentelemetry_otlp", "tonic", "hyper_util", "reqwest"] {
        assert!(FLOORED.contains(&crate_name), "{crate_name}");
    }

    assert_eq!(
        directives(shown(Some("lablet=debug"), false)),
        floored_beside(&["lablet=debug"], &[])
    );
    assert_eq!(
        directives(shown(Some("lablet=debug"), true)),
        floored_beside(&["lablet=debug"], &[])
    );
    assert_eq!(
        directives(shown(Some("debug"), false)),
        floored_beside(&["debug"], &[])
    );
}

/// A crate `RUST_LOG` names, as a target or a module within one, is shown
/// as asked, and the others are still held.
#[test]
fn a_floored_crate_rust_log_names_is_shown_as_asked() {
    assert_eq!(
        directives(shown(Some("opentelemetry_otlp=debug"), false)),
        floored_beside(&["opentelemetry_otlp=debug"], &["opentelemetry_otlp"])
    );
    assert_eq!(
        directives(shown(
            Some("opentelemetry::trace=trace,hyper_util=info"),
            false
        )),
        floored_beside(
            &["opentelemetry::trace=trace", "hyper_util=info"],
            &["opentelemetry", "hyper_util"]
        )
    );
    assert_eq!(
        directives(shown(
            Some("opentelemetry_otlp_x=debug,hyper_x=debug"),
            false
        )),
        floored_beside(&["opentelemetry_otlp_x=debug", "hyper_x=debug"], &[]),
        "a name that only begins with a crate's doesn't name it"
    );
}

#[test]
fn with_the_telemetry_on_standard_error_there_is_no_log_unless_rust_log_asks_for_one() {
    assert_eq!(shown(None, true), None);
    assert_eq!(shown(Some(""), true), None);
}

/// What the log writes, kept.
#[derive(Debug, Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Written {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The SDK names its events as below, under its crate's name as the
/// target. A warning about the sampler it read for itself is left out, and
/// the SDK's other warnings and lablet's own are kept, whatever `RUST_LOG`
/// asks for.
#[test]
fn the_log_leaves_out_the_sdks_warnings_about_its_own_configuration_and_nothing_else() {
    for rust_log in [None, Some("warn,opentelemetry_sdk=debug")] {
        let written = Written::default();
        let log = subscriber(
            filter(rust_log.map(OsStr::new), false).unwrap(),
            written.clone(),
            false,
        );

        tracing::subscriber::with_default(log, || {
            tracing::warn!(
                name: "TracerProvider.Config.InvalidSamplerType",
                target: "opentelemetry_sdk",
                name = "TracerProvider.Config.InvalidSamplerType",
                message = "Using fallback sampler: ParentBased(AlwaysOn)",
            );
            tracing::warn!(
                name: "TracerProvider.Config.InvalidSamplerArgument",
                target: "opentelemetry_sdk",
                name = "TracerProvider.Config.InvalidSamplerArgument",
            );
            tracing::warn!(
                name: "BatchSpanProcessor.Export.Error",
                target: "opentelemetry_sdk",
                name = "BatchSpanProcessor.Export.Error",
            );
            tracing::warn!(
                name: "TracerProvider.Config.Elsewhere",
                target: "lablet",
                "lablet's own"
            );
        });

        let text = String::from_utf8(written.0.lock().unwrap().clone()).unwrap();
        assert!(!text.contains("fallback"), "{rust_log:?}: {text}");
        assert!(
            !text.contains("InvalidSamplerArgument"),
            "{rust_log:?}: {text}"
        );
        assert!(
            text.contains("BatchSpanProcessor.Export.Error"),
            "{rust_log:?}: {text}"
        );
        assert!(text.contains("lablet's own"), "{rust_log:?}: {text}");
    }
}
