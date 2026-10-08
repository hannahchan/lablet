//! The `prompt` section: the system prompt and the skills.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::written::path;

/// The `prompt` section. A config states exactly one of `system` and
/// `system_file`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Prompt {
    /// The system prompt.
    pub system: Option<String>,
    /// The file that holds the system prompt.
    #[serde(serialize_with = "path::optional")]
    pub system_file: Option<PathBuf>,
    /// The `SKILL.md` files of the run's skills.
    #[serde(serialize_with = "path::list")]
    pub skills: Vec<PathBuf>,
    /// How the model gets a skill's body.
    pub skills_mode: SkillsMode,
}

/// How the model gets a skill's body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillsMode {
    /// The system prompt lists each skill, and a tool loads a body when the
    /// model asks for it.
    #[default]
    Tool,
    /// Every body follows the system prompt.
    Inline,
}
