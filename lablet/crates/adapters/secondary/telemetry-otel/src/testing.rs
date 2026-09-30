//! What this crate's tests share: a run's events, made by hand, and the
//! registry's key lists to hold a signal to.

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::time::Duration;

use lablet_model::{
    CacheScope, CompletionMode, ContentBlock, Cost, Effort, Endpoint, FinishReason, ModelRef,
    Prompts, ProviderApi, ProviderResponse, RequestParams, Responded, Run, RunContext, RunId,
    RunLabels, RunSetup, RunSummary, StopReason, Thinking, ToolCallId, ToolConcurrency, ToolInput,
    ToolName, ToolSource, ToolSpec, ToolUse, TurnRecord, Usage,
};
use serde_json::json;

use crate::attributes::{Attributes, Held};
use crate::run::Opening;

pub(crate) use lablet_test_support::{PROMPT, SYSTEM, Scratch};

pub(crate) const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
pub(crate) const STARTED_UNIX_MS: u64 = 1_790_000_000_000;
pub(crate) const CONFIG_DIGEST: &str =
    "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
pub(crate) const MODEL: &str = "scripted-1";

pub(crate) fn run_id() -> RunId {
    RunId::new(RUN).unwrap()
}

pub(crate) fn call_id(id: &str) -> ToolCallId {
    ToolCallId::new(id).unwrap()
}

pub(crate) fn tool_name(name: &str) -> ToolName {
    ToolName::new(name).unwrap()
}

pub(crate) fn labelled() -> RunLabels {
    RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: Some("terse-tool-descriptions".to_owned()),
        trial: Some("3".to_owned()),
    }
}

pub(crate) fn context() -> RunContext {
    RunContext {
        run_id: run_id(),
        labels: RunLabels::default(),
        started_unix_ms: STARTED_UNIX_MS,
        config_digest: CONFIG_DIGEST.to_owned(),
        agent_version: "0.1.0".to_owned(),
        transcript_path: None,
        skills_count: 0,
        mcp: None,
        capture_content: false,
    }
}

pub(crate) fn model() -> ModelRef {
    ModelRef {
        api: ProviderApi::Script,
        name: MODEL.to_owned(),
        replays_reasoning: false,
    }
}

/// The parameters of a run that set nothing a provider has a default for.
pub(crate) fn request() -> RequestParams {
    RequestParams {
        max_tokens: 4_096,
        temperature: None,
        thinking: Thinking::ProviderDefault,
        effort: None,
        seed: None,
        cache_scope: CacheScope::Shared,
    }
}

/// The parameters of a run that set every one.
pub(crate) fn every_parameter() -> RequestParams {
    RequestParams {
        temperature: Some(0.7),
        effort: Some(Effort::High),
        seed: Some(-42),
        ..request()
    }
}

pub(crate) fn endpoint() -> Endpoint {
    Endpoint {
        host: "api.example.com".to_owned(),
        port: 443,
    }
}

pub(crate) fn spec(name: &str, source: ToolSource) -> ToolSpec {
    ToolSpec {
        name: tool_name(name),
        description: format!("What {name} does."),
        input_schema: json!({ "type": "object" }),
        source,
        concurrency: ToolConcurrency::Exclusive,
    }
}

pub(crate) fn docs() -> ToolSource {
    ToolSource::Mcp {
        server: "docs".to_owned(),
    }
}

/// The tools of these runs: one built in, and one served over MCP.
pub(crate) fn tools() -> Vec<ToolSpec> {
    vec![
        spec("bash", ToolSource::Builtin),
        spec("mcp__docs__search", docs()),
    ]
}

/// A run that captures no content.
pub(crate) fn opening() -> Opening {
    Opening {
        context: context(),
        model: model(),
        endpoint: None,
        request: request(),
        tools: tools(),
        system_prompt: None,
        prompt: None,
    }
}

/// The same run, capturing content.
pub(crate) fn capturing() -> Opening {
    Opening {
        context: RunContext {
            capture_content: true,
            ..context()
        },
        system_prompt: Some(SYSTEM.to_owned()),
        prompt: Some(PROMPT.to_owned()),
        ..opening()
    }
}

pub(crate) fn every_count() -> Usage {
    Usage {
        input_tokens: 1_200,
        output_tokens: 80,
        reasoning_output_tokens: Some(30),
        cache_read_tokens: Some(1_000),
        cache_write_tokens: Some(100),
    }
}

/// What a provider that reports neither reasoning nor its cache says a
/// call used.
pub(crate) fn two_counts() -> Usage {
    Usage {
        input_tokens: 1_200,
        output_tokens: 80,
        ..Usage::default()
    }
}

pub(crate) fn record(usage: Usage, started_ms: u64, latency_ms: u64) -> TurnRecord {
    TurnRecord {
        usage,
        finish: FinishReason::ToolUse,
        response_id: Some("msg_01".to_owned()),
        response_model: Some("scripted-2026-09".to_owned()),
        started_ms,
        latency_ms,
        attempts: 1,
    }
}

pub(crate) fn said(text: &str) -> ContentBlock {
    ContentBlock::Text(text.to_owned())
}

pub(crate) fn calls(id: &str, name: &str) -> ContentBlock {
    ContentBlock::ToolUse(ToolUse {
        id: call_id(id),
        name: tool_name(name),
        input: ToolInput::Json(json!({ "command": "cargo test" })),
    })
}

fn setup(context: &RunContext) -> RunSetup {
    RunSetup {
        run_id: context.run_id.clone(),
        labels: context.labels.clone(),
        model: model(),
        endpoint: None,
        tools: tools().into_iter().map(|spec| spec.name).collect(),
        tools_bytes: 0,
        tools_digest: String::new(),
        system_prompt_digest: String::new(),
        completion: CompletionMode::Natural,
        max_turns: NonZeroU32::new(30),
        timeout: Duration::from_secs(600),
        request: request(),
    }
}

/// How long the runs these summaries are of took.
pub(crate) const DURATION_MS: u64 = 12_345;

/// The summary of a run of `context` that stopped for `reason` without a
/// response, so its totals are those of no turns. A reason that's a failure
/// has the provider's words for its error.
pub(crate) fn stopped(context: &RunContext, reason: StopReason) -> RunSummary {
    Run::start(setup(context), Prompts::new(SYSTEM, PROMPT).unwrap())
        .finish(
            reason,
            Duration::from_millis(DURATION_MS),
            None,
            Some("529 overloaded".to_owned()),
            None,
            None,
        )
        .summary
}

/// The summary of a run of `context` that completed on its first response,
/// which used `usage` and cost `cost`.
pub(crate) fn completed(context: &RunContext, usage: Usage, cost: Option<Cost>) -> RunSummary {
    let response = ProviderResponse::new(
        vec![said("Done.")],
        usage,
        FinishReason::EndTurn,
        None,
        None,
    )
    .unwrap();
    let run = Run::start(setup(context), Prompts::new(SYSTEM, PROMPT).unwrap());
    let Responded::Final(done) = run.responded(
        response,
        Duration::from_millis(5),
        Duration::from_millis(250),
    ) else {
        panic!("a response that calls no tool is the run's last");
    };
    done.finish(
        StopReason::Completed,
        Duration::from_millis(DURATION_MS),
        None,
        None,
        None,
        cost,
    )
    .summary
}

/// What `key` holds among `attributes`.
pub(crate) fn held<'a>(attributes: &'a Attributes, key: &str) -> &'a Held {
    attributes
        .held(key)
        .unwrap_or_else(|| panic!("`{key}` isn't among {:?}", attributes.keys()))
}

pub(crate) fn text(text: &str) -> Held {
    Held::Text(text.to_owned())
}

/// Holds `attributes` to the registry's lists for their signal: every key
/// the signal always carries is there, no key is there that the signal
/// doesn't declare, and none is there twice.
pub(crate) fn assert_declared(
    signal: &str,
    attributes: &Attributes,
    required: &[&str],
    declared: &[&str],
) {
    let keys = attributes.keys();
    let distinct: BTreeSet<_> = keys.iter().copied().collect();
    assert_eq!(
        keys.len(),
        distinct.len(),
        "{signal} holds a key twice: {keys:?}"
    );
    let missing: Vec<_> = required
        .iter()
        .filter(|key| !distinct.contains(**key))
        .collect();
    assert!(
        missing.is_empty(),
        "{signal} lacks {missing:?}, which the registry requires of it"
    );
    let undeclared: Vec<_> = distinct
        .iter()
        .filter(|key| !declared.contains(*key))
        .collect();
    assert!(
        undeclared.is_empty(),
        "{signal} holds {undeclared:?}, which the registry doesn't declare for it"
    );
}

/// The keys of `attributes` that the registry doesn't require of their
/// signal, in order: the ones a signal holds only when it has the value.
pub(crate) fn beyond_required<'a>(attributes: &'a Attributes, required: &[&str]) -> Vec<&'a str> {
    attributes
        .keys()
        .into_iter()
        .filter(|key| !required.contains(key))
        .collect()
}

/// Exporters that keep what they're handed, in memory.
pub(crate) mod memory {
    use std::future::{self, Future};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};

    use opentelemetry::InstrumentationScope;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
    use opentelemetry_sdk::logs::{LogBatch, LogExporter, SdkLogRecord};
    use opentelemetry_sdk::trace::{SpanData, SpanExporter};

    /// One log record, and the scope it was emitted under.
    pub(crate) type Logged = (SdkLogRecord, InstrumentationScope);

    /// One export, as the exporter it went to was handed it.
    #[derive(Debug, Clone)]
    pub(crate) enum Export {
        Spans(Vec<SpanData>),
        Records(Vec<Logged>),
        /// What the exporter of the wide event was handed.
        Wide(Vec<Logged>),
    }

    #[derive(Debug, Default)]
    struct Kept {
        exports: Mutex<Vec<Export>>,
        resources: Mutex<Vec<Resource>>,
        refuses_spans: AtomicBool,
        refuses_records: AtomicBool,
        refuses_wide: AtomicBool,
        held: Mutex<bool>,
        released: Condvar,
    }

    /// The memory of one destination's three exporters, which says what
    /// they were handed and decides what becomes of it.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct Memory(Arc<Kept>);

    #[derive(Debug)]
    pub(crate) struct Spans(Memory);

    #[derive(Debug)]
    pub(crate) struct Records {
        memory: Memory,
        wide: bool,
    }

    impl Memory {
        pub(crate) fn spans(&self) -> Spans {
            Spans(self.clone())
        }

        pub(crate) fn records(&self) -> Records {
            Records {
                memory: self.clone(),
                wide: false,
            }
        }

        pub(crate) fn wide(&self) -> Records {
            Records {
                memory: self.clone(),
                wide: true,
            }
        }

        /// From now on, or no longer, an export of spans fails, as one to
        /// a destination that can't be written does.
        pub(crate) fn refuse_spans(&self, refuses: bool) {
            self.0.refuses_spans.store(refuses, Ordering::SeqCst);
        }

        /// From now on, or no longer, an export of log records fails.
        pub(crate) fn refuse_records(&self, refuses: bool) {
            self.0.refuses_records.store(refuses, Ordering::SeqCst);
        }

        /// From now on, or no longer, an export of the wide event fails.
        pub(crate) fn refuse_wide(&self, refuses: bool) {
            self.0.refuses_wide.store(refuses, Ordering::SeqCst);
        }

        /// From now on an export waits, as one to a destination that
        /// doesn't answer does, until [`Memory::release`].
        pub(crate) fn hold(&self) {
            *self.0.held.lock().unwrap() = true;
        }

        pub(crate) fn release(&self) {
            *self.0.held.lock().unwrap() = false;
            self.0.released.notify_all();
        }

        /// Every export so far, in order.
        pub(crate) fn exports(&self) -> Vec<Export> {
            self.0.exports.lock().unwrap().clone()
        }

        /// Every span so far, in the order they were exported.
        pub(crate) fn exported_spans(&self) -> Vec<SpanData> {
            self.exports()
                .into_iter()
                .filter_map(|export| match export {
                    Export::Spans(spans) => Some(spans),
                    Export::Records(_) | Export::Wide(_) => None,
                })
                .flatten()
                .collect()
        }

        /// Every log record so far but the wide events, in the order they
        /// were exported.
        pub(crate) fn exported_records(&self) -> Vec<Logged> {
            self.exports()
                .into_iter()
                .filter_map(|export| match export {
                    Export::Records(records) => Some(records),
                    Export::Spans(_) | Export::Wide(_) => None,
                })
                .flatten()
                .collect()
        }

        /// Every wide event so far, in the order they were exported.
        pub(crate) fn exported_wide(&self) -> Vec<Logged> {
            self.exports()
                .into_iter()
                .filter_map(|export| match export {
                    Export::Wide(records) => Some(records),
                    Export::Spans(_) | Export::Records(_) => None,
                })
                .flatten()
                .collect()
        }

        /// The resource each exporter was told its exports come from.
        pub(crate) fn resources(&self) -> Vec<Resource> {
            self.0.resources.lock().unwrap().clone()
        }

        fn export(&self, export: Export, refuses: &AtomicBool) -> OTelSdkResult {
            let held = self.0.held.lock().unwrap();
            drop(self.0.released.wait_while(held, |held| *held).unwrap());
            if refuses.load(Ordering::SeqCst) {
                return Err(OTelSdkError::InternalFailure(
                    "the destination can't be written".to_owned(),
                ));
            }
            self.0.exports.lock().unwrap().push(export);
            Ok(())
        }

        fn described(&self, resource: &Resource) {
            self.0.resources.lock().unwrap().push(resource.clone());
        }
    }

    impl SpanExporter for Spans {
        fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
            future::ready(self.0.export(Export::Spans(batch), &self.0.0.refuses_spans))
        }

        fn set_resource(&mut self, resource: &Resource) {
            self.0.described(resource);
        }
    }

    impl LogExporter for Records {
        fn export(&self, batch: LogBatch<'_>) -> impl Future<Output = OTelSdkResult> + Send {
            let records = batch
                .iter()
                .map(|(record, scope)| (record.clone(), scope.clone()))
                .collect();
            future::ready(if self.wide {
                self.memory
                    .export(Export::Wide(records), &self.memory.0.refuses_wide)
            } else {
                self.memory
                    .export(Export::Records(records), &self.memory.0.refuses_records)
            })
        }

        fn set_resource(&mut self, resource: &Resource) {
            self.memory.described(resource);
        }
    }
}
