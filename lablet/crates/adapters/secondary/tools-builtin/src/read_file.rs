//! `read_file`: the text of a file under the root, or some lines of it.

use std::num::NonZeroU64;

use lablet_model::ToolConcurrency;
use lablet_run::{ToolError, ToolOutput};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::fs::File;
use tokio::io::AsyncReadExt as _;

use crate::Tool;
use crate::root::Root;
use crate::text::Text;
use crate::tool::{BuiltIn, Offer, Terms};

/// How much of a file is read at a time.
const PIECE_BYTES: usize = 64 * 1024;

const DESCRIPTION: &str = "Returns the text of a file. A path that isn't absolute starts at \
the run's root directory, which is where `bash` starts, and a path that leads outside that \
directory once symbolic links are followed is refused. `offset` and `limit` choose lines, so \
a file too long to be returned whole can be read in parts: `offset` is how many lines to \
skip, and `limit` is how many to return after them. Bytes that aren't UTF-8 are returned as \
U+FFFD.";

/// Reads files under the root.
pub(crate) struct ReadFile {
    pub(crate) root: Root,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    path: String,
    #[serde(default)]
    offset: u64,
    limit: Option<NonZeroU64>,
}

/// The lines of a file that were asked for, taken from the file's bytes as
/// they're read.
struct Lines {
    /// How many lines are still to be skipped.
    skip: u64,
    /// How many lines are still to be returned; `None` for every line.
    take: Option<u64>,
}

impl Lines {
    /// The bytes among `piece`, the next of the file, that are of the lines
    /// asked for.
    fn of<'a>(&mut self, mut piece: &'a [u8]) -> &'a [u8] {
        while self.skip > 0 {
            let Some(end) = end_of_line(piece) else {
                return &[];
            };
            piece = &piece[end..];
            self.skip -= 1;
        }
        let Some(take) = &mut self.take else {
            return piece;
        };
        let mut taken = 0;
        while *take > 0 {
            let Some(end) = end_of_line(&piece[taken..]) else {
                return piece;
            };
            taken += end;
            *take -= 1;
        }
        &piece[..taken]
    }

    /// Whether every line asked for has been returned, so nothing more of
    /// the file has to be read.
    const fn are_read(&self) -> bool {
        matches!(self.take, Some(0))
    }
}

/// How many of `bytes` the line they begin with takes, its newline
/// included; `None` when the line doesn't end among them.
fn end_of_line(bytes: &[u8]) -> Option<usize> {
    bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|newline| newline + 1)
}

impl ReadFile {
    /// The text of the lines asked for, or what the tool says of a file it
    /// can't return.
    async fn read(&self, arguments: &Arguments, terms: Terms<'_>) -> Result<Text, String> {
        let path = &arguments.path;
        let resolved = self
            .root
            .existing(path)
            .await
            .map_err(|refused| format!("{path} wasn't read: {refused}"))?;
        // Anything but a file could keep a read waiting for as long as
        // nothing writes to it, and a directory has no text.
        let is_file = tokio::fs::metadata(&resolved)
            .await
            .is_ok_and(|metadata| metadata.is_file());
        if !is_file {
            return Err(format!("{path} wasn't read: it isn't a file"));
        }
        let unread = |error: std::io::Error| format!("{path} wasn't read: {error}");

        let mut file = File::open(&resolved).await.map_err(unread)?;
        let mut lines = Lines {
            skip: arguments.offset,
            take: arguments.limit.map(NonZeroU64::get),
        };
        let mut text = Text::new(terms.keep, terms.secrets);
        let mut piece = vec![0; PIECE_BYTES];
        while !lines.are_read() {
            let read = file.read(&mut piece).await.map_err(unread)?;
            if read == 0 {
                break;
            }
            text.feed(lines.of(&piece[..read]));
        }
        Ok(text)
    }
}

#[async_trait::async_trait]
impl BuiltIn for ReadFile {
    fn tool(&self) -> Tool {
        Tool::ReadFile
    }

    fn offer(&self) -> Offer {
        Offer {
            description: DESCRIPTION,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The file to read."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "How many lines to skip from the start of the file. \
                                        None when left out."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "The most lines to return. Every line after the \
                                        offset when left out."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            concurrency: ToolConcurrency::Shared,
        }
    }

    async fn run(&self, input: Value, terms: Terms<'_>) -> Result<ToolOutput, ToolError> {
        let arguments: Arguments = match terms.arguments(Tool::ReadFile, input) {
            Ok(arguments) => arguments,
            Err(refusal) => return Ok(*refusal),
        };
        // A read that's given up changes nothing, so the work has stopped
        // when the wait for it has.
        let reading = self.read(&arguments, terms);
        match tokio::time::timeout(terms.limit, reading).await {
            Ok(Ok(text)) => Ok(ToolOutput {
                output: text.kept(),
                is_error: false,
                mcp: None,
            }),
            Ok(Err(says)) => Ok(terms.error(&says)),
            Err(_) => Err(terms.ran_out(Tool::ReadFile)),
        }
    }
}

#[cfg(test)]
mod tests;
