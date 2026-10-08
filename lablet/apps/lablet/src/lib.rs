//! The library root: the config, `build`, and the [`Lablet`] that runs the
//! loop. It selects its adapters through the root kernels it shares with the
//! command line's root, and depends on no other root.
//!
//! A [`Lablet`] is built from a [`Config`] and runs many times. Each run
//! takes a [`RunRequest`] and returns the [`FinishedRun`] the loop made of
//! it, whose summary holds the outcome, and its transcript is written when
//! the config names a place for one.
//!
//! A config's text is read into a [`RawConfig`] first, where an override
//! can state a setting over the text, and then into a [`Config`], which
//! holds `${VAR}` as it's written: the variables are read when the config is
//! checked or a `Lablet` is built, so the digest and every message see the
//! config as it's written. [`check`] checks a config as [`build`] does and
//! stops before the provider is selected. A run stops early when the
//! [`CancelHandle`] its request was given is fired.
//!
//! A `Lablet` runs on the OpenTelemetry its host provides, and configures
//! no SDK of its own. The host hands in the logger provider its records go
//! to, which is required, since OpenTelemetry's API has no global logger
//! provider: a host that wants no records hands in the API's
//! `NoopLoggerProvider`. It may hand in the tracer provider its spans go
//! to, and one it doesn't hand in is OpenTelemetry's global one, read once,
//! when the `Lablet` is built, so a host sets its globals before it builds
//! one. Each run's root span is the child of the context that's current
//! where the run is awaited, so a run under a span the host has open is in
//! the host's trace. The host's SDK samples, exports and flushes: a run
//! returns without flushing anything, and [`Lablet::shutdown`] does no
//! telemetry work. A host's `tracing` subscriber sees lablet's diagnostics
//! and none of its telemetry, and a tool executor finds its call's span in
//! the current OpenTelemetry context.
//!
//! What configured an SDK does nothing in library mode, and nothing is said
//! of it: the config's `telemetry.file`, `telemetry.otlp` and
//! `telemetry.resource`, every `OTEL_*` variable the SDK reads,
//! `OTEL_SDK_DISABLED`, and `TRACEPARENT`, `TRACESTATE` and `BAGGAGE`.
//! Content capture still applies, `telemetry.capture_content` and then
//! `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT`, and so does the cut
//! of the OTLP header variables' values and of the endpoint variables' user
//! information from what a command prints. The secrets the config's own
//! fields name are withheld from commands and cut as in the command line,
//! the values of `telemetry.otlp.headers` and the user information of
//! `telemetry.otlp.endpoint` among them, though no SDK uses them.
//!
//! A `Lablet` is built and run on a tokio runtime, which its adapters keep
//! their deadlines on.
//!
//! The provider `fake` plays a script in place of a model, so a run needs
//! no key. Here a host hands in providers of the SDK's own, over its
//! in-memory exporters:
//!
//! ```
//! use lablet::{Config, Format, Lablet, OutcomeDocument, RunId, RunRequest, StopReason};
//! use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
//! use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let directory = std::env::temp_dir().join(format!("lablet-doctest-{}", std::process::id()));
//! # std::fs::create_dir_all(directory.join("work"))?;
//! # let directory = directory.display();
//! std::fs::write(
//!     format!("{directory}/script.yaml"),
//!     "
//! - response:
//!     content:
//!       - text: I'll look at what's there.
//!       - tool_use: { id: call_1, name: bash, input: { json: { command: echo parser.rs } } }
//!     finish: tool_use
//!     usage: { input_tokens: 120, output_tokens: 30 }
//! - response:
//!     content:
//!       - text: There's one file, parser.rs.
//!     finish: end_turn
//!     usage: { input_tokens: 180, output_tokens: 12 }
//! ",
//! )?;
//! let config = Config::from_str(
//!     &format!(
//!         "
//! model:
//!   provider: fake
//!   script: {directory}/script.yaml
//!   name: scripted-1
//! prompt:
//!   system: You answer tersely.
//! tools:
//!   builtin:
//!     root: {directory}/work
//!     enabled: [bash]
//! "
//!     ),
//!     Format::Yaml,
//! )?;
//! let spans = InMemorySpanExporter::default();
//! let records = InMemoryLogExporter::default();
//! let tracer_provider = SdkTracerProvider::builder()
//!     .with_simple_exporter(spans.clone())
//!     .build();
//! let logger_provider = SdkLoggerProvider::builder()
//!     .with_simple_exporter(records.clone())
//!     .build();
//!
//! let runtime = tokio::runtime::Runtime::new()?;
//! runtime.block_on(async {
//!     let mut lablet = Lablet::builder(config, logger_provider.clone())
//!         .with_tracer_provider(tracer_provider.clone())
//!         .build()
//!         .await?;
//!
//!     let first = lablet.run(RunRequest::new("What's in the directory?")?).await;
//!     let named = RunId::new("the-second-run")?;
//!     let second = lablet
//!         .run(RunRequest::new("What's in the directory?")?.run_id(named.clone())?)
//!         .await;
//!     lablet.shutdown().await;
//!
//!     // Every run of one `Lablet` hears the script from its first entry.
//!     let (first, second) = (first.summary.outcome, second.summary.outcome);
//!     for outcome in [&first, &second] {
//!         assert_eq!(outcome.stop_reason(), StopReason::Completed);
//!         assert_eq!(outcome.result().text, "There's one file, parser.rs.");
//!         assert_eq!((outcome.turns, outcome.tool_calls), (2, 1));
//!         assert_eq!(outcome.usage.total(), 342);
//!     }
//!     assert_eq!(second.run_id, named);
//!     assert_ne!(first.run_id, second.run_id);
//!
//!     // The outcome as a document, which is what the two runs share once
//!     // what names a run and what times it are taken out.
//!     let shared = |outcome| {
//!         let mut document = serde_json::to_value(OutcomeDocument::from(outcome))?;
//!         for key in ["run_id", "duration_ms"] {
//!             document.as_object_mut().and_then(|keys| keys.remove(key));
//!         }
//!         Ok::<_, serde_json::Error>(document)
//!     };
//!     assert_eq!(shared(first)?, shared(second)?);
//!
//!     // The host's providers hold both runs, and each run's one wide event.
//!     let wide_events: Vec<_> = records
//!         .get_emitted_logs()?
//!         .into_iter()
//!         .filter(|log| log.record.event_name() == Some("lablet.run"))
//!         .collect();
//!     assert_eq!(wide_events.len(), 2);
//!     let roots = spans
//!         .get_finished_spans()?
//!         .into_iter()
//!         .filter(|span| span.name.starts_with("invoke_agent"))
//!         .count();
//!     assert_eq!(roots, 2);
//!     Ok::<_, Box<dyn std::error::Error>>(())
//! })?;
//! # std::fs::remove_dir_all(directory.to_string())?;
//! # Ok(())
//! # }
//! ```

mod build;
mod clock;
mod fallback;
mod lablet;
pub mod telemetry;

pub mod config {
    //! The config: what a `Lablet` is built from, read from YAML or JSON.
    //!
    //! A key the config doesn't know is an error, and that's the whole
    //! policy: nothing is ignored, so nothing a config states is without
    //! effect.
    //!
    //! A path is taken as the config writes it. One that isn't absolute
    //! starts at the working directory, wherever the config's own file is,
    //! so a config read from text and the same config read from a file name
    //! the same files.

    pub use lablet_config::{
        Api, Applied, Builtin, BuiltinTool, CacheScope, Completion, Config, ConfigError, Context,
        Effort, Env, Format, KeyPath, McpLifetime, McpNames, McpResult, McpServer, Model, Otlp,
        OtlpProtocol, OutputCut, Place, Pricing, Prompt, Provider, RawConfig, Refusal,
        ResolvedBuiltin, ResolvedConfig, ResolvedModel, ResolvedTools, Run, Setting, SkillsMode,
        Substituted, Telemetry, TelemetryFile, Thinking, Tools, TranscriptFormat, schema,
    };
}

pub use build::{Builder, build, check};
pub use config::{
    Config, ConfigError, Format, Place, RawConfig, Refusal, ResolvedConfig, Substituted, schema,
};
pub use lablet::Lablet;
pub use lablet_documents::OutcomeDocument;
pub use lablet_model::{
    BlankTask, ConfigDigest, FinishedRun, IdError, RunId, RunLabels, RunOutcome, StopReason,
    ToolSpec,
};
pub use lablet_prepare::{BuildError, Checked, ErrorClass, OwnFile, Unsupported};
pub use lablet_run::FilterList;
pub use lablet_run_request::{CancelHandle, RunIdRefused, RunRequest};

/// The version of lablet, which a run's record names as
/// `gen_ai.agent.version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
