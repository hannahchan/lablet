//! The tools a checked config gives a run, built for each root: the
//! built-in tools under their root, which may hold none of lablet's own
//! files, and the one set of every executor's tools, after `tools.allow`
//! and `tools.deny`.

use std::path::Path;
use std::sync::Arc;

use lablet_config::{Config, KeyPath, Refusal};
use lablet_prepare::{BuildError, OwnFile, Prepared, held};
use lablet_run::{FilterList, ToolExecutor, ToolSet, ToolSetError};
use lablet_tools_builtin::{BuiltinTools, SettingsError};

/// The tool set of the config `written`, which `prepared` checked: the
/// built-in tools, when the config enables any, under the secrets it
/// derived, and the set every run of it is offered.
///
/// # Errors
///
/// Returns [`BuildError::RootHolds`] when `tools.builtin.root` holds one of
/// lablet's own files, [`BuildError::Config`] for a root that can't be
/// used or a variable of `tools.builtin.env` that can't be set,
/// [`BuildError::UnknownTool`] when `tools.allow` or `tools.deny` names a
/// tool the run doesn't have, and [`BuildError::Tools`] when the tools
/// can't be settled into one set.
pub async fn tools(written: &Config, prepared: &Prepared) -> Result<Arc<ToolSet>, BuildError> {
    let refused = |refusal| BuildError::Config(written.refused(refusal));
    let real = prepared.real();
    let settings = prepared.settings();
    let executors = match settings.builtin.clone() {
        Some(builtin) => {
            outside_root(&builtin.root, real, written, prepared.telemetry_file())?;
            let builtin = lablet_tools_builtin::Settings {
                withheld: prepared.withheld().clone(),
                ..builtin
            };
            let tools = BuiltinTools::new(builtin).map_err(|error| match error {
                SettingsError::Root { reason, .. } => {
                    refused(Refusal::invalid("tools.builtin.root", reason))
                }
                SettingsError::RootIsNoDirectory { .. } => refused(Refusal::invalid(
                    "tools.builtin.root",
                    "it isn't a directory",
                )),
                SettingsError::Variable { name, reason } => refused(Refusal::Invalid {
                    key: KeyPath::of("tools.builtin.env").key(&name),
                    reason: reason.to_owned(),
                }),
            })?;
            vec![Arc::new(tools) as Arc<dyn ToolExecutor>]
        }
        None => Vec::new(),
    };
    ToolSet::build(executors, &settings.filter, settings.completion, None)
        .await
        .map(Arc::new)
        .map_err(|error| match error {
            ToolSetError::UnknownFilterName { name, list } => {
                unknown_tool(real, written, list, name.as_str())
            }
            error @ (ToolSetError::DuplicateName { .. } | ToolSetError::Specs(_)) => {
                BuildError::Tools {
                    reason: error.to_string(),
                }
            }
        })
}

/// The refusal of a name in `tools.allow` or `tools.deny` that no tool
/// has, which shows the name as `written` writes it.
fn unknown_tool(real: &Config, written: &Config, list: FilterList, name: &str) -> BuildError {
    let (names, key, written_names) = match list {
        FilterList::Allow => (
            real.tools.allow.as_deref().unwrap_or_default(),
            "tools.allow",
            written.tools.allow.as_deref().unwrap_or_default(),
        ),
        FilterList::Deny => (
            real.tools.deny.as_slice(),
            "tools.deny",
            written.tools.deny.as_slice(),
        ),
    };
    let index = names.iter().position(|named| named == name);
    BuildError::UnknownTool {
        list,
        place: index.and_then(|index| written.place_of_item(key, index)),
        name: index
            .and_then(|index| written_names.get(index))
            .map_or_else(|| name.to_owned(), Clone::clone),
    }
}

/// Refuses a root that holds a file of lablet's own. The files are those
/// of `real`, which is what runs, and the refusal shows each as `written`
/// writes it.
fn outside_root(
    root: &Path,
    real: &Config,
    written: &Config,
    telemetry: Option<&Path>,
) -> Result<(), BuildError> {
    let resolved = std::fs::canonicalize(root).map_err(|error| {
        BuildError::Config(
            written.refused(Refusal::invalid("tools.builtin.root", error.to_string())),
        )
    })?;
    let files = [
        (OwnFile::Config, real.source()),
        (OwnFile::SystemPrompt, real.prompt.system_file.as_deref()),
        (OwnFile::TaskPrompt, real.prompt_file()),
        (OwnFile::Transcript, real.run.transcript_path.as_deref()),
        (OwnFile::Telemetry, telemetry),
    ];
    let files = files
        .into_iter()
        .filter_map(|(file, path)| Some((file, path?)));
    let Some((holds, path)) = held(&resolved, files) else {
        return Ok(());
    };
    let shown = |key: &str, real: &Path| {
        written
            .written_text(key)
            .unwrap_or_else(|| real.display().to_string())
    };
    let path = match holds {
        OwnFile::SystemPrompt => shown("prompt.system_file", path),
        OwnFile::Transcript => shown("run.transcript_path", path),
        OwnFile::Telemetry => shown("telemetry.file.path", path),
        OwnFile::Config | OwnFile::TaskPrompt => path.display().to_string(),
    };
    Err(BuildError::RootHolds {
        place: written.place_of("tools.builtin.root"),
        root: shown("tools.builtin.root", root),
        holds,
        path,
    })
}
