//! A run from its start to its wide event: what names it, the root span the
//! loop runs under, the transcript, and the one record that sums it up.

use std::sync::Arc;

use lablet_model::{ConfigDigest, FinishedRun, Prompts, RunContext, RunId, RunLabels};
use opentelemetry::Context;
use opentelemetry::global::BoxedTracer;
use opentelemetry::trace::{FutureExt as _, Span as _, TraceContextExt as _, Tracer as _};
use ulid::Ulid;

use crate::cancellation::RunCancellation;
use crate::telemetry::generated::LabletInvokeAgent;
use crate::telemetry::{Logger, root_span, time_at, wide};
use crate::transcript::{RunTranscript, TranscriptError, TranscriptWriter};
use crate::{Cancellation, Clock, RunService, ToolSet};

/// What a run is asked to do, what it's known by, and what may stop it.
pub struct RunStart {
    /// The task, under no system prompt: the runner has the system prompt.
    pub task: Prompts,
    /// The run's id, or `None` for a fresh ULID of the time it starts.
    pub run_id: Option<RunId>,
    /// What names the run's task, its experiment and its trial.
    pub labels: RunLabels,
    /// What stops the run, or `None` when nothing may.
    pub cancellation: Option<Arc<dyn Cancellation>>,
}

/// What every run of one [`Runner`] shares, beside the loop.
pub struct Shared {
    /// The system prompt every run's task is put under.
    pub system: String,
    /// The digest of the config the runner was built from.
    pub config_digest: ConfigDigest,
    /// Whether a run's content goes into its records.
    pub capture_content: bool,
    /// Where a run's transcript goes, when the config names a place for one.
    pub transcript: Option<Arc<dyn TranscriptWriter>>,
    /// The agent's version, which every run's signals carry.
    pub agent_version: String,
}

/// What a run came to, and why its transcript wasn't written, when it
/// wasn't: the runner has no diagnostics of its own, so the caller reports
/// it.
#[derive(Debug)]
pub struct Ran {
    /// The run, whatever it came to.
    pub finished: FinishedRun,
    /// Why the run's transcript has no place or wasn't written, when the
    /// config named a place for one.
    pub transcript: Option<TranscriptError>,
}

/// Runs a run around the loop: its id and its context, its root span, its
/// transcript and its wide event.
pub struct Runner {
    service: RunService,
    tools: Arc<ToolSet>,
    /// The tracer the root span is opened with, of the scope the loop's is.
    tracer: BoxedTracer,
    /// The logger the wide event goes through, of the scope the loop's is.
    logger: Box<dyn Logger>,
    clock: Arc<dyn Clock>,
    /// The cancellation `service` was built with, which each run sets.
    cancellation: Arc<RunCancellation>,
    shared: Shared,
}

impl Runner {
    /// A runner of `service`, which was built with `tools`, `clock` and
    /// `cancellation`, and whose tracer and logger are of the scope
    /// `tracer` and `logger` are.
    #[must_use]
    pub fn new(
        service: RunService,
        tools: Arc<ToolSet>,
        tracer: BoxedTracer,
        logger: Box<dyn Logger>,
        clock: Arc<dyn Clock>,
        cancellation: Arc<RunCancellation>,
        shared: Shared,
    ) -> Self {
        Self {
            service,
            tools,
            tracer,
            logger,
            clock,
            cancellation,
            shared,
        }
    }

    /// Runs `start` to its outcome.
    ///
    /// The run starts at one reading of the wall clock, and a fresh id holds
    /// that time. Its root span is a child of the context this is called in,
    /// and the loop runs in the root span's context, so every span of the
    /// loop is the root span's child. Once the loop has returned, the root
    /// span is filled and ends with the run's measured duration, the
    /// transcript is written, when the config names a place for it, and the
    /// wide event is emitted then, so the transcript it names is whole.
    ///
    /// Never fails: every way a run can go wrong is a stop reason of its
    /// outcome, and a transcript that has no place or wasn't written is
    /// handed back for the caller to report.
    pub async fn run(&mut self, start: RunStart) -> Ran {
        let RunStart {
            task,
            run_id,
            labels,
            cancellation,
        } = start;
        let started = self.clock.wall();
        let run_id = run_id.unwrap_or_else(|| RunId::ulid(Ulid::from_datetime(started).0));
        let (place, unplaced) = match &self.shared.transcript {
            None => (None, None),
            Some(writer) => match writer.place(&run_id) {
                Ok(place) => (Some((Arc::clone(writer), place)), None),
                Err(error) => (None, Some(error)),
            },
        };
        let context = RunContext {
            run_id,
            labels,
            started,
            config_digest: self.shared.config_digest.clone(),
            agent_version: self.shared.agent_version.clone(),
            transcript_path: place.as_ref().map(|(_, place)| place.clone()),
            skills_count: 0,
            mcp: None,
            capture_content: self.shared.capture_content,
        };
        let task_prompt = task.task().to_owned();
        let prompts = task.with_system(self.shared.system.as_str());
        // Every run sets its own, so a handle of an earlier run never
        // reaches this one.
        self.cancellation.set(cancellation);

        // The span stays here, where it's filled and ended, since the
        // generated struct records onto an owned span. The loop runs under a
        // context that holds the span's context alone, as each tool call
        // does under its own, and the parent's other entries, its baggage
        // among them.
        let parent = Context::current();
        let mut root = self
            .tracer
            .span_builder(root_span::name())
            .with_kind(LabletInvokeAgent::KIND)
            .with_start_time(context.started)
            .start_with_context(&self.tracer, &parent);
        let within = parent.with_remote_span_context(root.span_context().clone());
        let finished = self
            .service
            .run(context.clone(), prompts)
            .with_context(within)
            .await;

        let summary = &finished.summary;
        let end = time_at(context.started, finished.duration);
        root_span::invoke_agent(&context, summary).record(&mut root);
        root.set_status(root_span::status(&summary.outcome));
        root.end_with_timestamp(end);
        let wide = wide::wide_event(&context, summary).record(end, root.span_context());

        let transcript = match place {
            Some((writer, place)) => {
                let transcript = RunTranscript {
                    context,
                    model: summary.model.clone(),
                    tools: self.tools.specs().to_vec(),
                    task: task_prompt,
                    transcript: finished.transcript.clone(),
                };
                writer.write(place, transcript).await.err()
            }
            None => unplaced,
        };
        self.logger.emit(wide);
        Ran {
            finished,
            transcript,
        }
    }
}

#[cfg(test)]
mod tests;
