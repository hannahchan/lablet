//! What holds for the crate as a whole.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use lablet_telemetry_registry::{attribute, signals};

/// Every key of every signal the registry declares, and the keys of the
/// resource.
fn declared() -> BTreeSet<&'static str> {
    [
        signals::SPAN_LABLET_INVOKE_AGENT_KEYS,
        signals::SPAN_LABLET_CHAT_KEYS,
        signals::SPAN_LABLET_EXECUTE_TOOL_KEYS,
        signals::EVENT_LABLET_RUN_KEYS,
        signals::EVENT_LABLET_RETRY_KEYS,
        signals::EVENT_GEN_AI_CLIENT_OPERATION_EXCEPTION_KEYS,
        signals::EVENT_GEN_AI_CLIENT_INFERENCE_OPERATION_DETAILS_KEYS,
        &[
            attribute::SERVICE_NAME,
            attribute::SERVICE_VERSION,
            attribute::TELEMETRY_SDK_NAME,
            attribute::TELEMETRY_SDK_LANGUAGE,
            attribute::TELEMETRY_SDK_VERSION,
        ],
    ]
    .concat()
    .into_iter()
    .collect()
}

/// The sources of the crate that aren't its tests.
fn sources(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let of_the_tests = ["tests.rs", "testing.rs"]
            .iter()
            .any(|name| path.file_name().is_some_and(|file| file == *name));
        if path.is_dir() {
            sources(&path, found);
        } else if !of_the_tests {
            found.push(path);
        }
    }
}

/// The text between each pair of double quotes of `line`.
fn quoted(line: &str) -> impl Iterator<Item = &str> {
    line.split('"').skip(1).step_by(2)
}

#[test]
fn the_text_between_quotes_is_found_wherever_it_is_in_a_line() {
    let found: Vec<_> =
        quoted(r#".with("lablet.turn", turn).with(key::LABLET_ATTEMPT, "x") // "y"#).collect();

    assert_eq!(found, ["lablet.turn", "x", "y"]);
    assert_eq!(quoted("no quotes here").count(), 0);
}

/// An attribute's name comes from the registry crate, so a name the
/// registry doesn't declare can't be emitted and a name it renames doesn't
/// compile. A name written out would be neither.
#[test]
fn no_attribute_is_named_by_a_string_literal() {
    let declared = declared();
    assert!(declared.len() > 100, "{declared:?}");
    let mut found = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    assert!(found.len() >= 8, "{found:?}");

    let mut written_out = Vec::new();
    for path in found {
        let source = std::fs::read_to_string(&path).unwrap();
        for (number, line) in source.lines().enumerate() {
            // A comment may name an attribute to say something about it.
            if line.trim_start().starts_with("//") {
                continue;
            }
            for literal in quoted(line).filter(|literal| declared.contains(literal)) {
                written_out.push(format!("{}:{}: {literal:?}", path.display(), number + 1));
            }
        }
    }

    assert_eq!(written_out, Vec::<String>::new());
}
