//! What `lablet check` prints for a config that passed its check.

use lablet::Checked;

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

/// The line a check that passed ends with on standard error, which says
/// how many tools a run is offered, as `passed: 2 tools`.
pub(crate) fn summary(checked: &Checked) -> String {
    let tools = u64::try_from(checked.tools().len()).unwrap_or(u64::MAX);
    format!("passed: {}", counted(tools, "tool", "tools"))
}

#[cfg(test)]
mod tests;
