//! The crate's own documentation, held to the reader it documents.

use crate::{Script, ScriptFormat, ScriptSource};

/// The examples the crate's documentation gives in `language`, each as the
/// text a person would copy.
fn examples(language: &str) -> Vec<String> {
    let opening = format!("//! ```{language}");
    let mut examples = Vec::new();
    let mut lines = include_str!("lib.rs").lines();
    while let Some(line) = lines.next() {
        if line == opening {
            let example = lines
                .by_ref()
                .take_while(|line| *line != "//! ```")
                .map(|line| line.strip_prefix("//! ").unwrap_or_default())
                .collect::<Vec<_>>();
            examples.push(example.join("\n"));
        }
    }
    examples
}

#[test]
fn every_script_the_documentation_shows_is_one_the_reader_accepts() {
    let shown = [
        (ScriptFormat::Yaml, examples("yaml"), [1, 1, 1].as_slice()),
        (ScriptFormat::Json, examples("json"), [2].as_slice()),
    ];

    for (format, examples, entries) in shown {
        let read = examples
            .iter()
            .map(|text| {
                Script::read(ScriptSource {
                    name: "the documentation",
                    text,
                    format,
                })
                .map(|script| script.entries())
                .map_err(|refused| format!("{refused}\n{text}"))
            })
            .collect::<Result<Vec<_>, _>>();

        assert_eq!(read.as_deref(), Ok(entries), "{format}");
    }
}

#[test]
fn the_refusal_the_documentation_shows_is_the_one_the_reader_gives() {
    let refused = Script::read(ScriptSource {
        name: "scripts/smoke.yaml",
        text: "
- response: { content: [], finish: end_turn }
- error: { kind: fatal }
- response:
    content:
      - text: Listing.
      - tool_use: { id: call_1, nme: bash, input: { json: {} } }
    finish: tool_use
",
        format: ScriptFormat::Yaml,
    })
    .unwrap_err();

    assert_eq!(examples("text"), [refused.to_string()]);
}
