//! Hand-written stand-ins for every port, so the loop can be driven without
//! a provider, a tool, a clock, or a wait.
//!
//! They're `#[cfg(test)]` rather than a feature, because nothing outside this
//! crate's tests should build a run out of them: the product's own scripted
//! provider is an adapter, written at phase 4, and answers to a config.

use std::collections::VecDeque;
use std::future::poll_fn;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

use lablet_model::{
    Endpoint, KeptOutput, ModelRef, ProviderErrorKind, ProviderResponse, ToolName, ToolSpec,
};

use crate::{
    Cancellation, Clock, ModelProvider, ProviderError, ProviderRequest, RunEvent, RunObserver,
    ToolCall, ToolError, ToolExecutor, ToolOutput,
};

/// How many times a call in flight yields to the runtime, once the run is
/// cancelled, before it gives up and returns. A loop that races the call
/// against the cancellation drops it at the first; one that doesn't sees it
/// return, with what says so, and the test fails rather than waiting for
/// ever.
const HOLDS_ON: usize = 64;

/// Yields [`HOLDS_ON`] times.
async fn hold_on() {
    for _ in 0..HOLDS_ON {
        tokio::task::yield_now().await;
    }
}

/// A clock that only moves when a test moves it, and never waits.
///
/// `sleep` adds the wait to the reading instead of blocking, so a run with a
/// ten-minute backoff finishes in microseconds and the transcript still says
/// ten minutes passed.
pub struct FakeClock {
    origin: Instant,
    elapsed: Mutex<Duration>,
    slept: Mutex<Vec<Duration>>,
    interrupted: Mutex<Option<(Arc<FakeCancel>, Duration)>>,
}

impl FakeClock {
    /// A clock reading zero.
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
            slept: Mutex::new(Vec::new()),
            interrupted: Mutex::new(None),
        }
    }

    /// The next wait is cut short: `after` of it passes and the run is
    /// cancelled, as a wait is that's under way when the user presses
    /// Ctrl-C. A loop that goes on waiting sees it end at the time it was
    /// asked for.
    pub fn interrupts_the_next_sleep(&self, cancel: Arc<FakeCancel>, after: Duration) {
        *self
            .interrupted
            .lock()
            .expect("the fake clock isn't poisoned") = Some((cancel, after));
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
        let interrupted = self
            .interrupted
            .lock()
            .expect("the fake clock isn't poisoned")
            .take();
        if let Some((cancel, after)) = interrupted {
            self.advance(after);
            cancel.cancel();
            hold_on().await;
            self.advance(duration.saturating_sub(after));
            return;
        }
        self.advance(duration);
    }
}

/// Cancellation a test turns on when it likes: from the start, after a
/// count of the times the loop asked, or from inside the run, as a user's
/// Ctrl-C would.
pub struct FakeCancel {
    cancelled: AtomicBool,
    /// Cancels once the loop has asked this many times, so a test can cancel
    /// at a chosen point in the loop rather than from the start.
    after_polls: Option<usize>,
    polls: AtomicUsize,
    /// What's waiting to hear of the cancellation, which it wakes.
    waiting: Mutex<Vec<Waker>>,
}

impl FakeCancel {
    const fn new(cancelled: bool, after_polls: Option<usize>) -> Self {
        Self {
            cancelled: AtomicBool::new(cancelled),
            after_polls,
            polls: AtomicUsize::new(0),
            waiting: Mutex::new(Vec::new()),
        }
    }

    /// Never cancels.
    pub const fn never() -> Self {
        Self::new(false, None)
    }

    /// Cancelled before the run starts.
    pub const fn already() -> Self {
        Self::new(true, None)
    }

    /// Answers `false` the first `polls` times the loop asks whether the run
    /// is cancelled, and `true` from then on. Waiting for the cancellation
    /// isn't asking: it hears of it once the loop has been told.
    pub const fn after(polls: usize) -> Self {
        Self::new(false, Some(polls))
    }

    /// Cancels the run from here on, whatever the count, so a scripted call
    /// can cancel it while it's in flight.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        let waiting = std::mem::take(&mut *self.waiting.lock().expect("the fake isn't poisoned"));
        for waker in waiting {
            waker.wake();
        }
    }

    fn is_set(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl Cancellation for FakeCancel {
    fn is_cancelled(&self) -> bool {
        let polls = self.polls.fetch_add(1, Ordering::Relaxed);
        if self.after_polls.is_some_and(|after| polls >= after) && !self.is_set() {
            self.cancel();
        }
        self.is_set()
    }

    async fn cancelled(&self) {
        poll_fn(|context| {
            if self.is_set() {
                return Poll::Ready(());
            }
            self.waiting
                .lock()
                .expect("the fake isn't poisoned")
                .push(context.waker().clone());
            // Asked again once the waker is in, so a cancellation between the
            // two looks isn't missed.
            if self.is_set() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
    }
}

/// Records, when it's dropped, that the call `id` was dropped in flight,
/// unless the call returned first.
struct Dropped<'a> {
    id: String,
    log: &'a Mutex<Vec<String>>,
}

impl Dropped<'_> {
    /// The call returned, so it wasn't dropped in flight.
    fn returned(self) {
        std::mem::forget(self);
    }
}

impl Drop for Dropped<'_> {
    fn drop(&mut self) {
        self.log
            .lock()
            .expect("the log isn't poisoned")
            .push(format!("x{}", self.id));
    }
}

/// One scripted answer to a provider call.
pub enum Answer {
    /// The call succeeds, after `latency`.
    Responds(Box<ProviderResponse>, Duration),
    /// The call fails, after `latency`, with whatever usage and hint the
    /// error carries.
    Fails(ProviderError, Duration),
    /// The call takes `latency` and then cancels the run, as an attempt does
    /// that's in flight when the user presses Ctrl-C. A loop that doesn't
    /// drop it sees it answer with a response that says so.
    Cancels(Arc<FakeCancel>, Duration),
}

impl Answer {
    /// A response that took no time.
    pub fn now(response: ProviderResponse) -> Self {
        Self::Responds(Box::new(response), Duration::ZERO)
    }

    /// A failure of `kind` that took no time.
    pub fn fails(kind: ProviderErrorKind) -> Self {
        Self::failing(ProviderError::new(
            kind,
            format!("the fake provider failed: {kind}"),
        ))
    }

    /// The failure `error`, which took no time.
    pub const fn failing(error: ProviderError) -> Self {
        Self::Fails(error, Duration::ZERO)
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
    sent: Mutex<Vec<serde_json::Value>>,
    shown: Mutex<Vec<Shown>>,
    deadlines: Mutex<Vec<Duration>>,
    ran_out: AtomicBool,
    dropped: Mutex<Vec<String>>,
}

/// What one attempt carried beside the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The system prompt.
    pub system: String,
    /// The tool specs, in the order they were offered.
    pub tools: Vec<ToolSpec>,
    /// The cache key, when the request had one.
    pub cache_key: Option<String>,
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
            sent: Mutex::new(Vec::new()),
            shown: Mutex::new(Vec::new()),
            deadlines: Mutex::new(Vec::new()),
            ran_out: AtomicBool::new(false),
            dropped: Mutex::new(Vec::new()),
        }
    }

    /// The attempts that were dropped before they returned, as `x` and the
    /// attempt's number, counted from 1.
    pub fn dropped(&self) -> Vec<String> {
        self.dropped
            .lock()
            .expect("the fake provider isn't poisoned")
            .clone()
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

    /// The messages each attempt was sent, in the model's serde form, so a
    /// test can assert what the model was told rather than what was recorded.
    pub fn sent(&self) -> Vec<serde_json::Value> {
        self.sent
            .lock()
            .expect("the fake provider isn't poisoned")
            .clone()
    }

    /// What each attempt carried beside the conversation, in order.
    pub fn shown(&self) -> Vec<Shown> {
        self.shown
            .lock()
            .expect("the fake provider isn't poisoned")
            .clone()
    }

    /// The deadline each attempt was given, in order. The script decides how
    /// long an attempt takes, so a deadline is kept here and not enforced.
    pub fn deadlines(&self) -> Vec<Duration> {
        self.deadlines
            .lock()
            .expect("the fake provider isn't poisoned")
            .clone()
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
        request: ProviderRequest<'_>,
    ) -> Result<ProviderResponse, ProviderError> {
        let attempt = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        self.sent
            .lock()
            .expect("the fake provider isn't poisoned")
            .push(serde_json::to_value(request.messages).expect("messages serialise"));
        self.shown
            .lock()
            .expect("the fake provider isn't poisoned")
            .push(Shown {
                system: request.system.to_owned(),
                tools: request.tools.to_vec(),
                cache_key: request.cache_key.map(str::to_owned),
            });
        self.deadlines
            .lock()
            .expect("the fake provider isn't poisoned")
            .push(request.deadline);
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
            Some(Answer::Fails(error, latency)) => {
                self.clock.advance(latency);
                Err(error)
            }
            Some(Answer::Cancels(cancel, latency)) => {
                let dropped = Dropped {
                    id: attempt.to_string(),
                    log: &self.dropped,
                };
                self.clock.advance(latency);
                cancel.cancel();
                hold_on().await;
                dropped.returned();
                Ok(ProviderResponse::new(
                    vec![lablet_model::ContentBlock::Text(
                        "the attempt in flight was never dropped".to_owned(),
                    )],
                    lablet_model::Usage::default(),
                    lablet_model::FinishReason::EndTurn,
                    None,
                    None,
                )
                .expect("a response with no calls is a response"))
            }
            // A script that runs out is a test that didn't say what happens
            // next, which is worth failing loudly rather than looping. The
            // failure ends a run, so a loop that calls again is retrying what
            // no attempt could answer, and would for ever.
            None => {
                assert!(
                    !self.ran_out.swap(true, Ordering::Relaxed),
                    "the loop called again after the fake provider's script ran out"
                );
                Err(ProviderError::new(
                    ProviderErrorKind::Fatal,
                    "the fake provider's script ran out",
                ))
            }
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
    /// Cancels the run while the call is running, and then returns this
    /// text at once, as a call does that ended just as the user pressed
    /// Ctrl-C.
    Cancelling(Arc<FakeCancel>, String),
    /// Starts, lets every other call of its group start, takes `latency`,
    /// and cancels the run when it holds a cancellation, as a call does
    /// that's in flight when the user presses Ctrl-C. Its drop is recorded
    /// among the spans as `x` and the call's id. A loop that doesn't drop it
    /// sees it return a result that says so.
    Stalls(Option<Arc<FakeCancel>>, Duration),
}

/// An executor serving named tools with scripted answers.
pub struct FakeTools {
    specs: Vec<ToolSpec>,
    answers: Mutex<Vec<(ToolName, Answers)>>,
    latency: Duration,
    clock: Arc<FakeClock>,
    listing: Option<crate::ToolErrorKind>,
    taken: Mutex<Vec<ToolCall>>,
    yielding: bool,
    spans: Mutex<Vec<String>>,
    hoarding: bool,
    kept: Mutex<Vec<u64>>,
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
            yielding: false,
            spans: Mutex::new(Vec::new()),
            hoarding: false,
            kept: Mutex::new(Vec::new()),
        }
    }

    /// Every call's output is held whole, whatever the call says to keep,
    /// as an executor does that reads a tool's output to its end before it
    /// looks at the size.
    pub const fn hoarding(mut self) -> Self {
        self.hoarding = true;
        self
    }

    /// How many bytes of each output this executor held when it returned
    /// it, in order.
    pub fn kept(&self) -> Vec<u64> {
        self.kept
            .lock()
            .expect("the fake executor isn't poisoned")
            .clone()
    }

    /// A tool's text, fed as a real executor feeds it: a piece at a time, to
    /// what the call says to keep. A character is the smallest piece there
    /// is, so every limit is met part-way through the text.
    fn fed(&self, call: &ToolCall, text: &str) -> KeptOutput {
        let mut output = KeptOutput::new(call.keep.filter(|_| !self.hoarding));
        let mut piece = [0; 4];
        for character in text.chars() {
            output.push(character.encode_utf8(&mut piece));
        }
        self.kept
            .lock()
            .expect("the fake executor isn't poisoned")
            .push(output.kept_bytes());
        output
    }

    /// Every call yields to the runtime once part-way, so calls the loop
    /// runs together overlap instead of each finishing before the next is
    /// polled.
    pub const fn yielding(mut self) -> Self {
        self.yielding = true;
        self
    }

    /// When each call started and ended, as `+id` and `-id`, in order.
    pub fn spans(&self) -> Vec<String> {
        self.spans
            .lock()
            .expect("the fake executor isn't poisoned")
            .clone()
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

    /// Every call takes this long, unless it reaches its deadline first or
    /// at the same moment, and then it takes until its deadline and has
    /// timed out.
    pub const fn taking(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    /// The next answer to `name`, taken, when it's one that stalls.
    fn stalling(&self, name: &ToolName) -> Option<Answers> {
        let mut answers = self
            .answers
            .lock()
            .expect("the fake executor isn't poisoned");
        let at = answers
            .iter()
            .position(|(answers_to, _)| answers_to == name)?;
        matches!(answers[at].1, Answers::Stalls(..)).then(|| answers.remove(at).1)
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
        self.taken
            .lock()
            .expect("the fake executor isn't poisoned")
            .push(call.clone());
        // An executor resolves only the names it offered, so a call routed
        // here by mistake is refused, as a real executor refuses it, rather
        // than answered as if a tool had run.
        if !self.specs.iter().any(|spec| spec.name == call.name) {
            return Err(ToolError::new(
                crate::ToolErrorKind::Unknown,
                format!("the fake executor serves no tool named {}", call.name),
            ));
        }
        let id = call.id.as_str().to_owned();
        self.spans
            .lock()
            .expect("the fake executor isn't poisoned")
            .push(format!("+{id}"));
        if let Some(Answers::Stalls(cancel, latency)) = self.stalling(&call.name) {
            let dropped = Dropped {
                id,
                log: &self.spans,
            };
            tokio::task::yield_now().await;
            self.clock.advance(latency);
            if let Some(cancel) = cancel {
                cancel.cancel();
            }
            hold_on().await;
            dropped.returned();
            return Ok(ToolOutput {
                output: KeptOutput::whole("the call in flight was never dropped"),
                is_error: false,
                mcp: None,
            });
        }
        if self.yielding {
            tokio::task::yield_now().await;
        }
        self.spans
            .lock()
            .expect("the fake executor isn't poisoned")
            .push(format!("-{id}"));
        // The port's contract: a call takes no longer than its deadline, and
        // one that reached it has timed out and stopped by the time it
        // returns.
        self.clock.advance(self.latency.min(call.deadline));
        if self.latency >= call.deadline {
            return Err(ToolError::new(
                crate::ToolErrorKind::Timeout,
                format!("{} was stopped at its deadline", call.name),
            ));
        }
        let mut answers = self
            .answers
            .lock()
            .expect("the fake executor isn't poisoned");
        let found = answers.iter().position(|(name, _)| *name == call.name);
        let answer = found.map(|at| answers.remove(at).1);
        match answer {
            Some(Answers::Text(text)) => Ok(ToolOutput {
                output: self.fed(&call, &text),
                is_error: false,
                mcp: None,
            }),
            Some(Answers::ToolError(text)) => Ok(ToolOutput {
                output: self.fed(&call, &text),
                is_error: true,
                mcp: None,
            }),
            Some(Answers::Fails(kind, message)) => Err(ToolError::new(kind, message)),
            Some(Answers::OverMcp(text, mcp)) => Ok(ToolOutput {
                output: self.fed(&call, &text),
                is_error: false,
                mcp: Some(mcp),
            }),
            Some(Answers::FailsOverMcp(kind, message, mcp)) => {
                Err(ToolError::new(kind, message).over_mcp(mcp))
            }
            Some(Answers::Stalls(..)) => {
                unreachable!("an answer that stalls is taken before the call takes its time")
            }
            Some(Answers::Cancelling(cancel, text)) => {
                cancel.cancel();
                Ok(ToolOutput {
                    output: self.fed(&call, &text),
                    is_error: false,
                    mcp: None,
                })
            }
            // Nothing scripted: the tool ran and said so, which keeps a test
            // that only cares about the loop's shape short.
            None => Ok(ToolOutput {
                output: self.fed(&call, &format!("{} ran", call.name)),
                is_error: false,
                mcp: None,
            }),
        }
    }
}

/// An observer that keeps every event it was handed.
pub struct Recorder {
    events: Mutex<Vec<RunEvent>>,
    slow: Option<Slow>,
}

/// The events an observer takes time over. The loop waits for an observer,
/// so time can pass in one, and this is how a test makes it pass between
/// two things the loop does that take none themselves.
struct Slow {
    clock: Arc<FakeClock>,
    over: &'static str,
    taking: Duration,
}

impl Recorder {
    /// An observer that has seen nothing.
    pub const fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            slow: None,
        }
    }

    /// The same, which takes `taking` on `clock` over every event named
    /// `over`.
    pub const fn slow_over(clock: Arc<FakeClock>, over: &'static str, taking: Duration) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            slow: Some(Slow {
                clock,
                over,
                taking,
            }),
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
        if let Some(slow) = &self.slow
            && slow.over == event.kind.name()
        {
            slow.clock.advance(slow.taking);
        }
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
