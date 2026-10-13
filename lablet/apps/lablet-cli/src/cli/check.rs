//! What `lablet check` prints for a config that passed its check.

use std::fmt::Write as _;

use lablet_cli::compose::Report;
use lablet_config::{KeyPath, Place};
use lablet_prepare::Checked;
use lablet_secrets::Credential;
use serde_json::Value;

use crate::cli::report::counted;

/// What `lablet check` prints on standard output for a config that passed:
/// the name of each tool a run of it is offered, a line each, in the order
/// they're offered, and nothing when it offers none. With `resolved`, what
/// the file wrote of the lists the config reorders, it's the config with
/// every default filled in instead, as YAML, which reads back as a config
/// with the same digest, each setting followed by where it came from:
/// `# line N` for one the file wrote, `# --set` for one an override set,
/// and `# environment` for one a variable filled, whose value is written as
/// the file wrote it. An item of a list the config holds in an order of its
/// own is told by its value among the items the file wrote, and has no
/// comment when none of them has it.
///
/// # Errors
///
/// Returns why the resolved config couldn't be written as YAML.
pub(crate) fn answer(checked: &Checked, resolved: Option<&Written>) -> Result<String, String> {
    if let Some(written) = resolved {
        let filled = checked.resolved().filled();
        let places = checked.places();
        let held = held(checked);
        let source = |key: &KeyPath| -> Option<String> {
            let key = &as_written(&held, written, key)?;
            if filled
                .at(key)
                .is_some_and(|fill| !fill.variables.is_empty())
            {
                return Some("environment".to_owned());
            }
            Some(match places.of(key)? {
                Place::Line(line) => format!("line {line}"),
                Place::Override => "--set".to_owned(),
            })
        };
        let yaml = serde_saphyr::to_string(&checked.resolved().commented(&source))
            .map_err(|error| error.to_string())?;
        return Ok(yaml.trim_end().to_owned());
    }
    let names: Vec<&str> = checked
        .tools()
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    Ok(names.join("\n"))
}

/// The lists the config holds in an order of its own, whatever order the
/// file wrote them in: the sets, each item once and sorted, and the
/// propagators, each once in the order the file first wrote it, whose
/// names are matched in any case and trimmed.
const REORDERED: [(&str, bool); 4] = [
    ("tools.builtin.enabled", false),
    ("tools.allow", false),
    ("tools.deny", false),
    ("telemetry.propagators", true),
];

/// What the file wrote of each list of [`REORDERED`].
pub(crate) struct Written(Vec<List>);

/// What the file wrote of one list.
enum List {
    /// The text of each item, in the file's order.
    Items(Vec<Option<String>>),
    /// One text that holds every item, as the propagators may be written,
    /// whose place is the list's.
    Text,
}

/// What the file wrote of each list of [`REORDERED`], from `tree`, the tree
/// the config was read from as the fill left it: all `--resolved` keeps of
/// a tree that holds every value a variable gave.
pub(crate) fn written(tree: &Value) -> Written {
    Written(
        REORDERED
            .iter()
            .map(|(list, _)| match KeyPath::of(list).find(tree) {
                Some(Value::String(_)) => List::Text,
                Some(Value::Array(items)) => List::Items(
                    items
                        .iter()
                        .map(|item| item.as_str().map(str::to_owned))
                        .collect(),
                ),
                _ => List::Items(Vec::new()),
            })
            .collect(),
    )
}

/// The items the config holds of each list of [`REORDERED`], in its order.
fn held(checked: &Checked) -> Vec<Vec<String>> {
    let resolved = checked.resolved();
    let texts = |value: Result<Value, serde_json::Error>| -> Vec<String> {
        value
            .ok()
            .as_ref()
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let propagators = resolved
        .telemetry
        .propagators
        .as_ref()
        .map(|propagators| {
            propagators
                .names()
                .iter()
                .map(|name| name.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default();
    vec![
        texts(serde_json::to_value(&resolved.tools.builtin.enabled)),
        texts(serde_json::to_value(&resolved.tools.allow)),
        texts(serde_json::to_value(&resolved.tools.deny)),
        propagators,
    ]
}

/// The setting whose place `key`'s comment says: `key` itself, but for an
/// item of a list of [`REORDERED`], whose place by its index would be
/// another item's, the first item the file wrote with its value, and none
/// when no item has it; and the list itself for an item of a list the file
/// wrote as one text.
fn as_written(held: &[Vec<String>], written: &Written, key: &KeyPath) -> Option<KeyPath> {
    for (((list, loosely), held), written) in REORDERED.iter().zip(held).zip(&written.0) {
        let list = KeyPath::of(list);
        let Some(text) = (0..held.len())
            .find(|index| *key == list.index(*index))
            .map(|index| held[index].as_str())
        else {
            continue;
        };
        let written = match written {
            List::Items(items) => items,
            List::Text => return Some(list),
        };
        let index = written.iter().position(|item| {
            item.as_deref().is_some_and(|item| {
                if *loosely {
                    item.trim().eq_ignore_ascii_case(text)
                } else {
                    item == text
                }
            })
        })?;
        return Some(list.index(index));
    }
    Some(key.clone())
}

/// The lines a check that passed ends with on standard error, each
/// labelled: `own:`, each of lablet's own credentials, its variable and
/// line, the field it fills and who gets it; `given:`, each credential
/// `bash` is given, and under which name; `env:`, each entry of
/// `tools.builtin.env` passed as set; with `bash`, `inherited:`, the
/// variables a command inherits as set and the context variables it's given
/// in their place; `unread:`, the variables lablet read before its settings
/// came from the config alone that are set, and read only where the file
/// writes `${NAME}`; `telemetry:`, where a run sends and writes it, or that
/// nothing is named; `digest:`, the config's; with `bash`, `limit:`, what no
/// drop can hide from it; and `passed:`, how many tools a run is offered,
/// as `passed: 2 tools`. A list with nothing in it reads `none`, so a null
/// effect is visible, and no line holds a value.
pub(crate) fn summary(report: &Report) -> String {
    let checked = report.checked();
    let mut lines = Vec::new();
    let mut each = |label: &str, items: Vec<String>| {
        if items.is_empty() {
            lines.push(line(label, "none"));
        }
        for item in items {
            lines.push(line(label, &item));
        }
    };
    each(
        "own",
        checked.own().iter().map(Credential::as_own).collect(),
    );
    each(
        "given",
        checked.given().iter().map(Credential::as_given).collect(),
    );
    each(
        "env",
        checked.env().iter().map(ToString::to_string).collect(),
    );
    if let Some(environment) = checked.environment() {
        let mut inherited = if environment.inherited.is_empty() {
            "none".to_owned()
        } else {
            format!("{}, as set", environment.inherited.join(", "))
        };
        if !environment.context.is_empty() {
            let _ = write!(
                inherited,
                "; {}, its tool span's",
                environment.context.join(", ")
            );
        }
        lines.push(line("inherited", &inherited));
    }
    let unread = if report.unread().is_empty() {
        "none".to_owned()
    } else {
        let names: Vec<String> = report
            .unread()
            .iter()
            .map(|retired| match retired.field {
                Some(_) => retired.name.to_owned(),
                None => format!("{} (no field yet)", retired.name),
            })
            .collect();
        format!(
            "{}: set, and read only where the file writes ${{NAME}}",
            names.join(", ")
        )
    };
    lines.push(line("unread", &unread));
    let telemetry = if report.telemetry().is_empty() {
        "none named, so a run sends and writes nothing".to_owned()
    } else {
        report.telemetry().join("; ")
    };
    lines.push(line("telemetry", &telemetry));
    lines.push(line("digest", &checked.digest().to_string()));
    if checked.environment().is_some() {
        let limit = if cfg!(target_os = "macos") {
            "bash can read any file you can, and your shell's environment and lablet's"
        } else {
            "bash can read any file you can, and your shell's environment"
        };
        lines.push(line("limit", limit));
    }
    let tools = u64::try_from(checked.tools().len()).unwrap_or(u64::MAX);
    lines.push(line("passed", &counted(tools, "tool", "tools")));
    lines.join("\n")
}

/// One line of the summary: its label, padded so every value starts in one
/// column, and its text.
fn line(label: &str, text: &str) -> String {
    format!("{label}:{} {text}", " ".repeat(LABEL_WIDTH - label.len()))
}

/// The longest labels, `inherited` and `telemetry`.
const LABEL_WIDTH: usize = "inherited".len();

#[cfg(test)]
mod tests;
