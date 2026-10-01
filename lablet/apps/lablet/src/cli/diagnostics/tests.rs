use std::collections::BTreeSet;
use std::ffi::OsStr;

use super::{FLOORED, filter};

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
