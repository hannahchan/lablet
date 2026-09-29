//! Integration tests of `lablet-documents`, through its public surface only:
//! each document against the example of it that's checked in.

#[cfg(test)]
mod outcome;
#[cfg(test)]
mod run;
#[cfg(test)]
mod transcript;

/// A document as the checked-in example of it holds one: indented, one key
/// to a line, and ended by a newline, which is the form the repository's
/// formatter leaves alone.
#[cfg(test)]
fn as_checked_in(document: &impl serde::Serialize) -> String {
    let mut written = serde_json::to_string_pretty(document).unwrap();
    written.push('\n');
    written
}
