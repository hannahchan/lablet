//! The tools one run offers, fixed when the run is built.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lablet_model::{CompletionMode, ToolName, ToolSource, ToolSpec};

use crate::{ToolCall, ToolError, ToolErrorKind, ToolExecutor, ToolOutput};

/// Which tools a run offers, out of what its executors serve.
///
/// An empty allow list offers everything; a name in both lists is denied,
/// because denying is the safer reading of a contradiction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolFilter {
    /// Only these names are offered; empty offers all of them.
    pub allow: Vec<ToolName>,
    /// These names are never offered.
    pub deny: Vec<ToolName>,
}

impl ToolFilter {
    fn offers(&self, name: &ToolName) -> bool {
        !self.deny.contains(name) && (self.allow.is_empty() || self.allow.contains(name))
    }
}

/// Why a run's tools couldn't be settled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolSetError {
    /// Two executors serve the same name, so a call couldn't be routed.
    #[error("tool name {name} is served by more than one executor")]
    DuplicateName {
        /// The repeated name.
        name: ToolName,
    },
    /// An executor couldn't say what it offers.
    #[error("an executor couldn't list its tools: {0}")]
    Specs(#[from] Box<ToolError>),
}

/// Where a call goes, and what the run reports about the tool that took it.
#[derive(Debug, Clone)]
struct Routed {
    executor: usize,
    source: ToolSource,
}

/// The tools of one run: a composite over several [`ToolExecutor`]s that
/// routes a call by name. Deliberately not a `ToolExecutor` itself, because
/// the loop needs [`ToolSet::source`] and the port can't answer it.
///
/// It also holds the run's [`CompletionMode`], because it's the one value
/// whose shape depends on it: the mode decides whether `task_complete` is
/// registered, so a set built for the wrong mode is a set with the wrong
/// tools in it. Everything else that needs the mode reads it back from here.
///
/// The set is settled once, when the run is built, and never asked again.
/// That's what makes the port's obligation structural rather than a rule an
/// adapter is trusted to keep: [`ToolSet::source`] is the only place the loop
/// can learn where a tool came from, and it answers from this fixed map. A
/// server that announces a new tool part-way through a run can't produce a
/// [`lablet_model::ToolCallStatus::Ran`] in it, so the per-tool telemetry
/// keys stay bounded whatever an executor does later.
pub struct ToolSet {
    executors: Vec<Arc<dyn ToolExecutor>>,
    routes: BTreeMap<ToolName, Routed>,
    specs: Vec<ToolSpec>,
    task_complete: Option<ToolName>,
    completion: CompletionMode,
}

impl ToolSet {
    /// The tools `executors` serve, filtered, plus `task_complete` when the
    /// run completes explicitly.
    ///
    /// The `task_complete` tool is offered to the model and never executed:
    /// the loop intercepts the call and reads its argument as the run's
    /// structured result. It's registered whatever the executors serve, and
    /// the filter doesn't apply to it, because a run that completes
    /// explicitly has no way to finish without it.
    ///
    /// # Errors
    ///
    /// Returns [`ToolSetError::Specs`] when an executor can't list its tools,
    /// and [`ToolSetError::DuplicateName`] when two serve the same name, or
    /// when one serves `task_complete` in explicit mode.
    pub async fn build(
        executors: Vec<Arc<dyn ToolExecutor>>,
        filter: &ToolFilter,
        completion: CompletionMode,
        completion_schema: Option<serde_json::Value>,
    ) -> Result<Self, ToolSetError> {
        let mut claimed = BTreeSet::new();
        let mut routes = BTreeMap::new();
        let mut specs = Vec::new();
        // Claimed before any executor is asked, because the loop intercepts
        // this name rather than routing it: an executor that also serves it
        // would be offered twice, which every provider rejects, and its own
        // tool could never run.
        if completion == CompletionMode::Explicit {
            claimed.insert(ToolName::task_complete());
        }
        for (index, executor) in executors.iter().enumerate() {
            for spec in executor.specs().await.map_err(Box::new)? {
                // Claimed before the filter is asked, so a second executor
                // serving a denied name is still a duplicate rather than a
                // silent substitute for the tool that was filtered out.
                if !claimed.insert(spec.name.clone()) {
                    return Err(ToolSetError::DuplicateName { name: spec.name });
                }
                if !filter.offers(&spec.name) {
                    continue;
                }
                routes.insert(
                    spec.name.clone(),
                    Routed {
                        executor: index,
                        source: spec.source.clone(),
                    },
                );
                specs.push(spec);
            }
        }

        let mut task_complete = None;
        if completion == CompletionMode::Explicit {
            let name = ToolName::task_complete();
            specs.push(task_complete_spec(name.clone(), completion_schema));
            task_complete = Some(name);
        }

        Ok(Self {
            executors,
            routes,
            specs,
            task_complete,
            completion,
        })
    }

    /// How the run this set was built for decides that the model has
    /// finished. The run's one copy of the value.
    #[must_use]
    pub const fn completion(&self) -> CompletionMode {
        self.completion
    }

    /// Every tool offered to the model, in the order the executors served
    /// them, with `task_complete` last when there is one.
    #[must_use]
    pub fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    /// Where the tool called `name` comes from, or `None` when this run
    /// offers no such tool. The one place the loop learns a
    /// [`ToolSource`], so a tool that wasn't offered can't be reported as
    /// one that ran.
    #[must_use]
    pub fn source(&self, name: &ToolName) -> Option<&ToolSource> {
        self.routes.get(name).map(|routed| &routed.source)
    }

    /// Whether `name` is the tool that ends the run rather than one to run.
    #[must_use]
    pub fn is_task_complete(&self, name: &ToolName) -> bool {
        self.task_complete.as_ref() == Some(name)
    }

    /// Runs one call, on whichever executor serves its name.
    ///
    /// # Errors
    ///
    /// Returns [`ToolErrorKind::Unknown`] when this run offers no tool by
    /// that name, and whatever the executor returns otherwise.
    pub async fn execute(&self, call: ToolCall) -> Result<ToolOutput, Box<ToolError>> {
        let Some(routed) = self.routes.get(&call.name) else {
            return Err(Box::new(ToolError::new(
                ToolErrorKind::Unknown,
                format!("no tool named {} is offered by this run", call.name),
            )));
        };
        self.executors[routed.executor]
            .execute(call)
            .await
            .map_err(Box::new)
    }
}

/// The tool a run in explicit mode ends with. Its schema is the run's
/// `completion_schema` when there is one, and a free-form object otherwise.
fn task_complete_spec(name: ToolName, schema: Option<serde_json::Value>) -> ToolSpec {
    ToolSpec {
        name,
        description: "Call this when the task is complete, with the result.".to_owned(),
        input_schema: schema.unwrap_or_else(|| serde_json::json!({ "type": "object" })),
        source: ToolSource::Builtin,
    }
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "an executor is a trait object with nothing to print; what a run offers is the routes and the specs"
)]
impl core::fmt::Debug for ToolSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolSet")
            .field("routes", &self.routes)
            .field("specs", &self.specs)
            .field("task_complete", &self.task_complete)
            .finish()
    }
}
