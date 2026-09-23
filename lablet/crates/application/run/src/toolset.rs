//! The tools one run offers, fixed when the run is built.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lablet_model::{CompletionMode, ToolConcurrency, ToolName, ToolSource, ToolSpec};

use crate::{ToolCall, ToolError, ToolErrorKind, ToolExecutor, ToolOutput};

/// Which tools a run offers, out of what its executors serve.
///
/// An empty allow list offers everything; a name in both lists is denied,
/// because denying is the safer reading of a contradiction. A name that no
/// executor serves is refused when the set is built, so a misspelt deny entry
/// can't leave the tool it meant to deny on offer.
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
    /// A filter names a tool that no executor serves.
    #[error("the {list} list names {name}, which no tool is called")]
    UnknownFilterName {
        /// The name no executor serves.
        name: ToolName,
        /// The list it's in.
        list: FilterList,
    },
    /// An executor couldn't say what it offers.
    #[error("an executor couldn't list its tools: {0}")]
    Specs(#[from] Box<ToolError>),
}

/// One of a [`ToolFilter`]'s two lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterList {
    /// The allow list.
    Allow,
    /// The deny list.
    Deny,
}

impl core::fmt::Display for FilterList {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        })
    }
}

/// What the run reports about a tool it offers.
#[derive(Debug, Clone)]
struct Offered {
    source: ToolSource,
    concurrency: ToolConcurrency,
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
    routes: BTreeMap<ToolName, Arc<dyn ToolExecutor>>,
    offered: BTreeMap<ToolName, Offered>,
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
    /// [`ToolSetError::DuplicateName`] when two serve the same name, or when
    /// one serves `task_complete` in explicit mode, and
    /// [`ToolSetError::UnknownFilterName`] when `filter` names a tool no
    /// executor serves.
    pub async fn build(
        executors: Vec<Arc<dyn ToolExecutor>>,
        filter: &ToolFilter,
        completion: CompletionMode,
        completion_schema: Option<serde_json::Value>,
    ) -> Result<Self, ToolSetError> {
        let mut claimed = BTreeSet::new();
        let mut served = Vec::new();
        let mut specs = Vec::new();
        // Claimed before any executor is asked, because the loop intercepts
        // this name rather than routing it: an executor that also serves it
        // would be offered twice, which every provider rejects, and its own
        // tool could never run.
        if completion == CompletionMode::Explicit {
            claimed.insert(ToolName::task_complete());
        }
        for executor in executors {
            for spec in executor.specs().await.map_err(Box::new)? {
                // Claimed before the filter is asked, so a second executor
                // serving a denied name is still a duplicate rather than a
                // silent substitute for the tool that was filtered out.
                if !claimed.insert(spec.name.clone()) {
                    return Err(ToolSetError::DuplicateName { name: spec.name });
                }
                served.push((spec, Arc::clone(&executor)));
            }
        }
        for (names, list) in [
            (&filter.allow, FilterList::Allow),
            (&filter.deny, FilterList::Deny),
        ] {
            if let Some(name) = names
                .iter()
                .find(|name| !served.iter().any(|(spec, _)| &spec.name == *name))
            {
                return Err(ToolSetError::UnknownFilterName {
                    name: name.clone(),
                    list,
                });
            }
        }

        let mut routes = BTreeMap::new();
        for (spec, executor) in served {
            if filter.offers(&spec.name) {
                routes.insert(spec.name.clone(), executor);
                specs.push(spec);
            }
        }
        let mut task_complete = None;
        if completion == CompletionMode::Explicit {
            let name = ToolName::task_complete();
            specs.push(task_complete_spec(name.clone(), completion_schema));
            task_complete = Some(name);
        }
        let offered = specs
            .iter()
            .map(|spec| {
                let offered = Offered {
                    source: spec.source.clone(),
                    concurrency: spec.concurrency,
                };
                (spec.name.clone(), offered)
            })
            .collect();

        Ok(Self {
            routes,
            offered,
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
    /// one that ran. `task_complete` is offered, so a call to it whose
    /// arguments didn't parse is a real name with bad arguments.
    #[must_use]
    pub fn source(&self, name: &ToolName) -> Option<&ToolSource> {
        self.offered.get(name).map(|offered| &offered.source)
    }

    /// Whether a call to `name` may run beside other calls of its turn. A
    /// name this run doesn't offer is answered by the loop without an
    /// executor, so it changes nothing and may.
    #[must_use]
    pub fn concurrency(&self, name: &ToolName) -> ToolConcurrency {
        self.offered
            .get(name)
            .map_or(ToolConcurrency::Shared, |offered| offered.concurrency)
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
        let Some(executor) = self.routes.get(&call.name) else {
            return Err(Box::new(ToolError::new(
                ToolErrorKind::Unknown,
                format!("no tool named {} is offered by this run", call.name),
            )));
        };
        executor.execute(call).await.map_err(Box::new)
    }
}

/// The tool a run in explicit mode ends with. Its schema is the run's
/// `completion_schema` when there is one, and a free-form object otherwise.
/// It's never executed, so it can't change what another call sees.
fn task_complete_spec(name: ToolName, schema: Option<serde_json::Value>) -> ToolSpec {
    ToolSpec {
        name,
        description: "Call this when the task is complete, with the result.".to_owned(),
        input_schema: schema.unwrap_or_else(|| serde_json::json!({ "type": "object" })),
        source: ToolSource::Builtin,
        concurrency: ToolConcurrency::Shared,
    }
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "an executor is a trait object with nothing to print; what a run offers is what it reports and the specs"
)]
impl core::fmt::Debug for ToolSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolSet")
            .field("offered", &self.offered)
            .field("specs", &self.specs)
            .field("task_complete", &self.task_complete)
            .finish()
    }
}
