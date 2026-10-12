//! The tools a checked config gives a run, built for each root: the
//! built-in tools under their root, which may hold none of lablet's own
//! files, their commands' environment computed from the base the root
//! handed in, and the one set of every executor's tools, after
//! `tools.allow` and `tools.deny`.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use lablet_config::{Config, KeyPath, Refusal};
use lablet_env_carrier::{CONTEXT_VARIABLES, variable};
use lablet_prepare::{BuildError, Environment, OwnFile, Prepared, held};
use lablet_run::{FilterList, ToolExecutor, ToolSet, ToolSetError};
use lablet_tools_builtin::{BuiltinTools, Dropped, SettingsError};
use opentelemetry::propagation::TextMapPropagator;

/// What building a config's tools came to.
pub struct Built {
    /// The set every run of the config is offered.
    pub tools: Arc<ToolSet>,
    /// What `bash` inherits of the base, and what was dropped on the way;
    /// `None` when the config enables no `bash`.
    pub environment: Option<Environment>,
}

/// The tool set of the config `prepared` checked: the built-in tools, when
/// the config enables any, their commands starting from the base the root
/// handed in less lablet's credentials, injecting a command's context
/// through `propagator`, the one its root was handed, and the set every run
/// of it is offered.
///
/// # Errors
///
/// Returns [`BuildError::RootHolds`] when `tools.builtin.root` holds one of
/// lablet's own files, [`BuildError::Config`] for a root that can't be
/// used or a variable of `tools.builtin.env` or `tools.builtin.credentials`
/// that can't be set, [`BuildError::UnknownTool`] when `tools.allow` or
/// `tools.deny` names a tool the run doesn't have, and [`BuildError::Tools`]
/// when the tools can't be settled into one set.
pub async fn tools(
    prepared: &Prepared,
    propagator: Arc<dyn TextMapPropagator + Send + Sync>,
) -> Result<Built, BuildError> {
    let config = prepared.config();
    let refused = |refusal| BuildError::Config(config.refused(refusal));
    let settings = prepared.settings();
    let mut environment = None;
    let executors = match &settings.builtin {
        Some(builtin) => {
            outside_root(&builtin.root, config, prepared.telemetry_file())?;
            let dropped = command_environment(prepared).map_err(|error| match error {
                SettingsError::Variable { name, reason } => refused(Refusal::Invalid {
                    key: KeyPath::of("tools.builtin.env").key(&name),
                    reason: reason.to_owned(),
                }),
                SettingsError::Credential { name, reason } => refused(Refusal::Invalid {
                    key: KeyPath::of("tools.builtin.credentials").key(&name),
                    reason: reason.to_owned(),
                }),
                SettingsError::Root { .. } | SettingsError::RootIsNoDirectory { .. } => {
                    refused(Refusal::invalid("tools.builtin.root", error.to_string()))
                }
            })?;
            if builtin.enabled.contains(&lablet_tools_builtin::Tool::Bash) {
                environment = Some(inherited(prepared, &dropped, propagator.as_ref()));
            }
            let kept = config
                .tools
                .builtin
                .env
                .keys()
                .chain(config.tools.builtin.credentials.keys())
                .cloned()
                .chain(dropped.by_name.iter().cloned())
                .chain(dropped.by_value.iter().cloned())
                .collect();
            let settings = lablet_tools_builtin::Settings {
                root: builtin.root.clone(),
                enabled: builtin.enabled.clone(),
                timeout: builtin.timeout,
                environment: dropped.environment,
                kept,
            };
            let tools = BuiltinTools::new(settings, propagator).map_err(|error| match error {
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
                SettingsError::Credential { name, reason } => refused(Refusal::Invalid {
                    key: KeyPath::of("tools.builtin.credentials").key(&name),
                    reason: reason.to_owned(),
                }),
            })?;
            vec![Arc::new(tools) as Arc<dyn ToolExecutor>]
        }
        None => Vec::new(),
    };
    let tools = ToolSet::build(executors, &settings.filter, settings.completion, None)
        .await
        .map(Arc::new)
        .map_err(|error| match error {
            ToolSetError::UnknownFilterName { name, list } => {
                unknown_tool(config, list, name.as_str())
            }
            error @ (ToolSetError::DuplicateName { .. } | ToolSetError::Specs(_)) => {
                BuildError::Tools {
                    reason: error.to_string(),
                }
            }
        })?;
    Ok(Built { tools, environment })
}

/// The environment the built-in tools' commands start from: the base the
/// root handed in, less the variables that filled an own or given field and
/// the ones holding an own or given value, then `tools.builtin.env`, then
/// `tools.builtin.credentials`. With no base handed in, which only a config
/// without `bash` reaches here, it's the config's own entries alone.
fn command_environment(prepared: &Prepared) -> Result<Dropped, SettingsError> {
    let config = prepared.config();
    let secrets = prepared.secrets();
    let credentials: BTreeMap<String, String> = config
        .tools
        .builtin
        .credentials
        .iter()
        .map(|(name, secret)| (name.clone(), secret.expose_secret().to_owned()))
        .collect();
    lablet_tools_builtin::environment(
        prepared.base().unwrap_or_default().iter().cloned(),
        &secrets.dropped,
        &secrets.dropping,
        config.tools.builtin.env.clone(),
        credentials,
    )
}

/// What `bash` inherits of the base, as `check` reports it: the names left
/// after the drops and less the context variables, sorted, the drops, each
/// copy of a credential's value attributed to its field, and the context
/// variables a command is given through `propagator`.
fn inherited(
    prepared: &Prepared,
    dropped: &Dropped,
    propagator: &dyn TextMapPropagator,
) -> Environment {
    let config = prepared.config();
    let context: Vec<String> = propagator
        .fields()
        .map(variable)
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    let own: Vec<String> = config
        .tools
        .builtin
        .env
        .keys()
        .chain(config.tools.builtin.credentials.keys())
        .cloned()
        .collect();
    let inherited = dropped
        .environment
        .keys()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| {
            !CONTEXT_VARIABLES.contains(&name.as_str())
                && !context.contains(name)
                && !own.contains(name)
        })
        .collect();
    let base = prepared.base().unwrap_or_default();
    let value_of = |name: &str| -> Option<&OsString> {
        base.iter()
            .find(|(set, _)| set.to_str() == Some(name))
            .map(|(_, value)| value)
    };
    let secrets = prepared.secrets();
    let mut copies = Vec::new();
    for name in &dropped.by_value {
        let Some(value) = value_of(name) else {
            continue;
        };
        let value = value.to_string_lossy();
        for credential in secrets.own.iter().chain(&secrets.given) {
            if credential.holds(&value)
                && !copies.contains(&(credential.field.clone(), name.clone()))
            {
                copies.push((credential.field.clone(), name.clone()));
            }
        }
    }
    Environment {
        inherited,
        by_name: dropped.by_name.clone(),
        by_value: dropped.by_value.clone(),
        copies,
        context,
    }
}

/// The refusal of a name in `tools.allow` or `tools.deny` that no tool
/// has, which shows the name as `config` writes it.
fn unknown_tool(config: &Config, list: FilterList, name: &str) -> BuildError {
    let (names, key) = match list {
        FilterList::Allow => (
            config.tools.allow.as_deref().unwrap_or_default(),
            "tools.allow",
        ),
        FilterList::Deny => (config.tools.deny.as_slice(), "tools.deny"),
    };
    let index = names.iter().position(|named| named == name);
    let written = index.and_then(|index| config.written_text_at(&KeyPath::of(key).index(index)));
    BuildError::UnknownTool {
        list,
        place: index.and_then(|index| config.place_of_item(key, index)),
        name: written.unwrap_or_else(|| name.to_owned()),
    }
}

/// Refuses a root that holds a file of lablet's own, each shown as `config`
/// writes it.
fn outside_root(root: &Path, config: &Config, telemetry: Option<&Path>) -> Result<(), BuildError> {
    let resolved = std::fs::canonicalize(root).map_err(|error| {
        BuildError::Config(
            config.refused(Refusal::invalid("tools.builtin.root", error.to_string())),
        )
    })?;
    let files = [
        (OwnFile::Config, config.source()),
        (OwnFile::SystemPrompt, config.prompt.system_file.as_deref()),
        (OwnFile::TaskPrompt, config.prompt_file()),
        (OwnFile::Transcript, config.run.transcript_path.as_deref()),
        (OwnFile::Telemetry, telemetry),
    ];
    let files = files
        .into_iter()
        .filter_map(|(file, path)| Some((file, path?)));
    let Some((holds, path)) = held(&resolved, files) else {
        return Ok(());
    };
    let shown = |key: &str, real: &Path| {
        config
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
        place: config.place_of("tools.builtin.root"),
        root: shown("tools.builtin.root", root),
        holds,
        path,
    })
}
