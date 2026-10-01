//! Why a step could not run to a verdict, told with what a person needs to
//! act on it. A step that ran and failed is not one of these: its verdict is
//! a [`crate::gates::Failure`] of its own.

use std::fmt::{self, Write as _};
use std::io;
use std::path::{Path, PathBuf};

use crate::process::Invocation;
use crate::workspace::unsupported;

/// Something xtask could not do, naming the file or the command it was
/// doing it to.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file or directory could not be read or changed.
    #[error("could not {verb} {}", .path.display())]
    File {
        /// What was being done to it.
        verb: Verb,
        /// The file or directory.
        path: PathBuf,
        /// Why it failed.
        source: io::Error,
    },
    /// A file was read and does not hold what it should.
    #[error("could not parse {}", .path.display())]
    Parse {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: Malformed,
    },
    /// A manifest uses a Cargo feature the lints refuse rather than model.
    #[error("{}: {}", .path.display(), unsupported(.feature))]
    Unsupported {
        /// The manifest.
        path: PathBuf,
        /// The feature, as a person names it.
        feature: String,
    },
    /// A command did not start.
    #[error("could not run `{command}` (in {})", .command.place())]
    Start {
        /// The command, and where it was to run.
        command: Invocation,
        /// Why it did not start.
        source: NotStarted,
    },
    /// A command ran and exited unsuccessfully.
    #[error("{}{}", .command.failed(), on_a_line_of_its_own(.stderr))]
    Failed {
        /// The command, and where it ran.
        command: Invocation,
        /// What it wrote on standard error, trimmed.
        stderr: String,
    },
    /// A command that was started never did what the step waited on, and
    /// was stopped.
    #[error("`{command}` (in {}) did not {expected}{}", .command.place(), on_a_line_of_its_own(.output))]
    Stalled {
        /// The command, and where it ran.
        command: Invocation,
        /// What was waited on and for how long, as `serve GET /health
        /// within 60s`.
        expected: String,
        /// What it wrote on both streams before it was stopped, trimmed.
        /// Boxed to keep the enum small, as clippy holds it to.
        output: Box<str>,
    },
    /// Something a step needs is not on this machine or in this checkout.
    #[error("{what}. {remedy}")]
    Missing {
        /// What is missing, and what needs it.
        what: String,
        /// What a person does to provide it.
        remedy: String,
    },
}

impl Error {
    /// For `map_err`: doing `verb` to `path` failed.
    pub fn file(verb: Verb, path: &Path) -> impl FnOnce(io::Error) -> Self {
        move |source| Self::File {
            verb,
            path: path.to_path_buf(),
            source,
        }
    }

    /// For `map_err`: `path` did not parse.
    pub fn parse<E: Into<Malformed>>(path: &Path) -> impl FnOnce(E) -> Self {
        move |source| Self::Parse {
            path: path.to_path_buf(),
            source: source.into(),
        }
    }
}

fn on_a_line_of_its_own(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        format!("\n{text}")
    }
}

/// What was being done to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Reading a file, or listing a directory.
    Read,
    /// Writing a file.
    Write,
    /// Creating a directory.
    Create,
    /// Removing a file.
    Remove,
    /// Replacing a directory with another.
    Replace,
}

impl fmt::Display for Verb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Create => "create",
            Self::Remove => "remove",
            Self::Replace => "replace",
        })
    }
}

/// What is wrong with a file that was read.
#[derive(Debug, thiserror::Error)]
pub enum Malformed {
    /// It isn't TOML, or not TOML of the shape expected.
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    /// It isn't JSON, or not JSON of the shape expected.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// It parses, but lacks what must be there.
    #[error("it has no {0}")]
    Lacks(&'static str),
}

/// Why a command did not start.
#[derive(Debug, thiserror::Error)]
pub enum NotStarted {
    /// The system would not start it.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The directory it was to run in does not exist.
    #[error("the directory does not exist")]
    NoDirectory,
    /// It would run a tool mise.toml pins, and mise does not provide it.
    #[error("{bin} is pinned in mise.toml, but {why}")]
    NotProvided {
        /// The tool's executable.
        bin: String,
        /// Why mise does not provide it, and what to do.
        why: String,
    },
}

/// `error` and each cause below it, in order, as a person reads them.
/// thiserror's `Display` shows only the outermost message.
pub fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        let _ = write!(text, ": {next}");
        cause = next.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stands for a library error that has a cause of its own.
    #[derive(Debug, thiserror::Error)]
    #[error("the outer layer")]
    struct Layered(#[source] io::Error);

    #[test]
    fn a_file_error_names_the_file_and_what_was_done_to_it_and_keeps_why() {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        let error = Error::file(Verb::Replace, Path::new("/tree/src"))(denied);
        assert_eq!(
            chain(&error),
            "could not replace /tree/src: permission denied"
        );
        let verbs = [
            Verb::Read,
            Verb::Write,
            Verb::Create,
            Verb::Remove,
            Verb::Replace,
        ];
        let spelt: Vec<String> = verbs.iter().map(Verb::to_string).collect();
        assert_eq!(spelt, ["read", "write", "create", "remove", "replace"]);
    }

    #[test]
    fn a_parse_error_names_the_file_and_says_what_is_wrong_with_it() {
        let path = Path::new("/ws/Cargo.toml");
        let toml = toml::from_str::<toml::Value>("[alias").unwrap_err();
        let expected = format!("could not parse /ws/Cargo.toml: {toml}");
        assert_eq!(chain(&Error::parse(path)(toml)), expected);
        let json = serde_json::from_str::<u8>("{").unwrap_err();
        let expected = format!("could not parse /ws/Cargo.toml: {json}");
        assert_eq!(chain(&Error::parse(path)(json)), expected);
        let lacking = Error::parse(path)(Malformed::Lacks("`[workspace]` table"));
        assert_eq!(
            chain(&lacking),
            "could not parse /ws/Cargo.toml: it has no `[workspace]` table"
        );
    }

    #[test]
    fn every_cause_is_told_after_the_error_it_explains() {
        let inner = io::Error::other(Layered(io::Error::from(io::ErrorKind::NotFound)));
        let error = Error::file(Verb::Read, Path::new("/a"))(inner);
        // An io::Error that wraps another shows it and hands on its cause.
        assert_eq!(
            chain(&error),
            "could not read /a: the outer layer: entity not found"
        );
        assert_eq!(error.to_string(), "could not read /a");
    }

    #[test]
    fn a_stalled_command_is_named_with_what_it_did_not_do_and_what_it_wrote() {
        let root = crate::workspace::repo_root();
        let command = Invocation::new(&root, "weaver", &["registry", "live-check"]);
        let stalled = |output: &str| Error::Stalled {
            command: command.clone(),
            expected: "serve GET /health within 60s".to_owned(),
            output: output.into(),
        };
        assert_eq!(
            chain(&stalled("Resolving registry")),
            "`weaver registry live-check` (in the repository root) did not serve GET /health \
             within 60s\nResolving registry"
        );
        assert_eq!(
            chain(&stalled("")),
            "`weaver registry live-check` (in the repository root) did not serve GET /health \
             within 60s"
        );
    }

    #[test]
    fn something_missing_is_told_with_what_to_do_about_it() {
        let missing = Error::Missing {
            what: "no base to compare with".to_owned(),
            remedy: "Fetch origin".to_owned(),
        };
        assert_eq!(chain(&missing), "no base to compare with. Fetch origin");
    }
}
