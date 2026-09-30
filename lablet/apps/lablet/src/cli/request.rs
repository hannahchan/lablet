//! The run request: the task prompt from where the arguments say, and the
//! names they give the run.

use std::fmt;
use std::io::Read;
use std::path::PathBuf;

use lablet::{RunId, RunLabels, RunRequest};

use crate::cli::args::{Names, PromptArgs};
use crate::cli::refusal::Refusal;

/// Where the task prompt is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// `--prompt`, which holds it.
    Given(String),
    /// `--prompt-file`, which names the file that holds it.
    File(PathBuf),
    /// Standard input, read to its end, when neither flag is given.
    Stdin,
}

impl From<PromptArgs> for Source {
    fn from(args: PromptArgs) -> Self {
        // The flags are in a group that takes one at most.
        match (args.prompt, args.prompt_file) {
            (Some(text), _) => Self::Given(text),
            (None, Some(file)) => Self::File(file),
            (None, None) => Self::Stdin,
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Given(_) => f.write_str("from --prompt"),
            Self::File(file) => write!(f, "from --prompt-file {}", file.display()),
            Self::Stdin => f.write_str("from standard input"),
        }
    }
}

/// The task prompt `source` holds, and `stdin` is read to its end when it's
/// the source.
///
/// # Errors
///
/// Returns a `config:` [`Refusal`] when the file or standard input can't be
/// read as text.
pub(crate) fn read(source: &Source, mut stdin: impl Read) -> Result<String, Refusal> {
    let unreadable = |error: std::io::Error| {
        Refusal::config(format!("the task prompt {source} can't be read: {error}"))
    };
    match source {
        Source::Given(text) => Ok(text.clone()),
        Source::File(file) => std::fs::read_to_string(file).map_err(unreadable),
        Source::Stdin => {
            let mut text = String::new();
            stdin.read_to_string(&mut text).map_err(unreadable)?;
            Ok(text)
        }
    }
}

/// The request to run the task `prompt`, which was read from `source`,
/// under the names `names` gives.
///
/// # Errors
///
/// Returns a `config:` [`Refusal`] when the prompt is blank, and when the run id
/// is refused.
pub(crate) fn request(
    prompt: String,
    source: &Source,
    names: Names,
) -> Result<RunRequest, Refusal> {
    let Names {
        run_id,
        task,
        experiment,
        trial,
    } = names;
    let request = RunRequest::new(prompt)
        .map_err(|_| Refusal::config(format!("the task prompt {source} is blank")))?
        .labels(RunLabels {
            task,
            experiment,
            trial,
        });
    match run_id {
        Some(id) => {
            let id =
                RunId::new(id).map_err(|error| Refusal::config(format!("--run-id: {error}")))?;
            request
                .run_id(id)
                .map_err(|refused| Refusal::config(format!("--run-id: {refused}")))
        }
        None => Ok(request),
    }
}

#[cfg(test)]
mod tests;
