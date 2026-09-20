//! Hand-written stand-ins for every port, so the loop can be driven without
//! a provider, a tool, a clock, or a wait.
//!
//! They're `#[cfg(test)]` rather than a feature, because nothing outside this
//! crate's tests should build a run out of them: the product's own scripted
//! provider is an adapter, written at phase 4, and answers to a config.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lablet_model::{
    Endpoint, ModelRef, ProviderErrorKind, ProviderResponse, ToolName, ToolResultContent, ToolSpec,
};

use crate::{
    Cancellation, Clock, ModelProvider, ProviderError, ProviderRequest, RunEvent, RunObserver,
    ToolCall, ToolError, ToolExecutor, ToolOutput,
};

/// A clock that only moves when a test moves it, and never waits.
///
/// `sleep` adds the wait to the reading instead of blocking, so a run with a
/// ten-minute backoff finishes in microseconds and the transcript still says
/// ten minutes passed.
pub struct FakeClock {
    origin: Instant,
    elapsed: Mutex<Duration>,
    slept: Mutex<Vec<Duration>>,
}

impl FakeClock {
    /// A clock reading zero.
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
            slept: Mutex::new(Vec::new()),
        }
    }

    /// Moves the reading on, as a provider call or a tool call would.
    pub fn advance(&self, by: Duration) {
        *self.elapsed.lock().expect("the fake clock isn't poisoned") += by;
    }

    /// Every wait the loop asked for, in order.
    pub fn sleeps(&self) -> Vec<Duration> {
        self.slept
            .lock()
            .expect("the fake clock isn't poisoned")
            .clone()
    }
}

#[async_trait::async_trait]
impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.origin + *self.elapsed.lock().expect("the fake clock isn't poisoned")
    }

    async fn sleep(&self, duration: Duration) {
        self.slept
            .lock()
            .expect("the fake clock isn't poisoned")
            .push(duration);
        self.advance(duration);
    }
}

/// Cancellation a test turns on when it likes.
pub struct FakeCancel {
    cancelled: AtomicBool,
    /// Cancels once this many polls have been answered, so a test can cancel
    /// at a chosen point in the loop rather than from the start.
    after_polls: Option<usize>,
    polls: AtomicUsize,
}

impl FakeCancel {
    /// Never cancels.
    pub const fn never() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            after_polls: None,
            polls: AtomicUsize::new(0),
        }
    }

    /// Answers `false` `polls` times and `true` after that.
    pub const fn after(polls: usize) -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            after_polls: Some(polls),
            polls: AtomicUsize::new(0),
        }
    }
}

impl Cancellation for FakeCancel {
    fn is_cancelled(&self) -> bool {
        let polls = self.polls.fetch_add(1, Ordering::Relaxed);
        self.after_polls.is_some_and(|after| polls >= after)
            || self.cancelled.load(Ordering::Relaxed)
    }
}

/// One scripted answer to a provider call.
pub enum Answer {
    /// The call succeeds, after `latency`.
    Responds(Box<ProviderResponse>, Duration),
    /// The call fails, after `latency`.
    Fails(ProviderErrorKind, String, Duration),
}

impl Answer {
    /// A response that took no time.
    pub fn now(response: ProviderResponse) -> Self {
        Self::Responds(Box::new(response), Duration::ZERO)
    }

    /// A failure of `kind` that took no time.
    pub fn fails(kind: ProviderErrorKind) -> Self {
        Self::Fails(
            kind,
            format!("the fake provider failed: {kind}"),
            Duration::ZERO,
        )
    }
}

/// A provider that plays a script, one answer per attempt.
///
/// It advances the clock by each answer's latency, so a run's recorded
/// timings are the script's and a test asserts them exactly.
pub struct FakeProvider {
    model: ModelRef,
    endpoint: Option<Endpoint>,
    script: Mutex<VecDeque<Answer>>,
    clock: Arc<FakeClock>,
    calls: AtomicUsize,
}

impl FakeProvider {
    /// A provider that answers with `script`, in order.
    pub fn new(model: ModelRef, clock: Arc<FakeClock>, script: Vec<Answer>) -> Self {
        Self {
            model,
            endpoint: None,
            script: Mutex::new(script.into()),
            clock,
            calls: AtomicUsize::new(0),
        }
    }

    /// The same, reached over the network at `endpoint`.
    pub fn at(mut self, endpoint: Endpoint) -> Self {
        self.endpoint = Some(endpoint);
        self
    }

    /// How many attempts the loop made.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl ModelProvider for FakeProvider {
    fn model(&self) -> &ModelRef {
        &self.model
    }

    fn endpoint(&self) -> Option<Endpoint> {
        self.endpoint.clone()
    }

    async fn complete(
        &self,
        _request: ProviderRequest<'_>,
    ) -> Result<ProviderResponse, ProviderError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let answer = self
            .script
            .lock()
            .expect("the fake provider isn't poisoned")
            .pop_front();
        match answer {
            Some(Answer::Responds(response, latency)) => {
                self.clock.advance(latency);
                Ok(*response)
            }
            Some(Answer::Fails(kind, message, latency)) => {
                self.clock.advance(latency);
                Err(ProviderError::new(kind, message))
            }
            // A script that runs out is a test that didn't say what happens
            // next, which is worth failing loudly rather than looping.
            None => Err(ProviderError::new(
                ProviderErrorKind::Fatal,
                "the fake provider's script ran out",
            )),
        }
    }
}

/// What a fake tool does when it's called.
pub enum Answers {
    /// Returns this text.
    Text(String),
    /// Returns this text, reported by the tool as its own failure.
    ToolError(String),
    /// Fails, so no tool answered.
    Fails(crate::ToolErrorKind, String),
    /// Answers over MCP, with the transport metadata an MCP adapter attaches.
    OverMcp(String, crate::McpCallMeta),
    /// Fails over MCP, so the metadata comes back on the error instead.
    FailsOverMcp(crate::ToolErrorKind, String, crate::McpCallMeta),
}

/// An executor serving named tools with scripted answers.
pub struct FakeTools {
    specs: Vec<ToolSpec>,
    answers: Mutex<Vec<(ToolName, Answers)>>,
    latency: Duration,
    clock: Arc<FakeClock>,
    listing: Option<crate::ToolErrorKind>,
    taken: Mutex<Vec<ToolCall>>,
}

impl FakeTools {
    /// An executor offering `specs`, answering each call by name.
    pub fn new(clock: Arc<FakeClock>, specs: Vec<ToolSpec>) -> Self {
        Self {
            specs,
            answers: Mutex::new(Vec::new()),
            latency: Duration::ZERO,
            clock,
            listing: None,
            taken: Mutex::new(Vec::new()),
        }
    }

    /// Every call this executor was handed, in order.
    pub fn taken(&self) -> Vec<ToolCall> {
        self.taken
            .lock()
            .expect("the fake executor isn't poisoned")
            .clone()
    }

    /// The next call to `name` answers with `answer`. Answers are taken in
    /// the order they were added, so a tool can behave differently each time.
    pub fn answers(self, name: &str, answer: Answers) -> Self {
        self.answers
            .lock()
            .expect("the fake executor isn't poisoned")
            .push((
                ToolName::new(name).expect("a test's tool name is valid"),
                answer,
            ));
        self
    }

    /// Every call takes this long.
    pub const fn taking(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    /// Listing the tools fails, which ends the run before its first call.
    pub const fn cannot_list(mut self, kind: crate::ToolErrorKind) -> Self {
        self.listing = Some(kind);
        self
    }
}

#[async_trait::async_trait]
impl ToolExecutor for FakeTools {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError> {
        match self.listing {
            Some(kind) => Err(ToolError::new(
                kind,
                "the fake executor can't list its tools",
            )),
            None => Ok(self.specs.clone()),
        }
    }

    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError> {
        self.clock.advance(self.latency);
        self.taken
            .lock()
            .expect("the fake executor isn't poisoned")
            .push(call.clone());
        let mut answers = self
            .answers
            .lock()
            .expect("the fake executor isn't poisoned");
        let found = answers.iter().position(|(name, _)| *name == call.name);
        let answer = found.map(|at| answers.remove(at).1);
        match answer {
            Some(Answers::Text(text)) => Ok(ToolOutput {
                content: vec![ToolResultContent::Text(text)],
                is_error: false,
                mcp: None,
            }),
            Some(Answers::ToolError(text)) => Ok(ToolOutput {
                content: vec![ToolResultContent::Text(text)],
                is_error: true,
                mcp: None,
            }),
            Some(Answers::Fails(kind, message)) => Err(ToolError::new(kind, message)),
            Some(Answers::OverMcp(text, mcp)) => Ok(ToolOutput {
                content: vec![ToolResultContent::Text(text)],
                is_error: false,
                mcp: Some(mcp),
            }),
            Some(Answers::FailsOverMcp(kind, message, mcp)) => {
                Err(ToolError::new(kind, message).over_mcp(mcp))
            }
            // Nothing scripted: the tool ran and said so, which keeps a test
            // that only cares about the loop's shape short.
            None => Ok(ToolOutput {
                content: vec![ToolResultContent::Text(format!("{} ran", call.name))],
                is_error: false,
                mcp: None,
            }),
        }
    }
}

/// An observer that keeps every event it was handed.
pub struct Recorder {
    events: Mutex<Vec<RunEvent>>,
}

impl Recorder {
    /// An observer that has seen nothing.
    pub const fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    /// Every event, in the order the loop emitted them.
    pub fn events(&self) -> Vec<RunEvent> {
        self.events
            .lock()
            .expect("the recorder isn't poisoned")
            .clone()
    }

    /// The name of each event, which is what a test asserts a run's shape by.
    pub fn names(&self) -> Vec<&'static str> {
        self.events()
            .iter()
            .map(|event| event.kind.name())
            .collect()
    }
}

#[async_trait::async_trait]
impl RunObserver for Recorder {
    async fn on(&self, event: RunEvent) {
        self.events
            .lock()
            .expect("the recorder isn't poisoned")
            .push(event);
    }
}

/// An observer that hands out a span for every call, so the handoff from
/// observer to executor can be asserted.
pub struct Tracer;

#[async_trait::async_trait]
impl RunObserver for Tracer {
    async fn on(&self, _event: RunEvent) {}

    fn trace_context(&self, call_id: &lablet_model::ToolCallId) -> Option<crate::TraceContext> {
        Some(crate::TraceContext {
            traceparent: format!("00-trace-{}-01", call_id.as_str()),
            tracestate: None,
        })
    }
}
