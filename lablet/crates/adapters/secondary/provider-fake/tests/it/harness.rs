//! The loop, built around a scripted provider, on tokio's clock. A test
//! pauses that clock, so a script's latencies and the loop's waits pass at
//! once and are measured exactly.

use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lablet_model::{
    CacheScope, CompletionMode, FinishedRun, Prompts, RequestParams, RunContext, RunId, RunLabels,
    Thinking,
};
use lablet_policy::{RetryPolicy, RetrySettings, StopPolicy};
use lablet_provider_fake::{FakeProvider, Script, ScriptFormat, ScriptSource};
use lablet_run::{
    CallLimits, Cancellation, Clock, EventKind, RunEvent, RunObserver, RunService, ToolFilter,
    ToolSet,
};

pub const SCRIPT: &str = "scripts/run.yaml";

/// How long one attempt may take in these runs.
pub const PROVIDER_TIMEOUT: Duration = Duration::from_secs(1);

/// The clock the provider waits on, so the loop measures the waits the
/// script asked for.
struct TokioClock;

#[async_trait::async_trait]
impl Clock for TokioClock {
    fn now(&self) -> Instant {
        tokio::time::Instant::now().into_std()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

struct NeverCancelled;

impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Keeps every event, in the order the loop emitted them.
#[derive(Default)]
pub struct Recorder {
    events: Mutex<Vec<EventKind>>,
}

#[async_trait::async_trait]
impl RunObserver for Recorder {
    async fn on(&self, event: RunEvent) {
        self.events.lock().unwrap().push(event.kind);
    }
}

impl Recorder {
    /// How many attempts of a provider call the loop began.
    pub fn attempts(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, EventKind::ProviderCallStarted { .. }))
            .count()
    }

    /// Each failed attempt, in order: the failure's message, how long the
    /// attempt took, and the wait before the next, when there was one.
    pub fn failures(&self) -> Vec<(String, u64, Option<Duration>)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                EventKind::ProviderCallFailed {
                    error,
                    latency_ms,
                    retry,
                    ..
                } => Some((error.message().to_owned(), *latency_ms, *retry)),
                _ => None,
            })
            .collect()
    }
}

/// A loop with no tools, around a provider that plays a script.
pub struct Harness {
    pub provider: Arc<FakeProvider>,
    pub recorder: Arc<Recorder>,
    service: RunService,
}

impl Harness {
    /// The loop around the YAML script `text`, which retries a failed
    /// attempt up to `max_retries` times.
    pub async fn playing(text: &str, max_retries: u32) -> Self {
        let script = Script::read(ScriptSource {
            name: SCRIPT,
            text,
            format: ScriptFormat::Yaml,
        })
        .unwrap();
        let provider = Arc::new(FakeProvider::new("scripted-1", script));
        let recorder = Arc::new(Recorder::default());
        let tools = ToolSet::build(
            Vec::new(),
            &ToolFilter::default(),
            CompletionMode::Natural,
            None,
        )
        .await
        .unwrap();
        let service = RunService::new(
            Arc::clone(&provider) as _,
            Arc::new(tools),
            Arc::clone(&recorder) as _,
            Arc::new(TokioClock),
            Arc::new(NeverCancelled),
            StopPolicy {
                max_turns: None,
                timeout: Duration::from_secs(600),
                max_total_tokens: None,
                max_consecutive_invalid_turns: NonZeroU32::new(3),
            },
            RetryPolicy::new(RetrySettings {
                max_retries,
                base: Duration::from_millis(100),
                max: Duration::from_secs(10),
                factor: 2.0,
                hint_max: Duration::from_secs(60),
                jitter: 0.0,
            })
            .unwrap(),
            RequestParams {
                max_tokens: 4_096,
                temperature: None,
                thinking: Thinking::ProviderDefault,
                effort: None,
                seed: None,
                cache_scope: CacheScope::Shared,
            },
            None,
            CallLimits {
                provider_timeout: PROVIDER_TIMEOUT,
                output_cap: None,
                max_concurrent_tool_calls: NonZeroU32::MIN,
            },
        );
        Self {
            provider,
            recorder,
            service,
        }
    }

    /// One run, under the run id `run_id`.
    pub async fn run(&mut self, run_id: &str) -> FinishedRun {
        let context = RunContext {
            run_id: RunId::new(run_id).unwrap(),
            labels: RunLabels::default(),
            started_unix_ms: 1_790_000_000_000,
            config_digest: "0".repeat(64),
            agent_version: "0.1.0".to_owned(),
            resource: Vec::new(),
            transcript_path: None,
            skills_count: 0,
            mcp: None,
            capture_content: false,
        };
        let prompts = Prompts::new("You fix tests.", "Fix the failing test.").unwrap();
        self.service.run(context, prompts).await
    }
}
