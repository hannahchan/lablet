//! Everything that drives the loop without a provider, a tool or a wait,
//! and what holds for the crate's sources as a whole.

use std::path::{Path, PathBuf};

mod errors;
mod fakes;
mod scenarios;
mod toolset;

/// How an attribute key of the registry's begins, when one is written out
/// as text: the prefixes of every key the crate's signals carry.
const KEY_PREFIXES: [&str; 10] = [
    "\"gen_ai.",
    "\"lablet.",
    "\"session.",
    "\"server.",
    "\"error.type",
    "\"exception.",
    "\"mcp.",
    "\"jsonrpc.",
    "\"rpc.",
    "\"network.",
];

/// The sources of the crate that are neither its tests nor the module the
/// generator writes, which is the one place a key is spelt out.
fn hand_written(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if path.is_dir() {
            if name != "tests" && name != "generated" {
                hand_written(&path, found);
            }
        } else if name != "tests.rs" {
            found.push(path);
        }
    }
}

/// An attribute's key comes from the generated `key` module, so a key the
/// registry doesn't declare can't be emitted and one it renames doesn't
/// compile. A key written out would be neither.
#[test]
fn no_production_file_writes_an_attribute_key_as_a_string_literal() {
    let mut found = Vec::new();
    hand_written(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    assert!(found.len() >= 10, "{found:?}");
    assert!(
        found
            .iter()
            .any(|path| path.ends_with("telemetry/conversation.rs")),
        "the walk reaches the modules below `telemetry`"
    );

    let mut written_out = Vec::new();
    for path in found {
        let source = std::fs::read_to_string(&path).unwrap();
        for (number, line) in source.lines().enumerate() {
            // A comment may name an attribute to say something about it.
            if line.trim_start().starts_with("//") {
                continue;
            }
            if KEY_PREFIXES.iter().any(|prefix| line.contains(prefix)) {
                written_out.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert_eq!(written_out, Vec::<String>::new());
}
