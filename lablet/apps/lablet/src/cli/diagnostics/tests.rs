use std::ffi::OsStr;

use lablet::{Config, Format};

use super::{filter, telemetry_on_stderr};

fn shown(rust_log: Option<&str>, telemetry_on_stderr: bool) -> Option<String> {
    filter(rust_log.map(OsStr::new), telemetry_on_stderr).map(|filter| filter.to_string())
}

#[test]
fn the_log_shows_warnings_when_rust_log_asks_for_nothing() {
    assert_eq!(shown(None, false).as_deref(), Some("warn"));
    assert_eq!(shown(Some(""), false).as_deref(), Some("warn"));
}

#[test]
fn the_log_shows_what_rust_log_asks_for() {
    assert_eq!(
        shown(Some("lablet=debug"), false).as_deref(),
        Some("lablet=debug")
    );
    assert_eq!(
        shown(Some("lablet=debug"), true).as_deref(),
        Some("lablet=debug")
    );
}

#[test]
fn with_the_telemetry_on_standard_error_there_is_no_log_unless_rust_log_asks_for_one() {
    assert_eq!(shown(None, true), None);
    assert_eq!(shown(Some(""), true), None);
}

#[test]
fn only_a_telemetry_file_path_of_a_dash_is_standard_error() {
    let on_stderr = |telemetry: &str| {
        let config = Config::from_str(&format!("telemetry: {telemetry}"), Format::Yaml).unwrap();
        telemetry_on_stderr(&config)
    };
    assert!(on_stderr(r#"{ file: { path: "-" } }"#));
    assert!(!on_stderr(r#"{ file: { path: "-.jsonl" } }"#));
    assert!(!on_stderr(r#"{ file: { path: "telemetry.jsonl" } }"#));
    assert!(!on_stderr("{ file: { path: null } }"));
}
