//! The loop as the tests of adapters build it: around a provider and the
//! executors a test hands it, on tokio's clock, never cancelled, and with
//! policies that stay out of a test's way unless it says otherwise.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use lablet_model::{
    CacheScope, CompletionMode, OutputCap, Prompts, RequestParams, RunContext, RunId, RunLabels,
    Secrets, Thinking,
};
use lablet_policy::{Pricing, RetryPolicy, RetrySettings, StopPolicy};
use lablet_provider_fake::{FakeProvider, Script, ScriptFormat, ScriptSource};
use lablet_run::telemetry::{Bridge, Logger};
use lablet_run::{
    CallLimits, Cancellation, ModelProvider, RunEvent, RunObserver, RunService, ToolExecutor,
    ToolFilter, ToolSet,
};
use opentelemetry::global::BoxedTracer;
use opentelemetry::logs::{LoggerProvider as _, NoopLoggerProvider};
use opentelemetry::trace::noop::NoopTracer;

use crate::must;
use crate::{NeverCancelled, TokioClock};

/// The name a script is read under, which a refusal and a failed call name.
pub const SCRIPT: &str = "scripts/run.yaml";

/// The model a scripted provider says it is.
pub const MODEL: &str = "scripted-1";

/// The system prompt of a run.
pub const SYSTEM: &str = "You fix tests, tersely.";

/// The task a run is given.
pub const PROMPT: &str = "Fix the failing test in the parser.";

/// When a run's context says it started, in milliseconds since the epoch.
pub const STARTED_UNIX_MS: u64 = 1_790_000_000_000;

/// The digest a run's context gives its config.
pub const CONFIG_DIGEST: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

/// The version of lablet a run's context names.
pub const AGENT_VERSION: &str = "0.4.2";

/// A provider that plays the YAML script `text`, read under [`SCRIPT`], as
/// [`MODEL`].
///
/// # Panics
///
/// When the script can't be read.
#[must_use]
pub fn scripted(text: &str) -> Arc<FakeProvider> {
    let script = Script::read(ScriptSource {
        name: SCRIPT,
        text,
        format: ScriptFormat::Yaml,
    });
    Arc::new(FakeProvider::new(MODEL, must(script, "reading the script")))
}

/// The context of the run `run`: no labels, no transcript, no skills, no
/// MCP servers, and no content captured.
///
/// # Panics
///
/// When `run` isn't a run id.
#[must_use]
pub fn context(run: &str) -> RunContext {
    RunContext {
        run_id: must(RunId::new(run), "naming the run"),
        labels: RunLabels::default(),
        started_unix_ms: STARTED_UNIX_MS,
        config_digest: must(
            lablet_model::ConfigDigest::new(CONFIG_DIGEST),
            "naming the config",
        ),
        agent_version: AGENT_VERSION.to_owned(),
        transcript_path: None,
        skills_count: 0,
        mcp: None,
        capture_content: false,
    }
}

/// [`SYSTEM`] and [`PROMPT`].
#[must_use]
pub fn prompts() -> Prompts {
    must(Prompts::new(SYSTEM, PROMPT), "making the prompts")
}

/// What every provider call of a built loop asks for, unless a test says
/// otherwise: 4,096 tokens, and the provider's own defaults for the rest.
#[must_use]
pub fn request() -> RequestParams {
    RequestParams {
        max_tokens: 4_096,
        temperature: None,
        thinking: Thinking::ProviderDefault,
        effort: None,
        seed: None,
        cache_scope: CacheScope::Shared,
    }
}

/// An observer that keeps nothing.
pub struct Unobserved;

#[async_trait::async_trait]
impl RunObserver for Unobserved {
    async fn on(&self, _: RunEvent) {}
}

/// Builds the loop around a provider. Unless a test says otherwise the loop
/// offers no tool, tells no observer, emits its spans and records to no one,
/// is never cancelled, has no cap on turns and an hour to run, stops at the
/// third invalid turn in a row, tries a failed call again three times after
/// waits of 100 ms doubled each time with no jitter, asks for [`request`],
/// prices nothing, gives an attempt a minute, sends a tool's output whole,
/// holds no secret, and runs one tool call at a time.
pub struct RunBuilder {
    provider: Arc<dyn ModelProvider>,
    tools: Vec<Arc<dyn ToolExecutor>>,
    observer: Arc<dyn RunObserver>,
    tracer: BoxedTracer,
    logger: Box<dyn Logger>,
    cancel: Arc<dyn Cancellation>,
    max_turns: Option<NonZeroU32>,
    max_retries: u32,
    request: RequestParams,
    pricing: Option<Pricing>,
    provider_timeout: Duration,
    output_cap: Option<OutputCap>,
    max_concurrent_tool_calls: NonZeroU32,
    secrets: Arc<Secrets>,
}

impl RunBuilder {
    /// The loop around `provider`.
    #[must_use]
    pub fn new(provider: Arc<dyn ModelProvider>) -> Self {
        Self {
            provider,
            tools: Vec::new(),
            observer: Arc::new(Unobserved),
            tracer: BoxedTracer::new(Box::new(NoopTracer::new())),
            logger: Box::new(Bridge::new(NoopLoggerProvider::new().logger("lablet"))),
            cancel: Arc::new(NeverCancelled),
            max_turns: None,
            max_retries: 3,
            request: request(),
            pricing: None,
            provider_timeout: Duration::from_secs(60),
            output_cap: None,
            max_concurrent_tool_calls: NonZeroU32::MIN,
            secrets: Arc::default(),
        }
    }

    /// Offers the tools of `executors`, unfiltered.
    #[must_use]
    pub fn tools(mut self, executors: Vec<Arc<dyn ToolExecutor>>) -> Self {
        self.tools = executors;
        self
    }

    /// Tells `observer` of every run.
    #[must_use]
    pub fn observer(mut self, observer: Arc<dyn RunObserver>) -> Self {
        self.observer = observer;
        self
    }

    /// Opens every span of a run through `tracer`.
    #[must_use]
    pub fn tracer(mut self, tracer: BoxedTracer) -> Self {
        self.tracer = tracer;
        self
    }

    /// Writes every record of a run through `logger`.
    #[must_use]
    pub fn logger(mut self, logger: Box<dyn Logger>) -> Self {
        self.logger = logger;
        self
    }

    /// Asks `cancel` whether a run should stop.
    #[must_use]
    pub fn cancellation(mut self, cancel: Arc<dyn Cancellation>) -> Self {
        self.cancel = cancel;
        self
    }

    /// Stops a run after `max_turns` turns, when there's a cap.
    #[must_use]
    pub fn max_turns(mut self, max_turns: Option<NonZeroU32>) -> Self {
        self.max_turns = max_turns;
        self
    }

    /// Tries a failed call again up to `max_retries` times.
    #[must_use]
    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Asks every provider call for `request`.
    #[must_use]
    pub fn request(mut self, request: RequestParams) -> Self {
        self.request = request;
        self
    }

    /// Prices a run at `pricing`, when it's priced.
    #[must_use]
    pub fn pricing(mut self, pricing: Option<Pricing>) -> Self {
        self.pricing = pricing;
        self
    }

    /// Gives one attempt of a provider call `timeout`.
    #[must_use]
    pub fn provider_timeout(mut self, timeout: Duration) -> Self {
        self.provider_timeout = timeout;
        self
    }

    /// Caps what the model is sent of a tool's output at `cap`.
    #[must_use]
    pub fn output_cap(mut self, cap: OutputCap) -> Self {
        self.output_cap = Some(cap);
        self
    }

    /// Runs up to `max` calls of a turn at once.
    #[must_use]
    pub fn max_concurrent_tool_calls(mut self, max: NonZeroU32) -> Self {
        self.max_concurrent_tool_calls = max;
        self
    }

    /// Hands `secrets` to every executor with each call, and cuts them out
    /// of every text the loop writes itself.
    #[must_use]
    pub fn secrets(mut self, secrets: Arc<Secrets>) -> Self {
        self.secrets = secrets;
        self
    }

    /// The loop.
    ///
    /// # Panics
    ///
    /// When the executors' tools can't be offered together, as when two
    /// share a name.
    pub async fn build(self) -> RunService {
        let tools = ToolSet::build(
            self.tools,
            &ToolFilter::default(),
            CompletionMode::Natural,
            None,
        )
        .await;
        let retry = RetryPolicy::new(RetrySettings {
            max_retries: self.max_retries,
            base: Duration::from_millis(100),
            max: Duration::from_secs(10),
            factor: 2.0,
            hint_max: Duration::from_secs(60),
            jitter: 0.0,
        });
        RunService::new(
            self.provider,
            Arc::new(must(tools, "offering the tools")),
            self.observer,
            self.tracer,
            self.logger,
            Arc::new(TokioClock),
            self.cancel,
            StopPolicy {
                max_turns: self.max_turns,
                timeout: Duration::from_secs(3_600),
                max_total_tokens: None,
                max_consecutive_invalid_turns: NonZeroU32::new(3),
            },
            must(retry, "making the retry policy"),
            self.request,
            self.pricing,
            CallLimits {
                provider_timeout: self.provider_timeout,
                output_cap: self.output_cap,
                max_concurrent_tool_calls: self.max_concurrent_tool_calls,
            },
            self.secrets,
        )
    }
}

#[cfg(test)]
mod tests;
