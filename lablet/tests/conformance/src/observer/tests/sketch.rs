//! A small observer to hold the cases to: it keeps the contract they check,
//! or breaks it in the one way a case is there to see, as the fake executor
//! does for the executor cases.
//!
//! It makes a span of each step from the loop's events and sums its own
//! spans into the wide event, so a step it misreads is misread alike in
//! both, as it would be by an observer that misread an event.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lablet_model::{RunSummary, StopReason, ToolCallId, ToolCallStatus, TurnRecord, Usage};
use lablet_run::{EventKind, ProviderError, RunEvent, RunObserver};
use lablet_telemetry_registry::attribute as key;
use lablet_telemetry_registry::signals::{
    EVENT_LABLET_RUN_NAME, EVENT_LABLET_RUN_REQUIRED, EventLabletRunTemplate,
};
use serde_json::{Value, json};

use crate::observer::Subject;
use crate::otlp::{Attributes, Exported, LogRecord, ReadError, Scope, Span, SpanKind, Status};

/// The span id of every run's root span. Each run has a trace of its own.
const ROOT: &str = "b7ad6b7169203331";

/// The one way the observer breaks the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fault {
    /// It keeps the contract.
    None,
    /// It has no wide event for a run that didn't complete.
    ForgetsARunThatFailed,
    /// Every flush exports the last wide event again, whether or not a run
    /// has ended since the flush before.
    ExportsTheLastWideEventAtEveryFlush,
    /// It puts a run's wide event in the context of the run's first chat
    /// span.
    PutsTheWideEventUnderAChatSpan,
    /// The total of calls its wide event holds leaves out the call to a
    /// tool the run doesn't have, which its spans hold.
    LeavesTheUnknownCallOutOfTheTotal,
    /// It reads a token more than the first response reported, in its
    /// spans and its wide event alike.
    CountsATokenTooMany,
    /// It reads no call's output as cut, in its spans and its wide event
    /// alike.
    SeesNothingCut,
    /// It takes a while over each event, and the run waits for it.
    SlowsTheRun,
    /// Its flush says its destination was written when it can't be.
    SaysItWrote,
}

/// The observer, and the destination it exports to.
pub(super) struct Sketch {
    fault: Fault,
    writable: bool,
    kept: Mutex<Kept>,
}

impl Sketch {
    /// An observer whose destination takes everything it's handed.
    pub(super) fn writing(fault: Fault) -> Arc<Self> {
        Self::new(fault, true)
    }

    /// An observer whose destination can't be written.
    pub(super) fn unwritable(fault: Fault) -> Arc<Self> {
        Self::new(fault, false)
    }

    fn new(fault: Fault, writable: bool) -> Arc<Self> {
        Arc::new(Self {
            fault,
            writable,
            kept: Mutex::default(),
        })
    }
}

#[async_trait::async_trait]
impl RunObserver for Sketch {
    async fn on(&self, event: RunEvent) {
        if self.fault == Fault::SlowsTheRun {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        self.kept.lock().unwrap().take(event, self.fault);
    }
}

#[async_trait::async_trait]
impl Subject for Arc<Sketch> {
    fn observer(&self) -> Arc<dyn RunObserver> {
        Arc::clone(self) as _
    }

    async fn flush(&self) -> Result<(), String> {
        self.kept.lock().unwrap().flush(self.fault, self.writable);
        if self.writable || self.fault == Fault::SaysItWrote {
            Ok(())
        } else {
            Err("the destination can't be written".to_owned())
        }
    }

    fn exported(&self) -> Result<Exported, ReadError> {
        Ok(self.kept.lock().unwrap().exported.clone())
    }
}

#[derive(Default)]
struct Kept {
    open: Option<Open>,
    /// The runs that have ended since the last flush, and what the loop
    /// returned of each.
    ended: Vec<(Open, RunSummary)>,
    /// What the destination was written.
    exported: Exported,
    last_wide: Option<LogRecord>,
    runs: u64,
}

impl Kept {
    fn take(&mut self, event: RunEvent, fault: Fault) {
        if let EventKind::RunStarted { context, tools, .. } = &event.kind {
            self.runs += 1;
            self.open = Some(Open {
                run: context.run_id.as_str().to_owned(),
                started_unix_ms: context.started_unix_ms,
                trace: format!("{:032x}", self.runs),
                offered: tools
                    .iter()
                    .map(|spec| spec.name.as_str().to_owned())
                    .collect(),
                chats: Vec::new(),
                tools: Vec::new(),
                calls: BTreeMap::new(),
                made: 0,
            });
            return;
        }
        let Some(open) = &mut self.open else {
            return;
        };
        match event.kind {
            EventKind::ProviderCallFinished {
                turn,
                attempt,
                record,
                ..
            } => open.answered((turn, attempt), &record, fault),
            EventKind::ProviderCallFailed {
                turn,
                attempt,
                error,
                started_ms,
                latency_ms,
                ..
            } => open.failed((turn, attempt), &error, (started_ms, latency_ms)),
            EventKind::ToolCallStarted {
                call_id,
                name,
                input_bytes,
                ..
            } => {
                open.calls
                    .insert(call_id, (name.as_str().to_owned(), input_bytes));
            }
            EventKind::ToolCallFinished {
                call_id,
                status,
                started_ms,
                latency_ms,
                output_bytes,
                truncated_from_bytes,
                ..
            } => {
                let cut = truncated_from_bytes.is_some() && fault != Fault::SeesNothingCut;
                open.call_ended(
                    &call_id,
                    &status,
                    (started_ms, latency_ms),
                    (output_bytes, cut),
                );
            }
            EventKind::RunFinished { summary, .. } => {
                if let Some(open) = self.open.take() {
                    self.ended.push((open, *summary));
                }
            }
            EventKind::RunStarted { .. }
            | EventKind::TurnStarted { .. }
            | EventKind::ProviderCallStarted { .. } => {}
        }
    }

    /// Exports the runs that have ended since the last flush, when the
    /// destination takes them.
    fn flush(&mut self, fault: Fault, writable: bool) {
        let ended = std::mem::take(&mut self.ended);
        let mut spans = Vec::new();
        let mut records = Vec::new();
        if ended.is_empty() && fault == Fault::ExportsTheLastWideEventAtEveryFlush {
            records.extend(self.last_wide.clone());
        }
        for (open, summary) in ended {
            let (made, wide) = open.close(&summary, fault);
            spans.extend(made);
            if let Some(wide) = wide {
                self.last_wide = Some(wide.clone());
                records.push(wide);
            }
        }
        if writable {
            self.exported.lines += spans.len() + records.len();
            self.exported.spans.extend(spans);
            self.exported.records.extend(records);
        }
    }
}

/// A run in progress, and the spans made of it so far.
struct Open {
    run: String,
    started_unix_ms: u64,
    trace: String,
    /// The names of the tools the run offered.
    offered: Vec<String>,
    chats: Vec<Span>,
    tools: Vec<Span>,
    /// The name and the size of the arguments of each call that has
    /// begun and not yet ended.
    calls: BTreeMap<ToolCallId, (String, u64)>,
    /// How many spans beneath the root have been made, which names the
    /// next.
    made: u64,
}

impl Open {
    /// A span beneath the root that began `timing.0` milliseconds into the
    /// run and lasted `timing.1`.
    fn span(&mut self, name: String, timing: (u64, u64), mut attributes: Attributes) -> Span {
        self.made += 1;
        attributes.insert(key::GEN_AI_CONVERSATION_ID.to_owned(), json!(self.run));
        let (started_ms, lasted_ms) = timing;
        let start_unix_nano = (self.started_unix_ms + started_ms) * 1_000_000;
        Span {
            line: 0,
            resource: Attributes::new(),
            scope: scope(),
            trace_id: self.trace.clone(),
            span_id: format!("{:016x}", self.made),
            parent_span_id: Some(ROOT.to_owned()),
            flags: 1,
            name,
            kind: SpanKind::Internal,
            start_unix_nano,
            end_unix_nano: start_unix_nano + lasted_ms * 1_000_000,
            attributes,
            events: Vec::new(),
            status: Status::Unset,
        }
    }

    /// The span of the attempt `attempt.1` of turn `attempt.0`.
    fn chat(&mut self, attempt: (u32, u32), timing: (u64, u64), mut attributes: Attributes) {
        attributes.insert(key::LABLET_TURN.to_owned(), json!(attempt.0));
        attributes.insert(key::LABLET_ATTEMPT.to_owned(), json!(attempt.1));
        let chat = self.span("chat scripted-1".to_owned(), timing, attributes);
        self.chats.push(chat);
    }

    fn answered(&mut self, attempt: (u32, u32), record: &TurnRecord, fault: Fault) {
        let mut attributes = spent(&record.usage);
        let first = self.chats.iter().all(failed);
        if fault == Fault::CountsATokenTooMany && first {
            attributes.insert(
                key::GEN_AI_USAGE_INPUT_TOKENS.to_owned(),
                json!(record.usage.input_tokens + 1),
            );
        }
        attributes.insert(
            key::GEN_AI_RESPONSE_FINISH_REASONS.to_owned(),
            json!([record.finish.as_str()]),
        );
        self.chat(attempt, (record.started_ms, record.latency_ms), attributes);
    }

    fn failed(&mut self, attempt: (u32, u32), error: &ProviderError, timing: (u64, u64)) {
        let mut attributes = error.usage.as_ref().map(spent).unwrap_or_default();
        attributes.insert(key::ERROR_TYPE.to_owned(), json!(error.kind.as_str()));
        self.chat(attempt, timing, attributes);
    }

    /// The span of the call `call`, which sent the model `sent.0` bytes,
    /// and cut them when `sent.1` says so.
    fn call_ended(
        &mut self,
        call: &ToolCallId,
        status: &ToolCallStatus,
        timing: (u64, u64),
        sent: (u64, bool),
    ) {
        let Some((name, input_bytes)) = self.calls.remove(call) else {
            return;
        };
        let attributes = [
            (key::GEN_AI_TOOL_NAME, json!(name)),
            (key::LABLET_TOOL_STATUS, json!(status.as_str())),
            (key::LABLET_TOOL_IS_ERROR, json!(status.is_error())),
            (key::LABLET_TOOL_INPUT_BYTES, json!(input_bytes)),
            (key::LABLET_TOOL_OUTPUT_BYTES, json!(sent.0)),
            (key::LABLET_TOOL_OUTPUT_TRUNCATED, json!(sent.1)),
        ];
        let tool = self.span(
            format!("execute_tool {name}"),
            timing,
            attributes
                .into_iter()
                .map(|(key, held)| (key.to_owned(), held))
                .collect(),
        );
        self.tools.push(tool);
    }

    /// The run's spans, its root last, and its wide event.
    fn close(self, summary: &RunSummary, fault: Fault) -> (Vec<Span>, Option<LogRecord>) {
        let answered: Vec<&Span> = self.chats.iter().filter(|chat| !failed(chat)).collect();
        let mut of_the_root = tokens(&answered);
        of_the_root.insert(key::GEN_AI_CONVERSATION_ID.to_owned(), json!(self.run));
        of_the_root.insert(key::LABLET_RUN_TURNS.to_owned(), json!(answered.len()));
        of_the_root.insert(
            key::LABLET_TOOL_CALLS_TOTAL.to_owned(),
            json!(self.tools.len()),
        );
        let start_unix_nano = self.started_unix_ms * 1_000_000;
        let root = Span {
            line: 0,
            resource: Attributes::new(),
            scope: scope(),
            trace_id: self.trace.clone(),
            span_id: ROOT.to_owned(),
            parent_span_id: None,
            flags: 1,
            name: "invoke_agent lablet".to_owned(),
            kind: SpanKind::Internal,
            start_unix_nano,
            end_unix_nano: start_unix_nano + summary.outcome.duration_ms * 1_000_000,
            attributes: of_the_root,
            events: Vec::new(),
            status: Status::Unset,
        };
        let under = match (fault, self.chats.first()) {
            (Fault::PutsTheWideEventUnderAChatSpan, Some(chat)) => chat.span_id.clone(),
            _ => ROOT.to_owned(),
        };
        let completed = summary.outcome.stop_reason() == StopReason::Completed;
        let wide = (completed || fault != Fault::ForgetsARunThatFailed).then(|| LogRecord {
            line: 0,
            resource: Attributes::new(),
            scope: scope(),
            event_name: EVENT_LABLET_RUN_NAME.to_owned(),
            severity_number: 9,
            severity_text: "INFO".to_owned(),
            time_unix_nano: root.end_unix_nano,
            observed_time_unix_nano: root.end_unix_nano,
            trace_id: self.trace.clone(),
            span_id: under,
            flags: 1,
            attributes: self.sums(summary, fault),
            body: None,
        });

        let mut spans = self.chats;
        spans.extend(self.tools);
        spans.push(root);
        (spans, wide)
    }

    /// What the wide event holds: the sums of the run's spans, and what the
    /// loop returned of how long the run took and why it stopped.
    fn sums(&self, summary: &RunSummary, fault: Fault) -> Attributes {
        let chats: Vec<&Span> = self.chats.iter().collect();
        let tools: Vec<&Span> = self.tools.iter().collect();
        let (failures, answered): (Vec<&Span>, Vec<&Span>) =
            chats.iter().copied().partition(|chat| failed(chat));
        let turns: BTreeSet<Option<u64>> = chats
            .iter()
            .map(|chat| chat.attributes[key::LABLET_TURN].as_u64())
            .collect();
        let unknown = how_many(&tools, |tool| tool[key::LABLET_TOOL_STATUS] == "unknown");
        let total = tools.len() as u64
            - match fault {
                Fault::LeavesTheUnknownCallOutOfTheTotal => unknown,
                _ => 0,
            };
        let reasons: Vec<Value> = answered
            .iter()
            .filter_map(|chat| chat.attributes[key::GEN_AI_RESPONSE_FINISH_REASONS].as_array())
            .flatten()
            .cloned()
            .collect();

        let mut wide = tokens(&answered);
        for (key, held) in [
            (key::GEN_AI_CONVERSATION_ID, json!(self.run)),
            (
                key::LABLET_RUN_STOP_REASON,
                json!(summary.outcome.stop_reason().as_str()),
            ),
            (
                key::LABLET_RUN_DURATION_MS,
                json!(summary.outcome.duration_ms),
            ),
            (key::LABLET_TOOLS_NAMES, json!(self.offered)),
            (key::LABLET_TOOLS_COUNT, json!(self.offered.len())),
            (key::LABLET_RUN_TURNS, json!(answered.len())),
            (
                key::LABLET_PROVIDER_RETRIES,
                json!(chats.len() - turns.len()),
            ),
            (key::LABLET_PROVIDER_LATENCY_MS_TOTAL, json!(lasted(&chats))),
            (
                key::LABLET_PROVIDER_LATENCY_MS_MAX,
                json!(
                    chats
                        .iter()
                        .map(|chat| chat.duration_ms())
                        .max()
                        .unwrap_or(0)
                ),
            ),
            (key::GEN_AI_RESPONSE_FINISH_REASONS, json!(reasons)),
            (key::LABLET_TOOL_CALLS_TOTAL, json!(total)),
            (
                key::LABLET_TOOL_CALLS_ERRORS,
                json!(how_many(&tools, is_error)),
            ),
            (key::LABLET_TOOL_CALLS_UNKNOWN, json!(unknown)),
            (
                key::LABLET_TOOL_CALLS_TRUNCATED,
                json!(how_many(&tools, |tool| tool
                    [key::LABLET_TOOL_OUTPUT_TRUNCATED]
                    == true)),
            ),
            (
                key::LABLET_TOOL_CALLS_LATENCY_MS_TOTAL,
                json!(lasted(&tools)),
            ),
            (
                key::LABLET_TOOL_CALLS_INPUT_BYTES_TOTAL,
                json!(sum(&tools, key::LABLET_TOOL_INPUT_BYTES).unwrap_or(0)),
            ),
            (
                key::LABLET_TOOL_CALLS_OUTPUT_BYTES_TOTAL,
                json!(sum(&tools, key::LABLET_TOOL_OUTPUT_BYTES).unwrap_or(0)),
            ),
        ] {
            wide.insert(key.to_owned(), held);
        }
        wide.extend(failed_tokens(&failures));
        wide.extend(shares(&tools));
        // The registry requires more of the wide event than the cases
        // check the value of, and those are there to be there.
        for required in EVENT_LABLET_RUN_REQUIRED {
            wide.entry((*required).to_owned()).or_insert(json!(0));
        }
        wide
    }
}

/// What the attempts that failed spent, as the wide event holds it.
fn failed_tokens(failures: &[&Span]) -> Attributes {
    let mut spent = Attributes::new();
    for (key, of_a_chat) in [
        (
            key::LABLET_PROVIDER_FAILED_INPUT_TOKENS,
            key::GEN_AI_USAGE_INPUT_TOKENS,
        ),
        (
            key::LABLET_PROVIDER_FAILED_OUTPUT_TOKENS,
            key::GEN_AI_USAGE_OUTPUT_TOKENS,
        ),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_READ_INPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
        ),
        (
            key::LABLET_PROVIDER_FAILED_CACHE_WRITE_INPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
        ),
    ] {
        if let Some(summed) = sum(failures, of_a_chat) {
            spent.insert(key.to_owned(), json!(summed));
        }
    }
    spent
}

/// The per-tool keys of the wide event, for each tool the run offered that
/// a call named.
fn shares(tools: &[&Span]) -> Attributes {
    let mut called: BTreeMap<&str, Vec<&Span>> = BTreeMap::new();
    for tool in tools {
        if tool.attributes[key::LABLET_TOOL_STATUS] != "unknown" {
            let name = tool.attributes[key::GEN_AI_TOOL_NAME].as_str().unwrap();
            called.entry(name).or_default().push(tool);
        }
    }
    let mut shares = Attributes::new();
    for (name, calls) in &called {
        for template in EventLabletRunTemplate::ALL {
            let held = match template {
                EventLabletRunTemplate::LabletToolCalls => calls.len() as u64,
                EventLabletRunTemplate::LabletToolErrors => how_many(calls, is_error),
                EventLabletRunTemplate::LabletToolLatencyMs => lasted(calls),
            };
            shares.insert(format!("{}.{name}", template.prefix()), json!(held));
        }
    }
    shares
}

fn scope() -> Scope {
    Scope {
        name: "sketch".to_owned(),
        version: "0.1.0".to_owned(),
        schema_url: lablet_telemetry_registry::SCHEMA_URL.to_owned(),
    }
}

/// The token counts of `usage`, as the span of a provider call holds them.
fn spent(usage: &Usage) -> Attributes {
    let mut spent = Attributes::new();
    for (key, count) in [
        (key::GEN_AI_USAGE_INPUT_TOKENS, Some(usage.input_tokens)),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, Some(usage.output_tokens)),
        (
            key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
            usage.reasoning_output_tokens,
        ),
        (
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
            usage.cache_read_tokens,
        ),
        (
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
            usage.cache_write_tokens,
        ),
    ] {
        if let Some(count) = count {
            spent.insert(key.to_owned(), json!(count));
        }
    }
    spent
}

/// What the attempts that were answered spent, as the root span and the
/// wide event hold it: a count that none of them reported is left out,
/// but for the two that a run with no answer at all has as none.
fn tokens(answered: &[&Span]) -> Attributes {
    let mut tokens = Attributes::new();
    for (key, always) in [
        (key::GEN_AI_USAGE_INPUT_TOKENS, true),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, true),
        (key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS, false),
        (key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS, false),
        (key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS, false),
    ] {
        if let Some(summed) = sum(answered, key).or(always.then_some(0)) {
            tokens.insert(key.to_owned(), json!(summed));
        }
    }
    tokens
}

fn failed(chat: &Span) -> bool {
    chat.attributes.contains_key(key::ERROR_TYPE)
}

fn is_error(tool: &Attributes) -> bool {
    tool[key::LABLET_TOOL_IS_ERROR] == true
}

fn sum(spans: &[&Span], key: &str) -> Option<u64> {
    spans
        .iter()
        .filter_map(|span| span.attributes.get(key).and_then(Value::as_u64))
        .reduce(|sum, count| sum + count)
}

fn lasted(spans: &[&Span]) -> u64 {
    spans.iter().map(|span| span.duration_ms()).sum()
}

fn how_many(spans: &[&Span], holds: impl Fn(&Attributes) -> bool) -> u64 {
    spans.iter().filter(|span| holds(&span.attributes)).count() as u64
}
