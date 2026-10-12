//! What `lablet check` prints for a config that passed its check.

use std::fmt::Write as _;

use lablet_prepare::Checked;
use lablet_secrets::Credential;

use crate::cli::report::counted;

/// What `lablet check` prints on standard output for a config that passed:
/// the name of each tool a run of it is offered, a line each, in the order
/// they're offered, and nothing when it offers none. With `resolved`, it's
/// the config with every default filled in instead, as YAML, which reads
/// back as a config with the same digest.
///
/// # Errors
///
/// Returns why the resolved config couldn't be written as YAML.
pub(crate) fn answer(checked: &Checked, resolved: bool) -> Result<String, String> {
    if resolved {
        let yaml =
            serde_saphyr::to_string(checked.resolved()).map_err(|error| error.to_string())?;
        return Ok(yaml.trim_end().to_owned());
    }
    let names: Vec<&str> = checked
        .tools()
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    Ok(names.join("\n"))
}

/// The lines a check that passed ends with on standard error, each
/// labelled: `own:`, each of lablet's own credentials, its variable and
/// line, the field it fills and who gets it; `given:`, each credential
/// `bash` is given, and under which name; `env:`, each entry of
/// `tools.builtin.env` passed as set; with `bash`, `inherited:`, the
/// variables a command inherits as set and the context variables it's given
/// in their place; `cut:`, the names whose values the seam still sends and
/// so cuts, each with a note when its value isn't cut; `digest:`, the
/// config's; with `bash`, `limit:`, what no drop can hide from it; and
/// `passed:`, how many tools a run is offered, as `passed: 2 tools`. A list
/// with nothing in it reads `none`, so a null effect is visible, and no line
/// holds a value.
pub(crate) fn summary(checked: &Checked) -> String {
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
    lines.push(line("cut", &listed(checked.cut())));
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

/// The longest label, `inherited`.
const LABEL_WIDTH: usize = "inherited".len();

/// `names` on one line, or `none`.
fn listed(names: impl IntoIterator<Item = String>) -> String {
    let names: Vec<String> = names.into_iter().collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
mod tests;
