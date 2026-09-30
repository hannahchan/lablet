//! `write_file`: a file under the root, made or replaced.

use lablet_model::ToolConcurrency;
use lablet_run::{ToolError, ToolOutput};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Tool;
use crate::root::Root;
use crate::tool::{BuiltIn, Offer, Terms};

const DESCRIPTION: &str = "Writes a text file: makes it, with the directories on the way to \
it that are missing, or replaces what it held. `content` is the whole of the file. A path \
that isn't absolute starts at the run's root directory, which is where `bash` starts, and a \
path that leads outside that directory once symbolic links are followed is refused.";

/// Writes files under the root.
pub(crate) struct WriteFile {
    pub(crate) root: Root,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    path: String,
    content: String,
}

impl WriteFile {
    /// Writes the file, or says why it wasn't written.
    async fn write(&self, arguments: &Arguments) -> Result<(), String> {
        let Arguments { path, content } = arguments;
        let resolved = self
            .root
            .reachable(path)
            .await
            .map_err(|refused| format!("{path} wasn't written: {refused}"))?;
        let unwritten = |error: std::io::Error| format!("{path} wasn't written: {error}");

        // Anything that's there and isn't a file could keep a write waiting
        // for as long as nothing reads from it.
        match tokio::fs::metadata(&resolved).await {
            Ok(metadata) if !metadata.is_file() => {
                return Err(format!("{path} wasn't written: it isn't a file"));
            }
            Ok(_) | Err(_) => {}
        }
        if let Some(directory) = resolved.parent() {
            tokio::fs::create_dir_all(directory)
                .await
                .map_err(unwritten)?;
        }
        tokio::fs::write(&resolved, content)
            .await
            .map_err(unwritten)
    }
}

#[async_trait::async_trait]
impl BuiltIn for WriteFile {
    fn tool(&self) -> Tool {
        Tool::WriteFile
    }

    fn offer(&self) -> Offer {
        Offer {
            description: DESCRIPTION,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The file to write."
                    },
                    "content": {
                        "type": "string",
                        "description": "The text the file is to hold."
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
            concurrency: ToolConcurrency::Exclusive,
        }
    }

    /// A write isn't given up at the call's limit: it's one write of text
    /// the model produced, to a file, and a call that returned while it went
    /// on would have the next call read a file that's still being written.
    async fn run(&self, input: Value, terms: Terms<'_>) -> Result<ToolOutput, ToolError> {
        let arguments: Arguments = match terms.arguments(Tool::WriteFile, input) {
            Ok(arguments) => arguments,
            Err(refusal) => return Ok(*refusal),
        };
        Ok(match self.write(&arguments).await {
            Ok(()) => terms.says(&format!(
                "wrote {} bytes to {}",
                arguments.content.len(),
                arguments.path
            )),
            Err(says) => terms.error(&says),
        })
    }
}
