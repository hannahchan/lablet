use std::ffi::OsStr;

use super::filter;

fn shown(rust_log: Option<&str>, telemetry_on_stderr: bool) -> Option<String> {
    filter(rust_log.map(OsStr::new), telemetry_on_stderr).map(|filter| filter.to_string())
}

#[test]
fn the_log_shows_warnings_when_rust_log_asks_for_nothing() {
    assert_eq!(shown(None, false).as_deref(), Some("warn"));
    assert_eq!(shown(Some(""), false).as_deref(), Some("warn"));
}

/// The OpenTelemetry crates' targets are held to `warn` beside what's
/// asked for, so their events at `debug` don't print an endpoint.
#[test]
fn the_log_shows_what_rust_log_asks_for_with_the_opentelemetry_crates_held_to_warn() {
    let floored = "opentelemetry_otlp=warn,opentelemetry_sdk=warn,opentelemetry=warn";

    assert_eq!(
        shown(Some("lablet=debug"), false).as_deref(),
        Some(format!("{floored},lablet=debug").as_str())
    );
    assert_eq!(
        shown(Some("lablet=debug"), true).as_deref(),
        Some(format!("{floored},lablet=debug").as_str())
    );
    assert_eq!(
        shown(Some("debug"), false).as_deref(),
        Some(format!("{floored},debug").as_str())
    );
}

/// A crate `RUST_LOG` names, as a target or a module within one, is shown
/// as asked, and the other two are still held.
#[test]
fn an_opentelemetry_crate_rust_log_names_is_shown_as_asked() {
    assert_eq!(
        shown(Some("opentelemetry_otlp=debug"), false).as_deref(),
        Some("opentelemetry_otlp=debug,opentelemetry_sdk=warn,opentelemetry=warn")
    );
    assert_eq!(
        shown(
            Some("opentelemetry::trace=trace,opentelemetry_sdk=info"),
            false
        )
        .as_deref(),
        Some("opentelemetry::trace=trace,opentelemetry_otlp=warn,opentelemetry_sdk=info")
    );
    assert_eq!(
        shown(Some("opentelemetry_otlp_x=debug"), false).as_deref(),
        Some(
            "opentelemetry_otlp_x=debug,opentelemetry_otlp=warn,opentelemetry_sdk=warn,\
             opentelemetry=warn"
        ),
        "a name that only begins with a crate's doesn't name it"
    );
}

#[test]
fn with_the_telemetry_on_standard_error_there_is_no_log_unless_rust_log_asks_for_one() {
    assert_eq!(shown(None, true), None);
    assert_eq!(shown(Some(""), true), None);
}
